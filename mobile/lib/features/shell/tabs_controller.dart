import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/api/api_client.dart';
import '../../core/connection.dart';
import '../../core/polling/poller.dart';

/// One `.apptab` chip of `#tabbar`.
@immutable
class TabEntry {
  const TabEntry({required this.session, required this.label, this.closable = true});

  final String session;
  final String label;

  /// `⌂ local` cannot be closed.
  final bool closable;

  @override
  bool operator ==(Object other) =>
      other is TabEntry && other.session == session && other.label == label && other.closable == closable;

  @override
  int get hashCode => Object.hash(session, label, closable);
}

/// The desktop's tabs, in the web's order: `GET /tabs` (the desktop order),
/// `local` first, then the favourites of `GET /prefs` (`favorites`), then the
/// rest; a stable sort, like `ordered()` in app_coordinator_tabs.rs. Refreshed
/// every 5 s while visible (the web's `tabsPollTs` throttle). Notifies only
/// when the list changed.
class TabsController extends ChangeNotifier {
  TabsController(this._connection) {
    _poller = Poller(interval: interval, task: _tick);
  }

  static const Duration interval = Duration(seconds: 5);

  final Connection _connection;
  late final Poller _poller;
  bool _visible = false;
  List<TabEntry> _tabs = const <TabEntry>[];
  bool _loaded = false;

  List<TabEntry> get tabs => _tabs;
  bool get loaded => _loaded;

  void setVisible(bool visible) {
    if (_visible == visible) return;
    _visible = visible;
    if (visible) {
      _poller.start();
    } else {
      _poller.stop();
    }
  }

  /// Fetch now (after creating, recovering or closing a tab).
  void poke() => _poller.poke();

  TabEntry? find(String session) {
    for (final TabEntry t in _tabs) {
      if (t.session == session) return t;
    }
    return null;
  }

  Future<Duration?> _tick() async {
    final ComandosApi? api = _connection.api;
    if (api == null) return const Duration(seconds: 10);
    try {
      final List<Object?> results = await Future.wait(<Future<Object?>>[
        api.request('GET', '/tabs').then((ApiResponse r) => r.ok ? r.json : null),
        api
            .request('GET', '/prefs')
            .then((ApiResponse r) => r.ok ? r.json : null)
            .catchError((Object _) => null),
      ]);
      final Object? tabs = results[0];
      if (tabs is! List) return const Duration(seconds: 10);
      final Object? prefs = results[1];
      final Set<String> favs = <String>{
        if (prefs is Map && prefs['favorites'] is List)
          for (final Object? f in prefs['favorites'] as List<Object?>)
            if (f is String && f != 'local') f,
      };
      final List<TabEntry> next = orderTabs(tabs, favs);
      final bool first = !_loaded;
      _loaded = true;
      if (first || !listEquals(next, _tabs)) {
        _tabs = next;
        notifyListeners();
      }
      return null;
    } catch (_) {
      return const Duration(seconds: 10);
    }
  }

  /// Pure: the `/tabs` JSON and the favourites → chips in the web's order.
  static List<TabEntry> orderTabs(List<Object?> json, Set<String> favs) {
    final List<TabEntry> all = <TabEntry>[];
    final Set<String> seen = <String>{};
    for (final Object? t in json) {
      if (t is! Map) continue;
      final Object? s = t['session'];
      if (s is! String || s.isEmpty || !seen.add(s)) continue;
      final Object? l = t['label'];
      all.add(TabEntry(
        session: s,
        label: l is String && l.trim().isNotEmpty ? l : s,
        closable: s != 'local' && t['closable'] != false,
      ));
    }
    int rank(TabEntry t) => t.session == 'local' ? 0 : (favs.contains(t.session) ? 1 : 2);
    final List<(int, TabEntry)> indexed = <(int, TabEntry)>[for (int i = 0; i < all.length; i++) (i, all[i])];
    indexed.sort(((int, TabEntry) a, (int, TabEntry) b) {
      final int c = rank(a.$2).compareTo(rank(b.$2));
      return c != 0 ? c : a.$1.compareTo(b.$1);
    });
    return <TabEntry>[for (final (int, TabEntry) x in indexed) x.$2];
  }

  /// `POST /tab-close` (the «Cerrar pestaña» modal), then refresh.
  Future<bool> close(String session) async {
    final ComandosApi? api = _connection.api;
    if (api == null || session == 'local') return false;
    try {
      final ApiResponse r =
          await api.request('POST', '/tab-close', body: <String, Object?>{'session': session});
      if (!r.ok) return false;
      _tabs = <TabEntry>[
        for (final TabEntry t in _tabs)
          if (t.session != session) t,
      ];
      notifyListeners();
      poke();
      return true;
    } catch (_) {
      return false;
    }
  }

  @override
  void dispose() {
    _poller.stop();
    super.dispose();
  }
}
