import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:comandos_mobile/core/api/api_client.dart';
import 'package:comandos_mobile/core/config/credential_store.dart';
import 'package:comandos_mobile/core/config/paired_server.dart';
import 'package:comandos_mobile/core/connection.dart';
import 'package:comandos_mobile/core/locator.dart';
import 'package:comandos_mobile/core/outbox/drafts.dart';
import 'package:comandos_mobile/core/outbox/message_transport.dart';
import 'package:comandos_mobile/core/outbox/outbox.dart';
import 'package:comandos_mobile/core/outbox/outbox_store.dart';
import 'package:comandos_mobile/core/polling/app_visibility.dart';
import 'package:comandos_mobile/features/car/car_mode_screen.dart';
import 'package:comandos_mobile/features/car/on_device.dart';
import 'package:comandos_mobile/features/voice/voice_services.dart';
import 'package:comandos_mobile/ui/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import '../voice/voice_fakes.dart';

const PairedServer server = PairedServer(host: 'equipo.ejemplo.ts.net', token: 'TOKEN');

http.Response _json(Object body) => http.Response.bytes(utf8.encode(jsonEncode(body)), 200,
    headers: <String, String>{'content-type': 'application/json; charset=utf-8'});

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
  late List<String> calls;
  late FakeDeviceSpeech tts;
  late FakeDeviceListener stt;

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('car');
    sl.reset();
    tts = FakeDeviceSpeech();
    stt = FakeDeviceListener();
    DeviceVoice.override(tts: tts, stt: stt);
    calls = <String>[];
    final MockClient client = MockClient((http.Request r) async {
      calls.add(r.url.path);
      switch (r.url.path) {
        case '/session-feed':
          return _json(<String, Object?>{
            'now': 1000.0,
            'sessions': <Object?>[
              <String, Object?>{'session': 's1', 'label': 'Pagos con un nombre bastante largo', 'state': 'work', 'ts': 990.0, 'pane': '%1'},
              <String, Object?>{'session': 's2', 'label': 'Docs', 'state': 'idle', 'ts': 100.0},
            ],
          });
        case '/chat/transcript':
          return _json(<String, Object?>{
            'ok': true, 'pane': '%1', 'token': 't1', 'age': 5,
            'messages': <Object?>[
              <String, Object?>{'r': 'u', 't': 'revisa'},
              <String, Object?>{'r': 'a', 't': 'Revisé todo. ${'Texto largo de respuesta. ' * 30}'},
            ],
          });
        case '/session-seen':
          return http.Response('{"ok":true}', 200);
      }
      return http.Response('{}', 404);
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
    sl
      ..register<AppVisibility>(AppVisibility())
      ..register<Connection>(connection)
      ..register<Outbox>(outbox)
      ..register<DraftStore>(DraftStore('${dir.path}/drafts.json'));
  });

  tearDown(() async {
    sl.reset();
    DeviceVoice.override();
    VoiceServices.instance = null;
    await dir.delete(recursive: true);
  });

  Future<void> pumpCar(WidgetTester tester, Size size,
      {String? key = 'sk-test-0000000000000000000000abcd', Widget home = const CarModeScreen(session: 's1')}) async {
    VoiceServices.instance = testVoiceServices(dir: dir.path, key: key);
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(MaterialApp(theme: buildTheme(), home: home));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 80)));
    await tester.pump();
    await tester.pump();
  }

  /// Lets the call run a while (real I/O for the earcon, fake timers).
  Future<void> run(WidgetTester tester, {int rounds = 10}) async {
    for (int i = 0; i < rounds; i++) {
      await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
      await tester.pump(const Duration(milliseconds: 200));
    }
  }

  Future<void> leave(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(seconds: 40)); // the idle player release timer (30 s)
  }

  testWidgets('portrait: a call UI with a huge hang-up area; the call starts with no tap', (WidgetTester tester) async {
    await pumpCar(tester, const Size(400, 820));
    await run(tester);
    expect(tester.takeException(), isNull);
    expect(find.text('Colgar'), findsOneWidget);
    expect(find.text('Pagos con un nombre bastante largo'), findsOneWidget);
    expect(tester.getSize(find.ancestor(of: find.text('Colgar'), matching: find.byType(SizedBox)).first).height,
        greaterThanOrEqualTo(120));
    // Spoken with the phone's own voice, without a tap: the count, then
    // the split it was opened for, then its state.
    expect(tts.said.first, 'Ningún split te espera. Pagos con un nombre bastante largo, split 1.');
    expect(tts.said, contains('Trabajando.'));
    // Then it listened with the phone's recognizer (free).
    expect(stt.dictations, isNotEmpty);
    await leave(tester);
  });

  testWidgets('landscape fits too', (WidgetTester tester) async {
    await pumpCar(tester, const Size(820, 380));
    await run(tester);
    expect(tester.takeException(), isNull);
    expect(find.text('Colgar'), findsOneWidget);
    await leave(tester);
  });

  testWidgets('saying «cuelga» ends the call and closes the screen', (WidgetTester tester) async {
    stt.answers.add('cuelga');
    await pumpCar(tester, const Size(400, 820),
        home: Builder(
          builder: (BuildContext c) => TextButton(
            onPressed: () => Navigator.of(c).push(MaterialPageRoute<void>(builder: (_) => const CarModeScreen(session: 's1'))),
            child: const Text('abrir'),
          ),
        ));
    await tester.tap(find.text('abrir'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Colgar'), findsOneWidget);
    await run(tester, rounds: 20);
    expect(tts.said, contains('Llamada terminada.'));
    await tester.pumpAndSettle();
    expect(find.text('Colgar'), findsNothing);
    expect(find.text('abrir'), findsOneWidget);
    await leave(tester);
  });

  testWidgets('8: hidden app → no polling; back → polling resumes', (WidgetTester tester) async {
    await pumpCar(tester, const Size(400, 820));
    int polls() => calls.where((String p) => p == '/session-feed' || p == '/chat/transcript').length;
    Future<void> run(Duration d) async {
      for (int i = 0; i < d.inSeconds; i++) {
        await tester.pump(const Duration(seconds: 1));
        await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 5)));
      }
    }

    await run(const Duration(seconds: 3));
    expect(polls(), greaterThan(2), reason: 'polls while visible');
    sl.get<AppVisibility>().value = false;
    await run(const Duration(seconds: 2)); // let a request in flight finish
    final int hidden = polls();
    await run(const Duration(seconds: 20));
    expect(polls(), hidden, reason: 'nothing while hidden');
    sl.get<AppVisibility>().value = true;
    await run(const Duration(seconds: 4));
    expect(polls(), greaterThan(hidden));
    await leave(tester);
  });

  testWidgets('without an OpenAI key the call still works (the phone\'s voice and recognizer)',
      (WidgetTester tester) async {
    await pumpCar(tester, const Size(400, 820), key: null);
    await run(tester);
    expect(find.text('Configurar la voz'), findsNothing);
    expect(find.text('Colgar'), findsOneWidget);
    expect(tts.said, isNotEmpty);
    expect(stt.dictations, isNotEmpty);
    await leave(tester);
  });
}
