import 'dart:convert';

import 'package:comandos_mobile/core/api/api_client.dart';
import 'package:comandos_mobile/core/config/credential_store.dart';
import 'package:comandos_mobile/core/config/paired_server.dart';
import 'package:comandos_mobile/core/connection.dart';
import 'package:comandos_mobile/core/locator.dart';
import 'package:comandos_mobile/core/models/session_feed.dart';
import 'package:comandos_mobile/features/sessions/feed_row.dart';
import 'package:comandos_mobile/features/sessions/feed_view.dart';
import 'package:comandos_mobile/features/sessions/sections.dart';
import 'package:comandos_mobile/features/sessions/sessions_controller.dart';
import 'package:comandos_mobile/features/shell/nav_bar.dart';
import 'package:comandos_mobile/features/shell/session_switcher.dart';
import 'package:comandos_mobile/features/shell/shell_controller.dart';
import 'package:comandos_mobile/features/shell/tabs_controller.dart';
import 'package:comandos_mobile/ui/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/widgets.dart' as widgets show debugOnRebuildDirtyWidget;
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

// Synthetic data only.

http.Response _json(Object body) => http.Response.bytes(utf8.encode(jsonEncode(body)), 200,
    headers: <String, String>{'content-type': 'application/json; charset=utf-8'});

Map<String, Object?> _entry(String s, String label,
        {String state = 'idle', bool unread = false, double ts = 0, String reply = 'Listo', String prompt = ''}) =>
    <String, Object?>{
      'session': s, 'label': label, 'state': state, 'unread': unread, 'ts': ts, 'pane': '%1',
      'agent': 'claude', 'model': 'opus-5', 'reply': reply, 'prompt': prompt,
    };

class _MemoryStore implements ShellStore {
  Map<String, Object?>? value;
  @override
  Future<Map<String, Object?>?> read() async => value;
  @override
  Future<void> write(Map<String, Object?> v) async => value = v;
}

