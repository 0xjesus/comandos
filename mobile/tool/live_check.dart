// Read-only check of the app's parsers against a real ComandOS server.
//
//   dart run tool/live_check.dart [http://127.0.0.1:4777]
//
// Calls only the READ routes the app uses and parses every answer with the
// app's own model classes. It prints shapes, counts and field names, never
// values (no project names, messages or tokens). It never calls a route that
// types, sends, kills or marks anything: see [_allowed].
//
// Loopback needs no token. Do not point this at a remote host.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:comandos_mobile/core/api/api_response.dart';
import 'package:comandos_mobile/core/models/session_feed.dart';
import 'package:comandos_mobile/core/models/transcript.dart';
import 'package:comandos_mobile/core/outbox/send_result.dart';
import 'package:comandos_mobile/features/notifications/notice.dart';

/// The only (method, path) pairs this script may call.
const Set<String> _allowed = <String>{
  'GET /session-feed',
  'POST /chat/transcript', // reads the transcript tail; changes nothing
  'GET /notices',
  'GET /notices/watch',
  'GET /session/message', // receipt lookup by a random id: always unknown
};

final List<String> _problems = <String>[];
void _problem(String s) {
  _problems.add(s);
  stdout.writeln('  ✗ $s');
}

void _ok(String s) => stdout.writeln('  ✓ $s');
void _info(String s) => stdout.writeln('  · $s');

late Uri _base;
final HttpClient _http = HttpClient()..connectionTimeout = const Duration(seconds: 5);

Future<(int, Object?, Duration)> _call(String method, String path,
    {Map<String, String>? query, Map<String, Object?>? body}) async {
  if (!_allowed.contains('$method $path')) {
    throw StateError('refusing $method $path: not a read route');
  }
  final Stopwatch sw = Stopwatch()..start();
  final HttpClientRequest req =
      await _http.openUrl(method, _base.replace(path: path, queryParameters: query));
  req.headers.set('Accept', 'application/json');
  if (body != null) {
    // Content-Length, not chunked: the server does not read chunked bodies
    // (the app's package:http client always sends a length).
    final List<int> bytes = utf8.encode(jsonEncode(body));
    req.headers.contentType = ContentType.json;
    req.contentLength = bytes.length;
    req.add(bytes);
  }
  final HttpClientResponse res = await req.close().timeout(const Duration(seconds: 40));
  final String text = await res.transform(utf8.decoder).join();
  sw.stop();
  Object? json;
  if (text.isNotEmpty) {
    try {
      json = jsonDecode(text);
    } on FormatException {
      json = const _NotJson();
    }
  }
  return (res.statusCode, json, sw.elapsed);
}

class _NotJson {
  const _NotJson();
}

String _type(Object? v) => switch (v) {
      null => 'null',
      bool _ => 'bool',
      int _ => 'int',
      double _ => 'double',
      String _ => 'string',
      List<Object?> _ => 'list',
      Map<Object?, Object?> _ => 'map',
      _ => v.runtimeType.toString(),
    };

/// field → {type → count}, over many objects.
class _Shape {
  final Map<String, Map<String, int>> fields = <String, Map<String, int>>{};
  int objects = 0;

  void add(Map<String, Object?> m) {
    objects++;
    m.forEach((String k, Object? v) {
      final Map<String, int> t = fields.putIfAbsent(k, () => <String, int>{});
      t[_type(v)] = (t[_type(v)] ?? 0) + 1;
    });
  }

  /// Checks every [expected] field's types; reports unknown ones as info.
  void check(String what, Map<String, Set<String>> expected, {Set<String> required = const <String>{}}) {
    for (final MapEntry<String, Set<String>> e in expected.entries) {
      final Map<String, int>? seen = fields[e.key];
      if (seen == null) {
        if (required.contains(e.key)) {
          _problem('$what.${e.key}: missing in all $objects objects');
        } else {
          _info('$what.${e.key}: absent (the app uses a default)');
        }
        continue;
      }
      final int present = seen.values.fold(0, (int a, int b) => a + b);
      if (required.contains(e.key) && present < objects) {
        _problem('$what.${e.key}: missing in ${objects - present}/$objects objects');
      }
      for (final MapEntry<String, int> t in seen.entries) {
        if (!e.value.contains(t.key)) {
          _problem('$what.${e.key}: ${t.key} in ${t.value}/$objects (app expects ${e.value.join('|')})');
        }
      }
    }
    final List<String> extra = fields.keys.where((String k) => !expected.containsKey(k)).toList()..sort();
    if (extra.isNotEmpty) _info('$what: fields the app ignores: ${extra.join(', ')}');
  }
}

