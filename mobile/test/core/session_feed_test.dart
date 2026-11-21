import 'dart:convert';

import 'package:comandos_mobile/core/models/session_feed.dart';
import 'package:comandos_mobile/core/models/transcript.dart';
import 'package:comandos_mobile/features/sessions/sections.dart';
import 'package:flutter_test/flutter_test.dart';

/// Shapes from the API contract (`GET /session-feed`).
const String feedJson = '''
{"now": 1794250700.0,
 "sessions": [
  {"session":"term-r1","label":"Dueño","state":"idle","unread":false,"ts":1794250100.0,
   "seen":1794250000.0,"pane":"%1","panes":1,"agent":"claude","model":"opus-5",
   "prompt":"pushea todo","reply":"Listo.","ask":null,
   "paneList":[{"pane":"%1","state":"idle","ts":1794250100.0,"agent":"claude","model":"opus-5","account":"main","reply":"Listo."}]},
  {"session":"term-r2","label":"Pagos","state":"ask","unread":true,"ts":1794250552.0,
   "pane":"%51","panes":2,"agent":"claude","model":"opus-5","prompt":"","reply":"",
   "ask":{"question":"Do you want to proceed?","options":[{"key":"1","label":"Yes"},{"key":"2","label":"No"}]},
   "paneList":[]},
  {"session":"term-r3","label":"Web","state":"work","unread":false,"ts":1794250690.0,"agent":"codex","model":"gpt"},
  {"session":"term-r4","label":"Docs","state":"idle","unread":true,"ts":1794250300.0,"reply":"Terminé"},
  {"session":"term-r5","label":"Cuota","state":"limit","unread":true,"ts":1794250200.0},
  {"session":"term-r6","label":"Viejo","state":"idle","unread":false,"ts":1794240000.0},
  {"label":"sin nombre de sesión"},
  "basura"
 ]}
''';

void main() {
  test('parses the session-feed shape and skips malformed entries', () {
    final SessionFeed feed = SessionFeed.tryParse(jsonDecode(feedJson))!;
    expect(feed.now, 1794250700.0);
    expect(feed.sessions.map((SessionSummary s) => s.session),
        <String>['term-r1', 'term-r2', 'term-r3', 'term-r4', 'term-r5', 'term-r6']);
    final SessionSummary r1 = feed.sessions.first;
    expect(r1.label, 'Dueño');
    expect(r1.pane, '%1');
    expect(r1.agentModel, 'claude · opus-5');
    expect(r1.paneList.single.account, 'main');
    final SessionSummary r2 = feed.sessions[1];
    expect(r2.state, SessionState.ask);
    expect(r2.ask!.options.map((o) => o.key), <String>['1', '2']);
    expect(r2.panes, 2);
  });

  test('wrong types degrade to defaults instead of throwing', () {
    final SessionFeed? feed = SessionFeed.tryParse(jsonDecode(
        '{"now":"x","sessions":[{"session":"s","state":7,"unread":"yes","ts":null,"ask":{"options":[{"key":"x"}]}}]}'));
    expect(feed, isNotNull);
    final SessionSummary s = feed!.sessions.single;
    expect(s.state, SessionState.idle);
    expect(s.unread, isFalse);
    expect(s.ask, isNull);
    expect(SessionFeed.tryParse('nope'), isNull);
    expect(SessionFeed.tryParse(<String, Object?>{'now': 1}), isNull);
  });

  test('signature changes only when something visible changes', () {
    final SessionFeed a = SessionFeed.tryParse(jsonDecode(feedJson))!;
    final SessionFeed b = SessionFeed.tryParse(jsonDecode(feedJson))!;
    expect(a.sessions.first.signature, b.sessions.first.signature);
    final Map<String, Object?> m = (jsonDecode(feedJson) as Map<String, Object?>);
    ((m['sessions']! as List<Object?>).first! as Map<String, Object?>)['reply'] = 'Otra cosa';
    final SessionFeed c = SessionFeed.tryParse(m)!;
    expect(c.sessions.first.signature, isNot(a.sessions.first.signature));
  });

  test('feed layout: «Te espera» pinned oldest first, the rest in time buckets newest first', () {
    final SessionFeed feed = SessionFeed.tryParse(jsonDecode(feedJson))!;
    // Day starts 1 h before `now`: r6 (≈3 h ago) falls in «Antes».
    final FeedLayout lay = feedLayout(feed.sessions, FeedChip.all, feed.now, feed.now - 3600 - 1);
    expect(lay.pinned.map((SessionSummary s) => s.session), <String>['term-r5', 'term-r2']);
    expect(lay.groups.map(((FeedGroup, List<SessionSummary>) g) => g.$1),
        <FeedGroup>[FeedGroup.now, FeedGroup.lastHour, FeedGroup.before]);
    expect(lay.groups[0].$2.single.session, 'term-r3'); // 10 s ago
    expect(lay.groups[1].$2.map((SessionSummary s) => s.session), <String>['term-r4', 'term-r1']);
    expect(lay.groups[2].$2.single.session, 'term-r6');
    expect(FeedGroup.values.map((FeedGroup g) => g.label), <String>['Ahora', 'Última hora', 'Hoy', 'Antes']);
  });

  test('feed chips filter and count like the web', () {
    final SessionFeed feed = SessionFeed.tryParse(jsonDecode(feedJson))!;
    final FeedCounts c = feedCounts(feed.sessions);
    expect(<int>[c.all, c.unread, c.perm, c.work, c.waiting], <int>[6, 3, 1, 1, 2]);
    final FeedLayout perm = feedLayout(feed.sessions, FeedChip.perm, feed.now, 0);
    expect(perm.pinned.single.session, 'term-r2');
    expect(perm.groups, isEmpty);
    expect(feedLayout(feed.sessions, FeedChip.work, feed.now, 0).groups.single.$2.single.session, 'term-r3');
  });

  test('parses the chat transcript shapes', () {
    final TranscriptResponse t = TranscriptResponse.tryParse(jsonDecode('''
{"ok":true,"agent":"claude","pane":"%51","token":"1:2:3","age":12,
 "messages":[{"r":"u","t":"revisa las reglas","h":"<div>"},
             {"r":"a","t":"Voy.","h":"<div>"},
             {"r":"t","t":"Bash · ls -la\\nRead · /a.rs","h":"<div>"},
             {"r":"x","t":"desconocido"}],
 "ask":{"question":"Do you want to proceed?","detail":["Bash command","","git push origin main"],
        "options":[{"key":"1","label":"Yes","on":true},{"key":"2","label":"No","on":false}]}}
'''))!;
    expect(t.unchanged, isFalse);
    expect(t.messages.map((ChatMessage m) => m.role),
        <ChatRole>[ChatRole.user, ChatRole.assistant, ChatRole.tool]);
    expect(t.ask!.options.first.highlighted, isTrue);
    expect(t.ask!.detail.last, 'git push origin main');
    expect(t.age, 12);

    final TranscriptResponse u =
        TranscriptResponse.tryParse(jsonDecode('{"ok":true,"unchanged":true,"token":"t","pane":"%51","age":3,"ask":null}'))!;
    expect(u.unchanged, isTrue);
    expect(u.messages, isEmpty);
    final TranscriptResponse none =
        TranscriptResponse.tryParse(jsonDecode('{"ok":true,"agent":null,"pane":null,"messages":[]}'))!;
    expect(none.agent, isNull);
  });
}
