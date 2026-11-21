import 'json_read.dart';

// Pure Dart port of the «Consumos» cards of the side panel
// (crates/comandos-web-view/src/command_sidebar.rs: acc_limits, limits_html,
// limit_detail, token_text, hours, tone). Source: the `accounts` of
// `GET /analytics/week?offset=0&sidebar=1`.
//
// Owner rule: no money anywhere. `measured.costUsd` («costo · semana», «Costo
// semana») is never read, even though the server sends it.

/// One quota meter of an account («Semana», the model's weekly cap, «5 horas»).
class UsageLimit {
  const UsageLimit({required this.name, required this.used, this.left = '', this.reset = ''});

  final String name;

  /// Percent used, rounded half up and clamped to 0…100.
  final int used;

  /// «6d 11h» (time until the reset), or empty.
  final String left;

  /// «sáb 28 nov, 02:00» (when it resets), or empty.
  final String reset;

  /// The line under the meter.
  String get resetLine => left.isNotEmpty ? 'se reinicia en $left' : 'sin reinicio reportado';

  /// The detail row value: «6% · reinicia sáb 28 nov, 02:00».
  String get detail {
    final String r = reset.isNotEmpty
        ? 'reinicia $reset'
        : left.isNotEmpty
            ? 'se reinicia en $left'
            : 'sin reinicio reportado';
    return '$used% · $r';
  }

  /// Lit segments of the 20-segment bar.
  int get segments => (used / 5 + .5).floor();

  @override
  bool operator ==(Object other) =>
      other is UsageLimit && other.name == name && other.used == used && other.left == left && other.reset == reset;

  @override
  int get hashCode => Object.hash(name, used, left, reset);
}

/// Severity of a percentage: `bad` ≥ 90, `warn` ≥ 70.
enum UsageTone { normal, warn, bad }

UsageTone usageTone(num u) => u >= 90
    ? UsageTone.bad
    : u >= 70
        ? UsageTone.warn
        : UsageTone.normal;

/// One card.
class UsageAccount {
  const UsageAccount({
    required this.id,
    required this.provider,
    required this.cli,
    required this.alias,
    this.color = '',
    this.limits = const <UsageLimit>[],
    this.today = '—',
    this.week = '—',
    this.plan = '',
    this.sessions = '—',
    this.tokens = '—',
    this.models = '—',
  });

  /// Null for rows without a string `id` (the web skips them).
  static UsageAccount? fromJson(Map<String, Object?> a) {
    final Object? id = a['id'];
    if (id is! String) return null;
    final Map<String, Object?> measured = asMap(a['measured']) ?? const <String, Object?>{};
    final Object? models = measured['models'];
    final String modelText = models is List ? models.map(_txt).join(' · ') : '';
    return UsageAccount(
      id: id,
      provider: _txt(a['provider']),
      cli: _txt(a['cli']),
      alias: _txt(a['alias']),
      color: _txt(a['color']),
      limits: accountLimits(a),
      today: hours(a['hoy']),
      week: hours(a['sem']),
      plan: _yes(a['plan']) ? _txt(a['plan']) : '',
      sessions: _measured(measured['sessions']) == null ? '—' : numText(_measured(measured['sessions'])!),
      tokens: tokenText(measured['tokens']),
      models: modelText.isEmpty ? '—' : modelText,
    );
  }

  final String id;
  final String provider;
  final String cli;
  final String alias;

  /// `#rrggbb` (validated by [colorHex]).
  final String color;
  final List<UsageLimit> limits;

  /// «2.8 h · 12 ses» (or «—»).
  final String today;
  final String week;
  final String plan;

  /// Free cards («sin cuota»): sessions and tokens of the week.
  final String sessions;
  final String tokens;

  /// «claude-fable-5-1 · claude-opus-5-5» (detail of a free card).
  final String models;

  /// No quota reported: the «sin cuota» card.
  bool get free => limits.isEmpty;

  /// The highest meter (the big number); null on a free card.
  UsageLimit? get peak {
    UsageLimit? best;
    for (final UsageLimit l in limits) {
      if (best == null || l.used >= best.used) best = l;
    }
    return best;
  }