Map<String, Object?> _map(Object? v) => (v as Map<Object?, Object?>).cast<String, Object?>();

Future<void> main(List<String> args) async {
  _base = Uri.parse(args.isNotEmpty ? args.first : 'http://127.0.0.1:4777');
  if (!<String>{'127.0.0.1', 'localhost', '::1'}.contains(_base.host)) {
    stderr.writeln('This script only talks to a loopback server.');
    exitCode = 2;
    return;
  }
  stdout.writeln('ComandOS live check against ${_base.scheme}://${_base.host}:${_base.port}');
  try {
    final List<SessionSummary> sessions = await _feed();
    await _transcripts(sessions);
    final Set<String> names = <String>{for (final SessionSummary s in sessions) s.session};
    await _notices(names);
    await _receipt();
  } on SocketException catch (e) {
    _problem('cannot reach the server (${e.osError?.message ?? e.message})');
  } finally {
    _http.close(force: true);
  }
  stdout.writeln('');
  if (_problems.isEmpty) {
    stdout.writeln('All parsed. No problems.');
  } else {
    stdout.writeln('${_problems.length} problem(s):');
    for (final String p in _problems) {
      stdout.writeln(' - $p');
    }
    exitCode = 1;
  }
}

Future<List<SessionSummary>> _feed() async {
  stdout.writeln('\nGET /session-feed');
  final (int status, Object? json, Duration t) = await _call('GET', '/session-feed');
  _info('status $status in ${t.inMilliseconds} ms');
  if (status != 200) {
    _problem('/session-feed answered $status');
    return const <SessionSummary>[];
  }
  final SessionFeed? feed = SessionFeed.tryParse(json);
  if (feed == null) {
    _problem('/session-feed: SessionFeed.tryParse returned null (top type ${_type(json)})');
    return const <SessionSummary>[];
  }
  final Map<String, Object?> top = _map(json);
  _ShapeTop(top).check('feed', <String, Set<String>>{
    'now': <String>{'double', 'int'},
    'sessions': <String>{'list'},
  });
  final List<Object?> raw = top['sessions']! as List<Object?>;
  _ok('parsed ${feed.sessions.length}/${raw.length} sessions');
  if (feed.sessions.length != raw.length) {
    _problem('${raw.length - feed.sessions.length} session(s) dropped by the parser');
  }
  final _Shape s = _Shape();
  final _Shape pane = _Shape();
  final _Shape ask = _Shape();
  final _Shape opt = _Shape();
  final Map<String, int> states = <String, int>{};
  for (final Object? o in raw) {
    if (o is! Map) continue;
    final Map<String, Object?> m = o.cast<String, Object?>();
    s.add(m);
    final Object? st = m['state'];
    states['$st'] = (states['$st'] ?? 0) + 1;
    for (final Object? p in (m['paneList'] is List ? m['paneList']! as List<Object?> : const <Object?>[])) {
      if (p is Map) pane.add(p.cast<String, Object?>());
    }
    if (m['ask'] is Map) {
      final Map<String, Object?> a = _map(m['ask']);
      ask.add(a);
      for (final Object? x in (a['options'] is List ? a['options']! as List<Object?> : const <Object?>[])) {
        if (x is Map) opt.add(x.cast<String, Object?>());
      }
    }
  }
  _info('states: $states');
  for (final String st in states.keys) {
    if (!<String>{'ask', 'limit', 'work', 'idle'}.contains(st)) _problem('unknown session state "$st"');
  }
  s.check('session', <String, Set<String>>{
    'session': <String>{'string'},
    'label': <String>{'string'},
    'state': <String>{'string'},
    'unread': <String>{'bool'},
    'ts': <String>{'double', 'int'},
    'seen': <String>{'double', 'int', 'null'},
    'pane': <String>{'string', 'null'},
    'panes': <String>{'int'},
    'agent': <String>{'string', 'null'},
    'model': <String>{'string', 'null'},
    'prompt': <String>{'string', 'null'},
    'reply': <String>{'string', 'null'},
    'ask': <String>{'map', 'null'},
    'paneList': <String>{'list'},
  }, required: <String>{'session', 'state', 'ts'});
  if (pane.objects > 0) {
    pane.check('paneList[]', <String, Set<String>>{
      'pane': <String>{'string'},
      'state': <String>{'string'},
      'ts': <String>{'double', 'int'},
      'agent': <String>{'string', 'null'},
      'model': <String>{'string', 'null'},
      'account': <String>{'string', 'null'},
      'reply': <String>{'string', 'null'},
    }, required: <String>{'pane'});
  }
  if (ask.objects > 0) {
    ask.check('ask', <String, Set<String>>{
      'question': <String>{'string'},
      'detail': <String>{'list', 'null'},
      'options': <String>{'list'},
    });
    opt.check('ask.options[]', <String, Set<String>>{
      'key': <String>{'string'},
      'label': <String>{'string'},
      'on': <String>{'bool'},
    });
  } else {
    _info('no session has an open permission menu right now (ask not exercised)');
  }
  // Values the app derives: empty labels fall back to the session name.
  final int noPane = feed.sessions.where((SessionSummary x) => x.pane == null).length;
  if (noPane > 0) _info('$noPane session(s) without "pane" (chat starts without one)');
  return feed.sessions;
}

