import 'dart:async';

import 'package:flutter/material.dart';

import '../../core/api/api_client.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/models/usage_limits.dart';
import '../../core/polling/app_visibility.dart';
import '../../core/polling/poller.dart';
import '../../core/util/clock.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import '../new_session/wizard_api.dart';
import 'usage_logos.dart';

// The «Consumos» tab of the side panel: the web's `.et-lim` block
// (crates/comandos-web-view/src/command_sidebar.rs `limits_html`, behaviour in
// crates/comandos-web/src/components/app_coordinator_sidebar.rs `limits`,
// look in dash/workspace.css `#command-sidebar .cs-lim …`, «Noche» values).
//
// Not copied on purpose: the OpenRouter balance block (`.cs-credits`) and the
// «costo · semana» / «Costo semana» figures. The owner wants no money shown.

/// `#command-sidebar` tokens of the default («Noche») theme.
abstract final class _Cs {
  static const Color panel = Color(0xFF171B24);
  static const Color panel2 = Color(0xFF121722);
  static const Color line = Color(0xFF2A303E);
  static const Color text = Color(0xFFEAF0FB);
  static const Color dim = Color(0xFF949DB0);
  static const Color brand = Color(0xFFAD9CFF);
  static const Color green = Color(0xFF8ACEAC);
  static const Color amber = Color(0xFFE6BA75);
  static const Color red = Color(0xFFE08A8A);

  /// `.cs-empty-terms { --faint: color-mix(--cs-dim 62%, --cs-panel2) }`.
  static final Color faint = webMix(dim, .62, panel2);
}

/// What the web keeps in `SB_LIMITS` (and localStorage): the last accounts and
/// when they were read. Shared by every [UsageSummary] while the app lives.
class UsageCache {
  UsageCache._();

  static List<UsageAccount>? accounts;

  /// When [accounts] were read (milliseconds since epoch).
  static int? at;

  /// Clock (tests drive it).
  static NowMs now = systemNowMs;

  /// Test hook.
  @visibleForTesting
  static void reset() {
    accounts = null;
    at = null;
    now = systemNowMs;
  }
}

/// The "Consumos" tab of the side panel.
class UsageSummary extends StatefulWidget {
  const UsageSummary({super.key});

  /// Re-read at most once a minute (`limits()`), 15 s after a failure.
  static const Duration refresh = Duration(seconds: 60);
  static const Duration retry = Duration(seconds: 15);

  @override
  State<UsageSummary> createState() => _UsageSummaryState();
}

class _UsageSummaryState extends State<UsageSummary> {
  late final Poller _poller = Poller(interval: UsageSummary.refresh, task: _tick);
  AppVisibility? _visibility;
  String _signature = '';
  String _openId = '';

  @override
  void initState() {
    super.initState();
    _signature = _sig(UsageCache.accounts);
    _visibility = sl.maybe<AppVisibility>();
    _visibility?.addListener(_onVisibility);
    if (_visibility?.value ?? true) _poller.start();
  }

  @override
  void dispose() {
    _visibility?.removeListener(_onVisibility);
    _poller.stop();
    super.dispose();
  }

  void _onVisibility() {
    if (_visibility!.value) {
      _poller.start();
    } else {
      _poller.stop();
    }
  }

  static String _sig(List<UsageAccount>? a) =>
      a == null ? '-' : '#${a.map((UsageAccount x) => x.signature).join('\n')}';

  Future<Duration?> _tick() async {
    final int? at = UsageCache.at;
    final int age = at == null ? 1 << 40 : UsageCache.now() - at;
    if (age < UsageSummary.refresh.inMilliseconds) {
      return Duration(milliseconds: UsageSummary.refresh.inMilliseconds - age);
    }
    final ComandosApi? api = sl.maybe<Connection>()?.api;
    if (api == null) return UsageSummary.retry;
    try {
      final Object? json = await api.getJson(
        '/analytics/week',
        query: const <String, String>{'offset': '0', 'sidebar': '1'},
      );
      final List<UsageAccount>? accounts = parseUsageAccounts(json);
      if (accounts == null) return UsageSummary.retry;
      UsageCache.accounts = accounts;
      UsageCache.at = UsageCache.now();
      final String sig = _sig(accounts);
      if (mounted && sig != _signature) {
        setState(() {
          _signature = sig;
          if (!accounts.any((UsageAccount a) => a.id == _openId)) _openId = '';
        });
      }
      return UsageSummary.refresh;
    } on Object {
      return UsageSummary.retry;
    }
  }

