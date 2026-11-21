import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../core/api/api_client.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';

/// «Salta a…» (`#sw-ov`, the web's Ctrl+K switcher opened by `#tab-open` and
/// `#btn-switch`): every live session of `GET /state?live=1` plus the closed tabs
/// of `GET /tab-history` («↺ …», «recuperar»), filtered as you type with the same
/// fuzzy score and order as foundation_navigation.rs. Completes with the chosen
/// session name or null.
Future<String?> showSessionSwitcher(BuildContext context) {
  return showGeneralDialog<String>(
    context: context,
    barrierDismissible: true,
    barrierLabel: 'Cerrar',
    barrierColor: const Color(0x8C04060A), // rgba(4,6,10,.55)
    transitionDuration: Duration.zero,
    pageBuilder: (BuildContext context, Animation<double> a, Animation<double> b) => const _Switcher(),
  );
}

/// One row of the list: a live session or a closed tab to recover.
@immutable
class SwitcherItem {
  const SwitcherItem({
    required this.session,
    required this.label,
    required this.hint,
    required this.color,
    this.recover,
  });

  final String session;
  final String label;
  final String hint;
  final Color color;

  /// The `/tab-history` row to `POST /recover-tab`, for closed tabs.
  final Map<String, Object?>? recover;
}

/// Fuzzy score of the web (`score_units`): each query character must appear in
/// order; the score adds the gaps, plus 1 % of the target length. Null = no match.
double? switcherScore(String query, String target) {
  final String q = query.toLowerCase();
  final String t = target.toLowerCase();
  int score = 0;
  int position = 0;
  for (final int rune in q.runes) {
    final String needle = String.fromCharCode(rune);
    if (position > t.length) return null;
    final int found = t.indexOf(needle, position);
    if (found < 0) return null;
    score += found - position;
    position = found + 1;
  }
  return score + t.length * 0.01;
}

bool _truthy(Object? v) => v != null && v != false && v != '' && v != 0;

String _str(Object? v) => v is String ? v : (v == null ? '' : '$v');

/// `quickVisible`: live, not a zombie, and either a named session, a tab, an agent
/// or something not idle.
bool _quickVisible(Map<String, Object?> it) {
  if (it['alive'] == false || _truthy(it['zombie'])) return false;
  final String status = _str(it['status']);
  final bool idle = status.isEmpty || status == 'idle';
  return !_str(it['session']).startsWith('term-') || _truthy(it['tabbed']) || _truthy(it['agent']) || !idle;
}

/// Pure: the rows of `/state?live=1` and `/tab-history` → the list for [query].
List<SwitcherItem> switcherItems(List<Object?> state, List<Object?> history, String query) {
  final String q = query.trim();
  final Set<String> seen = <String>{};
  final List<(double, double, int, SwitcherItem)> rows = <(double, double, int, SwitcherItem)>[];
  int n = 0;
  for (final Object? raw in state) {
    if (raw is! Map) continue;
    final Map<String, Object?> it = raw.cast<String, Object?>();
    final String session = _str(it['session']);
    if (!_quickVisible(it) || seen.contains(session)) continue;
    seen.add(session);
    final String project = _str(it['project']);
    final double? score = q.isEmpty ? 0 : switcherScore(q, '$project $session');
    if (score == null) continue;
    final String status = _str(it['status']);
    final double priority = switch (status) {
      'waiting' => 0,
      'done' => 1,
      'working' => 2,
      _ => 3,
    };
    final Color color = switch (status) {
      'waiting' => Orbita.waiting,
      'working' => Orbita.working,
      'done' => Orbita.done,
      _ => Orbita.faint,
    };
    rows.add((
      priority,
      score,
      n++,
      SwitcherItem(
        session: session,
        label: project.isEmpty ? session : project,
        hint: status == 'waiting' ? 'espera TU respuesta' : status,
        color: color,
      ),
    ));
  }
  for (final Object? raw in history) {
    if (raw is! Map) continue;
    final Map<String, Object?> rec = raw.cast<String, Object?>();
    final String session = _str(rec['session']);
    if (session.isEmpty || seen.contains(session)) continue;
    final String label = _str(rec['label']);
    final String cwd = _str(rec['cwd']);
    final double? score = q.isEmpty ? 0 : switcherScore(q, '$label $session $cwd');
    if (score == null) continue;
    rows.add((
      4,
      score,
      n++,
      SwitcherItem(
        session: session,
        label: '↺ ${label.isEmpty ? session : label}',
        hint: 'recuperar',
        color: Orbita.faint,
        recover: rec,
      ),
    ));
  }
  rows.sort(((double, double, int, SwitcherItem) a, (double, double, int, SwitcherItem) b) {
    final int p = a.$1.compareTo(b.$1);
    if (p != 0) return p;
    final int s = a.$2.compareTo(b.$2);
    return s != 0 ? s : a.$3.compareTo(b.$3);
  });
  final Iterable<SwitcherItem> items = rows.map(((double, double, int, SwitcherItem) r) => r.$4);
  return q.isEmpty ? items.toList() : items.take(12).toList();
}

class _Switcher extends StatefulWidget {
  const _Switcher();

  @override
  State<_Switcher> createState() => _SwitcherState();
}

class _SwitcherState extends State<_Switcher> {
  final TextEditingController _input = TextEditingController();
  final FocusNode _focus = FocusNode();
  List<Object?> _state = const <Object?>[];
  List<Object?> _history = const <Object?>[];
  List<SwitcherItem> _items = const <SwitcherItem>[];
  int _sel = 0;
  bool _going = false;

  @override
  void initState() {
    super.initState();
    _input.addListener(_fill);
    unawaited(_load());
  }

