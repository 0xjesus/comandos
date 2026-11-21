import 'dart:io';

import 'package:comandos_mobile/core/api/api_errors.dart';
import 'package:comandos_mobile/core/config/credential_store.dart';
import 'package:comandos_mobile/core/config/paired_server.dart';
import 'package:comandos_mobile/core/connection.dart';
import 'package:comandos_mobile/core/platform/native_channel.dart';
import 'package:comandos_mobile/core/util/atomic_json_file.dart';
import 'package:comandos_mobile/features/notifications/notice.dart';
import 'package:comandos_mobile/features/notifications/notice_alerts.dart';
import 'package:comandos_mobile/features/notifications/notice_watcher.dart';
import 'package:comandos_mobile/features/notifications/notifications_controller.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

import 'notice_watcher_test.dart' show FakeCall, FakeTimers, FakeTransport, settle, w;

class FakePlatform implements NotificationPlatform {
  bool allowed = true;
  bool grant = true;
  bool keepAlive = false;
  int requests = 0;
  final List<LocalAlert> shown = <LocalAlert>[];
  final List<String> cancelled = <String>[];
  int cancelAlls = 0;
  ({String? session, String? pane})? tap;
  void Function()? tapCallback;

  @override
  Future<bool> notificationsAllowed() async => allowed;
  @override
  Future<bool> requestNotifications() async {
    requests++;
    allowed = grant;
    return grant;
  }

  bool lost = false;

  @override
  Future<bool> takeKeepAliveLost() async {
    final bool l = lost;
    lost = false;
    return l;
  }

  @override
  Future<bool> startKeepAlive() async => keepAlive = true;
  @override
  Future<void> stopKeepAlive() async => keepAlive = false;
  @override
  Future<void> show(LocalAlert alert) async => shown.add(alert);
  @override
  Future<void> cancel(String tag) async => cancelled.add(tag);
  @override
  Future<void> cancelAll() async => cancelAlls++;
  @override
  Future<({String? session, String? pane})?> takeTap() async {
    final ({String? session, String? pane})? t = tap;
    tap = null;
    return t;
  }

  @override
  set onTap(void Function()? callback) => tapCallback = callback;
  @override
  Future<void> openSettings({bool notifications = false}) async {}
}

/// Waits (real time: the controller saves to a real file) until the
/// watcher has an open request, then returns it.
Future<FakeCall> open(FakeTransport t) async {
  for (int i = 0; i < 400; i++) {
    if (t.calls.isNotEmpty && !t.last.answer.isCompleted) return t.last;
    await Future<void>.delayed(const Duration(milliseconds: 5));
  }
  throw StateError('no open request');
}

Future<void> until(bool Function() ok) async {
  for (int i = 0; i < 400 && !ok(); i++) {
    await Future<void>.delayed(const Duration(milliseconds: 5));
  }
}

