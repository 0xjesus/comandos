import 'dart:convert';

import 'package:comandos_mobile/core/api/api_response.dart';
import 'package:comandos_mobile/core/config/paired_server.dart';
import 'package:comandos_mobile/features/webpanel/term_interaction.dart';
import 'package:comandos_mobile/features/webpanel/terminal_pane.dart';
import 'package:comandos_mobile/features/webpanel/web_panel.dart';
import 'package:comandos_mobile/features/webpanel/web_scripts.dart';
import 'package:comandos_mobile/features/webpanel/web_session.dart';
import 'package:comandos_mobile/features/webpanel/web_surface.dart';
import 'package:comandos_mobile/features/webpanel/web_urls.dart';
import 'package:comandos_mobile/ui/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

typedef DebugPrintCallbackType = void Function(String? message, {int? wrapWidth});

const String kToken = 'tok_SECRET-123abc';
const PairedServer kServer = PairedServer(host: 'mi-equipo.mi-red.ts.net', token: kToken);

class FakeSurface implements WebSurface {
  FakeSurface(this.events);

  final WebSurfaceEvents events;
  final List<Uri> loads = <Uri>[];
  final List<String> scripts = <String>[];
  bool disposed = false;
  int views = 0;

  @override
  Future<void> load(Uri url) async => loads.add(url);

  @override
  Future<void> runJs(String source) async => scripts.add(source);

  @override
  Widget view() {
    views++;
    return const SizedBox.expand(key: Key('fake-web'));
  }

  @override
  Future<void> dispose() async => disposed = true;

  void send(Map<String, Object?> m) => events.onMessage(jsonEncode(m));

  /// The messages posted to the page with [postToPage].
  List<Map<String, Object?>> get posted => <Map<String, Object?>>[
        for (final String s in scripts)
          if (s.startsWith('window.postMessage('))
            (jsonDecode(s.substring('window.postMessage('.length, s.lastIndexOf(',location.origin'))) as Map)
                .cast<String, Object?>(),
      ];
}

class FakeStore implements WebStore {
  final List<String> calls = <String>[];

  @override
  Future<void> setTokenCookie(String host, String token) async =>
      calls.add('cookie $host ${token == kToken ? 'TOKEN' : 'other'}');

  @override
  Future<void> clearCookies() async => calls.add('clearCookies');

  @override
  Future<void> clearAll() async => calls.add('clearAll');
}

