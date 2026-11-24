import 'package:comandos_mobile/core/platform/native_channel.dart';
import 'package:comandos_mobile/features/notifications/notice.dart';
import 'package:comandos_mobile/features/notifications/notice_alerts.dart';
import 'package:flutter_test/flutter_test.dart';

// Synthetic data only: invented projects and sessions.
Notice n(
  int seq, {
  String kind = 'turn_completed',
  String category = 'done',
  bool needsHuman = false,
  String? project = 'Pagos',
  String? session = 'term-r1',
  String? pane = '%1',
  String excerpt = '',
  String title = '',
  bool read = false,
}) =>
    Notice(
      eventId: 'e$seq',
      sequence: seq,
      kind: kind,
      category: category,
      needsHuman: needsHuman,
      occurredAtMs: 1000 + seq,
      project: project,
      sessionKey: session,
      paneId: pane,
      excerpt: excerpt,
      title: title,
      read: read,
    );

Notice ask(int seq, {String session = 'term-r1', String? pane = '%1', String excerpt = ''}) => n(seq,
    kind: 'permission_requested',
    category: 'attention',
    needsHuman: true,
    session: session,
    pane: pane,
    excerpt: excerpt);

void main() {
  group('parsing (synthetic shapes)', () {
    test('a notice with nulls where the server allows them', () {
      final Notice? x = Notice.tryParse(<String, Object?>{
        'eventId': 'e1', 'sequence': 7, 'kind': 'news_edition', 'category': 'news',
        'needsHuman': false, 'projectKey': null, 'project': null, 'sessionKey': null,
        'paneKey': null, 'paneId': null, 'title': '', 'excerpt': '', 'occurredAtMs': 5,
        'read': false, 'float': <String, Object?>{'show': true, 'ms': 6000}, 'group': 'e1',
        'editionId': null,
      });
      expect(x, isNotNull);
      expect(x!.phoneKind, NoticeKind.other);
      expect(x.sessionKey, isNull);
    });

    test('rejects entries without id or sequence; pages default nextAfter', () {
      expect(Notice.tryParse(<String, Object?>{'eventId': 'e1'}), isNull);
      final NoticePage? p = NoticePage.tryParse(<String, Object?>{
        'notices': <Object?>[<String, Object?>{'sequence': 1}],
      }, after: 9);
      expect(p!.notices, isEmpty);
      expect(p.nextAfter, 9);
      expect(NoticePage.tryParse(<String, Object?>{'nextAfter': 1}, after: 0), isNull);
    });

    test('watch needs rev and latest', () {
      expect(NoticeWatch.tryParse(<String, Object?>{'rev': '3.1.0', 'latest': 3})!.latest, 3);
      expect(NoticeWatch.tryParse(<String, Object?>{'rev': '3.1.0'}), isNull);
      expect(NoticeWatch.tryParse(<String, Object?>{'rev': 3, 'latest': 3}), isNull);
    });
  });

  group('alertsFor', () {
    test('needs a human → urgent "te espera" with the excerpt', () {
      final List<LocalAlert> a = alertsFor(<Notice>[ask(1, excerpt: '¿Puedo  borrar\n la rama?')],
          pending: <String>{'e1'});
      expect(a, hasLength(1));
      expect(a.single.urgent, isTrue);
      expect(a.single.title, 'Pagos te espera');
      expect(a.single.body, '¿Puedo borrar la rama?');
      expect(a.single.tag, tagForSession('term-r1'));
      expect(a.single.session, 'term-r1');
      expect(a.single.pane, '%1');
    });

    test('done / error / cancelled titles and default bodies', () {
      expect(alertFor(n(1)).title, 'Pagos terminó');
      expect(alertFor(n(1)).body, 'Terminó su turno.');
      expect(alertFor(n(1)).urgent, isFalse);
      expect(alertFor(n(2, kind: 'turn_failed', category: 'error')).title, 'Pagos falló');
      expect(alertFor(n(3, kind: 'turn_cancelled')).title, 'Pagos se detuvo');
      expect(alertFor(n(4, title: 'Resumen')).body, 'Resumen');
      expect(alertFor(n(5, project: null)).title, 'term-r1 terminó');
    });

    test('long excerpts are clipped', () {
      final LocalAlert a = alertFor(n(1, excerpt: 'x' * 1000));
      expect(a.body.length, 240);
      expect(a.body.endsWith('…'), isTrue);
    });

    test('news, usage, info and already-read events never notify', () {
      expect(
        alertsFor(<Notice>[
          n(1, kind: 'news_edition', category: 'news', session: null),
          n(2, kind: 'usage_alert', category: 'usage'),
          n(3, kind: 'x', category: 'info'),
          n(4, read: true),
        ]),
        isEmpty,
      );
    });

    test('an answered request (no longer pending) does not notify', () {
      expect(alertsFor(<Notice>[ask(1)], pending: <String>{}), isEmpty);
      // Without a pane the server cannot track it: notify anyway.
      expect(alertsFor(<Notice>[ask(2, pane: null)], pending: <String>{}), hasLength(1));
    });

    test('the open chat is skipped', () {
      expect(alertsFor(<Notice>[n(1)], openChat: 'term-r1'), isEmpty);
      expect(alertsFor(<Notice>[n(1)], openChat: 'term-r2'), hasLength(1));
    });

    test('done can be turned off, errors and requests still notify', () {
      final List<LocalAlert> a = alertsFor(<Notice>[
        n(1, session: 's1'),
        n(2, kind: 'turn_failed', category: 'error', session: 's2'),
        ask(3, session: 's3'),
      ], includeDone: false, pending: <String>{'e3'});
      expect(a.map((LocalAlert x) => x.session), <String>['s2', 's3']);
    });

    test('one alert per session: the newest event wins; order by sequence', () {
      final List<LocalAlert> a = alertsFor(<Notice>[
        ask(1, session: 'a'),
        n(2, session: 'b'),
        n(3, session: 'a', kind: 'turn_completed'),
      ], pending: <String>{'e1'});
      expect(a, hasLength(2));
      expect(a[0].session, 'b');
      expect(a[1].session, 'a');
      expect(a[1].urgent, isFalse); // the wait ended: replaced by "terminó"
    });

    test('global events without a session get their own tag', () {
      final LocalAlert a = alertFor(n(9, session: null, kind: 'turn_failed', category: 'error'));
      expect(a.tag, 'e:e9');
      expect(a.session, isNull);
    });
  });

  test('resolvedTags: only trackable requests no longer pending', () {
    final Map<String, ShownAlert> shown = <String, ShownAlert>{
      's:a': ShownAlert.of(alertFor(ask(1, session: 'a'))),
      's:b': ShownAlert.of(alertFor(ask(2, session: 'b'))),
      's:c': ShownAlert.of(alertFor(ask(3, session: 'c', pane: null))),
      's:d': ShownAlert.of(alertFor(n(4, session: 'd'))),
    };
    expect(resolvedTags(shown, <String>{'e2'}), <String>['s:a']);
  });

  test('the car hears the state first, a short clean summary and who says it', () {
    final LocalAlert a = alertFor(n(1, excerpt: '**Listo.** Subí `x` con la llave sk-proj-AbCdEf1234567890xyz.'));
    expect(a.sender, 'Pagos');
    expect(a.speech, startsWith('Terminó. Listo.'));
    expect(a.speech, isNot(contains('sk-proj')));
    expect(alertFor(ask(2, excerpt: '¿Borro la rama?')).speech, startsWith('Te espera.'));
    expect(alertFor(n(3, kind: 'turn_failed', category: 'error')).speech, 'Falló.');
    final LocalAlert private = alertFor(n(4, excerpt: 'Datos del cliente'), carContent: false);
    expect(private.speech, 'Terminó.', reason: 'private: only who needs you');
    expect(private.body, 'Datos del cliente', reason: 'the phone shade keeps its text');
    expect(alertFor(n(5, session: null)).speech, isNull, reason: 'no session: nowhere to reply');
  });
}
