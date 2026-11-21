import 'dart:async';

import 'dart:convert';
import 'dart:io';

import 'package:comandos_mobile/app_navigator.dart';
import 'package:comandos_mobile/core/api/api_client.dart';
import 'package:comandos_mobile/core/config/credential_store.dart';
import 'package:comandos_mobile/core/config/paired_server.dart';
import 'package:comandos_mobile/core/connection.dart';
import 'package:comandos_mobile/core/error_reporter.dart';
import 'package:comandos_mobile/core/locator.dart';
import 'package:comandos_mobile/core/outbox/drafts.dart';
import 'package:comandos_mobile/core/outbox/message_transport.dart';
import 'package:comandos_mobile/core/outbox/outbox.dart';
import 'package:comandos_mobile/core/outbox/outbox_store.dart';
import 'package:comandos_mobile/core/polling/app_visibility.dart';
import 'package:comandos_mobile/features/car/car_mode.dart';
import 'package:comandos_mobile/features/chat/chat_screen.dart';
import 'package:comandos_mobile/features/notifications/notifications_hook.dart';
import 'package:comandos_mobile/features/sessions/session_row.dart';
import 'package:comandos_mobile/features/sessions/sessions_screen.dart';
import 'package:comandos_mobile/features/voice/voice.dart';
import 'package:comandos_mobile/ui/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/widgets.dart' as widgets show debugOnRebuildDirtyWidget;
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

// Synthetic data only.

http.Response _json(Object body) => http.Response.bytes(utf8.encode(jsonEncode(body)), 200,
    headers: <String, String>{'content-type': 'application/json; charset=utf-8'});

class CountingVisibility extends AppVisibility {
  int listeners = 0;
  @override
  void addListener(VoidCallback l) {
    listeners++;
    super.addListener(l);
  }

  @override
  void removeListener(VoidCallback l) {
    listeners--;
    super.removeListener(l);
  }
}

class CountingOutbox extends Outbox {
  CountingOutbox({required super.store, required super.transport, super.timer});
  int listeners = 0;
  @override
  void addListener(VoidCallback l) {
    listeners++;
    super.addListener(l);
  }

  @override
  void removeListener(VoidCallback l) {
    listeners--;
    super.removeListener(l);
  }
}

class _NoTimer implements Timer {
  @override
  void cancel() {}
  @override
  bool get isActive => false;
  @override
  int get tick => 0;
}

