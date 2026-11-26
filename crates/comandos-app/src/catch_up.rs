//! «Ponerse al día» (like Slack's «Catch up»), the same queue as the Android app
//! (`mobile/lib/features/shell/catch_up.dart`) and the web remote: every split that is
//! not working, those waiting for the person first (a permission, a limit), then the
//! ones that finished, newest first. Busy splits and plain terminals stay out.
//! Pure logic; the chat view walks it (`ui::app_chat`).
use crate::feed::{Feed, FeedState};

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub session: String,
    /// tmux pane id; empty for a session the feed lists without panes.
    pub pane: String,
    pub state: FeedState,
    pub ts: f64,
}

fn pane_number(pane: &str) -> u64 {
    pane.trim_start_matches('%').parse().unwrap_or(u64::MAX)
}

/// The splits to go through, taken from the feed when the queue starts.
pub fn deck(feed: Option<&Feed>) -> Vec<Card> {
    let mut cards = Vec::new();
    for entry in feed.into_iter().flat_map(|f| f.sessions.values()) {
        if entry.pane_list.is_empty() {
            if entry.state != FeedState::Work {
                cards.push(Card { session: entry.session.clone(), pane: entry.pane.clone(), state: entry.state, ts: entry.ts });
            }
            continue;
        }
        let mut panes: Vec<_> = entry.pane_list.iter().collect();
        panes.sort_by_key(|p| pane_number(&p.pane));
        for p in panes {
            // Busy: nothing to tell it yet. A plain terminal has nothing to catch up on.
            if p.state == FeedState::Work || p.agent.is_empty() {
                continue;
            }
            cards.push(Card { session: entry.session.clone(), pane: p.pane.clone(), state: p.state, ts: p.ts });
        }
    }
    let rank = |c: &Card| if c.state.waits() { 0 } else { 1 };
    cards.sort_by(|a, b| rank(a).cmp(&rank(b)).then(b.ts.total_cmp(&a.ts)));
    cards
}

/// A fixed queue taken when it starts (cards do not jump while the person writes),
/// walked with ‹ ›; sending or answering moves on to the next split.
#[derive(Debug, Default)]
pub struct Queue {
    deck: Vec<Card>,
    at: usize,
    on: bool,
}

impl Queue {
    pub fn on(&self) -> bool {
        self.on
    }
    pub fn len(&self) -> usize {
        self.deck.len()
    }
    pub fn is_empty(&self) -> bool {
        self.deck.is_empty()
    }
    pub fn current(&self) -> Option<&Card> {
        self.deck.get(self.at).filter(|_| self.on)
    }
    /// «3 de 9».
    pub fn position(&self) -> String {
        format!("{} de {}", self.at + 1, self.deck.len())
    }
    /// Starts on `deck`. False (and stays off) when there is nothing to go through.
    pub fn start(&mut self, deck: Vec<Card>) -> bool {
        if deck.is_empty() {
            self.stop();
            return false;
        }
        self.deck = deck;
        self.at = 0;
        self.on = true;
        true
    }
    /// One split forward (or back). Before the first it stays put; past the last one
    /// the queue ends: `None`.
    pub fn step(&mut self, delta: i32) -> Option<&Card> {
        if !self.on {
            return None;
        }
        let next = self.at as i64 + i64::from(delta);
        if next < 0 {
            return self.current();
        }
        if next as usize >= self.deck.len() {
            self.stop();
            return None;
        }
        self.at = next as usize;
        self.current()
    }
    pub fn stop(&mut self) {
        self.on = false;
        self.deck.clear();
        self.at = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::parse_feed;
    use serde_json::json;

    fn pane(id: &str, state: &str, ts: f64, agent: &str) -> serde_json::Value {
        json!({"pane": id, "state": state, "ts": ts, "agent": agent, "model": "claude-opus-5-5"})
    }

    #[test]
    fn the_deck_goes_split_by_split_waiting_first_busy_and_terminals_out() {
        let feed = parse_feed(&json!({"now": 100, "sessions": [
            {"session": "a", "state": "idle", "pane": "%3", "paneList": [
                pane("%3", "idle", 50., "claude"), pane("%1", "work", 90., "claude"), pane("%2", "ask", 10., "claude"),
            ]},
            {"session": "b", "state": "idle", "pane": "%7", "paneList": [
                pane("%7", "idle", 80., "claude"), pane("%8", "idle", 99., ""),
            ]},
            {"session": "c", "state": "work", "pane": "%9"},
            {"session": "d", "state": "limit", "pane": "%4", "ts": 5},
        ]}))
        .unwrap();
        let got: Vec<String> = deck(Some(&feed)).iter().map(|c| format!("{}{}", c.session, c.pane)).collect();
        assert_eq!(got, ["a%2", "d%4", "b%7", "a%3"]);
        assert!(deck(None).is_empty());
    }

    #[test]
    fn the_queue_walks_forward_and_back_and_ends_after_the_last_split() {
        let card = |s: &str| Card { session: s.into(), pane: "%1".into(), state: FeedState::Idle, ts: 1. };
        let mut q = Queue::default();
        assert!(!q.start(Vec::new()));
        assert!(!q.on());
        assert!(q.start(vec![card("a"), card("b")]));
        assert_eq!(q.position(), "1 de 2");
        assert_eq!(q.step(-1).map(|c| c.session.clone()).as_deref(), Some("a"), "the first split stays put going back");
        assert_eq!(q.step(1).map(|c| c.session.clone()).as_deref(), Some("b"));
        assert_eq!(q.position(), "2 de 2");
        assert!(q.step(1).is_none());
        assert!(!q.on(), "past the last split the queue ends");
        assert!(q.current().is_none());
    }
}
