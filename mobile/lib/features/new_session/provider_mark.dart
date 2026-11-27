import 'package:flutter/widgets.dart';

import '../../ui/theme.dart';
import '../../ui/web_kit.dart';

// `provider_mark(agent, model, size)` of foundation_helpers_web.rs: the logo of
// the provider whose pattern (from `/model-tiers` → `providers`) matches
// "<agent> <model>", or a two-letter mono badge ("LA", "OP", "TE").

/// One `providers[]` rule of `/model-tiers`.
class ProviderRule {
  const ProviderRule(this.match, this.icon);

  final String match;
  final String icon;
}

abstract final class ProviderMarks {
  /// The rules shipped today in `model_tiers.json`; replaced by the server's
  /// own list once `/model-tiers` answers ([setRules]).
  static const List<ProviderRule> defaults = <ProviderRule>[
    ProviderRule('grok|xai', 'xai'),
    ProviderRule('claude|anthropic|fable|mythos|opus|sonnet|haiku', 'anthropic'),
    ProviderRule(r'codex|openai|^gpt|^o[0-9]', 'openai'),
    ProviderRule('gemini|agy|google', 'gemini'),
    ProviderRule('glm|zhipu|zai', 'glm'),
    ProviderRule('opencode', 'opencode'),
    ProviderRule('omp|oh-my-pi|refuseless', 'omp'),
  ];

  static List<(RegExp, String)> _rules = _compile(defaults);

  static List<(RegExp, String)> _compile(List<ProviderRule> rules) => <(RegExp, String)>[
        for (final ProviderRule r in rules)
          if (_regex(r.match) case final RegExp re) (re, r.icon),
      ];

  static RegExp? _regex(String p) {
    try {
      return RegExp(p, caseSensitive: false);
    } on FormatException {
      return null;
    }
  }

  /// Reads `providers` of a `/model-tiers` answer; ignored when absent.
  static void setRules(Object? modelTiers) {
    if (modelTiers is! Map) return;
    final Object? list = modelTiers['providers'];
    if (list is! List) return;
    final List<ProviderRule> rules = <ProviderRule>[
      for (final Object? p in list)
        if (p is Map && p['match'] is String && p['icon'] is String)
          ProviderRule(p['match'] as String, p['icon'] as String),
    ];
    if (rules.isNotEmpty) _rules = _compile(rules);
  }

  /// Restores [defaults] (tests).
  static void reset() => _rules = _compile(defaults);

  /// The `foundation_icons.json` glyphs a provider can use.
  static const WebIconData xai = WebIconData(<String>[
    'pg:226.83 411.15 501.31 803.15 623.31 803.15 348.82 411.15',
    'pg:348.72 628.87 226.69 803.15 348.77 803.15 409.76 716.05',
    'pg:651.23 196.85 440.28 498.12 501.32 585.29 773.31 196.85',
    'pg:673.31 383.25 673.31 803.15 773.31 803.15 773.31 240.44',
  ], fill: true, box: 1000);

  /// A one-colour OpenCode mark (the frame of its logo).
  static const WebIconData opencode = WebIconData(<String>['r:6.5 4.5 11 15 1', 'r:10 11 4 5'], stroke: 2.2);

  /// Chips of the wizard: the line glyph of an agent the provider rules leave
  /// as letters (ComandOS ACP, OpenCode, Shell).
  static WebIconData? chipGlyph(String icon) => switch (icon) {
        'opencode' => opencode,
        'layers' => WebIcons.layers,
        'terminal' => WebIcons.terminal,
        _ => null,
      };

  static WebIconData? glyph(String icon) => switch (icon) {
        'xai' || 'grok' => xai,
        _ => WebIcons.byName[icon],
      };

  /// Agents with a glyph of their own even when the server's rules (from
  /// `/model-tiers`) predate them.
  static const Map<String, String> _own = <String, String>{'omp': 'omp', 'oh-my-pi': 'omp'};

  /// The glyph for [agent]/[model], or null when a mono badge is due.
  static WebIconData? resolve(String agent, String model) {
    final String probe = '$agent $model';
    final String? own = _own[agent];
    for (final (RegExp re, String icon) in _rules) {
      // A provider without a glyph (GLM) keeps the agent's own mark.
      if (re.hasMatch(probe)) return glyph(icon) ?? (own == null ? null : glyph(own));
    }
    return own == null ? null : glyph(own);
  }

  /// `short_model(input)` first two UTF-16 units, upper-cased.
  static String mono(String agent, String model) {
    final String input = agent.isNotEmpty ? agent : model;
    final String short = input.startsWith('claude-') ? input.substring(7) : input;
    final String two = short.length > 2 ? short.substring(0, 2) : short;
    return two.toUpperCase();
  }
}

/// `.pmark`: the provider glyph at [size], or `.pmono` letters.
class ProviderMark extends StatelessWidget {
  const ProviderMark({
    super.key,
    required this.agent,
    this.model = '',
    this.size = 13,
    this.color,
    this.chip = false,
  });

  final String agent;
  final String model;
  final double size;
  final Color? color;

  /// In a wizard chip: [ProviderMarks.chipGlyph] before the letters.
  final bool chip;

  @override
  Widget build(BuildContext context) {
    final WebIconData? g =
        ProviderMarks.resolve(agent, model) ?? (chip ? ProviderMarks.chipGlyph(agent) : null);
    if (g != null) return WebIcon(g, size: size, color: color);
    final String m = ProviderMarks.mono(agent, model);
    if (m.isEmpty) return const SizedBox.shrink();
    return Text(
      m,
      style: TextStyle(
        fontFamily: Orbita.mono,
        fontSize: 10,
        fontWeight: FontWeight.w700,
        letterSpacing: .4,
        height: 1,
        color: color,
      ),
    );
  }
}