void main() {
  group('URLs', () {
    test('each panel loads the dashboard page the desktop app uses', () {
      final Map<WebPanel, String> want = <WebPanel, String>{
        WebPanel.settings: 'https://mi-equipo.mi-red.ts.net/?panel=settings',
        WebPanel.servers: 'https://mi-equipo.mi-red.ts.net/?panel=servers',
        WebPanel.remote: 'https://mi-equipo.mi-red.ts.net/?panel=remote',
        WebPanel.news: 'https://mi-equipo.mi-red.ts.net/?panel=news&news=',
        WebPanel.snippets: 'https://mi-equipo.mi-red.ts.net/?panel=snippets',
        WebPanel.fullPage: 'https://mi-equipo.mi-red.ts.net/',
      };
      for (final WebPanel p in WebPanel.values) {
        final String url = webPanelUrl(kServer, p).toString();
        expect(url, want[p]);
        expect(url, isNot(contains(kToken)));
      }
      expect(
        webPanelUrl(const PairedServer(host: 'h.ts.net', token: 't', port: 8443), WebPanel.settings).toString(),
        'https://h.ts.net:8443/?panel=settings',
      );
    });

    test('the token goes in a panel URL only when it cannot be a cookie', () {
      expect(webPanelUrl(kServer, WebPanel.remote, tokenInUrl: true).queryParameters['token'], kToken);
      expect(cookieSafeToken(kToken), isTrue);
      expect(cookieSafeToken('a;b'), isFalse);
      expect(cookieSafeToken('a b'), isFalse);
      expect(cookieSafeToken(''), isFalse);
    });

    test('terminal URL is the one the dashboard embeds, redacted for logs', () {
      final Uri url = terminalUrl(kServer, 'term-r23327',
          look: const TermLook(theme: 'dia', buttons: 'pixel'));
      expect(url.toString(),
          'https://mi-equipo.mi-red.ts.net/term/?auth=$kToken&arg=term-r23327&theme=dia&btn=pixel');
      final String safe = redactedUrl(url);
      expect(safe, isNot(contains(kToken)));
      expect(safe, contains('auth=***'));
      expect(safe, contains('arg=term-r23327'));
      expect(redactedUrl(webPanelUrl(kServer, WebPanel.news, tokenInUrl: true)), isNot(contains(kToken)));
    });

    test('navigation stays on the paired https host', () {
      bool ok(String u, {bool main = true}) => allowedNavigation(Uri.parse(u), kServer, mainFrame: main);
      expect(ok('https://mi-equipo.mi-red.ts.net/?panel=settings'), isTrue);
      expect(ok('https://MI-EQUIPO.mi-red.ts.net/term/'), isTrue);
      expect(ok('https://mi-equipo.mi-red.ts.net:443/x'), isTrue);
      expect(ok('http://mi-equipo.mi-red.ts.net/'), isFalse);
      expect(ok('https://mi-equipo.mi-red.ts.net:8443/'), isFalse);
      expect(ok('https://evil.example/'), isFalse);
      expect(ok('https://mi-equipo.mi-red.ts.net.evil.example/'), isFalse);
      expect(ok('https://user@mi-equipo.mi-red.ts.net/'), isFalse);
      expect(ok('javascript:alert(1)'), isFalse);
      expect(ok('data:text/html,hi'), isFalse);
      expect(ok('file:///etc/hosts'), isFalse);
      expect(ok('intent://x#Intent;end'), isFalse);
      expect(ok('about:blank'), isFalse);
      expect(ok('about:blank', main: false), isTrue);
      const PairedServer ported = PairedServer(host: 'h.ts.net', token: 't', port: 8443);
      expect(allowedNavigation(Uri.parse('https://h.ts.net:8443/'), ported), isTrue);
      expect(allowedNavigation(Uri.parse('https://h.ts.net/'), ported), isFalse);
    });

    test('terminal look from /prefs', () {
      expect(TermLook.fromPrefs(<String, Object?>{'theme': 'neon', 'button_style': 'arcade'}),
          const TermLook(theme: 'neon', buttons: 'arcade'));
      expect(TermLook.fromPrefs(<String, Object?>{'theme': 'nope', 'button_style': 'x'}), TermLook.fallback);
      final Map<String, Object?> colors = <String, Object?>{
        for (final String k in <String>['bg', 'fg', 'cursor', 'line2', 'panel', 'panel2', 'line', 'dim', 'faint', 'brand'])
          k: '#101010',
        for (int i = 0; i < 16; i++) 'pal$i': '#20202${i % 10}',
      };
      final TermLook user = TermLook.fromPrefs(<String, Object?>{
        'theme': 'mio',
        'user_themes': <Object?>[
          <String, Object?>{'id': 'mio', 'base': 'dia', 'colors': colors, 'button_style': 'tecla'},
        ],
      });
      expect(user.theme, 'dia');
      expect(user.buttons, 'tecla');
      expect(user.palette!['background'], '#101010');
      expect(user.palette!['brightWhite'], '#202025');
      expect(user.themeMessage['palette'], isNotNull);
      final TermLook broken = TermLook.fromPrefs(<String, Object?>{
        'theme': 'mio',
        'user_themes': <Object?>[
          <String, Object?>{'id': 'mio', 'base': 'dia', 'colors': <String, Object?>{...colors, 'bg': 'red'}},
        ],
      });
      expect(broken.palette, isNull);
      expect(broken.themeMessage.containsKey('palette'), isFalse);
    });

    test('page messages are validated', () {
      expect(PageMessage.parse('{"t":"open"}')!.t, 'open');
      expect(PageMessage.parse('{"t":"term","type":"interaction-request","selecting":true}')!.selecting, isTrue);
      expect(PageMessage.parse('{"t":"term","pane":"%1;x"}')!.pane, '');
      expect(PageMessage.parse('{"t":"term","pane":"%12"}')!.pane, '%12');
      expect(PageMessage.parse('{"t":"eval"}'), isNull);
      expect(PageMessage.parse('nope'), isNull);
      expect(PageMessage.parse('[1]'), isNull);
      expect(PageMessage.parse('{"t":"open","x":"${'a' * 3000}"}'), isNull);
    });

    test('scripts never carry the token and open the right panel', () {
      for (final WebPanel p in WebPanel.values) {
        expect(panelScript(p), isNot(contains(kToken)));
        expect(panelScript(p), contains(kAppChannel));
      }
      expect(panelScript(WebPanel.settings), contains('openCenterPanel({panel:"settings",tab:""})'));
      expect(panelScript(WebPanel.servers), contains('openCenterPanel({panel:"servers"})'));
      expect(panelScript(WebPanel.remote), contains('openCenterPanel({panel:"remote"})'));
      expect(panelScript(WebPanel.snippets), contains('openSnippets()'));
      expect(panelScript(WebPanel.news), contains('#news-reader'));
      expect(terminalScript(), contains('__comandosTerm'));
      expect(postToPage(kDockMessage), 'window.postMessage({"source":"comandos","type":"dock","on":true},location.origin);');
    });
  });

  group('TermInteraction', () {
    test('sync, select and restore follow the dashboard', () async {
      final List<String> calls = <String>[];
      final List<Map<String, Object?>> posted = <Map<String, Object?>>[];
      final TermInteraction i = TermInteraction(
        session: 'term-r1',
        request: (String m, String p, {Map<String, String>? query, Map<String, Object?>? body}) async {
          calls.add('$m $p ${jsonEncode(query ?? body)}');
          return ApiResponse(200, m == 'GET' ? <String, Object?>{'mouse': 'on'} : <String, Object?>{'ok': true});
        },
        post: posted.add,
      );
      await i.sync();
      expect(calls.single, 'GET /tmux-mouse {"session":"term-r1"}');
      expect(posted.first['busy'], isTrue);
      expect(posted.last, interactionMessage(known: true, busy: false, selecting: false));
      await i.request(true);
      expect(calls.last, 'POST /tmux-mouse {"session":"term-r1","enabled":false}');
      expect(posted.last, interactionMessage(known: true, busy: false, selecting: true));
      expect(i.temporary, isTrue);
      await i.restore();
      expect(calls.last, 'POST /tmux-mouse {"session":"term-r1","enabled":true}');
      expect(i.temporary, isFalse);
      final int n = calls.length;
      await i.restore();
      expect(calls.length, n, reason: 'nothing to restore');
    });
  });

  group('widgets', () {
    late List<FakeSurface> surfaces;
    late FakeStore store;
    late List<String> requests;
    late ValueNotifier<bool> visible;

    /// testWidgets that also fails if anything printed contains the token.
    void testWeb(String name, Future<void> Function(WidgetTester tester) body) {
      testWidgets(name, (WidgetTester tester) async {
        final List<String> logs = <String>[];
        final DebugPrintCallbackType saved = debugPrint;
        debugPrint = (String? m, {int? wrapWidth}) => logs.add(m ?? '');
        try {
          await body(tester);
        } finally {
          debugPrint = saved;
        }
        for (final String l in logs) {
          expect(l, isNot(contains(kToken)));
        }
      });
    }

    setUp(() {
      surfaces = <FakeSurface>[];
      store = FakeStore();
      requests = <String>[];
      visible = ValueNotifier<bool>(true);
      WebPorts.surfaces = (WebSurfaceEvents e) {
        final FakeSurface s = FakeSurface(e);
        surfaces.add(s);
        return s;
      };
      WebPorts.store = store;
      WebPorts.server = () => kServer;
      WebPorts.visibility = () => visible;
      WebPorts.request = (String m, String p, {Map<String, String>? query, Map<String, Object?>? body}) async {
        requests.add('$m $p');
        if (p == '/prefs') return const ApiResponse(200, <String, Object?>{'theme': 'noche', 'button_style': 'sutil'});
        if (p == '/tmux-mouse') return const ApiResponse(200, <String, Object?>{'mouse': 'on'});
        return const ApiResponse(404, null);
      };
    });

    tearDown(() {
      TerminalEngine.resetForTest();
      WebSession.resetForTest();
      WebPorts.reset();
      webDataDroppers.clear();
      TerminalBridge.active.value = null;
    });

    Widget app(Widget home) => MaterialApp(theme: buildTheme(), home: home);

    Future<void> openPanel(WidgetTester tester, WebPanel panel) async {
      await tester.pumpWidget(app(Builder(
        builder: (BuildContext c) => Center(
          child: GestureDetector(onTap: () => openWebPanel(c, panel), child: const Text('abrir')),
        ),
      )));
      await tester.tap(find.text('abrir'));
      await tester.pump();
      await tester.pump();
    }

    testWeb('a panel opens its page under the thin bar and closes with the page', (WidgetTester tester) async {
      await openPanel(tester, WebPanel.settings);
      expect(find.text('AJUSTES'), findsOneWidget);
      expect(find.text('Cargando…'), findsOneWidget);
      expect(surfaces, hasLength(1));
      final FakeSurface s = surfaces.single;
      expect(s.loads.single.toString(), 'https://mi-equipo.mi-red.ts.net/?panel=settings');
      expect(store.calls, <String>['cookie mi-equipo.mi-red.ts.net TOKEN']);
      expect(WebSession.users, 1);
      // Only the paired host is reachable from the page.
      expect(s.events.allow(Uri.parse('https://evil.example/'), mainFrame: true), isFalse);

      s.events.onPageFinished();
      expect(s.scripts.single, panelScript(WebPanel.settings));
      s.send(<String, Object?>{'t': 'open'});
      await tester.pump();
      expect(find.text('Cargando…'), findsNothing);
      expect(find.byKey(const Key('fake-web')), findsOneWidget);

      // The page's own ✕ closes the route; the WebView and the cookie go too.
      s.send(<String, Object?>{'t': 'closed'});
      await tester.pumpAndSettle();
      expect(find.text('AJUSTES'), findsNothing);
      expect(s.disposed, isTrue);
      expect(WebSession.users, 0);
      expect(store.calls.last, 'clearCookies');
    });

    testWeb('error state retries; back closes', (WidgetTester tester) async {
      await openPanel(tester, WebPanel.servers);
      final FakeSurface s = surfaces.single;
      s.events.onMainFrameError();
      await tester.pump();
      expect(find.text('No se pudo abrir Servidores.'), findsOneWidget);
      await tester.tap(find.text('Reintentar'));
      await tester.pump();
      expect(s.loads, hasLength(2));
      expect(find.text('Cargando…'), findsOneWidget);
      await tester.tap(find.byTooltip('Volver'));
      await tester.pumpAndSettle();
      expect(find.text('SERVIDORES'), findsNothing);
      expect(s.disposed, isTrue);
    });

    testWeb('a page that never shows its panel ends in the error state', (WidgetTester tester) async {
      await openPanel(tester, WebPanel.news);
      expect(surfaces.single.loads.single.query, 'panel=news&news=');
      await tester.pump(kPageReadyTimeout + const Duration(seconds: 6));
      expect(find.text('Reintentar'), findsOneWidget);
      await tester.tap(find.byTooltip('Volver'));
      await tester.pumpAndSettle();
    });

    Future<void> finish(WidgetTester tester) async {
      await tester.pumpWidget(const SizedBox());
      TerminalEngine.resetForTest();
      await tester.pump();
    }

    Widget pane(String session, {String? pane, bool show = true}) =>
        app(show ? TerminalPane(session: session, pane: pane) : const SizedBox());

    testWeb('terminal: one WebView, docked, follows session and pane', (WidgetTester tester) async {
      await tester.pumpWidget(pane('term-r1', pane: '%3'));
      await tester.pump();
      expect(surfaces, hasLength(1));
      final FakeSurface s = surfaces.single;
      expect(redactedUrl(s.loads.single),
          'https://mi-equipo.mi-red.ts.net/term/?auth=***&arg=term-r1&theme=noche&btn=sutil');
      expect(s.loads.single.queryParameters['auth'], kToken);
      expect(TerminalBridge.active.value, isNull);

      s.events.onPageFinished();
      expect(s.scripts.last, terminalScript());
      s.send(<String, Object?>{'t': 'ready'});
      await tester.pump();
      expect(s.posted.first, kDockMessage);
      expect(s.posted, anyElement(equals(selectPaneMessage('%3'))));
      expect(requests, containsAll(<String>['GET /prefs', 'GET /tmux-mouse']));
      expect(s.posted.last['type'], 'interaction-state');
      expect(TerminalBridge.active.value, isNotNull);
      expect(TerminalBridge.active.value!.action('history'), isTrue);
      expect(s.posted.last, toolbarMessage(<String, Object?>{'type': 'action', 'action': 'history'}));
      expect(TerminalBridge.active.value!.action('rm -rf'), isFalse);

      // «Seleccionar» from the page → tmux mouse off.
      s.send(<String, Object?>{'t': 'term', 'type': 'interaction-request', 'selecting': true});
      await tester.pump();
      expect(requests.last, 'POST /tmux-mouse');

      await tester.pumpWidget(pane('term-r1', pane: '%4'));
      expect(s.posted.last, selectPaneMessage('%4'));

      await tester.pumpWidget(pane('term-r2'));
      await tester.pump();
      expect(surfaces, hasLength(1), reason: 'same WebView navigates');
      expect(s.loads.last.queryParameters['arg'], 'term-r2');
      expect(requests.where((String r) => r == 'POST /tmux-mouse'), hasLength(2),
          reason: 'selection of the old session restored');

      await finish(tester);
    });

    testWeb('terminal kept warm after leaving, then destroyed', (WidgetTester tester) async {
      await tester.pumpWidget(pane('term-r1'));
      await tester.pump();
      final FakeSurface s = surfaces.single;
      s.send(<String, Object?>{'t': 'ready'});
      await tester.pump();
      expect(TerminalBridge.active.value, isNotNull);

      await tester.pumpWidget(pane('term-r1', show: false));
      await tester.pump();
      expect(TerminalBridge.active.value, isNull);
      expect(s.disposed, isFalse);

      await tester.pump(const Duration(seconds: 10));
      await tester.pumpWidget(pane('term-r1'));
      await tester.pump();
      expect(surfaces, hasLength(1), reason: 'warm WebView reused');
      expect(s.loads, hasLength(1), reason: 'same session: no reload');
      expect(s.views, 2);

      await tester.pumpWidget(pane('term-r1', show: false));
      await tester.pump(const Duration(seconds: 31));
      expect(s.disposed, isTrue);
      expect(TerminalEngine.live, isNull);
      expect(WebSession.users, 0);
      expect(store.calls.last, 'clearCookies');

      await tester.pumpWidget(pane('term-r1'));
      await tester.pump();
      expect(surfaces, hasLength(2));
      await finish(tester);
    });

    testWeb('terminal released while the app stays in the background', (WidgetTester tester) async {
      await tester.pumpWidget(pane('term-r1'));
      await tester.pump();
      final FakeSurface s = surfaces.single;
      visible.value = false;
      await tester.pump(const Duration(seconds: 31));
      expect(s.disposed, isTrue);
      expect(find.byKey(const Key('fake-web')), findsNothing);
      visible.value = true;
      await tester.pump();
      await tester.pump();
      expect(surfaces, hasLength(2));
      expect(find.byKey(const Key('fake-web')), findsOneWidget);
      await finish(tester);
    });

    testWeb('terminal error shows Reintentar and reloads', (WidgetTester tester) async {
      await tester.pumpWidget(pane('term-r1'));
      await tester.pump();
      final FakeSurface s = surfaces.single;
      s.events.onMainFrameError();
      await tester.pump();
      await tester.pump();
      expect(find.text('No pude conectar con la terminal.'), findsOneWidget);
      await tester.tap(find.text('Reintentar'));
      await tester.pump();
      expect(s.loads, hasLength(2));
      await finish(tester);
    });

    testWeb('clearWebData drops the terminal and wipes the store', (WidgetTester tester) async {
      await tester.pumpWidget(pane('term-r1'));
      await tester.pump();
      final FakeSurface s = surfaces.single;
      WebPorts.server = () => null; // unpaired
      await clearWebData();
      await tester.pump();
      expect(s.disposed, isTrue);
      expect(store.calls, contains('clearAll'));
      expect(surfaces, hasLength(1), reason: 'no desktop: nothing restarts');
    });

    test('no WebView before a panel or terminal is shown', () {
      int created = 0;
      WebPorts.surfaces = (WebSurfaceEvents e) {
        created++;
        return FakeSurface(e);
      };
      expect(TerminalEngine.live, isNull);
      expect(created, 0);
      WebPorts.reset();
    });
  });

  test('webPanelTitle copy', () {
    expect(WebPanel.values.map(webPanelTitle).toList(),
        <String>['Ajustes', 'Servidores', 'Snippets', 'Resúmenes', 'Remoto', 'ComandOS']);
  });
}
