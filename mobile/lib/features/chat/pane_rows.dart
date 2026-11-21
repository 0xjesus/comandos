import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/api/api_client.dart';
import '../../core/connection.dart';

/// One live pane of a session, from `GET /state?live=1` (the rows the web page
/// polls as `S.list`; `chat_view.rs` `rows()`).
@immutable
class PaneRow {
  const PaneRow({
    required this.pane,
    this.agent = '',
    this.model = '',
    this.status = '',
    this.active = false,
  });

  final String pane;
  final String agent;
  final String model;

  /// `waiting` | `working` | `done` | `idle` (the dot colour of `.cv-pane i`).
  final String status;

  /// tmux's active pane (`paneActive`).
  final bool active;

  @override
  bool operator ==(Object other) =>
      other is PaneRow &&
      other.pane == pane &&
      other.agent == agent &&
      other.model == model &&
      other.status == status &&
      other.active == active;

  @override
  int get hashCode => Object.hash(pane, agent, model, status, active);
}

int paneNumber(String pane) => int.tryParse(pane.startsWith('%') ? pane.substring(1) : pane) ?? 1 << 62;

/// The live panes of [session] in tmux order, from the `/state?live=1` rows
/// (dead rows and duplicates dropped), like `chat_view.rs` `rows()`.
List<PaneRow> paneRowsFor(Object? json, String session) {
  final List<PaneRow> out = <PaneRow>[];
  if (json is! List) return out;
  for (final Object? it in json) {
    if (it is! Map) continue;
    final Object? s = it['session'];
    final Object? p = it['pane'];
    if (s != session || p is! String || p.isEmpty || it['alive'] != true) continue;
    if (out.any((PaneRow r) => r.pane == p)) continue;
    String str(String k) => it[k] is String ? it[k] as String : '';
    out.add(PaneRow(
      pane: p,
      agent: str('agent'),
      model: str('model'),
      status: str('status'),
      active: it['paneActive'] == true,
    ));
  }
  out.sort((PaneRow a, PaneRow b) => paneNumber(a.pane).compareTo(paneNumber(b.pane)));
  return out;
}

/// The pane the phone writes to and reads as chat (`chat_view.rs` `choose`):
/// the one saved for this session while it lives, else tmux's active one,
/// else the first. A saved pane that closed gives way to the next one
/// (`closed: true`: tell the person). No rows yet: keep what was saved.
({String pane, bool closed}) choosePane(List<PaneRow> rows, String saved) {
  if (rows.isEmpty || rows.any((PaneRow r) => r.pane == saved)) return (pane: saved, closed: false);
  if (saved.isNotEmpty) {
    final int after = paneNumber(saved);
    final PaneRow next = rows.firstWhere((PaneRow r) => paneNumber(r.pane) > after, orElse: () => rows.first);
    return (pane: next.pane, closed: true);
  }
  final PaneRow active = rows.firstWhere((PaneRow r) => r.active, orElse: () => rows.first);
  return (pane: active.pane, closed: false);
}

/// «Claude · opus-5.5», «Codex · gpt-6-astra», «Terminal» (`pane_label`).
String paneLabel(PaneRow row) {
  final String agent;
  switch (row.agent) {
    case '':
      return 'Terminal';
    case 'claude':
      agent = 'Claude';
    case 'codex':
      agent = 'Codex';
    default:
      agent = row.agent[0].toUpperCase() + row.agent.substring(1);
  }
  String model = row.model.startsWith('claude-') ? row.model.substring(7) : row.model;
  final List<String> parts = model.split('-');
  final RegExp digits = RegExp(r'^\d+$');
  if (parts.length >= 3 && digits.hasMatch(parts[parts.length - 2]) && digits.hasMatch(parts.last)) {
    model = '${parts.sublist(0, parts.length - 2).join('-')}-${parts[parts.length - 2]}.${parts.last}';
  }
  return model.isEmpty ? agent : '$agent · $model';
}

/// The composer's placeholder: «Escribe a 1 · Claude · fable-5.2…» on a
/// split, «Escribe a la sesión…» otherwise (`Chat::chips`).
String composerHint(List<PaneRow> rows, String pane) {
  if (rows.length > 1) {
    final int i = rows.indexWhere((PaneRow r) => r.pane == pane);
    if (i >= 0) return 'Escribe a ${i + 1} · ${paneLabel(rows[i])}…';
  }
  return 'Escribe a la sesión…';
}

/// Which pane of each session this phone targets, for the life of the app
/// (the web keeps `comandos.pane.<session>` in localStorage).
final Map<String, String> savedPanes = <String, String>{};

/// Polls the pane rows of one session while visible. The web page polls
/// `/state?live=1` every 2 s for everything it draws; here only the dock
/// needs them, so every 5 s (names, models and dots change rarely) plus
/// right away on open and on [refresh]. Notifies only on a real change.
class PaneTracker extends ChangeNotifier {
  PaneTracker({required this.session, required Connection connection, this.every = const Duration(seconds: 5)})
      : _connection = connection;

  final String session;
  final Connection _connection;
  final Duration every;

  List<PaneRow> _rows = const <PaneRow>[];
  bool _visible = false;
  bool _busy = false;
  bool _disposed = false;
  Timer? _timer;

  List<PaneRow> get rows => _rows;

  /// Feeds rows from elsewhere (tests, or a caller that already has them).
  void setRows(List<PaneRow> rows) {
    if (listEquals(rows, _rows)) return;
    _rows = List<PaneRow>.unmodifiable(rows);
    notifyListeners();
  }

  void setVisible(bool visible) {
    if (_visible == visible) return;
    _visible = visible;
    _timer?.cancel();
    if (visible) unawaited(refresh());
  }

  Future<void> refresh() async {
    if (_busy || _disposed) return;
    final ComandosApi? api = _connection.api;
    _timer?.cancel();
    if (api == null) return;
    _busy = true;
    try {
      final ApiResponse r = await api.request('GET', '/state', query: const <String, String>{'live': '1'});
      if (!_disposed && r.ok && r.json is List) setRows(paneRowsFor(r.json, session));
    } catch (_) {
      // Offline: keep the chips as they were; the next round retries.
    } finally {
      _busy = false;
      if (!_disposed && _visible) _timer = Timer(every, () => unawaited(refresh()));
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    super.dispose();
  }
}