  @override
  void dispose() {
    _input.dispose();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    final ComandosApi? api = sl.maybe<Connection>()?.api;
    if (api == null) return;
    Future<List<Object?>> list(String path) async {
      try {
        final ApiResponse r = await api.request('GET', path);
        return r.ok && r.json is List ? r.json! as List<Object?> : const <Object?>[];
      } catch (_) {
        return const <Object?>[];
      }
    }

    final List<List<Object?>> r = await Future.wait(<Future<List<Object?>>>[
      list('/state?live=1'),
      list('/tab-history'),
    ]);
    if (!mounted) return;
    _state = r[0];
    _history = r[1];
    _fill();
  }

  void _fill() {
    setState(() {
      _items = switcherItems(_state, _history, _input.text);
      _sel = 0;
    });
  }

  Future<void> _go(int index) async {
    if (_going || index < 0 || index >= _items.length) return;
    final SwitcherItem it = _items[index];
    final NavigatorState nav = Navigator.of(context);
    if (it.recover == null) {
      nav.pop(it.session);
      return;
    }
    _going = true;
    String? session;
    try {
      final ApiResponse? r = await sl.maybe<Connection>()?.api?.request('POST', '/recover-tab', body: it.recover);
      final Object? s = r != null && r.ok ? r.map['session'] : null;
      if (s is String && s.isNotEmpty) session = s;
      if (mounted) {
        final String label = _str(r?.map['label']);
        showWebToast(context, session == null ? 'No se pudo recuperar' : '${label.isEmpty ? it.session : label} recuperada',
            error: session == null);
      }
    } catch (_) {
      if (mounted) showWebToast(context, 'No se pudo recuperar', error: true);
    }
    _going = false;
    if (mounted) nav.pop(session);
  }

  KeyEventResult _key(FocusNode node, KeyEvent e) {
    if (e is! KeyDownEvent) return KeyEventResult.ignored;
    if (e.logicalKey == LogicalKeyboardKey.arrowDown || e.logicalKey == LogicalKeyboardKey.arrowUp) {
      final int d = e.logicalKey == LogicalKeyboardKey.arrowDown ? 1 : -1;
      setState(() => _sel = (_sel + d).clamp(0, _items.isEmpty ? 0 : _items.length - 1));
      return KeyEventResult.handled;
    }
    if (e.logicalKey == LogicalKeyboardKey.escape) {
      Navigator.of(context).pop();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    final Size screen = MediaQuery.sizeOf(context);
    final double width = (screen.width * .92).clamp(0, 560).toDouble();
    final double top = screen.height * .12;
    return Material(
      type: MaterialType.transparency,
      child: Align(
        alignment: Alignment.topCenter,
        child: Padding(
          padding: EdgeInsets.only(top: top, bottom: 12 + MediaQuery.viewInsetsOf(context).bottom),
          child: Container(
            width: width,
            padding: const EdgeInsets.all(12),
            decoration: BoxDecoration(
              color: Orbita.panel2,
              borderRadius: BorderRadius.circular(12),
              border: Border.all(color: Orbita.line2),
              boxShadow: const <BoxShadow>[BoxShadow(color: Color(0x80000000), blurRadius: 60, offset: Offset(0, 18))],
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: <Widget>[
                Focus(
                  onKeyEvent: _key,
                  child: TextField(
                    key: const ValueKey<String>('sw-in'),
                    controller: _input,
                    focusNode: _focus,
                    autofocus: true,
                    autocorrect: false,
                    enableSuggestions: false,
                    textInputAction: TextInputAction.go,
                    onSubmitted: (_) => _go(_sel),
                    style: const TextStyle(fontFamily: Orbita.mono, fontSize: 13, color: Orbita.text),
                    cursorColor: Orbita.brand,
                    decoration: InputDecoration(
                      isDense: true,
                      filled: true,
                      fillColor: Orbita.bg,
                      hintText: 'Salta a...  (Enter va a la primera, ↑↓ navega, Esc cierra)',
                      hintMaxLines: 1,
                      hintStyle: const TextStyle(fontFamily: Orbita.mono, fontSize: 13, color: Orbita.faint),
                      contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
                      border: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(8), borderSide: const BorderSide(color: Orbita.line2)),
                      enabledBorder: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(8), borderSide: const BorderSide(color: Orbita.line2)),
                      focusedBorder: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(8), borderSide: const BorderSide(color: Orbita.brand)),
                    ),
                  ),
                ),
                const SizedBox(height: 8),
                Flexible(
                  child: ListView.builder(
                    shrinkWrap: true,
                    padding: EdgeInsets.zero,
                    itemCount: _items.length,
                    itemBuilder: (BuildContext context, int i) => _Row(
                      item: _items[i],
                      selected: i == _sel,
                      onTap: () => _go(i),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// `.sw-row`: dot, label, hint on the right; `.sel` = `--line` background.
class _Row extends StatelessWidget {
  const _Row({required this.item, required this.selected, required this.onTap});

  final SwitcherItem item;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
        decoration: BoxDecoration(
          color: selected ? Orbita.line : null,
          borderRadius: BorderRadius.circular(7),
        ),
        child: Row(
          children: <Widget>[
            Container(
              width: 8,
              height: 8,
              decoration: BoxDecoration(color: item.color, shape: BoxShape.circle),
            ),
            const SizedBox(width: 9),
            Expanded(
              child: Text(
                item.label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: Orbita.mono,
                  fontSize: 13,
                  color: selected ? Orbita.text : Orbita.dim,
                ),
              ),
            ),
            const SizedBox(width: 9),
            Text(item.hint, style: const TextStyle(fontFamily: Orbita.mono, fontSize: 11, color: Orbita.faint)),
          ],
        ),
      ),
    );
  }
}
