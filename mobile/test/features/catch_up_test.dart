import 'package:comandos_mobile/core/models/session_feed.dart';
import 'package:comandos_mobile/features/chat/pane_rows.dart';
import 'package:comandos_mobile/features/shell/catch_up.dart';
import 'package:flutter_test/flutter_test.dart';

PaneSummary pane(String id, SessionState state, double ts, {String agent = 'claude', String model = 'claude-opus-5-5'}) =>
    PaneSummary(pane: id, state: state, ts: ts, agent: agent, model: model);

SessionSummary session(String id, List<PaneSummary> panes, {SessionState state = SessionState.idle}) =>
    SessionSummary(session: id, label: id, state: state, unread: false, ts: 0, pane: panes.isEmpty ? null : panes.first.pane, paneList: panes);

void main() {
  test('the deck goes split by split: waiting first, busy and plain terminals out', () {
    final SessionFeed feed = SessionFeed(now: 100, sessions: <SessionSummary>[
      session('a', <PaneSummary>[
        pane('%3', SessionState.idle, 50),
        pane('%1', SessionState.work, 90),
        pane('%2', SessionState.ask, 10),
      ]),
      session('b', <PaneSummary>[
        pane('%7', SessionState.idle, 80),
        pane('%8', SessionState.idle, 99, agent: ''),
      ]),
    ]);
    final List<CatchUpCard> deck = buildCatchUpDeck(feed);
    expect(<String>[for (final CatchUpCard c in deck) '${c.session}${c.pane}'], <String>['a%2', 'b%7', 'a%3']);
    expect(buildCatchUpDeck(null), isEmpty);
  });

  test('the queue walks forward and back and ends after the last split', () {
    final CatchUp q = CatchUp();
    expect(q.start(const <CatchUpCard>[]), isFalse);
    expect(q.on, isFalse);
    final List<CatchUpCard> deck = <CatchUpCard>[
      const CatchUpCard(session: 'a', pane: '%1', state: SessionState.ask, ts: 1),
      const CatchUpCard(session: 'b', pane: '%2', state: SessionState.idle, ts: 1),
    ];
    expect(q.start(deck), isTrue);
    expect(q.position, '1 de 2');
    expect(q.step(-1)?.session, 'a', reason: 'the first split stays put going back');
    expect(q.step(1)?.session, 'b');
    expect(q.position, '2 de 2');
    expect(q.step(1), isNull);
    expect(q.on, isFalse, reason: 'past the last split the queue ends');
  });

  test('the split label names the split the phone talks to', () {
    final SessionSummary s = session('a', <PaneSummary>[
      pane('%9', SessionState.idle, 1, model: 'claude-fable-5-2'),
      pane('%2', SessionState.idle, 1, model: 'claude-opus-4-8'),
    ]);
    expect(splitLabel(s, '%9'), '2 · Claude · fable-5.2');
    expect(splitLabel(s, null), '2 · Claude · fable-5.2', reason: 'the feed pane when none was picked');
    expect(splitLabel(s, '%2'), '1 · Claude · opus-4.8');
    final SessionSummary one = session('b', <PaneSummary>[pane('%4', SessionState.idle, 1)]);
    expect(splitLabel(one, '%4'), 'Claude · opus-5.5', reason: 'no number without a split');
  });

  test('savePane bumps panePicks only on a change', () {
    final int before = panePicks.value;
    savePane('x', '%1');
    savePane('x', '%1');
    expect(panePicks.value, before + 1);
  });
}