  @override
  Widget build(BuildContext context) {
    final List<UsageAccount>? accounts = UsageCache.accounts;
    final int count = accounts == null || accounts.isEmpty ? 1 : accounts.length;
    return ColoredBox(
      color: _Cs.panel2,
      child: ListView.builder(
        // Clear Android's navigation buttons (edge-to-edge).
        padding: EdgeInsets.only(bottom: 18 + MediaQuery.paddingOf(context).bottom),
        itemCount: 1 + count,
        itemBuilder: (BuildContext context, int index) {
          if (index == 0) return const _Head();
          if (accounts == null) return const _Wait('Leyendo el uso…');
          if (accounts.isEmpty) return const _Wait('Sin límites que mostrar');
          final int i = index - 1;
          final UsageAccount a = accounts[i];
          return Padding(
            key: ValueKey<String>(a.id),
            padding: EdgeInsets.fromLTRB(14, i == 0 ? 0 : 10, 14, 0),
            child: UsageCard(
              account: a,
              open: a.id == _openId,
              onToggle: () => setState(() => _openId = _openId == a.id ? '' : a.id),
            ),
          );
        },
      ),
    );
  }
}

/// `.et-h`: «USO DE TUS CUENTAS» … «% usado».
class _Head extends StatelessWidget {
  const _Head();

  @override
  Widget build(BuildContext context) => Padding(
        padding: const EdgeInsets.fromLTRB(16, 6, 16, 10),
        child: Row(
          children: <Widget>[
            const Text(
              'USO DE TUS CUENTAS',
              style: TextStyle(
                fontFamily: Orbita.mono,
                fontSize: 10.5,
                fontWeight: FontWeight.w600,
                letterSpacing: 1.68,
                height: 1,
                color: _Cs.dim,
              ),
            ),
            const Spacer(),
            Text(
              '% usado',
              style: TextStyle(fontFamily: Orbita.mono, fontSize: 10.5, height: 1, color: _Cs.faint),
            ),
          ],
        ),
      );
}

/// `.cs-limits .wait`.
class _Wait extends StatelessWidget {
  const _Wait(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        child: Text(text, style: const TextStyle(fontFamily: Orbita.mono, fontSize: 12, height: 1.4, color: _Cs.dim)),
      );
}

Color _hex(String? h) {
  if (h == null) return _Cs.brand;
  String s = h.substring(1);
  if (s.length == 3 || s.length == 4) s = s.split('').map((String c) => '$c$c').join();
  final int v = int.parse(s, radix: 16);
  return s.length == 6 ? Color(0xFF000000 | v) : Color(((v & 0xFF) << 24) | (v >> 8));
}

Color _toneColor(UsageTone t, Color normal) => switch (t) {
      UsageTone.bad => _Cs.red,
      UsageTone.warn => _Cs.amber,
      UsageTone.normal => normal,
    };

/// One `.card`: logo, CLI and alias, the peak %, a meter per quota; tapping
/// opens the detail rows (`.det`).
class UsageCard extends StatelessWidget {
  const UsageCard({super.key, required this.account, required this.open, required this.onToggle});

  final UsageAccount account;
  final bool open;
  final VoidCallback onToggle;

  static const TextStyle _mono = TextStyle(fontFamily: Orbita.mono, fontSize: 12, height: 1.4, color: _Cs.text);

  @override
  Widget build(BuildContext context) {
    final UsageAccount a = account;
    final Color ac = _hex(a.colorHex);
    final UsageLimit? peak = a.peak;
    final List<Widget> rows = <Widget>[
      _nameRow(ac, peak),
      if (peak != null)
        for (final UsageLimit l in a.limits) _Meter(limit: l, color: _toneColor(usageTone(l.used), ac))
      else
        _freeNumbers(),
    ];
    return Semantics(
      button: true,
      expanded: open,
      label: a.semanticsLabel,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onToggle,
        child: Container(
          padding: const EdgeInsets.all(12),
          decoration: BoxDecoration(
            color: _Cs.panel,
            borderRadius: BorderRadius.circular(10),
            border: Border.all(color: _Cs.line),
          ),
          child: DefaultTextStyle(
            style: _mono,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: <Widget>[
                for (int i = 0; i < rows.length; i++) ...<Widget>[if (i > 0) const SizedBox(height: 10), rows[i]],
                if (open) _Detail(rows: a.details),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _nameRow(Color ac, UsageLimit? peak) {
    final UsageAccount a = account;
    final Widget logo = Container(
      width: 22,
      height: 22,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: webMix(ac, .14, _Cs.panel2),
        borderRadius: BorderRadius.circular(6),
        border: Border.all(color: ac.withValues(alpha: .28)),
      ),
      child: hasUsageLogo(a.provider)
          ? UsageLogo(a.provider)
          : Text(
              a.cli.isEmpty ? '?' : String.fromCharCode(a.cli.runes.first),
              style: const TextStyle(fontFamily: Orbita.mono, fontSize: 11, fontWeight: FontWeight.w700, height: 1),
            ),
    );
    final Widget trailing = peak != null
        ? Text(
            '${peak.used}%',
            style: TextStyle(
              fontFamily: Orbita.mono,
              fontSize: 17,
              fontWeight: FontWeight.w600,
              height: 1,
              color: _toneColor(usageTone(peak.used), _Cs.text),
              fontFeatures: const <FontFeature>[FontFeature.tabularFigures()],
            ),
          )
        : Container(
            padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 3),
            decoration: BoxDecoration(
              color: _Cs.green.withValues(alpha: .14),
              borderRadius: BorderRadius.circular(5),
            ),
            child: const Text(
              'SIN CUOTA',
              style: TextStyle(
                fontFamily: Orbita.mono,
                fontSize: 9.5,
                fontWeight: FontWeight.w600,
                letterSpacing: .57,
                height: 1,
                color: _Cs.green,
              ),
            ),
          );
    return Row(
      children: <Widget>[
        logo,
        const SizedBox(width: 9),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              Text(
                a.cli,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(fontSize: 13, fontWeight: FontWeight.w600, height: 1.25),
              ),
              const SizedBox(height: 2),
              Text(
                a.alias,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(fontSize: 11, height: 1.3, color: _Cs.dim),
              ),
            ],
          ),
        ),
        const SizedBox(width: 9),
        trailing,
      ],
    );
  }

