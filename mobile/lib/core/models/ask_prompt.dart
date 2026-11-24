import 'json_read.dart';

/// What a row of an agent's menu does (server `kind`).
enum AskKind {
  /// A numbered option.
  option,

  /// «Type something»: the answer is text typed on it.
  free,

  /// The unnumbered row that ends a multiple choice («Submit»).
  submit,

  /// «Chat about this»: leave the menu and talk instead.
  chat;

  static AskKind named(String name) => switch (name) {
        'free' => AskKind.free,
        'submit' => AskKind.submit,
        'chat' => AskKind.chat,
        _ => AskKind.option,
      };
}

/// One row of an agent's menu
/// (`{"key":"1","label":"Yes","desc":[…],"on":true,"checked":null,"kind":"option"}`).
class AskOption {
  const AskOption({
    required this.key,
    required this.label,
    this.highlighted = false,
    this.desc = const <String>[],
    this.checked,
    this.kind = AskKind.option,
    this.row = -1,
  });

  /// «1», «2»…; empty for an unnumbered row.
  final String key;
  final String label;

  /// The terminal's cursor is on it.
  final bool highlighted;

  /// The lines under it (Claude's question menus).
  final List<String> desc;

  /// Multiple choice: ticked or not (null: not a checkbox).
  final bool? checked;
  final AskKind kind;

  /// Its position in the menu: what `/chat/answer` moves the cursor to.
  final int row;

  static AskOption? tryParse(Map<String, Object?> m, {int row = -1}) {
    final String key = readString(m, 'key');
    final AskKind kind = AskKind.named(readString(m, 'kind'));
    final bool numbered = RegExp(r'^[1-9][0-9]?$').hasMatch(key);
    if (!numbered && !(key.isEmpty && kind == AskKind.submit)) return null;
    final Object? d = m['desc'];
    final Object? c = m['checked'];
    return AskOption(
      key: key,
      label: readString(m, 'label'),
      highlighted: readBool(m, 'on'),
      desc: d is List ? <String>[for (final Object? s in d) if (s is String) s] : const <String>[],
      checked: c is bool ? c : null,
      kind: kind,
      row: row,
    );
  }
}

/// A permission prompt / question menu visible in the agent's pane.
class AskPrompt {
  const AskPrompt({
    required this.question,
    required this.options,
    this.detail = const <String>[],
    this.tabs = const <(String, bool)>[],
    this.multi = false,
    this.sig = '',
  });

  final String question;
  final List<String> detail;

  /// Every row, in order (numbered options, «Submit», «Chat about this»…).
  final List<AskOption> options;

  /// The question tabs of a multi-question menu: (label, answered).
  final List<(String, bool)> tabs;

  /// A multiple choice (rows with checkboxes and a «Submit» row).
  final bool multi;

  /// The server's identity of this menu (ignores the cursor and the ticks).
  final String sig;

  /// The rows a voice can name: numbered, and not free text.
  List<AskOption> get choices =>
      <AskOption>[for (final AskOption o in options) if (o.key.isNotEmpty && o.kind != AskKind.free) o];

  /// Stable identity used to tell "the same menu" from a new one.
  String get signature => sig.isNotEmpty
      ? sig
      : '$question\u0001${detail.join('\n')}\u0001${options.map((AskOption o) => '${o.key}:${o.label}').join('|')}';

  static AskPrompt? tryParse(Object? json) {
    final Map<String, Object?>? m = asMap(json);
    if (m == null) return null;
    final List<Map<String, Object?>> raw = readMapList(m, 'options');
    final List<AskOption> options = <AskOption>[
      for (final (int i, Map<String, Object?> o) in raw.indexed)
        if (AskOption.tryParse(o, row: i) case final AskOption opt) opt,
    ];
    if (options.where((AskOption o) => o.key.isNotEmpty).isEmpty) return null;
    final Object? d = m['detail'];
    return AskPrompt(
      question: readString(m, 'question'),
      detail: d is List ? <String>[for (final Object? s in d) if (s is String) s] : const <String>[],
      options: options,
      tabs: <(String, bool)>[
        for (final Map<String, Object?> t in readMapList(m, 'tabs')) (readString(t, 'label'), readBool(t, 'done')),
      ],
      multi: readBool(m, 'multi'),
      sig: readString(m, 'sig'),
    );
  }
}
