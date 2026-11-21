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
import 'package:comandos_mobile/core/models/transcript.dart';
import 'package:comandos_mobile/core/outbox/drafts.dart';
import 'package:comandos_mobile/core/outbox/message_transport.dart';
import 'package:comandos_mobile/core/outbox/outbox.dart';
import 'package:comandos_mobile/core/outbox/outbox_store.dart';
import 'package:comandos_mobile/core/polling/app_visibility.dart';
import 'package:comandos_mobile/features/car/car_mode.dart';
import 'package:comandos_mobile/features/chat/chat_screen.dart';
import 'package:comandos_mobile/features/chat/message_view.dart';
import 'package:comandos_mobile/features/chat/session_view.dart';
import 'package:comandos_mobile/features/notifications/notifications_hook.dart';
import 'package:comandos_mobile/features/voice/voice.dart';
import 'package:comandos_mobile/ui/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

// Synthetic data only. The chat must draw what the server answers, as the
// web page does: its latest window from the last answer on, nothing kept
// from earlier windows, the end in view.

class _NoTimer implements Timer {
  @override
  void cancel() {}
  @override
  bool get isActive => false;
  @override
  int get tick => 0;
}

Map<String, Object?> m(String role, String text) => <String, Object?>{'r': role, 't': text};

/// One answer of `POST /chat/transcript`: a window, or «no agent here».
Map<String, Object?> window(String token, List<Map<String, Object?>> messages) =>
    <String, Object?>{'ok': true, 'agent': 'claude', 'pane': '%1', 'token': token, 'age': 500, 'messages': messages};
const Map<String, Object?> noAgent = <String, Object?>{'ok': true, 'agent': null, 'pane': '%1', 'messages': <Object?>[]};

const Key feed = ValueKey<String>('cv-feed');
const String same = 'Bash · cd /srv/app && git status';

