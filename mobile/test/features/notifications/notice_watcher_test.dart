import 'dart:async';

import 'package:comandos_mobile/core/api/api_errors.dart';
import 'package:comandos_mobile/features/notifications/notice.dart';
import 'package:comandos_mobile/features/notifications/notice_watcher.dart';
import 'package:flutter_test/flutter_test.dart';

class FakeCall {
  FakeCall(this.kind, {this.rev, this.after, this.wait});
  final String kind;
  final String? rev;
  final int? after;
  final int? wait;
  final Completer<Object> answer = Completer<Object>();

  void ok(Object v) => answer.complete(v);
  void fail(Object e) => answer.completeError(e);

  @override
  String toString() => kind == 'watch' ? 'watch($rev,$wait)' : 'list($after)';
}

class FakeTransport implements NoticeTransport {
  final List<FakeCall> calls = <FakeCall>[];
  FakeCall get last => calls.last;

  Future<T> _track<T>(FakeCall c, Future<void>? cancel) {
    calls.add(c);
    cancel?.then((_) {
      if (!c.answer.isCompleted) c.fail(const RequestTimeoutException());
    });
    return c.answer.future.then((Object v) => v as T);
  }

  @override
  Future<NoticeWatch> watch(String rev, {required int waitSec, Future<void>? cancel}) =>
      _track<NoticeWatch>(FakeCall('watch', rev: rev, wait: waitSec), cancel);

  @override
  Future<NoticePage> list(int after, {required int limit, Future<void>? cancel}) =>
      _track<NoticePage>(FakeCall('list', after: after), cancel);
}

class FakeTimer implements Timer {
  FakeTimer(this.delay, this.cb);
  final Duration delay;
  final void Function() cb;
  bool _active = true;
  @override
  void cancel() => _active = false;
  @override
  bool get isActive => _active;
  @override
  int get tick => 0;
}

class FakeTimers {
  final List<FakeTimer> all = <FakeTimer>[];
  List<FakeTimer> get active => all.where((FakeTimer t) => t.isActive).toList();
  Timer call(Duration d, void Function() cb) {
    final FakeTimer t = FakeTimer(d, cb);
    all.add(t);
    return t;
  }

  void fire() {
    final FakeTimer t = active.single;
    t.cancel();
    t.cb();
  }
}

Future<void> settle() async {
  for (int i = 0; i < 30; i++) {
    await Future<void>.delayed(Duration.zero);
  }
}

Notice _n(int seq) => Notice(
      eventId: 'e$seq',
      sequence: seq,
      kind: 'turn_completed',
      category: 'done',
      needsHuman: false,
      occurredAtMs: seq,
      sessionKey: 's',
    );

NoticeWatch w(String rev, int latest, [List<String> pending = const <String>[]]) =>
    NoticeWatch(rev: rev, latest: latest, pending: pending);

NoticePage page(List<int> seqs, int nextAfter) =>
    NoticePage(notices: <Notice>[for (final int s in seqs) _n(s)], nextAfter: nextAfter);