void main() {
  group('feed rules (session_feed.rs)', () {
    SessionSummary s(Map<String, Object?> m) => SessionSummary.tryParse(m)!;

    test('ago, verb, initials, glyph, model line', () {
      expect(feedAgo(1000, 990), 'ahora');
      expect(feedAgo(1000, 1000 - 14 * 60), '14 min');
      expect(feedAgo(100000, 100000 - 3 * 3600), '3 h');
      expect(feedAgo(400000, 400000 - 2 * 86400), '2 d');
      expect(feedAgo(1000, 0), '');
      expect(feedVerb(s(_entry('a', 'A', state: 'ask'))), 'pide permiso');
      expect(feedVerb(s(_entry('a', 'A', state: 'limit'))), 'llegó al límite de uso');
      expect(feedVerb(s(_entry('a', 'A', state: 'work'))), 'está trabajando');
      expect(feedVerb(s(_entry('a', 'A'))), 'respondió');
      expect(feedVerb(s(_entry('a', 'A', reply: ''))), 'sin actividad');
      expect(feedInitials('ComandOS ⎇'), 'CO');
      expect(feedInitials('Lola - Prospectos'), 'LP');
      expect(feedInitials('Tienda'), 'TI');
      expect(feedGlyph('claude'), '✻');
      expect(feedGlyph('codex'), '◎');
      expect(feedGlyph('gemini'), isNull);
      expect(tabModelShort('claude-fable-5-2'), 'fable-5-2');
      expect(tabModelShort('claude-opus-5'), 'opus');
      expect(tabModelShort('claude-sonnet-4-20250514'), 'sonnet-4');
      final SessionSummary two = SessionSummary.tryParse(<String, Object?>{
        'session': 'x',
        'paneList': <Object?>[
          <String, Object?>{'pane': '%1', 'model': 'fable-5-2'},
          <String, Object?>{'pane': '%2', 'model': 'fable-5-2'},
        ],
      })!;
      expect(modelLine(two), 'fable-5-2 +1');
    });

    test('marks follow K5: waiting > working > unread', () {
      expect(feedMark(s(_entry('a', 'A', state: 'ask'))), FeedMark.waiting);
      expect(feedMark(s(_entry('a', 'A', state: 'work', unread: true))), FeedMark.working);
      expect(feedMark(s(_entry('a', 'A', unread: true))), FeedMark.unread);
      expect(feedMark(s(_entry('a', 'A'))), FeedMark.none);
    });
  });

  group('tabs and switcher order', () {
    test('local first, then favourites, then the desktop order', () {
      final List<TabEntry> t = TabsController.orderTabs(<Object?>[
        <String, Object?>{'session': 'b', 'label': 'B'},
        <String, Object?>{'session': 'local', 'label': '⌂ local', 'closable': false},
        <String, Object?>{'session': 'c', 'label': 'C'},
        <String, Object?>{'session': 'd', 'label': ''},
        <String, Object?>{'session': 'c', 'label': 'dup'},
      ], <String>{'c'});
      expect(t.map((TabEntry e) => e.session), <String>['local', 'c', 'b', 'd']);
      expect(t.first.closable, isFalse);
      expect(t.last.label, 'd');
    });

    test('switcher: waiting, done, working, others, then closed tabs; fuzzy filter', () {
      final List<Object?> state = <Object?>[
        <String, Object?>{'session': 'term-1', 'project': 'Alfa', 'status': 'working', 'agent': 'claude'},
        <String, Object?>{'session': 'term-2', 'project': 'Beta', 'status': 'waiting', 'agent': 'claude'},
        <String, Object?>{'session': 'term-2', 'project': 'Beta %2', 'status': 'done', 'agent': 'claude'},
        <String, Object?>{'session': 'term-3', 'project': 'Gamma', 'status': 'done', 'agent': 'claude'},
        <String, Object?>{'session': 'term-4', 'project': 'Muerta', 'status': 'idle', 'alive': false},
        <String, Object?>{'session': 'term-5', 'project': 'Huérfana', 'status': 'idle'},
        <String, Object?>{'session': 'local', 'project': '⌂ local', 'status': 'idle'},
      ];
      final List<Object?> history = <Object?>[
        <String, Object?>{'session': 'term-2', 'label': 'ya viva'},
        <String, Object?>{'session': 'old', 'label': 'Vieja', 'cwd': '/x'},
      ];
      final List<SwitcherItem> all = switcherItems(state, history, '');
      expect(all.map((SwitcherItem i) => i.session), <String>['term-2', 'term-3', 'term-1', 'local', 'old']);
      expect(all.first.hint, 'espera TU respuesta');
      expect(all[1].hint, 'done');
      expect(all.last.label, '↺ Vieja');
      expect(all.last.hint, 'recuperar');
      expect(switcherItems(state, history, 'gma').map((SwitcherItem i) => i.session), <String>['term-3']);
      expect(switcherScore('ab', 'xaxb'), closeTo(2.04, 1e-9));
      expect(switcherScore('ba', 'ab'), isNull);
    });
  });

  group('shell state', () {
    test('open, panel toggle, chat toggle, restore and pending opens', () async {
      final _MemoryStore store = _MemoryStore()..value = <String, Object?>{'session': 'term-9', 'mode': 'term'};
      final ShellController c = ShellController(store: store);
      expect(c.panelOpen, isTrue);
      await c.restore();
      expect(c.active, 'term-9');
      expect(c.chatMode, isFalse, reason: 'the saved «term» mode');
      expect(c.toggleChat(), isTrue);
      expect(c.panelOpen, isFalse, reason: 'turning the chat on leaves the panel');
      c.showPanel();
      expect(c.panelOpen, isTrue, reason: 'cold start shows the panel; the toggle goes to the session');
      c.togglePanel();
      expect(c.panelOpen, isFalse);
      c.open('term-1', pane: '%3');
      expect(c.active, 'term-1');
      expect(c.pane, '%3');
      expect(store.value!['session'], 'term-1');
      expect(store.value!['mode'], 'chat');
      c.forget('term-1');
      expect(c.active, isNull);
      expect(c.panelOpen, isTrue);
      expect(c.toggleChat(), isTrue, reason: 'chat → terminal needs no session');
      expect(c.toggleChat(), isFalse, reason: '«Abre una sesión primero»');
      c.dispose();

      ShellController.openSession('term-7');
      final ShellController d = ShellController(store: _MemoryStore());
      await d.restore();
      expect(d.active, 'term-7');
      expect(d.panelOpen, isFalse);
      d.dispose();
    });
  });

  group('feed view', () {
    late double now;
    late String reply;
    late List<String> seen;

    setUp(() async {
      sl.reset();
      now = 10000;
      reply = 'Listo';
      seen = <String>[];
      final MockClient client = MockClient((http.Request r) async {
        switch (r.url.path) {
          case '/session-feed':
            now += 2.5;
            return _json(<String, Object?>{
              'now': now,
              'sessions': <Object?>[
                _entry('term-a', 'Pagos', state: 'ask', unread: true, ts: 9990)
                  ..['ask'] = <String, Object?>{'question': '¿Seguir?', 'options': <Object?>[]},
                _entry('term-b', 'ComandOS', state: 'work', ts: 9995, prompt: 'Está horrible'),
                _entry('term-c', 'Tienda', unread: true, ts: 9000),
                for (int i = 0; i < 3; i++) _entry('term-r$i', 'Proyecto $i', ts: 100, reply: i == 1 ? reply : 'Listo'),
              ],
            });
          case '/session-seen':
            seen.add((jsonDecode(r.body) as Map<String, Object?>)['session']! as String);
            return _json(<String, Object?>{'ok': true});
        }
        return http.Response('{"error":"Not found"}', 404);
      });
      final MemoryCredentialStore creds = MemoryCredentialStore()
        ..value = const PairedServer(host: 'mi-equipo.mi-red.ts.net', token: 'TOKEN');
      final Connection connection =
          Connection(creds, apiFactory: (PairedServer s) => ComandosApi(s, client: client));
      await connection.load();
      sl.register<Connection>(connection);
    });

    tearDown(() {
      widgets.debugOnRebuildDirtyWidget = null;
      sl.reset();
    });

    testWidgets('chips, «Te espera», groups and rows; an identical poll rebuilds nothing', (WidgetTester tester) async {
      final SessionsController feed = SessionsController(sl.get<Connection>());
      final ValueNotifier<FeedChip> chip = ValueNotifier<FeedChip>(FeedChip.all);
      final List<String> opened = <String>[];
      feed.setVisible(true);
      await tester.pumpWidget(MaterialApp(
        theme: buildTheme(),
        home: Scaffold(
          body: FeedView(controller: feed, chip: chip, onOpen: (SessionSummary e) => opened.add(e.session)),
        ),
      ));
      await tester.pump();
      await tester.pump();

      expect(find.text('TE ESPERA · 1'), findsOneWidget);
      expect(find.text('AHORA'), findsOneWidget);
      expect(find.byType(FeedRow), findsNWidgets(6));
      expect(find.textContaining('Tú: Está horrible', findRichText: true), findsOneWidget);
      expect(find.textContaining('Todas  6', findRichText: true), findsOneWidget);
      expect(find.textContaining('Trabajando  1', findRichText: true), findsOneWidget);

      int total = 0;
      int rows = 0;
      widgets.debugOnRebuildDirtyWidget = (Element e, bool _) {
        total++;
        if (e.widget is FeedRow) rows++;
      };
      await tester.pump(const Duration(milliseconds: 2600));
      await tester.pump(const Duration(milliseconds: 2600));
      expect(total, 0, reason: 'an identical poll must not rebuild any widget');

      reply = 'Otra respuesta';
      await tester.pump(const Duration(milliseconds: 2600));
      await tester.pump();
      expect(rows, 1, reason: 'only the row that changed rebuilds');
      widgets.debugOnRebuildDirtyWidget = null;

      await tester.tap(find.textContaining('Sin leer', findRichText: true));
      await tester.pump();
      expect(find.byType(FeedRow), findsNWidgets(2));

      await tester.tap(find.textContaining('Tienda', findRichText: true));
      expect(opened, <String>['term-c']);

      feed.markSeen('term-c');
      await tester.pump();
      expect(seen, <String>['term-c']);
      expect(feed.entry('term-c')!.unread, isFalse);
      expect(find.byType(FeedRow), findsOneWidget, reason: '«Sin leer» drops the row once seen');

      feed.dispose();
      chip.dispose();
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(seconds: 3));
    });

    testWidgets('tab chips: marks, active chip with its ×, long press', (WidgetTester tester) async {
      final SessionFeed f = SessionFeed.tryParse(<String, Object?>{
        'now': 1000.0,
        'sessions': <Object?>[
          _entry('a', 'A', state: 'ask', ts: 990),
          _entry('b', 'B', unread: true, ts: 990),
        ],
      })!;
      final List<String> log = <String>[];
      await tester.pumpWidget(MaterialApp(
        theme: buildTheme(),
        home: Scaffold(
          body: TabChips(
            tabs: const <TabEntry>[
              TabEntry(session: 'local', label: '⌂ local', closable: false),
              TabEntry(session: 'a', label: 'A'),
              TabEntry(session: 'b', label: 'B'),
            ],
            feed: f,
            active: 'b',
            onTap: (String s) => log.add('tap $s'),
            onClose: (TabEntry t) => log.add('close ${t.session}'),
            onLongPress: (String s) => log.add('long $s'),
          ),
        ),
      ));
      await tester.pump();
      expect(find.text('×'), findsOneWidget, reason: 'only the open chip shows ×');
      final TabChip a = tester.widget<TabChip>(find.ancestor(of: find.text('A'), matching: find.byType(TabChip)));
      expect(a.mark, FeedMark.waiting);
      await tester.tap(find.text('A'));
      await tester.longPress(find.text('⌂ local'));
      await tester.tap(find.text('×'));
      expect(log, <String>['tap a', 'long local', 'close b']);
      final Container chip = tester.widget<Container>(
          find.descendant(of: find.ancestor(of: find.text('B'), matching: find.byType(TabChip)), matching: find.byType(Container)).first);
      expect((chip.decoration! as BoxDecoration).border, Border.all(color: Orbita.brand));
    });
  });
}
