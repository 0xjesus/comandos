import 'dart:async';

import '../../core/api/api_client.dart';
import '../../core/api/api_errors.dart';
import '../../core/util/clock.dart';
import 'notice.dart';

/// Network side of the watcher. Throws [ApiException]s.
abstract class NoticeTransport {
  /// `GET /notices/watch?rev=&wait=`: returns when the revision differs
  /// from [rev] or after [waitSec] seconds.
  Future<NoticeWatch> watch(String rev, {required int waitSec, Future<void>? cancel});

  /// `GET /notices?after=&limit=`.
  Future<NoticePage> list(int after, {required int limit, Future<void>? cancel});
}

/// [NoticeTransport] over the paired desktop's API.
class ApiNoticeTransport implements NoticeTransport {
  ApiNoticeTransport(this.api);

  /// The current client (changes on re-pair), null when unpaired.
  final ComandosApi? Function() api;

  ComandosApi _client() => api() ?? (throw const NetworkException());

  @override
  Future<NoticeWatch> watch(String rev, {required int waitSec, Future<void>? cancel}) async {
    final ApiResponse r = await _client().requestOk('GET', '/notices/watch',
        query: <String, String>{'rev': rev, 'wait': '$waitSec'},
        timeout: waitSec == 0 ? ApiTimeouts.get : ApiTimeouts.longPoll,
        cancel: cancel);
    return NoticeWatch.tryParse(r.json) ?? (throw const BadResponseException());
  }

  @override
  Future<NoticePage> list(int after, {required int limit, Future<void>? cancel}) async {
    final ApiResponse r = await _client().requestOk('GET', '/notices',
        query: <String, String>{'after': '$after', 'limit': '$limit'}, cancel: cancel);
    return NoticePage.tryParse(r.json, after: after) ?? (throw const BadResponseException());
  }
}

/// Where the watcher is in its loop.
enum WatcherState {
  stopped,

  /// First run for this desktop: reading the current position (no replay).
  syncing,

  /// A long-poll is open.
  waiting,

  /// Waiting before retrying after a failure.
  backoff,

  /// The desktop rejected the token: stopped until paired again.
  unauthorized,

  /// The desktop has no `/notices` (old ComandOS): retrying rarely.
  unsupported,
}

/// Position in the desktop's notice stream. Persisted so a restart neither
/// replays old notices nor misses new ones.
class NoticeCursor {
  const NoticeCursor({required this.rev, required this.after});

  /// Last `rev` seen from `/notices/watch`.
  final String rev;

  /// Last event sequence already turned into notifications.
  final int after;
}

/// The long-poll loop: `GET /notices/watch?rev=` (25 s) → on change,
/// `GET /notices?after=` page by page → [onNotices] → cursor saved.
///
/// * First run (no cursor): only learns the current position, so pairing
///   never floods the phone with old notices.
/// * Long absence: only the last [maxCatchUp] events are fetched.
/// * Network trouble, 5xx, timeouts: backoff 2, 5, 10, 30, 60 s, then every
///   60 s; [nudge] (network back, app resumed) retries now. Progress is
///   kept page by page, so a failure mid-way resumes where it stopped.
/// * 401/403: stops and calls [onUnauthorized] (pair again).
/// * Server restart: just a network error and a retry; the rev changes and
///   the sequence continues, so nothing is lost or repeated. If the server's
///   history went backwards, the cursor follows it without replaying.
class NoticeWatcher {
  NoticeWatcher({
    required this.transport,
    required NoticeCursor? cursor,
    required this.saveCursor,
    required this.onNotices,
    required this.onPending,
    required this.onUnauthorized,
    this.onHealthy,
    TimerFactory timer = systemTimer,
    NowMs now = systemNowMs,
  })  : _cursor = cursor,
        _timer = timer,
        _now = now;

  static const int waitSec = 25;
  static const int pageSize = 50;
  static const int maxCatchUp = 100;
  static const List<Duration> backoff = <Duration>[
    Duration(seconds: 2),
    Duration(seconds: 5),
    Duration(seconds: 10),
    Duration(seconds: 30),
    Duration(seconds: 60),
  ];
  static const Duration unsupportedRetry = Duration(minutes: 5);

  /// Pause when the server answers an unchanged watch at once (it ignored
  /// `wait`), so the loop never spins.
  static const Duration quickReturnPause = Duration(seconds: 5);

  final NoticeTransport transport;
  final Future<void> Function(NoticeCursor cursor) saveCursor;

  /// New notices (oldest first) and the live pending request ids.
  final void Function(List<Notice> notices, Set<String> pending) onNotices;

