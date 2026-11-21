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
import 'package:comandos_mobile/features/chat/session_view.dart';
import 'package:comandos_mobile/features/sessions/feed_row.dart';
import 'package:comandos_mobile/features/shell/app_shell.dart';
import 'package:comandos_mobile/features/shell/nav_bar.dart';
import 'package:comandos_mobile/features/shell/shell_controller.dart';
import 'package:comandos_mobile/ui/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

// Synthetic data only.

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

class _MemoryStore implements ShellStore {
  Map<String, Object?>? value;
  @override
  Future<Map<String, Object?>?> read() async => value;
  @override
  Future<void> write(Map<String, Object?> v) async => value = v;
}

void main() {
  final List<String> seen = <String>[];
  late Directory dir;

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('shell');
    sl.reset();
    seen.clear();
    final MockClient client = MockClient((http.Request r) async {
      switch (r.url.path) {
        case '/session-feed':
          return _json(<String, Object?>{
            'now': 1000.0,
            'sessions': <Object?>[
              <String, Object?>{
                'session': 'term-a', 'label': 'Pagos', 'state': 'idle', 'unread': true, 'ts': 990.0,
                'pane': '%1', 'agent': 'claude', 'model': 'opus-5', 'reply': 'Listo',
                'paneList': <Object?>[
                  <String, Object?>{'pane': '%1', 'model': 'fable-5-2'},
                  <String, Object?>{'pane': '%2', 'model': 'opus-5'},
                ],
              },
              <String, Object?>{'session': 'term-b', 'label': 'Docs', 'state': 'work', 'ts': 995.0, 'reply': 'x'},
            ],
          });
        case '/tabs':
          return _json(<Object?>[
            <String, Object?>{'session': 'local', 'label': '⌂ local', 'closable': false},
            <String, Object?>{'session': 'term-b', 'label': 'Docs'},
            <String, Object?>{'session': 'term-a', 'label': 'Pagos'},
          ]);
        case '/prefs':
          return _json(<String, Object?>{'favorites': <Object?>['term-a']});
        case '/remote-state':
          return _json(<String, Object?>{'remoteOn': true});
        case '/session-seen':
          seen.add((jsonDecode(r.body) as Map<String, Object?>)['session']! as String);
          return _json(<String, Object?>{'ok': true});
        case '/chat/transcript':
          return _json(<String, Object?>{'ok': true, 'agent': 'claude', 'pane': '%1', 'token': 't', 'messages': <Object?>[]});
      }
      return http.Response('{"error":"Not found"}', 404);
    });
    final MemoryCredentialStore creds = MemoryCredentialStore()
      ..value = const PairedServer(host: 'mi-equipo.mi-red.ts.net', token: 'TOKEN');
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
      ..register<ErrorReporter>(ErrorReporter())
      ..register<AppVisibility>(AppVisibility())
      ..register<Connection>(connection)
      ..register<Outbox>(outbox)
      ..register<DraftStore>(DraftStore('${dir.path}/drafts.json'));
  });

  tearDown(() async {
    sl.reset();
    await dir.delete(recursive: true);
  });

  testWidgets('panel → chip opens the session → panel toggle comes back', (WidgetTester tester) async {
    await tester.pumpWidget(MaterialApp(
      theme: buildTheme(),
      navigatorKey: AppNavigator.key,
      navigatorObservers: <NavigatorObserver>[AppNavigator.routes],
      home: AppShell(store: _MemoryStore()),
    ));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
    await tester.pump();
    await tester.pump();

    // Nav keys, chips in the web order (local, favourites, rest), topbar, tools, feed.
    expect(find.byKey(const ValueKey<String>('tab-panel')), findsOneWidget);
    expect(find.byKey(const ValueKey<String>('tab-chat')), findsOneWidget);
    final List<String> chips =
        tester.widgetList<TabChip>(find.byType(TabChip)).map((TabChip c) => c.tab.session).toList();
    expect(chips, <String>['local', 'term-a', 'term-b']);
    expect(find.byKey(const ValueKey<String>('btn-settings')), findsOneWidget);
    expect(find.text('Sesiones'), findsOneWidget);
    expect(find.text('Consumos'), findsOneWidget);
    expect(find.byType(FeedRow), findsNWidgets(2));

    await tester.tap(find.descendant(of: find.byType(TabChip), matching: find.text('Pagos')));
    await tester.pump();
    final SessionView view = tester.widget<SessionView>(find.byType(SessionView));
    expect(view.session, 'term-a');
    expect(view.title, 'Pagos');
    expect(view.subtitle, 'fable-5-2 +1');
    expect(view.terminal, isFalse, reason: 'phones open in chat');
    expect(AppNavigator.visibleChat.value, 'term-a');
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
    expect(seen, contains('term-a'));

    view.onNext!();
    await tester.pump();
    expect(tester.widget<SessionView>(find.byType(SessionView)).session, 'term-b');

    await tester.tap(find.byKey(const ValueKey<String>('tab-panel')));
    await tester.pump(const Duration(milliseconds: 100));
    expect(find.byType(SessionView), findsNothing);
    expect(find.byType(FeedRow), findsNWidgets(2));
    expect(AppNavigator.visibleChat.value, isNull);

    // Chat → terminal with the panel open stays on the panel (the live terminal
    // needs a WebView, not available in widget tests); chat on again shows the session.
    await tester.tap(find.byKey(const ValueKey<String>('tab-chat')));
    await tester.pump(const Duration(milliseconds: 100));
    expect(find.byTooltip('Ver como chat'), findsOneWidget);
    expect(find.byType(SessionView), findsNothing);
    await tester.tap(find.byKey(const ValueKey<String>('tab-chat')));
    await tester.pump(const Duration(milliseconds: 100));
    expect(find.byTooltip('Ver terminal'), findsOneWidget);
    expect(tester.widget<SessionView>(find.byType(SessionView)).terminal, isFalse);
    await tester.tap(find.byKey(const ValueKey<String>('tab-panel')));
    await tester.pump(const Duration(milliseconds: 100));

    ShellController.openSession('term-a');
    await tester.pump();
    expect(tester.widget<SessionView>(find.byType(SessionView)).session, 'term-a');

    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(seconds: 6));
  });
}
