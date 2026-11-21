import 'dart:convert';

import 'package:comandos_mobile/core/api/api_client.dart';
import 'package:comandos_mobile/core/config/credential_store.dart';
import 'package:comandos_mobile/core/config/paired_server.dart';
import 'package:comandos_mobile/core/connection.dart';
import 'package:comandos_mobile/core/locator.dart';
import 'package:comandos_mobile/core/models/usage_limits.dart';
import 'package:comandos_mobile/core/polling/app_visibility.dart';
import 'package:comandos_mobile/features/usage/usage_summary.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

// `GET /analytics/week?offset=0&sidebar=1` of the live dashboard (21-nov-2026),
// aliases anonymised.
const String weekJson =
    r'''{"accounts":[{"id":"claude:main","provider":"claude","cli":"Claude","alias":"main","color":"#8B7CFF"'''
    r''',"week":6,"model":{"n":"Fable","v":8,"reset":"sáb 28 nov, 01:59","left":"6d 11h"},"h5":null,"reset":'''
    r'''"sáb 28 nov, 02:00","left":"6d 11h","h5Reset":null,"h5Left":null,"hoy":{"h":2.8206555554601884,"tok"'''
    r''':82,"ses":12},"sem":{"h":122.25188416679713,"tok":6.6,"ses":439},"weekUsed":6,"measured":{"sessions"'''
    r''':36,"tokens":6550342854,"costUsd":0.0,"models":["claude-fable-5-1","claude-fable-5-2","claude-haiku-'''
    r'''5-5","claude-opus-5-5","claude-sonnet-5-5"]}},{"id":"claude:work","provider":"claude","cli":"Claude"'''
    r''',"alias":"work","color":"#2fd3c0","week":0,"model":{"n":"Fable","v":0,"reset":"sáb 28 nov, 03:00","l'''
    r'''eft":"6d 12h"},"h5":null,"reset":"sáb 28 nov, 03:00","left":"6d 12h","h5Reset":null,"h5Left":null,"h'''
    r'''oy":{"h":3.1610966666539504,"tok":105,"ses":5},"sem":{"h":46.63217666665713,"tok":1.6,"ses":140},"we'''
    r'''ekUsed":0,"measured":{"sessions":18,"tokens":1593095872,"costUsd":0.0,"models":["claude-fable-5-2","'''
    r'''claude-haiku-5-5","claude-opus-5-5","claude-sonnet-5-5"]}},{"id":"claude:side","provider":"claude","'''
    r'''cli":"Claude","alias":"side","color":"#FF6B5B","week":45,"model":{"n":"Fable","v":53,"reset":"sáb 21'''
    r''' nov, 19:00","left":"4h 8m"},"h5":9,"reset":"sáb 21 nov, 19:00","left":"4h 8m","h5Reset":"sáb 21 nov'''
    r''', 19:40","h5Left":"4h 48m","hoy":{"h":0.1528636111153503,"tok":24,"ses":2},"sem":{"h":18.96893222232'''
    r'''6593,"tok":0.9,"ses":94},"weekUsed":45,"measured":{"sessions":8,"tokens":934631653,"costUsd":0.0,"mo'''
    r'''dels":["claude-fable-5-2","claude-opus-4-8","claude-opus-5-5"]}},{"id":"agy:main","provider":"agy","'''
    r'''cli":"Antigravity","alias":"main","color":"#5BD6A0","week":2,"model":{"n":"Claude·GPT","v":0,"reset"'''
    r''':"mié 25 nov, 10:46","left":"3d 19h"},"h5":0,"reset":"mar 24 nov, 21:08","left":"3d 6h","h5Reset":nu'''
    r'''ll,"h5Left":null,"hoy":{"h":0,"tok":0,"ses":0},"sem":{"h":0,"tok":0.0,"ses":0},"weekUsed":2,"plan":"'''
    r'''Google AI Pro"},{"id":"opencode:main","provider":"opencode","cli":"OpenCode","alias":"main","color":'''
    r'''"#2fd3c0","week":null,"model":null,"h5":null,"measured":{"sessions":143,"tokens":5141109,"costUsd":0'''
    r'''.0,"models":["opencode/big-pickle"]}}]}''';