  /// The account colour when it is a valid hex colour (`#rgb`, `#rgba`,
  /// `#rrggbb`, `#rrggbbaa`), else null (the theme's brand).
  String? get colorHex {
    final String v = color;
    if (!v.startsWith('#') || !<int>[4, 5, 7, 9].contains(v.length)) return null;
    return RegExp(r'^#[0-9a-fA-F]+$').hasMatch(v) ? v : null;
  }

  /// Accessible summary: «Claude main: 6% usado» / «… consumo medido».
  String get semanticsLabel =>
      '$cli $alias: ${peak == null ? 'consumo medido' : '${peak!.used}% usado'}';

  /// Rows of the detail block (no money: «Costo semana» is left out).
  List<(String, String)> get details => <(String, String)>[
        for (final UsageLimit l in limits) (l.name, l.detail),
        if (limits.isEmpty) ('Modelo', models) else ...<(String, String)>[('Hoy', today), ('Semana', week)],
        if (plan.isNotEmpty) ('Plan', plan),
      ];

  /// Same visible content (used to skip repaints when a poll brings nothing new).
  String get signature =>
      '$id|$provider|$cli|$alias|$color|${limits.map((UsageLimit l) => '${l.name}:${l.used}:${l.left}:${l.reset}').join(',')}|$today|$week|$plan|$sessions|$tokens|$models';
}

/// `acc_limits(a)`.
List<UsageLimit> accountLimits(Map<String, Object?> a) {
  final List<(String, Object?, Object?, Object?)> raw = <(String, Object?, Object?, Object?)>[];
  if (a['week'] != null) raw.add(('Semana', a['week'], a['left'], a['reset']));
  final Object? model = a['model'];
  if (_yes(model) && model is Map) {
    final Map<String, Object?> m = model.cast<String, Object?>();
    raw.add((
      _txt(m['n']),
      m['v'],
      m['left'] ?? a['left'],
      m['reset'] ?? a['reset'],
    ));
  }
  if (a['h5'] != null) raw.add(('5 horas', a['h5'], a['h5Left'], a['h5Reset']));
  return <UsageLimit>[
    for (final (String n, Object? u, Object? left, Object? reset) in raw)
      if (u is num && u.isFinite)
        UsageLimit(
          name: n,
          used: (u + .5).floor().clamp(0, 100),
          left: _yes(left) ? _txt(left) : '',
          reset: _yes(reset) ? _txt(reset) : '',
        ),
  ];
}

/// `hours(d)`: «2.8 h · 12 ses», or «—».
String hours(Object? d) {
  if (d is! Map) return '—';
  final Object? h = d['h'];
  if (h is! num || !h.isFinite) return '—';
  final Object? ses = d['ses'];
  return '${h.toStringAsFixed(1)} h · ${ses == null ? '0' : _txt(ses)} ses';
}

/// `token_text(v)`: 6550342854 → «6.6B».
String tokenText(Object? v) {
  final double? n = _measured(v);
  if (n == null) return '—';
  for (final (double scale, String unit) in const <(double, String)>[(1e9, 'B'), (1e6, 'M'), (1000, 'K')]) {
    if (n >= scale) return '${(n / scale).toStringAsFixed(1)}$unit';
  }
  return numText(n);
}

/// JS-like number text: 6.0 → «6», 2.5 → «2.5».
String numText(num n) => n == n.truncate() ? n.toInt().toString() : n.toString();

/// Parses the `accounts` list of `/analytics/week?…&sidebar=1`.
List<UsageAccount>? parseUsageAccounts(Object? json) {
  final Map<String, Object?>? m = asMap(json);
  if (m == null) return null;
  final Object? list = m['accounts'];
  if (list is! List) return const <UsageAccount>[];
  return <UsageAccount>[
    for (final Object? a in list)
      if (asMap(a) != null)
        if (UsageAccount.fromJson(asMap(a)!) case final UsageAccount acc) acc,
  ];
}

double? _measured(Object? v) => v is num && v.isFinite && v >= 0 ? v.toDouble() : null;

String _txt(Object? v) => switch (v) {
      null => '',
      String s => s,
      num n => numText(n),
      _ => v.toString(),
    };

bool _yes(Object? v) => switch (v) {
      null => false,
      bool b => b,
      num n => n != 0,
      String s => s.isNotEmpty,
      _ => true,
    };