/// Same checks for a single top-level object.
class _ShapeTop extends _Shape {
  _ShapeTop(Map<String, Object?> m) {
    add(m);
  }
}

Future<void> _transcripts(List<SessionSummary> sessions) async {
  stdout.writeln('\nPOST /chat/transcript');
  if (sessions.isEmpty) {
    _info('no sessions: skipped');
    return;
  }
  // Prefer sessions that are not mid-turn, then most recent; at most three.
  final List<SessionSummary> pick = <SessionSummary>[...sessions]
    ..sort((SessionSummary a, SessionSummary b) => b.ts.compareTo(a.ts));
  final _Shape top = _Shape();
  final _Shape msg = _Shape();
  final Map<String, int> roles = <String, int>{};
  int n = 0;
  for (final SessionSummary s in pick.take(3)) {
    n++;
    final (int status, Object? json, Duration t) = await _call('POST', '/chat/transcript',
        body: <String, Object?>{'session': s.session, 'pane': ?s.pane, 'since': ''});
    _info('session #$n: status $status in ${t.inMilliseconds} ms');
    if (status == 429) {
      _info('session #$n: chat busy (429), the app retries in 3 s');
      continue;
    }
    if (status != 200) {
      _problem('session #$n: /chat/transcript answered $status'
          '${json is Map && json['error'] is String ? ' "${json['error']}"' : ''}');
      continue;
    }
    final TranscriptResponse? r = TranscriptResponse.tryParse(json);
    if (r == null) {
      _problem('session #$n: TranscriptResponse.tryParse returned null');
      continue;
    }
    final Map<String, Object?> m = _map(json);
    top.add(m);
    final List<Object?> raw = m['messages'] is List ? m['messages']! as List<Object?> : const <Object?>[];
    for (final Object? e in raw) {
      if (e is! Map) continue;
      final Map<String, Object?> mm = e.cast<String, Object?>();
      msg.add(mm);
      roles['${mm['r']}'] = (roles['${mm['r']}'] ?? 0) + 1;
    }
    _info('session #$n: body ${utf8.encode(jsonEncode(json)).length} bytes');
    _ok('session #$n: ${r.messages.length}/${raw.length} messages parsed, '
        'agent ${r.agent == null ? 'null' : 'set'}, token ${r.token.isEmpty ? 'EMPTY' : 'set'}, '
        'age ${r.age == null ? 'null' : 'int'}, ask ${r.ask == null ? 'none' : 'yes'}');
    final int maxLen = r.messages.fold(0, (int a, ChatMessage x) => max(a, x.text.length));
    _info('session #$n: longest message $maxLen chars');
    if (r.token.isNotEmpty) {
      final (int s2, Object? j2, Duration t2) = await _call('POST', '/chat/transcript',
          body: <String, Object?>{'session': s.session, 'pane': ?(r.pane ?? s.pane), 'since': r.token});
      final TranscriptResponse? again = s2 == 200 ? TranscriptResponse.tryParse(j2) : null;
      if (again == null) {
        _problem('session #$n: second call with since=token answered $s2 / unparsable');
      } else if (again.unchanged) {
        _ok('session #$n: since=token → unchanged (${t2.inMilliseconds} ms, '
            '${again.messages.length} messages in body)');
        if (again.token.isEmpty) _problem('session #$n: unchanged answer has no token');
      } else {
        _info('session #$n: since=token → changed (session is active)');
      }
    }
  }
  if (top.objects == 0) return;
  _info('roles: $roles');
  for (final String r in roles.keys) {
    if (ChatRole.parse(r) == null) _problem('unknown message role "$r" (dropped by the app)');
  }
  top.check('transcript', <String, Set<String>>{
    'ok': <String>{'bool'},
    'agent': <String>{'string', 'null'},
    'pane': <String>{'string', 'null'},
    'token': <String>{'string'},
    'age': <String>{'int', 'double', 'null'},
    'messages': <String>{'list'},
    'ask': <String>{'map', 'null'},
    'unchanged': <String>{'bool'},
  });
  msg.check('message', <String, Set<String>>{
    'r': <String>{'string'},
    't': <String>{'string'},
    'h': <String>{'string', 'null'},
  }, required: <String>{'r', 't'});
}

