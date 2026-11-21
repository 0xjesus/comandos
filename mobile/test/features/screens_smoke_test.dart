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
import 'package:comandos_mobile/core/outbox/outbox_item.dart';
import 'package:comandos_mobile/core/outbox/outbox_store.dart';
import 'package:comandos_mobile/core/polling/app_visibility.dart';
import 'package:comandos_mobile/features/car/car_mode.dart';
import 'package:comandos_mobile/features/chat/chat_screen.dart';
import 'package:comandos_mobile/features/notifications/notifications_controller.dart';
import 'package:comandos_mobile/features/notifications/notifications_hook.dart';
import 'package:comandos_mobile/features/voice/voice.dart';
import 'package:comandos_mobile/features/voice/voice_services.dart';
import 'package:comandos_mobile/ui/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import '../voice/voice_fakes.dart';

const PairedServer server = PairedServer(host: 'mi-equipo.mi-red.ts.net', token: 'TOKEN');

http.Response json(Object body, [int status = 200]) => http.Response.bytes(
      utf8.encode(jsonEncode(body)),
      status,
      headers: <String, String>{'content-type': 'application/json; charset=utf-8'},
    );

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
  final List<String> posted = <String>[];
  bool askOpen = true;

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('smoke');
    posted.clear();
    askOpen = true;
    sl.reset();
    final MockClient client = MockClient((http.Request r) async {
      expect(r.headers['X-Comandos-Token'], 'TOKEN');
      switch (r.url.path) {
        case '/session-feed':
          return json(
            <String, Object?>{
              'now': 1000.0,
              'sessions': <Object?>[
                <String, Object?>{
                  'session': 'term-r1', 'label': 'Pagos', 'state': 'ask', 'unread': true,
                  'ts': 990.0, 'pane': '%1', 'agent': 'claude', 'model': 'opus-5',
                  'reply': 'Necesito permiso',
                  'ask': <String, Object?>{
                    'question': '¿Seguir?',
                    'options': <Object?>[<String, Object?>{'key': '1', 'label': 'Sí'}],
                  },
                },
                <String, Object?>{'session': 'term-r2', 'label': 'Docs', 'state': 'idle', 'unread': false, 'ts': 100.0},
              ],
            },
          );
        case '/chat/transcript':
          return json(
            <String, Object?>{
              'ok': true, 'agent': 'claude', 'pane': '%1', 'token': 't1', 'age': 500,
              'messages': <Object?>[
                <String, Object?>{'r': 'u', 't': 'revisa las reglas'},
                <String, Object?>{'r': 'a', 't': 'Listo. **Todo** bien con `cargo`.\n\n```\ncode\n```'},
                <String, Object?>{'r': 't', 't': 'Bash · ls\nRead · /a.rs'},
              ],
              if (askOpen)
                'ask': <String, Object?>{
                  'question': '¿Seguir?',
                  'detail': <Object?>['Bash command'],
                  'options': <Object?>[<String, Object?>{'key': '1', 'label': 'Sí', 'on': true}],
                },
            },
          );
        case '/session-seen':
          return http.Response('{"ok":true}', 200);
        case '/session/message':
          posted.add(utf8.decode(r.bodyBytes));
          if (utf8.decode(r.bodyBytes).contains('"text":"malo"')) {
            return json(<String, Object?>{'ok': false, 'code': 'invalid', 'status': 'rejected'}, 422);
          }
          return json(<String, Object?>{'ok': true, 'status': 'delivered', 'duplicate': false});
      }
      return http.Response('{"error":"Not found"}', 404);
    });
    final MemoryCredentialStore creds = MemoryCredentialStore()..value = server;
    final Connection connection =
        Connection(creds, apiFactory: (PairedServer s) => ComandosApi(s, client: client));
    await connection.load();
    final Outbox outbox = Outbox(
      store: MemoryOutboxStore(),
      transport: ApiMessageTransport(() => connection.api),
      timer: (Duration d, void Function() cb) => _NoTimer(),
    );
    await outbox.load();
    outbox.setHost(server.displayHost);
    sl
      ..register<ErrorReporter>(ErrorReporter())
      ..register<AppVisibility>(AppVisibility())
      ..register<Connection>(connection)
      ..register<Outbox>(outbox)
      ..register<DraftStore>(DraftStore('${dir.path}/drafts.json'));
    VoiceServices.instance = testVoiceServices(dir: dir.path); // no OpenAI key saved
    registerVoice();
    registerCarMode();
    registerNotifications();
    expect(sl.maybe<VoiceInput>(), isNotNull);
    expect(sl.maybe<Speaker>(), isNotNull);
    expect(sl.get<NotificationsHook>(), isA<NotificationsController>());
  });

  tearDown(() async {
    sl.maybe<NotificationsController>()?.dispose();
    sl.reset();
    VoiceServices.instance = null;
    await dir.delete(recursive: true);
  });

  testWidgets('chat → send goes through the outbox', (WidgetTester tester) async {
    // The sessions half of this flow (feed → tap) is covered by shell_feed_test.dart.
    await tester.pumpWidget(MaterialApp(
      theme: buildTheme(),
      navigatorKey: AppNavigator.key,
      navigatorObservers: <NavigatorObserver>[AppNavigator.routes],
      home: const Scaffold(),
    ));
    AppNavigator.key.currentState!.push<void>(MaterialPageRoute<void>(
      builder: (_) => const ChatScreen(session: 'term-r1', pane: '%1', label: 'Pagos'),
    ));
    await tester.pumpAndSettle();
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
    await tester.pump();
    await tester.pump();

    // Folded like the web: the last answer and what followed it.
    expect(find.text('revisa las reglas'), findsNothing);
    expect(find.text('Ver 1 mensaje anterior', skipOffstage: false), findsOneWidget);
    expect(find.text('Bash · ls'), findsOneWidget);
    expect(find.byTooltip('1. Sí'), findsOneWidget); // permission card
    expect(find.byTooltip('Dictar'), findsNothing); // no OpenAI key: mic hidden

    // 15a: while the prompt is open, typing works but sending does not.
    final Finder composer = find.byType(TextField).last;
    await tester.enterText(composer, 'hola agente');
    await tester.pump();
    expect(find.text('Responde primero el permiso de arriba'), findsOneWidget);
    await tester.tap(find.byTooltip('Enviar; vacío envía Enter'));
    await tester.pump();
    expect(posted, isEmpty, reason: 'refused while the prompt is open');
    expect(sl.get<Outbox>().items, isEmpty);
    expect(sl.get<DraftStore>().get('term-r1'), 'hola agente', reason: 'drafting still works');
    ScaffoldMessenger.of(tester.element(find.byType(TextField).last)).clearSnackBars();

    // The prompt is answered (on the desktop): the next poll unblocks sending.
    askOpen = false;
    await tester.pump(const Duration(seconds: 3));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
    await tester.pump();
    await tester.pumpAndSettle();
    expect(find.byTooltip('1. Sí'), findsNothing);
    expect(find.text('Responde primero el permiso de arriba'), findsNothing);
    expect(posted, isEmpty);
    await tester.runAsync(() async {
      await tester.tap(find.byTooltip('Enviar; vacío envía Enter'));
      await Future<void>.delayed(const Duration(milliseconds: 100));
      await sl.get<Outbox>().pump();
    });
    await tester.pump();

    expect(posted, hasLength(1));
    final Map<String, Object?> body = jsonDecode(posted.single) as Map<String, Object?>;
    expect(body['text'], 'hola agente');
    expect(body['session'], 'term-r1');
    expect(body['pane'], '%1');
    expect(body['submit'], isTrue);
    expect((body['clientMessageId']! as String).length, 36);
    expect(sl.get<Outbox>().items.single.state, OutboxState.delivered);
    expect(find.text('Entregado ✓'), findsOneWidget);
    final TextField field = tester.widget(composer);
    expect(field.controller!.text, isEmpty);

    // Leave the screens so their pollers stop.
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(seconds: 1));
  });

  testWidgets('10: Editar on a failed message holds the later ones (note + order)', (WidgetTester tester) async {
    askOpen = false;
    // Real-time waits on a condition (not fixed delays): stable under load.
    Future<void> until(bool Function() ok) async {
      for (int i = 0; i < 200 && !ok(); i++) {
        await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 10)));
        await tester.pump();
      }
      expect(ok(), isTrue);
    }

    final Outbox outbox = sl.get<Outbox>();
    await tester.runAsync(() async {
      await outbox.enqueue(session: 'term-r1', pane: '%1', text: 'malo');
      await outbox.pump();
      await outbox.enqueue(session: 'term-r1', pane: '%1', text: 'adelante');
      await outbox.pump();
    });
    expect(outbox.items.first.state, OutboxState.failed);
    expect(posted, hasLength(1), reason: '"adelante" waits behind the failed one');

    await tester.pumpWidget(MaterialApp(
      theme: buildTheme(),
      navigatorKey: AppNavigator.key,
      navigatorObservers: <NavigatorObserver>[AppNavigator.routes],
      home: const ChatScreen(session: 'term-r1', pane: '%1'),
    ));
    await until(() => find.text('Editar').evaluate().isNotEmpty);
    // At the bottom of the feed, where a new failed message shows.
    await tester.drag(find.byType(CustomScrollView), const Offset(0, -3000));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Editar'));
    await until(() => outbox.editingFor('term-r1') != null);
    expect(find.text('1 mensaje en espera hasta que envíes el texto editado'), findsOneWidget);
    expect(find.text('Descartar edición'), findsOneWidget);
    await tester.runAsync(() => outbox.pump());
    expect(posted, hasLength(1), reason: 'nothing jumps ahead while editing');

    await tester.enterText(find.byType(TextField).last, 'bueno');
    await tester.pump();
    await tester.tap(find.byTooltip('Enviar; vacío envía Enter'));
    await until(() => outbox.editingFor('term-r1') == null);
    await tester.runAsync(() => outbox.pump());
    await until(() => posted.length == 3);
    await tester.pump();
    final List<String> texts = <String>[
      for (final String p in posted.skip(1)) (jsonDecode(p) as Map<String, Object?>)['text']! as String,
    ];
    expect(texts, <String>['bueno', 'adelante'], reason: 'the edited text goes first');
    expect(find.text('Descartar edición'), findsNothing);

    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(seconds: 1));
  });
}