void main() {
  late FakeTransport t;
  late FakeTimers timers;
  late List<NoticeCursor> saved;
  late List<List<int>> got;
  late List<Set<String>> pendings;
  late int unauthorized;
  late int healthy;
  int nowMs = 0;

  NoticeWatcher make({NoticeCursor? cursor}) => NoticeWatcher(
        transport: t,
        cursor: cursor,
        saveCursor: (NoticeCursor c) async => saved.add(c),
        onNotices: (List<Notice> ns, Set<String> p) =>
            got.add(<int>[for (final Notice x in ns) x.sequence]),
        onPending: pendings.add,
        onUnauthorized: () => unauthorized++,
        onHealthy: () => healthy++,
        timer: timers.call,
        now: () => nowMs,
      );

  setUp(() {
    t = FakeTransport();
    timers = FakeTimers();
    saved = <NoticeCursor>[];
    got = <List<int>>[];
    pendings = <Set<String>>[];
    unauthorized = 0;
    healthy = 0;
    nowMs = 0;
  });

  test('first run only learns the position: no replay of old notices', () async {
    final NoticeWatcher x = make()..start();
    await settle();
    expect(x.state, WatcherState.syncing);
    expect(t.last.toString(), 'watch(,0)');
    t.last.ok(w('r1', 40, <String>['e40']));
    await settle();
    expect(saved.single.after, 40);
    expect(saved.single.rev, 'r1');
    expect(got, isEmpty);
    expect(pendings.single, <String>{'e40'});
    expect(t.last.toString(), 'watch(r1,25)');
    expect(x.state, WatcherState.waiting);
    x.stop();
  });

  test('a change drains new notices page by page and saves the cursor', () async {
    final NoticeWatcher x = make(cursor: const NoticeCursor(rev: 'r1', after: 10))..start();
    await settle();
    expect(t.last.toString(), 'watch(r1,25)'); // resumes, no sync
    nowMs = 3000;
    t.last.ok(w('r2', 70));
    await settle();
    expect(t.last.toString(), 'list(10)');
    t.last.ok(page(<int>[11, 12], 60)); // 50 events, 2 of them notices
    await settle();
    expect(t.last.toString(), 'list(60)');
    t.last.ok(page(<int>[70], 70));
    await settle();
    expect(got, <List<int>>[<int>[11, 12], <int>[70]]);
    expect(saved.last.rev, 'r2');
    expect(saved.last.after, 70);
    expect(t.last.toString(), 'watch(r2,25)');
    expect(healthy, 1);
    x.stop();
  });

  test('network failures back off 2, 5, 10, 30, 60, 60 s; nudge retries now', () async {
    final NoticeWatcher x = make(cursor: const NoticeCursor(rev: 'r1', after: 1))..start();
    final List<Duration> delays = <Duration>[];
    for (int i = 0; i < 6; i++) {
      await settle();
      t.last.fail(const NetworkException());
      await settle();
      expect(x.state, WatcherState.backoff);
      delays.add(timers.active.single.delay);
      timers.fire();
    }
    expect(delays.map((Duration d) => d.inSeconds), <int>[2, 5, 10, 30, 60, 60]);
    await settle();
    t.last.fail(const ServerException(status: 502));
    await settle();
    final int before = t.calls.length;
    x.nudge();
    await settle();
    expect(t.calls.length, before + 1);
    expect(timers.active, isEmpty);
    // A success resets the backoff.
    nowMs += 30000;
    t.last.ok(w('r1', 1));
    await settle();
    t.last.fail(const RequestTimeoutException());
    await settle();
    expect(timers.active.single.delay, const Duration(seconds: 2));
    x.stop();
    expect(timers.active, isEmpty);
  });

  test('401 stops for good and asks to pair again', () async {
    final NoticeWatcher x = make(cursor: const NoticeCursor(rev: 'r1', after: 1))..start();
    await settle();
    t.last.fail(const UnauthorizedException());
    await settle();
    expect(x.state, WatcherState.unauthorized);
    expect(x.isRunning, isFalse);
    expect(unauthorized, 1);
    expect(timers.active, isEmpty);
    final int n = t.calls.length;
    await settle();
    expect(t.calls.length, n);
  });

  test('an old server without /notices is retried every 5 minutes', () async {
    final NoticeWatcher x = make(cursor: const NoticeCursor(rev: 'r1', after: 1))..start();
    await settle();
    t.last.fail(const NotFoundException());
    await settle();
    expect(x.state, WatcherState.unsupported);
    expect(timers.active.single.delay, const Duration(minutes: 5));
    x.stop();
  });

  test('stop aborts the open long-poll and leaves nothing running', () async {
    final NoticeWatcher x = make(cursor: const NoticeCursor(rev: 'r1', after: 1))..start();
    await settle();
    final FakeCall open = t.last;
    x.stop();
    await settle();
    expect(open.answer.isCompleted, isTrue);
    expect(x.state, WatcherState.stopped);
    expect(t.calls, hasLength(1));
    expect(timers.active, isEmpty);
  });

  test('a long absence fetches only the most recent events', () async {
    final NoticeWatcher x = make(cursor: const NoticeCursor(rev: 'r1', after: 5))..start();
    await settle();
    nowMs = 5000;
    t.last.ok(w('r9', 1000));
    await settle();
    expect(t.last.toString(), 'list(${1000 - NoticeWatcher.maxCatchUp})');
    x.stop();
  });

  test('server history went backwards: follow it without replaying', () async {
    final NoticeWatcher x = make(cursor: const NoticeCursor(rev: 'r1', after: 50))..start();
    await settle();
    nowMs = 5000;
    t.last.ok(w('r0', 5));
    await settle();
    expect(saved.last.after, 5);
    expect(t.last.toString(), 'watch(r0,25)');
    expect(got, isEmpty);
    x.stop();
  });

  test('a failure mid-drain resumes from the last page (nothing lost or repeated)', () async {
    final NoticeWatcher x = make(cursor: const NoticeCursor(rev: 'r1', after: 10))..start();
    await settle();
    nowMs = 5000;
    t.last.ok(w('r2', 90));
    await settle();
    t.last.ok(page(<int>[20], 60));
    await settle();
    t.last.fail(const NetworkException()); // second page
    await settle();
    expect(saved.last.rev, 'r1'); // old rev: the next watch returns at once
    expect(saved.last.after, 60);
    timers.fire();
    await settle();
    expect(t.last.toString(), 'watch(r1,25)');
    nowMs = 9000;
    t.last.ok(w('r2', 90));
    await settle();
    expect(t.last.toString(), 'list(60)');
    t.last.ok(page(<int>[90], 90));
    await settle();
    expect(got, <List<int>>[<int>[20], <int>[90]]);
    x.stop();
  });

  test('a server that ignores wait does not make the loop spin', () async {
    final NoticeWatcher x = make(cursor: const NoticeCursor(rev: 'r1', after: 1))..start();
    await settle();
    t.last.ok(w('r1', 1)); // unchanged, instantly (now stays 0)
    await settle();
    expect(x.state, WatcherState.backoff);
    expect(timers.active.single.delay, NoticeWatcher.quickReturnPause);
    x.stop();
  });
}
