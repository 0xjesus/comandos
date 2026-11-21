import 'json_read.dart';

/// One option of a permission menu (`{"key":"1","label":"Yes","on":true}`).
class AskOption {
  const AskOption({required this.key, required this.label, this.highlighted = false});

  final String key;
  final String label;
  final bool highlighted;

  static AskOption? tryParse(Map<String, Object?> m) {
    final String key = readString(m, 'key');
    if (!RegExp(r'^[1-9]$').hasMatch(key)) return null;
    return AskOption(key: key, label: readString(m, 'label'), highlighted: readBool(m, 'on'));
  }
}

/// A permission prompt / numbered menu visible in the agent's pane.
class AskPrompt {
  const AskPrompt({required this.question, required this.options, this.detail = const <String>[]});

  final String question;
  final List<String> detail;
  final List<AskOption> options;

  /// Stable identity used to tell "the same menu" from a new one.
  String get signature =>
      '$question\u0001${detail.join('\n')}\u0001${options.map((AskOption o) => '${o.key}:${o.label}').join('|')}';

  static AskPrompt? tryParse(Object? json) {
    final Map<String, Object?>? m = asMap(json);
    if (m == null) return null;
    final List<AskOption> options = <AskOption>[
      for (final Map<String, Object?> o in readMapList(m, 'options'))
        if (AskOption.tryParse(o) case final AskOption opt) opt,
    ];
    if (options.isEmpty) return null;
    final Object? d = m['detail'];
    return AskPrompt(
      question: readString(m, 'question'),
      detail: d is List ? <String>[for (final Object? s in d) if (s is String) s] : const <String>[],
      options: options,
    );
  }
}