Future<void> _notices(Set<String> sessionNames) async {
  stdout.writeln('\nGET /notices/watch?rev=&wait=0');
  final (int s0, Object? j0, Duration t0) =
      await _call('GET', '/notices/watch', query: <String, String>{'rev': '', 'wait': '0'});
  _info('status $s0 in ${t0.inMilliseconds} ms');
  NoticeWatch? w = s0 == 200 ? NoticeWatch.tryParse(j0) : null;
  if (w == null) {
    _problem('/notices/watch (wait=0): $s0 / NoticeWatch.tryParse returned null');
    if (j0 is Map) _ShapeTop(_map(j0)).check('watch', const <String, Set<String>>{});
    return;
  }
  _ShapeTop(_map(j0)).check('watch', <String, Set<String>>{
    'rev': <String>{'string'},
    'latest': <String>{'int'},
    'pending': <String>{'list'},
    'badge': <String>{'int'},
    'unread': <String>{'list'},
  }, required: <String>{'rev', 'latest'});
  _ok('watch: rev set, latest is an int, ${w.pending.length} pending');
  final int unreadLen = (_map(j0)['unread'] as List<Object?>?)?.length ?? 0;
  _info('watch body carries $unreadLen unread ids (${utf8.encode(jsonEncode(j0)).length} bytes)');

  stdout.writeln('\nGET /notices/watch?rev=<current>&wait=1');
  final (int s1, Object? j1, Duration t1) =
      await _call('GET', '/notices/watch', query: <String, String>{'rev': w.rev, 'wait': '1'});
  final NoticeWatch? w1 = s1 == 200 ? NoticeWatch.tryParse(j1) : null;
  if (w1 == null) {
    _problem('/notices/watch (wait=1): $s1 / unparsable');
  } else {
    _ok('returned in ${t1.inMilliseconds} ms, rev ${w1.rev == w.rev ? 'unchanged' : 'changed'}');
    if (w1.rev == w.rev && t1.inMilliseconds < 800) {
      _problem('watch with the current rev returned before the 1 s wait (${t1.inMilliseconds} ms)');
    }
    w = w1;
  }

  final int after = max(0, w.latest - 60);
  stdout.writeln('\nGET /notices?after=latest-60&limit=60');
  final (int s2, Object? j2, Duration t2) = await _call('GET', '/notices',
      query: <String, String>{'after': '$after', 'limit': '60'});
  _info('status $s2 in ${t2.inMilliseconds} ms, ${j2 == null ? 0 : utf8.encode(jsonEncode(j2)).length} bytes');
  final NoticePage? page = s2 == 200 ? NoticePage.tryParse(j2, after: after) : null;
  if (page == null) {
    _problem('/notices: $s2 / NoticePage.tryParse returned null');
    return;
  }
  final Map<String, Object?> top = _map(j2);
  _ShapeTop(top).check('notices page', <String, Set<String>>{
    'notices': <String>{'list'},
    'nextAfter': <String>{'int'},
    'pending': <String>{'list'},
    'prefs': <String>{'map'},
    'focusActive': <String>{'bool'},
    'badge': <String>{'int'},
  }, required: <String>{'notices', 'nextAfter'});
  final List<Object?> raw = top['notices']! as List<Object?>;
  _ok('parsed ${page.notices.length}/${raw.length} notices; nextAfter ${page.nextAfter >= after ? '≥ after' : '< after (!)'}');
  if (page.notices.length != raw.length) _problem('${raw.length - page.notices.length} notice(s) dropped');
  if (page.nextAfter > w.latest) _problem('nextAfter is beyond watch.latest');
  final _Shape n = _Shape();
  final Map<String, int> kinds = <String, int>{};
  final Map<String, int> cats = <String, int>{};
  for (final Object? o in raw) {
    if (o is Map) n.add(o.cast<String, Object?>());
  }
  n.check('notice', <String, Set<String>>{
    'eventId': <String>{'string'},
    'sequence': <String>{'int'},
    'kind': <String>{'string'},
    'category': <String>{'string'},
    'needsHuman': <String>{'bool'},
    'projectKey': <String>{'string', 'null'},
    'project': <String>{'string', 'null'},
    'sessionKey': <String>{'string', 'null'},
    'paneKey': <String>{'string', 'null'},
    'paneId': <String>{'string', 'null'},
    'title': <String>{'string'},
    'excerpt': <String>{'string'},
    'occurredAtMs': <String>{'int'},
    'read': <String>{'bool'},
    'float': <String>{'map'},
    'group': <String>{'string'},
    'editionId': <String>{'string', 'null'},
  }, required: <String>{'eventId', 'sequence'});
  int withSession = 0, sessionLive = 0, paneOk = 0, withPane = 0, phone = 0, noProject = 0;
  for (final Notice x in page.notices) {
    kinds[x.kind] = (kinds[x.kind] ?? 0) + 1;
    cats[x.category] = (cats[x.category] ?? 0) + 1;
    if (x.phoneKind != NoticeKind.other) {
      phone++;
      if (x.project == null) noProject++;
      if (x.sessionKey != null) {
        withSession++;
        if (sessionNames.contains(x.sessionKey)) sessionLive++;
      }
      if (x.paneId != null) {
        withPane++;
        if (RegExp(r'^%\d+$').hasMatch(x.paneId!)) paneOk++;
      }
    }
  }
  _info('kinds: $kinds');
  _info('categories: $cats');
  _info('$phone notice(s) would notify the phone; $withSession carry sessionKey '
      '($sessionLive match a live /session-feed session), $withPane carry paneId '
      '($paneOk look like %N), $noProject without project');
  if (phone > 0 && withSession == 0) {
    _problem('no phone-worthy notice carries sessionKey: a tap could not open the chat');
  }
}

Future<void> _receipt() async {
  stdout.writeln('\nGET /session/message?clientMessageId=<random>');
  final Random r = Random.secure();
  String hex(int n) => List<String>.generate(n, (_) => r.nextInt(16).toRadixString(16)).join();
  final String id = '${hex(8)}-${hex(4)}-4${hex(3)}-a${hex(3)}-${hex(12)}';
  final (int status, Object? json, Duration t) =
      await _call('GET', '/session/message', query: <String, String>{'clientMessageId': id});
  final SendResult res = classifyMessageResponse(ApiResponse(status, json is _NotJson ? null : json));
  _info('status $status in ${t.inMilliseconds} ms → ${res.runtimeType}');
  if (res is UnknownReceipt) {
    _ok('unknown id is reported as unknown');
  } else if (res is RouteMissing) {
    _problem('server has no /session/message: the outbox falls back to legacy /send');
  } else {
    _problem('unexpected classification ${res.runtimeType} for a never-sent id');
  }
}
