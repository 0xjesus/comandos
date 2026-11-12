//! Foco + repisa (grill R2): one live terminal at a fixed column count and,
//! beside it, text cards for the other panes of the tab and for sessions that
//! wait for you. Cards are plain GTK labels: they never resize tmux and cost
//! nothing to paint, so moving the window only grows or shrinks the shelf.
use crate::feed::{Feed, FeedState};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Column steps: a window resize inside one step never reaches Claude.
pub const STEP: u16 = 12;
pub const MIN_COLS: u16 = 72;
pub const SHELF_MIN: f64 = 300.;
pub const SHELF_MAX: f64 = 440.;
/// Cards from other tabs shown under "te esperan".
pub const MAX_WAITING: usize = 5;

/// The shelf width the layout aims for in an area `avail` px wide.
pub fn shelf_target(avail: f64) -> f64 {
    (avail * 0.26).clamp(SHELF_MIN, SHELF_MAX)
}

/// Terminal width for `avail` px: `(cols, px)`, or `None` when there is no
/// room for a shelf (the terminal then fills the area as before).
/// `chrome` is everything around the cells (terminal padding and frame).
/// The shelf takes what is left of the last column step, so moving the
/// window changes the terminal only every `STEP` columns.
pub fn snap(avail: f64, cell_w: f64, chrome: f64) -> Option<(u16, f64)> {
    if cell_w.is_nan() || cell_w <= 0. || !avail.is_finite() {
        return None;
    }
    let fit = ((avail - shelf_target(avail) - chrome) / cell_w).floor();
    if fit < f64::from(MIN_COLS) {
        return None;
    }
    let cols = (fit.min(f64::from(u16::MAX)) as u16) / STEP * STEP;
    // Half a cell of slack: the terminal floors its width into columns.
    let px = (chrome + f64::from(cols) * cell_w + cell_w * 0.5).ceil();
    Some((cols, px))
}

/// Shelf width for a terminal of `term_px` inside `avail`.
pub fn shelf_width(avail: f64, term_px: f64) -> f64 {
    (avail - term_px).max(SHELF_MIN).floor()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    ThisTab,
    Waiting,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub section: Section,
    pub session: String,
    /// Set for a pane of the current tab: a click focuses it.
    pub pane: Option<String>,
    pub title: String,
    pub state: FeedState,
    pub text: String,
    pub meta: String,
}

pub fn state_text(state: FeedState, english: bool) -> &'static str {
    match (state, english) {
        (FeedState::Ask, false) => "Te espera",
        (FeedState::Ask, true) => "Waiting for you",
        (FeedState::Limit, false) => "Sin cuota",
        (FeedState::Limit, true) => "Out of quota",
        (FeedState::Work, false) => "Trabajando",
        (FeedState::Work, true) => "Working",
        (FeedState::Idle, false) => "Terminó",
        (FeedState::Idle, true) => "Done",
    }
}

pub fn state_class(state: FeedState) -> &'static str {
    match state {
        FeedState::Ask => "fs-ask",
        FeedState::Limit => "fs-limit",
        FeedState::Work => "fs-work",
        FeedState::Idle => "fs-idle",
    }
}

fn agent_name(agent: &str) -> String {
    let mut chars = agent.chars();
    chars.next().map_or_else(
        || "Agente".into(),
        |c| c.to_uppercase().chain(chars).collect(),
    )
}

