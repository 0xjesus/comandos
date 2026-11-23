import 'package:flutter/foundation.dart';

import '../../core/models/session_feed.dart';
import '../chat/pane_rows.dart';

/// One split to catch up on: a pane of a session that is not busy.
@immutable
class CatchUpCard {
  const CatchUpCard({required this.session, required this.pane, required this.state, required this.ts});

  final String session;

  /// tmux pane id, or null for a session the feed lists without panes.
  final String? pane;
  final SessionState state;
  final double ts;

  @override
  bool operator ==(Object other) => other is CatchUpCard && other.session == session && other.pane == pane;

  @override
  int get hashCode => Object.hash(session, pane);
}

/// The splits to go through, like Slack's «Catch up»: every split that is
/// not working, those waiting for the person (a permission, a limit) first,
/// then the ones that finished, newest first. Busy splits stay out: there is
/// nothing to tell them yet.
List<CatchUpCard> buildCatchUpDeck(SessionFeed? feed) {
  final List<CatchUpCard> cards = <CatchUpCard>[];
  for (final SessionSummary s in feed?.sessions ?? const <SessionSummary>[]) {
    if (s.paneList.isEmpty) {
      if (s.state != SessionState.work) {
        cards.add(CatchUpCard(session: s.session, pane: s.pane, state: s.state, ts: s.ts));
      }
      continue;
    }
    final List<PaneSummary> panes = <PaneSummary>[...s.paneList]
      ..sort((PaneSummary a, PaneSummary b) => paneNumber(a.pane).compareTo(paneNumber(b.pane)));
    for (final PaneSummary p in panes) {
      if (p.state == SessionState.work) continue;
      // A plain terminal has nothing to catch up on.
      if (p.agent.isEmpty) continue;
      cards.add(CatchUpCard(session: s.session, pane: p.pane, state: p.state, ts: p.ts));
    }
  }
  int rank(CatchUpCard c) => c.state.waitsForHuman ? 0 : 1;
  cards.sort((CatchUpCard a, CatchUpCard b) {
    final int r = rank(a).compareTo(rank(b));
    return r != 0 ? r : b.ts.compareTo(a.ts);
  });
  return cards;
}

/// «Ponerse al día»: a fixed queue of splits taken when it starts (cards do
/// not jump while the person writes), walked with the chat's ‹ › and sideways
/// swipe; sending or answering a prompt moves to the next split.
class CatchUp extends ChangeNotifier {
  List<CatchUpCard> _deck = const <CatchUpCard>[];
  int _at = 0;
  bool _on = false;

  bool get on => _on;
  List<CatchUpCard> get deck => _deck;
  int get at => _at;
  CatchUpCard? get current => _on && _at < _deck.length ? _deck[_at] : null;

  /// «3 de 9».
  String get position => '${_at + 1} de ${_deck.length}';

  /// Starts on [deck]. False (and stays off) when there is nothing to go through.
  bool start(List<CatchUpCard> deck) {
    if (deck.isEmpty) {
      stop();
      return false;
    }
    _deck = List<CatchUpCard>.unmodifiable(deck);
    _at = 0;
    _on = true;
    notifyListeners();
    return true;
  }

  /// One split forward (or back). Past the last one the queue ends: null.
  CatchUpCard? step(int delta) {
    if (!_on) return null;
    final int next = _at + delta;
    if (next < 0) return current;
    if (next >= _deck.length) {
      stop();
      return null;
    }
    _at = next;
    notifyListeners();
    return current;
  }

  void stop() {
    if (!_on) return;
    _on = false;
    _deck = const <CatchUpCard>[];
    _at = 0;
    notifyListeners();
  }
}