List<UsageAccount> accounts() => parseUsageAccounts(jsonDecode(weekJson))!;

void main() {
  group('usage model (real /analytics/week)', () {
    test('limits: Semana, the model cap, 5 horas; rounded and in order', () {
      final List<UsageAccount> a = accounts();
      expect(a.map((UsageAccount x) => x.id).toList(),
          <String>['claude:main', 'claude:work', 'claude:side', 'agy:main', 'opencode:main']);
      final UsageAccount side = a[2];
      expect(side.limits.map((UsageLimit l) => '${l.name} ${l.used}').toList(),
          <String>['Semana 45', 'Fable 53', '5 horas 9']);
      expect(side.peak!.used, 53);
      expect(side.limits[0].resetLine, 'se reinicia en 4h 8m');
      expect(side.limits[2].resetLine, 'se reinicia en 4h 48m');
      expect(side.limits[0].segments, 9);
      expect(a[0].today, '2.8 h · 12 ses');
      expect(a[0].week, '122.3 h · 439 ses');
    });

    test('detail rows: limits with reset date, Hoy, Semana, Plan; never money', () {
      final List<UsageAccount> a = accounts();
      expect(a[0].details, <(String, String)>[
        ('Semana', '6% · reinicia sáb 28 nov, 02:00'),
        ('Fable', '8% · reinicia sáb 28 nov, 01:59'),
        ('Hoy', '2.8 h · 12 ses'),
        ('Semana', '122.3 h · 439 ses'),
      ]);
      expect(a[3].details.last, ('Plan', 'Google AI Pro'));
      final UsageAccount free = a[4];
      expect(free.free, isTrue);
      expect(free.sessions, '143');
      expect(free.tokens, '5.1M');
      expect(free.details, <(String, String)>[('Modelo', 'opencode/big-pickle')]);
      for (final UsageAccount x in a) {
        for (final (String k, String v) in x.details) {
          expect('$k $v', isNot(contains(r'$')));
          expect(k.toLowerCase(), isNot(contains('costo')));
        }
      }
    });

    test('tone, colour and odd values', () {
      expect(usageTone(69), UsageTone.normal);
      expect(usageTone(70), UsageTone.warn);
      expect(usageTone(90), UsageTone.bad);
      expect(accountLimits(<String, Object?>{'week': '90', 'h5': null}), isEmpty);
      expect(accountLimits(<String, Object?>{'week': 999}).single.used, 100);
      expect(accountLimits(<String, Object?>{'week': 44.5}).single.used, 45);
      expect(const UsageAccount(id: 'x', provider: '', cli: '', alias: '', color: '#2fd3c0').colorHex, '#2fd3c0');
      expect(const UsageAccount(id: 'x', provider: '', cli: '', alias: '', color: 'red').colorHex, isNull);
      expect(tokenText(6550342854), '6.6B');
      expect(tokenText(null), '—');
      expect(hours(null), '—');
      expect(parseUsageAccounts(<String, Object?>{'accounts': <Object?>[<String, Object?>{'id': 3}]}), isEmpty);
    });
  });

  group('UsageSummary', () {
    late int calls;
    Map<String, String>? query;
    late http.Response Function() answer;

    setUp(() async {
      UsageCache.reset();
      sl.reset();
      calls = 0;
      answer = () => http.Response.bytes(utf8.encode(weekJson), 200,
          headers: <String, String>{'content-type': 'application/json; charset=utf-8'});
      final MockClient client = MockClient((http.Request r) async {
        if (r.url.path == '/analytics/week') {
          query = r.url.queryParameters;
          calls++;
          return answer();
        }
        return http.Response('{"error":"Not found"}', 404);
      });
      final MemoryCredentialStore creds = MemoryCredentialStore()
        ..value = const PairedServer(host: 'mi-equipo.mi-red.ts.net', token: 'TOKEN');
      final Connection connection =
          Connection(creds, apiFactory: (PairedServer s) => ComandosApi(s, client: client));
      await connection.load();
      sl
        ..register<Connection>(connection)
        ..register<AppVisibility>(AppVisibility());
    });

    tearDown(sl.reset);

    Future<void> pump(WidgetTester tester) async {
      UsageCache.now = () => tester.binding.clock.now().millisecondsSinceEpoch;
      tester.view.physicalSize = const Size(412, 915);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(const MaterialApp(home: Scaffold(body: UsageSummary())));
    }

    testWidgets('loading, then one card per account with meters and no money', (WidgetTester tester) async {
      await pump(tester);
      expect(find.text('Leyendo el uso…'), findsOneWidget);
      await tester.pumpAndSettle();
      expect(find.text('USO DE TUS CUENTAS'), findsOneWidget);
      expect(find.text('% usado'), findsOneWidget);
      expect(find.text('Claude'), findsNWidgets(3));
      expect(find.text('Antigravity'), findsOneWidget);
      expect(find.text('53%'), findsNWidgets(2)); // big number + Fable meter
      expect(find.text('5 horas'), findsNWidgets(2));
      expect(find.text('se reinicia en 4h 8m'), findsNWidgets(2));
      expect(find.text('SIN CUOTA'), findsOneWidget);
      expect(find.text('sesiones · semana'), findsOneWidget);
      expect(find.textContaining(r'$'), findsNothing);
      expect(find.textContaining('costo'), findsNothing);
      expect(find.textContaining('Costo'), findsNothing);
      expect(calls, 1);
      expect(query, <String, String>{'offset': '0', 'sidebar': '1'});
    });

    testWidgets('tapping a card opens its detail; tapping again closes it', (WidgetTester tester) async {
      await pump(tester);
      await tester.pumpAndSettle();
      expect(find.text('Hoy'), findsNothing);
      await tester.tap(find.text('side'));
      await tester.pump();
      expect(find.text('Hoy'), findsOneWidget);
      expect(find.text('45% · reinicia sáb 21 nov, 19:00'), findsOneWidget);
      await tester.tap(find.text('side'));
      await tester.pump();
      expect(find.text('Hoy'), findsNothing);
    });

    testWidgets('re-reads once a minute; an unchanged answer rebuilds nothing', (WidgetTester tester) async {
      await pump(tester);
      await tester.pumpAndSettle();
      expect(calls, 1);
      await tester.pump(const Duration(seconds: 30));
      expect(calls, 1);
      int rebuilt = 0;
      debugOnRebuildDirtyWidget = (Element e, bool builtOnce) => rebuilt++;
      await tester.pump(const Duration(seconds: 31));
      await tester.pumpAndSettle();
      debugOnRebuildDirtyWidget = null;
      expect(calls, 2);
      expect(rebuilt, 0);
    });

    testWidgets('a failure retries after 15 s; empty list says so', (WidgetTester tester) async {
      answer = () => http.Response('{"error":"Consumos no disponible"}', 503);
      await pump(tester);
      await tester.pumpAndSettle();
      expect(find.text('Leyendo el uso…'), findsOneWidget);
      answer = () => http.Response('{"accounts":[]}', 200);
      await tester.pump(const Duration(seconds: 16));
      await tester.pumpAndSettle();
      expect(calls, 2);
      expect(find.text('Sin límites que mostrar'), findsOneWidget);
    });

    testWidgets('stops polling while the app is hidden', (WidgetTester tester) async {
      await pump(tester);
      await tester.pumpAndSettle();
      sl.get<AppVisibility>().value = false;
      await tester.pump(const Duration(minutes: 3));
      expect(calls, 1);
      sl.get<AppVisibility>().value = true;
      await tester.pump();
      await tester.pump(const Duration(seconds: 1));
      expect(calls, 2);
    });
  });
}