void main() {
  late Directory dir;
  late CountingVisibility visibility;
  late CountingOutbox outbox;
  double now = 1000;
  String reply = 'Listo';
  int transcripts = 0;

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('rebuild');
    sl.reset();
    now = 1000;
    reply = 'Listo';
    transcripts = 0;
    final MockClient client = MockClient((http.Request r) async {
      switch (r.url.path) {
        case '/session-feed':
          now += 2.5;
          return _json(<String, Object?>{
            'now': now,
            'sessions': <Object?>[
              for (int i = 0; i < 5; i++)
                <String, Object?>{
                  'session': 'term-r$i', 'label': 'Proyecto $i', 'state': 'idle',
                  'unread': false, 'ts': 100.0, 'pane': '%$i', 'agent': 'claude',
                  'reply': i == 2 ? reply : 'Listo',
                },
            ],
          });
        case '/chat/transcript':
          transcripts++;
          final Map<String, Object?> body = jsonDecode(r.body) as Map<String, Object?>;
          if (body['since'] == 't1') {
            return _json(<String, Object?>{'ok': true, 'unchanged': true, 'token': 't1', 'age': 500});
          }
          return _json(<String, Object?>{
            'ok': true, 'agent': 'claude', 'pane': '%1', 'token': 't1', 'age': 500,
            'messages': <Object?>[
              for (int i = 0; i < 30; i++)
                <String, Object?>{'r': i.isEven ? 'u' : 'a', 't': 'mensaje $i con **negrita**'},
            ],
          });
        case '/session-seen':
          return _json(<String, Object?>{'ok': true});
      }
      return http.Response('{"error":"Not found"}', 404);
    });
    final MemoryCredentialStore creds = MemoryCredentialStore()
      ..value = const PairedServer(host: 'mi-equipo.mi-red.ts.net', token: 'TOKEN');
    final Connection connection =
        Connection(creds, apiFactory: (PairedServer s) => ComandosApi(s, client: client));
    await connection.load();
    outbox = CountingOutbox(
      store: MemoryOutboxStore(),
      transport: ApiMessageTransport(() => connection.api),
      timer: (Duration d, void Function() cb) => _NoTimer(),
    );
    await outbox.load();
    visibility = CountingVisibility();
    sl
      ..register<ErrorReporter>(ErrorReporter())
      ..register<AppVisibility>(visibility)
      ..register<Connection>(connection)
      ..register<Outbox>(outbox)
      ..register<DraftStore>(DraftStore('${dir.path}/drafts.json'))
      ..register<NotificationsHook>(NoNotifications());
    registerVoice();
    registerCarMode();
  });

  tearDown(() async {
    widgets.debugOnRebuildDirtyWidget = null;
    sl.reset();
    await dir.delete(recursive: true);
  });

  Future<void> app(WidgetTester tester) => tester.pumpWidget(MaterialApp(
        theme: buildTheme(),
        navigatorKey: AppNavigator.key,
        navigatorObservers: <NavigatorObserver>[AppNavigator.routes],
        home: const SessionsScreen(),
      ));

  testWidgets('sessions: an unchanged poll rebuilds nothing; a change rebuilds one row',
      (WidgetTester tester) async {
    await app(tester);
    await tester.pump();
    await tester.pump();
    expect(find.byType(SessionRow), findsNWidgets(5));

    int total = 0;
    int rows = 0;
    widgets.debugOnRebuildDirtyWidget = (Element e, bool _) {
      total++;
      if (e.widget is SessionRow) rows++;
    };
    // Two polls (2.5 s each) with the same content; only `now` moved.
    await tester.pump(const Duration(milliseconds: 2600));
    await tester.pump(const Duration(milliseconds: 2600));
    expect(total, 0, reason: 'an identical poll must not rebuild any widget');

    reply = 'Otra respuesta';
    await tester.pump(const Duration(milliseconds: 2600));
    await tester.pump();
    expect(rows, 1, reason: 'only the session that changed rebuilds its row');

    widgets.debugOnRebuildDirtyWidget = null;
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(seconds: 3));
  });

  testWidgets('chat: an unchanged transcript poll rebuilds nothing', (WidgetTester tester) async {
    await app(tester);
    await tester.pump();
    AppNavigator.openChat('term-r1', pane: '%1');
    await tester.pumpAndSettle();
    await tester.pump();
    expect(find.text('mensaje 29 con negrita'), findsOneWidget);

    int total = 0;
    widgets.debugOnRebuildDirtyWidget = (Element e, bool _) => total++;
    final int before = transcripts;
    await tester.pump(const Duration(milliseconds: 2100));
    await tester.pump(const Duration(milliseconds: 2100));
    expect(transcripts, greaterThan(before));
    expect(total, 0, reason: 'an "unchanged" transcript must not rebuild any widget');

    widgets.debugOnRebuildDirtyWidget = null;
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(seconds: 3));
  });

  testWidgets('opening and closing a chat 25 times releases every listener and timer',
      (WidgetTester tester) async {
    await app(tester);
    await tester.pump();
    await tester.pump();
    final int visBase = visibility.listeners;
    final int outboxBase = outbox.listeners;

    for (int i = 0; i < 25; i++) {
      AppNavigator.openChat('term-r${i % 5}', pane: '%${i % 5}');
      await tester.pumpAndSettle();
      await tester.pump(const Duration(milliseconds: 2100)); // one transcript poll
      expect(find.byType(ChatScreen), findsOneWidget);
      expect(AppNavigator.visibleChat.value, 'term-r${i % 5}');
      expect(visibility.listeners, greaterThan(visBase));
      AppNavigator.key.currentState!.pop();
      await tester.pumpAndSettle();
      expect(find.byType(ChatScreen), findsNothing);
      expect(visibility.listeners, visBase, reason: 'chat #$i left an AppVisibility listener');
      expect(outbox.listeners, outboxBase, reason: 'chat #$i left an Outbox listener');
      expect(AppNavigator.visibleChat.value, isNull);
    }
    // Back on the list: its own poller keeps one timer; nothing from chats.
    await tester.pumpWidget(const SizedBox());
    expect(visibility.listeners, 0);
    expect(outbox.listeners, 0);
    await tester.pump(const Duration(seconds: 3));
    // flutter_test fails the test if any timer is still pending here.
  });
}