void main() {
  late Directory dir;
  late FakePlatform platform;
  late FakeTransport transport;
  late FakeTimers timers;
  late Connection connection;
  late ValueNotifier<String?> visibleChat;
  late ValueNotifier<bool> uiReady;
  late List<String> opened;
  late NotificationsController c;

  NotificationsController make() => NotificationsController(
        platform: platform,
        connection: connection,
        file: () async => AtomicJsonFile('${dir.path}/notifications.json'),
        visibleChat: visibleChat,
        uiReady: uiReady,
        openChat: (String s, String? p) => opened.add('$s|$p'),
        transport: transport,
        timer: timers.call,
      );

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('notif');
    platform = FakePlatform();
    transport = FakeTransport();
    timers = FakeTimers();
    final MemoryCredentialStore creds = MemoryCredentialStore()
      ..value = const PairedServer(host: 'mi-equipo.mi-red.ts.net', token: 'TOKEN');
    connection = Connection(creds);
    await connection.load();
    visibleChat = ValueNotifier<String?>(null);
    uiReady = ValueNotifier<bool>(false);
    opened = <String>[];
    c = make();
  });

  tearDown(() async {
    c.dispose();
    await dir.delete(recursive: true);
  });

  test('off by default: pairing starts nothing', () async {
    await c.onPaired();
    expect(c.enabled, isFalse);
    expect(platform.keepAlive, isFalse);
    expect(transport.calls, isEmpty);
  });

  test('turning on asks Android first; denied keeps it off with a plain reason', () async {
    await c.onPaired();
    platform
      ..allowed = false
      ..grant = false;
    final String? problem = await c.setEnabled(true);
    expect(platform.requests, 1);
    expect(problem, contains('Sin permiso de notificaciones'));
    expect(c.enabled, isFalse);
    expect(platform.keepAlive, isFalse);
  });

  test('on: service + watcher; notices become notifications; the open chat is skipped', () async {
    await c.onPaired();
    expect(await c.setEnabled(true), isNull);
    expect(platform.keepAlive, isTrue);
    await settle();
    (await open(transport)).ok(w('r1', 10)); // first sync: no replay
    await settle();
    visibleChat.value = 'term-r2';
    (await open(transport)).ok(w('r2', 12, <String>['e11']));
    await settle();
    (await open(transport)).ok(NoticePage(notices: const <Notice>[
      Notice(eventId: 'e11', sequence: 11, kind: 'permission_requested', category: 'attention',
          needsHuman: true, occurredAtMs: 1, project: 'Pagos', sessionKey: 'term-r1', paneId: '%1'),
      Notice(eventId: 'e12', sequence: 12, kind: 'turn_completed', category: 'done',
          needsHuman: false, occurredAtMs: 2, project: 'Docs', sessionKey: 'term-r2', paneId: '%2'),
    ], nextAfter: 12, pending: <String>['e11']));
    await settle();
    expect(platform.shown.map((LocalAlert a) => a.title), <String>['Pagos te espera']);
    expect(platform.cancelled, contains(tagForSession('term-r2'))); // opening a chat clears it

    // The request was answered on the desktop: its notification goes away.
    (await open(transport)).ok(w('r3', 12));
    await until(() => platform.cancelled.contains(tagForSession('term-r1')));
    expect(platform.cancelled, contains(tagForSession('term-r1')));

    // State survives a restart: cursor and settings.
    c.dispose();
    c = make();
    await c.onPaired();
    expect(c.enabled, isTrue);
    expect((await open(transport)).toString(), 'watch(r3,25)');
  });

  test('token rejected: stops, tells the person, resumes after pairing again', () async {
    await c.onPaired();
    await c.setEnabled(true);
    await settle();
    (await open(transport)).ok(w('r1', 1));
    await settle();
    c.onForegroundChanged(false);
    (await open(transport)).fail(const UnauthorizedException());
    await settle();
    expect(connection.tokenRejected.value, isTrue);
    expect(platform.keepAlive, isFalse);
    expect(platform.shown.single.tag, tokenAlertTag);
    expect(c.watcherState, WatcherState.stopped);

    await connection.pair(const PairedServer(host: 'mi-equipo.mi-red.ts.net', token: 'NUEVO'));
    await c.onPaired();
    expect((await open(transport)).toString(), 'watch(r1,25)');
    expect(platform.keepAlive, isTrue);
    expect(platform.cancelled, contains(tokenAlertTag));
  });

  test('unpair stops everything and forgets the position', () async {
    await c.onPaired();
    await c.setEnabled(true);
    await settle();
    (await open(transport)).ok(w('r1', 5));
    await settle();
    await c.onUnpaired();
    await connection.unpair();
    expect(platform.keepAlive, isFalse);
    expect(platform.cancelAlls, greaterThan(0));
    final int n = transport.calls.length;
    await Future<void>.delayed(const Duration(milliseconds: 50));
    expect(transport.calls.length, n);
    expect(transport.last.answer.isCompleted, isTrue); // the long-poll was aborted
    expect(timers.active, isEmpty);
    // Pairing again starts from the desktop's current position.
    await connection.pair(const PairedServer(host: 'mi-equipo.mi-red.ts.net', token: 'TOKEN'));
    await c.onPaired();
    expect((await open(transport)).toString(), 'watch(,0)');
  });

  test('a notification tap opens that chat once the list is up', () async {
    await c.onPaired();
    platform.tap = (session: 'term-r7', pane: '%3');
    platform.tapCallback!();
    await settle();
    expect(opened, isEmpty); // navigation not ready yet: kept for later
    uiReady.value = true;
    await settle();
    expect(opened, <String>['term-r7|%3']);
  });

  test('keep-alive stopped by Android in the background: one notice on the next open', () async {
    final List<String> said = <String>[];
    await c.onPaired();
    await c.setEnabled(true);
    c.dispose();
    platform.lost = true; // WatchService could not start in the foreground
    c = NotificationsController(
      platform: platform,
      connection: connection,
      file: () async => AtomicJsonFile('${dir.path}/notifications.json'),
      visibleChat: visibleChat,
      uiReady: uiReady,
      openChat: (String s, String? p) {},
      transport: transport,
      timer: timers.call,
      onMessage: (String m, {bool openNotificationSettings = false}) => said.add(m),
    );
    await c.onPaired(); // app opened (boot): the list is not up yet
    expect(said, isEmpty);
    uiReady.value = true;
    expect(said, <String>[NotificationsController.keepAliveLostMessage]);
    c.onForegroundChanged(false);
    c.onForegroundChanged(true);
    await settle();
    expect(said, hasLength(1), reason: 'once');
    expect(platform.keepAlive, isTrue, reason: 'and the service is started again');
  });

  test('asks once after pairing; "Ahora no" is remembered', () async {
    int asked = 0;
    c.dispose();
    c = NotificationsController(
      platform: platform,
      connection: connection,
      file: () async => AtomicJsonFile('${dir.path}/notifications.json'),
      visibleChat: visibleChat,
      uiReady: uiReady,
      openChat: (String s, String? p) {},
      transport: transport,
      timer: timers.call,
      askUser: () async {
        asked++;
        return false;
      },
    );
    await c.onPaired();
    timers.fire();
    await until(() => asked == 1 && c.asked);
    expect(asked, 1);
    expect(c.asked, isTrue);
    expect(c.enabled, isFalse);
    c.onForegroundChanged(true);
    await c.onPaired();
    expect(timers.active, isEmpty);
    expect(asked, 1);
  });
}