void main() {
  late Directory dir;
  late Map<String, Object?> answer;

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('feed');
    sl.reset();
    final MockClient client = MockClient((http.Request r) async {
      Object body = <String, Object?>{'ok': true};
      if (r.url.path == '/chat/transcript') {
        final Object? since = (jsonDecode(r.body) as Map<String, Object?>)['since'];
        body = answer['token'] != null && since == answer['token']
            ? <String, Object?>{'ok': true, 'unchanged': true, 'token': answer['token'], 'pane': '%1', 'age': 500}
            : answer;
      } else if (r.url.path == '/state') {
        body = <Object?>[];
      }
      return http.Response.bytes(utf8.encode(jsonEncode(body)), 200,
          headers: <String, String>{'content-type': 'application/json; charset=utf-8'});
    });
    final MemoryCredentialStore creds = MemoryCredentialStore()
      ..value = const PairedServer(host: 'mi-equipo.mi-red.ts.net', token: 'TOKEN');
    final Connection connection = Connection(creds, apiFactory: (PairedServer s) => ComandosApi(s, client: client));
    await connection.load();
    final Outbox outbox = Outbox(
      store: MemoryOutboxStore(),
      transport: ApiMessageTransport(() => connection.api),
      timer: (Duration d, void Function() cb) => _NoTimer(),
    );
    await outbox.load();
    sl
      ..register<ErrorReporter>(ErrorReporter())
      ..register<AppVisibility>(AppVisibility())
      ..register<Connection>(connection)
      ..register<Outbox>(outbox)
      ..register<DraftStore>(DraftStore('${dir.path}/drafts.json'))
      ..register<NotificationsHook>(NoNotifications());
    registerVoice();
    registerCarMode();
  });

  tearDown(() async {
    sl.reset();
    await dir.delete(recursive: true);
  });

  Future<void> open(WidgetTester tester) async {
    tester.view.physicalSize = const Size(412, 915);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(MaterialApp(
      theme: buildTheme(),
      navigatorKey: AppNavigator.key,
      navigatorObservers: <NavigatorObserver>[AppNavigator.routes],
      home: const ChatScreen(session: 'term-r1'),
    ));
    await tester.pump();
    await tester.pump();
  }

  /// The next poll brings [next].
  Future<void> poll(WidgetTester tester, Map<String, Object?> next) async {
    answer = next;
    await tester.pump(const Duration(milliseconds: 2100));
    await tester.pump();
    await tester.pump();
  }

  Future<void> close(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(seconds: 3));
  }

  /// Texts of the messages on screen, top to bottom.
  List<String> drawn(WidgetTester tester) {
    final List<(double, String)> rows = <(double, String)>[
      for (final Element e in find.byType(MessageView).evaluate())
        (tester.getTopLeft(find.byElementPredicate((Element x) => identical(x, e))).dy, (e.widget as MessageView).message.text),
    ]..sort(((double, String) a, (double, String) b) => a.$1.compareTo(b.$1));
    return <String>[for (final (double, String) r in rows) r.$2];
  }

  List<String> expected(List<Map<String, Object?>> messages) {
    final List<ChatMessage> list = <ChatMessage>[for (final Map<String, Object?> e in messages) ChatMessage.tryParse(e)!];
    return <String>[for (final ChatMessage c in list.sublist(foldIndex(list))) c.text];
  }

  testWidgets('draws the server window and nothing else while its start moves and tool lines repeat',
      (WidgetTester tester) async {
    // A working agent: the window is a byte tail, so its first tool group
    // loses lines, and the same tool line shows again and again.
    final List<List<Map<String, Object?>>> windows = <List<Map<String, Object?>>>[
      <Map<String, Object?>>[m('t', 'Read · /a.rs\n$same'), m('a', 'Reviso el estado.'), m('t', same)],
      <Map<String, Object?>>[m('t', same), m('a', 'Reviso el estado.'), m('t', '$same\nRead · /b.rs'), m('u', 'sigue')],
      // No answer left in the window: the web starts at the last prompt.
      <Map<String, Object?>>[m('t', '$same\nRead · /b.rs'), m('u', 'sigue'), m('t', same)],
      <Map<String, Object?>>[m('u', 'sigue'), m('t', '$same\n$same'), m('a', 'Listo, quedó limpio.')],
      <Map<String, Object?>>[m('t', same), m('a', 'Listo, quedó limpio.'), m('t', same)],
      <Map<String, Object?>>[m('a', 'Listo, quedó limpio.'), m('t', '$same\n$same\n$same')],
    ];
    answer = window('t0', windows.first);
    await open(tester);
    for (int i = 0; i < windows.length; i++) {
      await poll(tester, window('t$i', windows[i]));
      expect(drawn(tester), expected(windows[i]), reason: 'window $i');
    }
    await close(tester);
  });

  testWidgets('«no agent here» is said on screen; a short gap keeps the conversation', (WidgetTester tester) async {
    answer = noAgent;
    await open(tester);
    final Finder note = find.textContaining('no tiene un agente con historial');
    expect(note, findsOneWidget);
    final Rect screen = tester.getRect(find.byType(ChatScreen));
    expect(screen.contains(tester.getCenter(note)), isTrue, reason: 'in view, not below the fold');

    // The agent shows up: its conversation replaces the note.
    await poll(tester, window('t1', <Map<String, Object?>>[m('u', 'hola'), m('a', 'Aquí estoy.')]));
    expect(note, findsNothing);
    expect(drawn(tester), <String>['Aquí estoy.']);

    // Two answers without agent (it is restarting): the chat stays.
    await poll(tester, noAgent);
    await poll(tester, noAgent);
    expect(drawn(tester), <String>['Aquí estoy.']);
    // It lasts: the chat says the truth instead of an old conversation.
    await poll(tester, noAgent);
    expect(drawn(tester), isEmpty);
    expect(note, findsOneWidget);
    await close(tester);
  });

  testWidgets('the end stays in view as the conversation grows; a reader scrolled up is not moved',
      (WidgetTester tester) async {
    final String long = List<String>.filled(60, 'Una línea de la respuesta.').join('\n\n');
    List<Map<String, Object?>> w(int tools) => <Map<String, Object?>>[
          m('u', 'hazlo'),
          m('a', long),
          if (tools > 0) m('t', List<String>.generate(tools, (int i) => 'Bash · paso $i').join('\n')),
        ];
    answer = window('t0', w(0));
    await open(tester);
    await poll(tester, window('t0', w(0)));
    final ScrollPosition p = Scrollable.of(tester.element(find.byType(MessageView).first)).position;
    expect(p.maxScrollExtent, greaterThan(500));
    expect(p.pixels, p.maxScrollExtent, reason: 'opens on the latest message');

    await poll(tester, window('t1', w(3)));
    expect(p.pixels, p.maxScrollExtent, reason: 'follows new lines');
    final Rect view = tester.getRect(find.byKey(feed));
    expect(tester.getRect(find.text('Bash · paso 2')).bottom, lessThanOrEqualTo(view.bottom));

    // The reader goes up to read: new lines arrive below and nothing moves.
    await tester.drag(find.byKey(feed), const Offset(0, 400));
    await tester.pump();
    final double reading = p.pixels;
    expect(reading, lessThan(p.maxScrollExtent - 120));
    await poll(tester, window('t2', w(9)));
    expect(p.pixels, reading);
    expect(drawn(tester).last, contains('Bash · paso 8'));
    await close(tester);
  });
}
