import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/api/api_client.dart';
import '../../core/api/api_errors.dart';
import '../../core/connection.dart';
import '../../core/models/session_feed.dart';
import '../../core/polling/poller.dart';
import '../../ui/format.dart';

/// Polls `/session-feed` every 2.5 s while visible. Keeps the previous
/// [SessionSummary] instance when a session's signature did not change, so
/// rows can be reused without rebuilding.
class SessionsController extends ChangeNotifier {
  SessionsController(this._connection) {
    _poller = Poller(interval: interval, task: _tick);
  }

  static const Duration interval = Duration(milliseconds: 2500);

  final Connection _connection;
  late final Poller _poller;

  SessionFeed? _feed;
  ApiException? _error;

  /// What the list showed after the last notify: an identical poll (same
  /// rows, same "hace N min" texts, same error) notifies nobody, so the
  /// screen schedules no frame at all.
  List<Object> _shown = const <Object>[];
  int _errors = 0;
  bool _visible = false;
  Completer<void>? _refreshing;

  SessionFeed? get feed => _feed;
  ApiException? get error => _error;
  bool get loadedOnce => _feed != null;

  /// Starts/stops polling. Polling runs only while visible.
  void setVisible(bool visible) {
    if (_visible == visible) return;
    _visible = visible;
    if (visible) {
      _poller.start();
    } else {
      _poller.stop();
    }
  }

  /// Pull to refresh: fetch now, complete when done.
  Future<void> refresh() {
    final Completer<void> c = _refreshing ??= Completer<void>();
    if (_poller.isRunning) {
      _poller.poke();
    } else {
      unawaited(_tick().then((_) {}));
    }
    return c.future;
  }

  /// Network came back.
  void poke() => _poller.poke();

  Future<Duration?> _tick() async {
    final ComandosApi? api = _connection.api;
    if (api == null) return const Duration(seconds: 10);
    Duration? next;
    try {
      final SessionFeed fresh = await api.sessionFeed();
      _feed = _reuse(_feed, fresh);
      _error = null;
      _errors = 0;
      _connection.tokenRejected.value = false;
    } on ApiException catch (e) {
      _error = e;
      _errors++;
      if (e is UnauthorizedException) _connection.tokenRejected.value = true;
      if (e is NetworkException || e is RequestTimeoutException) {
        next = const Duration(seconds: 10);
      } else if (e is NotFoundException) {
        next = const Duration(seconds: 60);
      } else {
        next = errorBackoff(_errors);
      }
    } catch (_) {
      _error = const BadResponseException();
      _errors++;
      next = errorBackoff(_errors);
    }
    final Completer<void>? r = _refreshing;
    _refreshing = null;
    r?.complete();
    final List<Object> shown = _viewKey();
    if (!_sameView(shown, _shown)) {
      _shown = shown;
      notifyListeners();
    }
    return next;
  }

  /// Identity of everything the list renders from this controller.
  List<Object> _viewKey() {
    final SessionFeed? f = _feed;
    return <Object>[
      _error?.runtimeType ?? '',
      _error?.userMessage ?? '',
      if (f != null)
        for (final SessionSummary s in f.sessions) ...<Object>[s, relativeTime(f.now, s.ts)],
    ];
  }

  static bool _sameView(List<Object> a, List<Object> b) {
    if (a.length != b.length) return false;
    for (int i = 0; i < a.length; i++) {
      final Object x = a[i];
      final Object y = b[i];
      if (x is SessionSummary ? !identical(x, y) : x != y) return false;
    }
    return true;
  }

  static SessionFeed _reuse(SessionFeed? old, SessionFeed fresh) {
    if (old == null) return fresh;
    final Map<String, SessionSummary> byId = <String, SessionSummary>{
      for (final SessionSummary s in old.sessions) s.session: s,
    };
    return SessionFeed(
      now: fresh.now,
      sessions: <SessionSummary>[
        for (final SessionSummary s in fresh.sessions)
          if (byId[s.session] case final SessionSummary prev when prev.signature == s.signature)
            prev
          else
            s,
      ],
    );
  }

  @override
  void dispose() {
    _poller.stop();
    super.dispose();
  }
}