fn meta(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|p| !p.trim().is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The cards for the shelf. `active` is the pane the terminal shows; `label`
/// names another session the way its tab does.
pub fn cards(
    feed: &Feed,
    current: &str,
    active: Option<&str>,
    label: &dyn Fn(&str) -> String,
    english: bool,
) -> Vec<Card> {
    let mut out = Vec::new();
    if let Some(entry) = feed.sessions.get(current) {
        for (index, pane) in entry.pane_list.iter().enumerate() {
            if Some(pane.pane.as_str()) == active {
                continue;
            }
            let title = format!(
                "{} {}",
                agent_name(&pane.agent),
                if english { "pane" } else { "panel" }
            );
            out.push(Card {
                section: Section::ThisTab,
                session: current.to_string(),
                pane: Some(pane.pane.clone()),
                title: format!("{title} {}", index + 1),
                state: pane.state,
                text: pane.reply.clone(),
                meta: meta(&[&pane.model, &pane.account, &crate::feed::time_ago(feed.now, pane.ts, english)]),
            });
        }
    }
    let mut waiting: Vec<_> = feed
        .sessions
        .values()
        .filter(|e| e.session != current)
        .filter(|e| e.state.waits() || (e.unread && e.state != FeedState::Work))
        .collect();
    waiting.sort_by(|a, b| {
        b.state
            .waits()
            .cmp(&a.state.waits())
            .then(b.ts.partial_cmp(&a.ts).unwrap_or(std::cmp::Ordering::Equal))
    });
    for entry in waiting.into_iter().take(MAX_WAITING) {
        let name = label(&entry.session);
        out.push(Card {
            section: Section::Waiting,
            session: entry.session.clone(),
            pane: None,
            title: if name.is_empty() { entry.label.clone() } else { name },
            state: entry.state,
            text: entry
                .ask
                .as_ref()
                .map(|a| a.question.clone())
                .filter(|q| !q.trim().is_empty())
                .unwrap_or_else(|| entry.reply.clone()),
            meta: meta(&[&entry.model, &crate::feed::time_ago(feed.now, entry.ts, english)]),
        });
    }
    out
}

/// What a click on a card asks the app to do.
pub type CardAction = Rc<dyn Fn(&Card)>;

pub struct FocusShelf {
    pub widget: gtk::ScrolledWindow,
    list: gtk::Box,
    signature: RefCell<String>,
    on_card: RefCell<Option<CardAction>>,
    on_all: RefCell<Option<Rc<dyn Fn()>>>,
    english: Cell<bool>,
}

impl FocusShelf {
    pub fn new() -> Rc<Self> {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 10);
        list.style_context().add_class("fs-shelf");
        let widget = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        widget.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        widget.set_hexpand(false);
        widget.style_context().add_class("fs-scroll");
        widget.add(&list);
        widget.set_no_show_all(true);
        Rc::new(Self {
            widget,
            list,
            signature: RefCell::new(String::from("\u{0}")),
            on_card: RefCell::new(None),
            on_all: RefCell::new(None),
            english: Cell::new(false),
        })
    }

    pub fn set_language(&self, english: bool) {
        if self.english.replace(english) != english {
            self.signature.replace(String::from("\u{0}"));
        }
    }

    pub fn connect_card(&self, action: CardAction) {
        self.on_card.replace(Some(action));
    }

    pub fn connect_show_all(&self, action: Rc<dyn Fn()>) {
        self.on_all.replace(Some(action));
    }

    /// Rebuilds the cards only when what they show changed.
    pub fn render(self: &Rc<Self>, cards: &[Card], panes: usize) {
        let signature = format!("{panes}|{cards:?}");
        if *self.signature.borrow() == signature {
            return;
        }
        self.signature.replace(signature);
        for child in self.list.children() {
            self.list.remove(&child);
        }
        let english = self.english.get();
        let this_tab = cards.iter().filter(|c| c.section == Section::ThisTab).count();
        self.list.add(&self.header(
            if english { "THIS TAB" } else { "EN ESTA PESTAÑA" },
            &if english {
                format!("{panes} pane{}", if panes == 1 { "" } else { "s" })
            } else {
                format!("{panes} panel{}", if panes == 1 { "" } else { "es" })
            },
            false,
        ));
        if this_tab == 0 {
            let empty = gtk::Label::new(Some(if english {
                "Only this terminal in the tab."
            } else {
                "Solo esta terminal en la pestaña."
            }));
            empty.set_xalign(0.);
            empty.style_context().add_class("fs-empty");
            self.list.add(&empty);
        }
        let mut waiting_header = false;
        for card in cards {
            if card.section == Section::Waiting && !waiting_header {
                waiting_header = true;
                self.list.add(&self.header(
                    if english { "OTHER TABS WAIT FOR YOU" } else { "OTRAS PESTAÑAS TE ESPERAN" },
                    "",
                    false,
                ));
            }
            self.list.add(&self.card(card));
        }
        self.list.show_all();
    }

    fn header(self: &Rc<Self>, title: &str, count: &str, show_all: bool) -> gtk::Widget {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.style_context().add_class("fs-head");
        let label = gtk::Label::new(Some(title));
        label.set_xalign(0.);
        label.style_context().add_class("fs-head-title");
        row.pack_start(&label, false, false, 0);
        if !count.is_empty() {
            let count = gtk::Label::new(Some(count));
            count.style_context().add_class("fs-head-count");
            row.pack_end(&count, false, false, 0);
        }
        if show_all {
            let english = self.english.get();
            let button = gtk::Button::with_label(if english { "Show all" } else { "Ver todos" });
            button.style_context().add_class("fs-all");
            button.set_tooltip_text(Some(if english {
                "Back to every pane side by side"
            } else {
                "Volver a ver todos los paneles juntos"
            }));
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(action) = weak.upgrade().and_then(|s| s.on_all.borrow().clone()) {
                    action();
                }
            });
            row.pack_end(&button, false, false, 0);
        }
        row.upcast()
    }

    fn card(self: &Rc<Self>, card: &Card) -> gtk::Widget {
        let english = self.english.get();
        let event = gtk::EventBox::new();
        event.set_visible_window(false);
        event.add_events(gdk::EventMask::BUTTON_PRESS_MASK);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let style = body.style_context();
        style.add_class("fs-card");
        style.add_class(state_class(card.state));
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let dot = gtk::Label::new(Some("●"));
        dot.style_context().add_class("fs-dot");
        top.pack_start(&dot, false, false, 0);
        let title = gtk::Label::new(Some(&card.title));
        title.set_xalign(0.);
        title.set_ellipsize(pango::EllipsizeMode::End);
        title.style_context().add_class("fs-title");
        top.pack_start(&title, true, true, 0);
        let state = gtk::Label::new(Some(state_text(card.state, english)));
        state.style_context().add_class("fs-state");
        top.pack_end(&state, false, false, 0);
        body.add(&top);
        let text = card.text.trim();
        if !text.is_empty() {
            let label = gtk::Label::new(Some(text));
            label.set_xalign(0.);
            label.set_line_wrap(true);
            label.set_line_wrap_mode(pango::WrapMode::WordChar);
            label.set_lines(5);
            label.set_ellipsize(pango::EllipsizeMode::End);
            label.set_max_width_chars(1);
            label.style_context().add_class("fs-text");
            body.add(&label);
        }
        if !card.meta.is_empty() {
            let meta = gtk::Label::new(Some(&card.meta));
            meta.set_xalign(0.);
            meta.set_ellipsize(pango::EllipsizeMode::End);
            meta.style_context().add_class("fs-meta");
            body.add(&meta);
        }
        event.add(&body);
        event.set_tooltip_text(Some(match (card.pane.is_some(), english) {
            (true, false) => "Ver este panel en grande",
            (true, true) => "Show this pane large",
            (false, false) => "Abrir esta pestaña",
            (false, true) => "Open this tab",
        }));
        let weak = Rc::downgrade(self);
        let clicked = card.clone();
        event.connect_button_press_event(move |_, ev| {
            if ev.button() == 1
                && let Some(action) = weak.upgrade().and_then(|s| s.on_card.borrow().clone())
            {
                action(&clicked);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        let hover = body.clone();
        event.connect_enter_notify_event(move |_, _| {
            hover.set_state_flags(gtk::StateFlags::PRELIGHT, false);
            glib::Propagation::Proceed
        });
        let hover = body.clone();
        event.connect_leave_notify_event(move |_, _| {
            hover.unset_state_flags(gtk::StateFlags::PRELIGHT);
            glib::Propagation::Proceed
        });
        event.upcast()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::{FeedEntry, PaneFeed};

    fn pane(id: &str, state: FeedState, reply: &str) -> PaneFeed {
        PaneFeed {
            pane: id.into(),
            state,
            ts: 10.,
            agent: "claude".into(),
            model: "opus-5".into(),
            account: "main".into(),
            reply: reply.into(),
        }
    }
    fn entry(session: &str, state: FeedState, unread: bool, panes: Vec<PaneFeed>) -> FeedEntry {
        FeedEntry {
            session: session.into(),
            label: session.into(),
            state,
            unread,
            ts: 5.,
            seen: 0.,
            pane: "%1".into(),
            panes: panes.len() as u64,
            agent: "claude".into(),
            model: "opus-5".into(),
            prompt: String::new(),
            reply: format!("reply of {session}"),
            ask: None,
            pane_list: panes,
        }
    }

    #[test]
    fn snap_moves_in_steps_and_leaves_the_rest_to_the_shelf() {
        let (cell, chrome) = (8., 14.);
        for avail in [1100., 1400., 1880., 2600.] {
            let (cols, px) = snap(avail, cell, chrome).unwrap();
            assert_eq!(cols % STEP, 0);
            let shelf = shelf_width(avail, px);
            assert!(shelf >= SHELF_MIN && shelf < SHELF_MAX + f64::from(STEP) * cell + cell, "{avail}: {shelf}");
        }
        // Small moves of the window keep the same terminal.
        assert_eq!(snap(1720., cell, chrome), snap(1735., cell, chrome));
        // No room for a shelf: the terminal fills the area as before.
        assert_eq!(snap(700., cell, chrome), None);
        // The terminal floors (px - chrome) / cell back to the same columns.
        let (cols, px) = snap(1500., 8.25, 14.).unwrap();
        assert_eq!(((px - 14.) / 8.25).floor() as u16, cols);
    }

    #[test]
    fn cards_show_other_panes_then_sessions_that_wait() {
        let mut feed = Feed { now: 20., ..Feed::default() };
        feed.sessions.insert(
            "term-a".into(),
            entry(
                "term-a",
                FeedState::Work,
                false,
                vec![
                    pane("%1", FeedState::Work, "uno"),
                    pane("%2", FeedState::Ask, "dos"),
                    pane("%3", FeedState::Idle, "tres"),
                ],
            ),
        );
        feed.sessions.insert("term-b".into(), entry("term-b", FeedState::Ask, true, vec![]));
        feed.sessions.insert("term-c".into(), entry("term-c", FeedState::Work, true, vec![]));
        feed.sessions.insert("term-d".into(), entry("term-d", FeedState::Idle, false, vec![]));
        let cards = cards(&feed, "term-a", Some("%1"), &|s| format!("T {s}"), false);
        let panes: Vec<_> = cards.iter().filter_map(|c| c.pane.as_deref()).collect();
        assert_eq!(panes, ["%2", "%3"]);
        let waiting: Vec<_> = cards
            .iter()
            .filter(|c| c.section == Section::Waiting)
            .map(|c| c.title.as_str())
            .collect();
        // Working or already read sessions are not "waiting for you".
        assert_eq!(waiting, ["T term-b"]);
    }
}