  /// Live "needs a human" request ids after every round.
  final void Function(Set<String> pending) onPending;
  final void Function() onUnauthorized;

  /// A round reached the desktop (used to resume the outbox).
  final void Function()? onHealthy;
  final TimerFactory _timer;
  final NowMs _now;

  NoticeCursor? _cursor;
  WatcherState _state = WatcherState.stopped;
  int _errors = 0;
  int _generation = 0;
  Completer<void>? _cancel;
  Completer<void>? _wake;
  Timer? _sleepTimer;

  WatcherState get state => _state;
  NoticeCursor? get cursor => _cursor;
  bool get isRunning =>
      _state != WatcherState.stopped && _state != WatcherState.unauthorized;

  void start() {
    if (isRunning) return;
    final int gen = ++_generation;
    _errors = 0;
    _cancel = Completer<void>();
    _state = WatcherState.waiting;
    unawaited(_loop(gen));
  }

  /// Stops at once: the open long-poll is aborted, no timer stays armed.
  void stop() {
    _generation++;
    if (_state != WatcherState.unauthorized) _state = WatcherState.stopped;
    _endSleep();
    final Completer<void>? c = _cancel;
    _cancel = null;
    if (c != null && !c.isCompleted) c.complete();
  }

  /// Retry now if waiting out a backoff.
  void nudge() {
    if (_state == WatcherState.backoff || _state == WatcherState.unsupported) _endSleep();
  }

  void _endSleep() {
    _sleepTimer?.cancel();
    _sleepTimer = null;
    final Completer<void>? w = _wake;
    _wake = null;
    if (w != null && !w.isCompleted) w.complete();
  }

  Future<void> _sleep(Duration d) {
    _endSleep();
    final Completer<void> w = _wake = Completer<void>();
    _sleepTimer = _timer(d, () {
      _sleepTimer = null;
      if (!w.isCompleted) w.complete();
    });
    return w.future;
  }

  Future<void> _loop(int gen) async {
    bool alive() => gen == _generation;
    while (alive()) {
      final Future<void>? cancel = _cancel?.future;
      try {
        final NoticeCursor? cur = _cursor;
        if (cur == null) {
          _state = WatcherState.syncing;
          final NoticeWatch w = await transport.watch('', waitSec: 0, cancel: cancel);
          if (!alive()) return;
          _cursor = NoticeCursor(rev: w.rev, after: w.latest);
          await saveCursor(_cursor!);
          if (!alive()) return;
          _errors = 0;
          onHealthy?.call();
          onPending(w.pending.toSet());
          continue;
        }
        _state = WatcherState.waiting;
        final int started = _now();
        final NoticeWatch w = await transport.watch(cur.rev, waitSec: waitSec, cancel: cancel);
        if (!alive()) return;
        _errors = 0;
        onHealthy?.call();
        int after = cur.after;
        if (w.latest < after) {
          after = w.latest; // history went backwards: follow, never replay
        } else if (w.latest - after > maxCatchUp) {
          after = w.latest - maxCatchUp; // long absence: only the recent ones
        }
        while (after < w.latest) {
          final NoticePage page = await transport.list(after, limit: pageSize, cancel: cancel);
          if (!alive()) return;
          if (page.notices.isNotEmpty) onNotices(page.notices, page.pending.toSet());
          if (page.nextAfter <= after) break;
          after = page.nextAfter;
          // Keep progress (old rev: the next watch returns at once if a
          // failure interrupts the drain).
          _cursor = NoticeCursor(rev: cur.rev, after: after);
          await saveCursor(_cursor!);
          if (!alive()) return;
        }
        final bool changed = w.rev != cur.rev || after != cur.after;
        _cursor = NoticeCursor(rev: w.rev, after: after);
        if (changed) {
          await saveCursor(_cursor!);
          if (!alive()) return;
        }
        onPending(w.pending.toSet());
        if (!changed && _now() - started < 1000) {
          _state = WatcherState.backoff;
          await _sleep(quickReturnPause);
        }
      } on UnauthorizedException {
        if (!alive()) return;
        _unauthorized();
        return;
      } on HostForbiddenException {
        if (!alive()) return;
        _unauthorized();
        return;
      } on NotFoundException {
        if (!alive()) return;
        _state = WatcherState.unsupported;
        await _sleep(unsupportedRetry);
      } catch (_) {
        if (!alive()) return;
        _state = WatcherState.backoff;
        final Duration d = _errors < backoff.length ? backoff[_errors] : backoff.last;
        _errors++;
        await _sleep(d);
      }
    }
  }

  void _unauthorized() {
    _generation++;
    _state = WatcherState.unauthorized;
    _endSleep();
    onUnauthorized();
  }
}