  /// `.free .fr`: sessions and tokens of the week (the cost cell is left out).
  Widget _freeNumbers() {
    Widget cell(String value, String label) => Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            Text(
              value,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(fontSize: 14, fontWeight: FontWeight.w600, height: 1.2),
            ),
            const SizedBox(height: 2),
            Text(label, style: const TextStyle(fontSize: 10.5, height: 1.3, color: _Cs.dim)),
          ],
        );
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        Expanded(child: cell(account.sessions, 'sesiones · semana')),
        const SizedBox(width: 12),
        Expanded(child: cell(account.tokens, 'tokens · semana')),
        const SizedBox(width: 12),
        const Expanded(child: SizedBox.shrink()),
      ],
    );
  }
}

/// `.meter`: name (62 px) · 20-segment bar · value (38 px), reset line below.
class _Meter extends StatelessWidget {
  const _Meter({required this.limit, required this.color});

  final UsageLimit limit;
  final Color color;

  @override
  Widget build(BuildContext context) {
    final UsageTone tone = usageTone(limit.used);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        Row(
          children: <Widget>[
            SizedBox(
              width: 62,
              child: Text(
                limit.name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(fontSize: 11, height: 1.3, color: _Cs.dim),
              ),
            ),
            const SizedBox(width: 8),
            Expanded(child: _Segments(lit: limit.segments, color: color)),
            const SizedBox(width: 8),
            SizedBox(
              width: 38,
              child: Text(
                '${limit.used}%',
                textAlign: TextAlign.right,
                style: TextStyle(
                  fontSize: 11,
                  fontWeight: FontWeight.w500,
                  height: 1.3,
                  color: _toneColor(tone, _Cs.text),
                  fontFeatures: const <FontFeature>[FontFeature.tabularFigures()],
                ),
              ),
            ),
          ],
        ),
        const SizedBox(height: 3),
        Padding(
          padding: const EdgeInsets.only(left: 70),
          child: Text(limit.resetLine, style: TextStyle(fontSize: 10.5, height: 1.3, color: _Cs.faint)),
        ),
      ],
    );
  }
}

/// `.seg`: 6 px track of 20 cells; lit cells take the meter colour.
class _Segments extends StatelessWidget {
  const _Segments({required this.lit, required this.color});

  final int lit;
  final Color color;

  @override
  Widget build(BuildContext context) => ClipRRect(
        borderRadius: BorderRadius.circular(3),
        child: SizedBox(
          height: 6,
          child: CustomPaint(painter: _SegPainter(lit, color)),
        ),
      );
}

class _SegPainter extends CustomPainter {
  _SegPainter(this.lit, this.color);

  final int lit;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = _Cs.line);
    if (lit <= 0) return;
    final double w = size.width / 20;
    // Lit cells plus their 1 px right shadow: one solid run.
    final double end = lit >= 20 ? size.width : w * lit + 1;
    canvas.drawRect(Rect.fromLTWH(0, 0, end, size.height), Paint()..color = color);
  }

  @override
  bool shouldRepaint(_SegPainter old) => old.lit != lit || old.color != color;
}

/// `.det`: the open card's key/value rows.
class _Detail extends StatelessWidget {
  const _Detail({required this.rows});

  final List<(String, String)> rows;

  @override
  Widget build(BuildContext context) => Container(
        margin: const EdgeInsets.only(top: 12),
        padding: const EdgeInsets.only(top: 12),
        decoration: const BoxDecoration(border: Border(top: BorderSide(color: _Cs.line))),
        child: Table(
          columnWidths: const <int, TableColumnWidth>{0: IntrinsicColumnWidth(), 1: FlexColumnWidth()},
          children: <TableRow>[
            for (int i = 0; i < rows.length; i++)
              TableRow(
                children: <Widget>[
                  Padding(
                    padding: EdgeInsets.only(right: 12, top: i == 0 ? 0 : 4),
                    child: Text(rows[i].$1, style: const TextStyle(fontSize: 11.5, height: 1.4, color: _Cs.dim)),
                  ),
                  Padding(
                    padding: EdgeInsets.only(top: i == 0 ? 0 : 4),
                    child: Text(
                      rows[i].$2,
                      style: const TextStyle(fontSize: 11, height: 1.4, fontWeight: FontWeight.w500, color: _Cs.text),
                    ),
                  ),
                ],
              ),
          ],
        ),
      );
}
