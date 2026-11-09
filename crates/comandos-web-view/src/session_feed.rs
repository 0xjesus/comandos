//! «Ficha viva» (design/ficha-viva-spec.md §2–3): the `/session-feed` model and its HTML.
//!
//! Pure and host-testable. The wasm glue (`comandos-web` `session_feed`) polls the route,
//! keeps the state and paints what these functions return: the K5 class of a top-bar tile,
//! the G4 rows of the «Sesiones» feed and the B4 preview card.
//!
//! K5: one accent (`--brand`), more intense = more urgent. `ask`/`limit` = te espera (solid),
//! `work` = ring, unread = dot, quiet = nothing. No traffic light.
use crate::escape::text as esc;
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Opt {
    pub key: String,
    pub label: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    Ask,
    Limit,
    Work,
    #[default]
    Idle,
}

impl State {
    pub fn parse(s: &str) -> Self {
        match s {
            "ask" => Self::Ask,
            "limit" => Self::Limit,
            "work" => Self::Work,
            _ => Self::Idle,
        }
    }
    /// Permission, question or failure: ComandOS is calling you.
    pub fn waiting(self) -> bool {
        matches!(self, Self::Ask | Self::Limit)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub session: String,
    pub label: String,
    pub state: State,
    pub unread: bool,
    pub ts: f64,
    pub seen: f64,
    pub pane: String,
    pub panes: u64,
    pub agent: String,
    pub model: String,
    pub prompt: String,
    pub reply: String,
    pub question: String,
    pub options: Vec<Opt>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feed {
    pub now: f64,
    pub sessions: Vec<Entry>,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().into()
}
fn n(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Collapses runs of whitespace (newlines included) into one space.
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The JSON of `GET /session-feed`. `None` when it is not the feed (an error page, a 404
/// body): the caller then shows nothing new.
pub fn parse(v: &Value) -> Option<Feed> {
    let list = v.get("sessions")?.as_array()?;
    let sessions = list
        .iter()
        .filter_map(|x| {
            let session = s(x, "session");
            if session.is_empty() {
                return None;
            }
            let ask = x.get("ask").filter(|a| a.is_object());
            let options = ask
                .and_then(|a| a.get("options"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|o| {
                    let key = s(o, "key");
                    let digit = key.len() == 1 && key.bytes().all(|b| (b'1'..=b'9').contains(&b));
                    digit.then(|| Opt {
                        key,
                        label: one_line(&s(o, "label")),
                    })
                })
                .collect();
            let label = s(x, "label");
            Some(Entry {
                label: if label.trim().is_empty() {
                    session.clone()
                } else {
                    label
                },
                session,
                state: State::parse(&s(x, "state")),
                unread: x.get("unread").and_then(Value::as_bool).unwrap_or(false),
                ts: n(x, "ts"),
                seen: n(x, "seen"),
                pane: s(x, "pane"),
                panes: x.get("panes").and_then(Value::as_u64).unwrap_or(0),
                agent: s(x, "agent"),
                model: s(x, "model"),
                prompt: one_line(&s(x, "prompt")),
                reply: s(x, "reply").trim().into(),
                question: ask.map(|a| one_line(&s(a, "question"))).unwrap_or_default(),
                options,
            })
        })
        .collect();
    Some(Feed {
        now: n(v, "now"),
        sessions,
    })
}

/// The K5 mark of a tile or row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Solid violet tile, violet verb and bar.
    Waiting,
    /// Faint violet ring with a gap, spinning.
    Working,
    /// 7 px violet dot; name in white 600.
    Unread,
    None,
}

pub fn mark(e: &Entry) -> Mark {
    if e.state.waiting() {
        Mark::Waiting
    } else if e.state == State::Work {
        Mark::Working
    } else if e.unread {
        Mark::Unread
    } else {
        Mark::None
    }
}

/// Class added to a top-bar tile (`#tabbar .apptab`) and to a feed row.
pub fn mark_class(m: Mark) -> &'static str {
    match m {
        Mark::Waiting => "fs-ask",
        Mark::Working => "fs-work",
        Mark::Unread => "fs-un",
        Mark::None => "",
    }
}

/// All the classes the feed may put on a tile; the glue clears them before applying one.
pub const TILE_CLASSES: [&str; 3] = ["fs-ask", "fs-work", "fs-un"];

pub fn verb(e: &Entry, en: bool) -> &'static str {
    match (e.state, en) {
        (State::Ask, false) => "pide permiso",
        (State::Ask, true) => "asks for permission",
        (State::Limit, false) => "llegó al límite de uso",
        (State::Limit, true) => "hit the usage limit",
        (State::Work, false) => "está trabajando",
        (State::Work, true) => "is working",
        (State::Idle, _) if e.reply.is_empty() => {
            if en {
                "no activity"
            } else {
                "sin actividad"
            }
        }
        (State::Idle, false) => "respondió",
        (State::Idle, true) => "replied",
    }
}

/// «hace cuánto»: `ahora`, `14 min`, `3 h`, `2 d`. Empty when the time is unknown.
pub fn ago(now: f64, ts: f64, en: bool) -> String {
    if ts <= 0.0 || now <= 0.0 {
        return String::new();
    }
    let minutes = ((now - ts).max(0.0) / 60.0).floor();
    if minutes < 1.0 {
        return if en { "now" } else { "ahora" }.into();
    }
    if minutes < 60.0 {
        return format!("{minutes} min");
    }
    let hours = (minutes / 60.0).round();
    if hours < 24.0 {
        return format!("{hours} h");
    }
    format!("{} d", (hours / 24.0).round())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Now,
    LastHour,
    Today,
    Before,
}

/// `day_start`: local midnight in epoch seconds (the browser knows the timezone).
pub fn group(now: f64, ts: f64, day_start: f64) -> Group {
    let age = now - ts;
    if ts <= 0.0 {
        Group::Before
    } else if age < 300.0 {
        Group::Now
    } else if age < 3600.0 {
        Group::LastHour
    } else if ts >= day_start {
        Group::Today
    } else {
        Group::Before
    }
}

pub fn group_label(g: Group, en: bool) -> &'static str {
    match (g, en) {
        (Group::Now, false) => "Ahora",
        (Group::Now, true) => "Now",
        (Group::LastHour, false) => "Última hora",
        (Group::LastHour, true) => "Last hour",
        (Group::Today, false) => "Hoy",
        (Group::Today, true) => "Today",
        (Group::Before, false) => "Antes",
        (Group::Before, true) => "Earlier",
    }
}

/// Avatar letters: first letter of the first two words, else the first two characters.
pub fn initials(label: &str) -> String {
    let words: Vec<&str> = label
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let out: String = match words.as_slice() {
        [] => String::new(),
        [one] => one.chars().take(2).collect(),
        [a, b, ..] => a.chars().take(1).chain(b.chars().take(1)).collect(),
    };
    out.to_uppercase()
}

/// Small corner glyph of the agent: ✻ Claude, ◎ Codex.
pub fn glyph(agent: &str) -> Option<(&'static str, &'static str)> {
    match agent {
        "claude" => Some(("✻", "claude")),
        "codex" => Some(("◎", "codex")),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Chip {
    #[default]
    All,
    Unread,
    Perm,
    Work,
}

impl Chip {
    pub fn parse(s: &str) -> Self {
        match s {
            "un" => Self::Unread,
            "perm" => Self::Perm,
            "work" => Self::Work,
            _ => Self::All,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Unread => "un",
            Self::Perm => "perm",
            Self::Work => "work",
        }
    }
    pub fn matches(self, e: &Entry) -> bool {
        match self {
            Self::All => true,
            Self::Unread => e.unread,
            Self::Perm => e.state == State::Ask,
            Self::Work => e.state == State::Work,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub all: usize,
    pub unread: usize,
    pub perm: usize,
    pub work: usize,
    /// `ask` + `limit`: the violet badge of the «Sesiones» tab.
    pub waiting: usize,
}

pub fn counts(list: &[Entry]) -> Counts {
    let count = |chip: Chip| list.iter().filter(|e| chip.matches(e)).count();
    Counts {
        all: list.len(),
        unread: count(Chip::Unread),
        perm: count(Chip::Perm),
        work: count(Chip::Work),
        waiting: list.iter().filter(|e| e.state.waiting()).count(),
    }
}

/// «TE ESPERA» (oldest first, outside the scroll) and the rest grouped by time, most
/// recent first. Both respect the chip.
pub fn layout(list: &[Entry], chip: Chip, now: f64, day_start: f64) -> Layout<'_> {
    let mut pinned: Vec<&Entry> = list
        .iter()
        .filter(|e| chip.matches(e) && e.state.waiting())
        .collect();
    pinned.sort_by(|a, b| a.ts.total_cmp(&b.ts));
    let mut rest: Vec<&Entry> = list
        .iter()
        .filter(|e| chip.matches(e) && !e.state.waiting())
        .collect();
    rest.sort_by(|a, b| b.ts.total_cmp(&a.ts));
    let mut groups: Vec<(Group, Vec<&Entry>)> = Vec::new();
    for e in rest {
        let g = group(now, e.ts, day_start);
        match groups.last_mut() {
            Some((last, rows)) if *last == g => rows.push(e),
            _ => groups.push((g, vec![e])),
        }
    }
    Layout { pinned, groups }
}

pub struct Layout<'a> {
    pub pinned: Vec<&'a Entry>,
    pub groups: Vec<(Group, Vec<&'a Entry>)>,
}

/// Digits the preview card answers with keys 1–9 (only over a visible menu).
pub fn digits(e: &Entry) -> Vec<&str> {
    if e.state != State::Ask {
        return Vec::new();
    }
    e.options.iter().map(|o| o.key.as_str()).collect()
}

/// Whether opening the session would move its «seen» mark (avoids redundant posts).
pub fn needs_seen(e: &Entry) -> bool {
    e.unread || e.ts > e.seen
}

/// The local effect of `POST /session-seen` until the next poll confirms it.
pub fn mark_seen(e: &mut Entry, now: f64) {
    e.seen = e.seen.max(now).max(e.ts);
    if !e.state.waiting() {
        e.unread = false;
    }
}

fn avatar(e: &Entry) -> String {
    let ag = glyph(&e.agent)
        .map(|(g, cls)| format!("<i class=\"fs-ag {cls}\">{g}</i>"))
        .unwrap_or_default();
    format!(
        "<span class=\"fs-av\" aria-hidden=\"true\">{}{ag}</span>",
        esc(&initials(&e.label))
    )
}

fn you(e: &Entry, en: bool) -> String {
    if e.prompt.is_empty() {
        return String::new();
    }
    format!(
        "<div class=\"fs-you\"><span class=\"fs-yl\">{}</span> <span>{}</span></div>",
        if en { "You:" } else { "Tú:" },
        esc(&e.prompt)
    )
}

/// The quoted block: the menu's question while it waits on one, else the agent's reply.
fn quote_text(e: &Entry) -> &str {
    if e.state == State::Ask && !e.question.is_empty() {
        &e.question
    } else {
        &e.reply
    }
}

fn head(e: &Entry, now: f64, en: bool) -> String {
    format!(
        "<div class=\"fs-l1\"><span class=\"fs-t\"><b>{}</b> <span class=\"fs-v\">{}</span></span><span class=\"fs-tm\">{}<i class=\"fs-mk\" aria-hidden=\"true\"></i></span></div>",
        esc(&e.label),
        verb(e, en),
        esc(&ago(now, e.ts, en))
    )
}

fn row_classes(e: &Entry, extra: &[(&str, bool)]) -> String {
    let mut c = vec!["fs-row"];
    let m = mark_class(mark(e));
    if !m.is_empty() {
        c.push(m);
    }
    if e.unread {
        c.push("un");
    }
    for (name, on) in extra {
        if *on {
            c.push(name);
        }
    }
    c.join(" ")
}

/// One G4 row of the feed (no buttons): avatar, name + verb + age + K5 mark, «Tú: …»
/// in one line, and the quoted question or reply (3 lines).
pub fn row_html(e: &Entry, now: f64, en: bool, hl: bool, active: bool) -> String {
    let q = quote_text(e);
    let quote = if q.is_empty() {
        String::new()
    } else {
        format!("<div class=\"fs-q\">{}</div>", esc(q))
    };
    format!(
        "<div class=\"{}\" data-session=\"{}\" role=\"button\" tabindex=\"0\">{}<div class=\"fs-bd\">{}{}{quote}</div></div>",
        row_classes(e, &[("hl", hl), ("on", active)]),
        esc(&e.session),
        avatar(e),
        head(e, now, en),
        you(e, en),
    )
}

/// The B4 preview card. `touch`: the phone's long-press card, whose menu options are
/// 44 px rows to tap (a phone has no 1 2 3 keys); otherwise numbered lines and a key footer.
pub fn card_html(e: &Entry, now: f64, en: bool, touch: bool) -> String {
    let menu = e.state == State::Ask && !e.options.is_empty();
    let q = quote_text(e);
    let quote = if q.is_empty() {
        String::new()
    } else {
        format!("<div class=\"fs-q\">{}</div>", esc(q))
    };
    let options = if !menu {
        String::new()
    } else if touch {
        let rows: String = e
            .options
            .iter()
            .map(|o| {
                format!(
                    "<button type=\"button\" data-flat class=\"fs-opt\" data-key=\"{}\"><kbd>{}</kbd><span>{}</span></button>",
                    esc(&o.key),
                    esc(&o.key),
                    esc(&o.label)
                )
            })
            .collect();
        format!("<div class=\"fs-opts\" role=\"group\">{rows}</div>")
    } else {
        let rows: String = e
            .options
            .iter()
            .map(|o| {
                format!(
                    "<div class=\"fs-ol\"><kbd>{}</kbd><span>{}</span></div>",
                    esc(&o.key),
                    esc(&o.label)
                )
            })
            .collect();
        format!("<div class=\"fs-opts\">{rows}</div>")
    };
    let foot = if touch {
        String::new()
    } else {
        let keys = if menu {
            let kbd: String = e
                .options
                .iter()
                .map(|o| format!("<kbd>{}</kbd>", esc(&o.key)))
                .collect();
            format!(
                "<span>{kbd}{}</span>",
                if en { "answer" } else { "responder" }
            )
        } else {
            String::new()
        };
        format!(
            "<div class=\"fs-ft\"><span><kbd>{}</kbd>{}</span>{keys}</div>",
            if en { "click" } else { "clic" },
            if en { "open" } else { "abrir" }
        )
    };
    format!(
        "<div class=\"{}\" data-session=\"{}\">{}<div class=\"fs-bd\">{}{}{quote}{options}</div></div>{foot}",
        row_classes(e, &[]),
        esc(&e.session),
        avatar(e),
        head(e, now, en),
        you(e, en),
    )
}

pub fn chips_html(c: &Counts, chip: Chip, en: bool) -> String {
    let items = [
        (Chip::All, if en { "All" } else { "Todas" }, c.all),
        (
            Chip::Unread,
            if en { "Unread" } else { "Sin leer" },
            c.unread,
        ),
        (
            Chip::Perm,
            if en { "Permissions" } else { "Permisos" },
            c.perm,
        ),
        (
            Chip::Work,
            if en { "Working" } else { "Trabajando" },
            c.work,
        ),
    ];
    let chips: String = items
        .iter()
        .map(|(k, label, count)| {
            let on = *k == chip;
            format!(
                "<button type=\"button\" data-flat class=\"fs-chip{}\" data-chip=\"{}\" aria-pressed=\"{on}\">{label}<i>{count}</i></button>",
                if on { " on" } else { "" },
                k.key()
            )
        })
        .collect();
    format!(
        "<div class=\"fs-chips\" role=\"toolbar\" aria-label=\"{}\">{chips}</div>",
        if en { "Filters" } else { "Filtros" }
    )
}

/// What the feed paints: chips, the pinned «TE ESPERA · N» and the grouped scroll.
pub struct View<'a> {
    pub chip: Chip,
    pub day_start: f64,
    pub en: bool,
    /// Session highlighted by the linked hover (row ↔ tile).
    pub hl: &'a str,
    /// Session open right now (its row gets `.on`).
    pub active: &'a str,
}

pub fn feed_html(feed: &Feed, v: &View) -> String {
    let c = counts(&feed.sessions);
    let lay = layout(&feed.sessions, v.chip, feed.now, v.day_start);
    let row = |e: &&Entry| row_html(e, feed.now, v.en, e.session == v.hl, e.session == v.active);
    let pinned = if lay.pinned.is_empty() {
        String::new()
    } else {
        format!(
            "<div class=\"fs-pin\"><div class=\"fs-gh\">{} · {}</div>{}</div>",
            if v.en { "Waiting for you" } else { "Te espera" },
            lay.pinned.len(),
            lay.pinned.iter().map(row).collect::<String>()
        )
    };
    let mut scroll = String::new();
    for (g, rows) in &lay.groups {
        scroll.push_str(&format!(
            "<div class=\"fs-gh\">{}</div>",
            group_label(*g, v.en)
        ));
        scroll.push_str(&rows.iter().map(row).collect::<String>());
    }
    if lay.pinned.is_empty() && lay.groups.is_empty() {
        scroll = format!(
            "<p class=\"fs-empty\">{}</p>",
            if v.en {
                "Nothing here."
            } else {
                "Nada por aquí."
            }
        );
    }
    format!(
        "{}{pinned}<div class=\"fs-scroll\">{scroll}</div>",
        chips_html(&c, v.chip, v.en)
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use scraper::{Html, Selector};
    use serde_json::json;

    fn sample() -> Value {
        json!({"now": 10_000.0, "sessions": [
            {"session":"term-r26549","label":"FerLongoria","state":"ask","unread":true,"ts":9_160.0,
             "seen":0.0,"pane":"%51","panes":1,"agent":"claude","model":"opus-5","prompt":"pushea\n todo",
             "reply":"¿Los subo?","ask":{"question":"Bash(git push origin main) · Do you want to proceed?",
             "options":[{"key":"1","label":"Yes"},{"key":"2","label":"Yes, and don't ask again"},
             {"key":"3","label":"No <esc>"}]}},
            {"session":"sava","label":"SAVA","state":"limit","unread":true,"ts":9_880.0,"agent":"claude",
             "reply":"Llegó al límite de uso.","ask":null},
            {"session":"deep","label":"DeepSearch","state":"work","unread":false,"ts":9_990.0,"agent":"codex",
             "reply":"Bash(curl -sI https://x)"},
            {"session":"lola","label":"Lola - Prospectos","state":"idle","unread":true,"ts":9_640.0,
             "seen":9_000.0,"agent":"claude","prompt":"haz páginas","reply":"One correction"},
            {"session":"old","label":"Relotto","state":"idle","unread":false,"ts":2_000.0,"seen":2_100.0,
             "agent":"gemini","reply":""},
            {"label":"no session"}
        ]})
    }

    #[test]
    fn parses_the_route_and_rejects_other_bodies() {
        let f = parse(&sample()).unwrap();
        assert_eq!(f.now, 10_000.0);
        assert_eq!(f.sessions.len(), 5, "entries without a session are dropped");
        let fer = &f.sessions[0];
        assert_eq!(fer.state, State::Ask);
        assert_eq!(fer.prompt, "pushea todo", "prompt in one line");
        assert_eq!(fer.options.len(), 3);
        assert_eq!(digits(fer), ["1", "2", "3"]);
        assert_eq!(f.sessions[1].question, "");
        assert!(digits(&f.sessions[1]).is_empty(), "limit has no menu");
        assert!(parse(&json!({"error":"not found"})).is_none());
        assert!(parse(&json!("<html>")).is_none());
        let blank = parse(&json!({"sessions":[{"session":"s","label":"  "}]})).unwrap();
        assert_eq!(blank.sessions[0].label, "s");
    }

    #[test]
    fn option_keys_are_single_digits_only() {
        let f = parse(
            &json!({"sessions":[{"session":"s","state":"ask","ask":{"question":"q",
            "options":[{"key":"0","label":"x"},{"key":"10","label":"y"},{"key":"9","label":"z"},
            {"key":"a","label":"w"}]}}]}),
        )
        .unwrap();
        assert_eq!(digits(&f.sessions[0]), ["9"]);
    }

    #[test]
    fn k5_marks_follow_urgency() {
        let f = parse(&sample()).unwrap();
        let marks: Vec<_> = f.sessions.iter().map(mark).collect();
        assert_eq!(
            marks,
            [
                Mark::Waiting,
                Mark::Waiting,
                Mark::Working,
                Mark::Unread,
                Mark::None
            ]
        );
        assert_eq!(mark_class(Mark::Waiting), "fs-ask");
        assert_eq!(mark_class(Mark::None), "");
        for m in [Mark::Waiting, Mark::Working, Mark::Unread] {
            assert!(TILE_CLASSES.contains(&mark_class(m)));
        }
    }

    #[test]
    fn verbs_in_both_languages() {
        let f = parse(&sample()).unwrap();
        let es: Vec<_> = f.sessions.iter().map(|e| verb(e, false)).collect();
        assert_eq!(
            es,
            [
                "pide permiso",
                "llegó al límite de uso",
                "está trabajando",
                "respondió",
                "sin actividad"
            ]
        );
        assert_eq!(verb(&f.sessions[3], true), "replied");
    }

    #[test]
    fn age_and_groups() {
        assert_eq!(ago(1000.0, 990.0, false), "ahora");
        assert_eq!(ago(1000.0, 990.0, true), "now");
        assert_eq!(ago(10_000.0, 9_160.0, false), "14 min");
        assert_eq!(ago(100_000.0, 100_000.0 - 3.0 * 3600.0, false), "3 h");
        assert_eq!(
            ago(10_000_000.0, 10_000_000.0 - 49.0 * 3600.0, false),
            "2 d"
        );
        assert_eq!(ago(10_000.0, 0.0, false), "");
        assert_eq!(ago(10_000.0, 10_050.0, false), "ahora", "clock skew");
        let day = 5_000.0;
        assert_eq!(group(10_000.0, 9_800.0, day), Group::Now);
        assert_eq!(group(10_000.0, 9_000.0, day), Group::LastHour);
        assert_eq!(group(10_000.0, 6_000.0, day), Group::Today);
        assert_eq!(group(10_000.0, 4_000.0, day), Group::Before);
        assert_eq!(group(10_000.0, 0.0, day), Group::Before);
        assert_eq!(group_label(Group::LastHour, false), "Última hora");
    }

    #[test]
    fn initials_and_glyphs() {
        assert_eq!(initials("FerLongoria"), "FE");
        assert_eq!(initials("Lola - Prospectos"), "LP");
        assert_eq!(initials("0xAI Website"), "0W");
        assert_eq!(initials("FerLongoria 2"), "F2");
        assert_eq!(initials("é"), "É");
        assert_eq!(initials("  "), "");
        assert_eq!(glyph("claude"), Some(("✻", "claude")));
        assert_eq!(glyph("codex"), Some(("◎", "codex")));
        assert_eq!(glyph("grok"), None);
    }

    #[test]
    fn chip_counts_and_filters() {
        let f = parse(&sample()).unwrap();
        let c = counts(&f.sessions);
        assert_eq!(
            c,
            Counts {
                all: 5,
                unread: 3,
                perm: 1,
                work: 1,
                waiting: 2
            }
        );
        assert_eq!(Chip::parse("perm"), Chip::Perm);
        assert_eq!(Chip::parse("x"), Chip::All);
        assert_eq!(Chip::Work.key(), "work");
    }

    #[test]
    fn pinned_oldest_first_rest_newest_first_grouped() {
        let f = parse(&sample()).unwrap();
        let lay = layout(&f.sessions, Chip::All, f.now, 5_000.0);
        let pinned: Vec<_> = lay.pinned.iter().map(|e| e.session.as_str()).collect();
        assert_eq!(pinned, ["term-r26549", "sava"], "the longest waiting first");
        let groups: Vec<_> = lay
            .groups
            .iter()
            .map(|(g, rows)| {
                (
                    *g,
                    rows.iter().map(|e| e.session.as_str()).collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            groups,
            [
                (Group::Now, vec!["deep"]),
                (Group::LastHour, vec!["lola"]),
                (Group::Before, vec!["old"])
            ]
        );
        let only_work = layout(&f.sessions, Chip::Work, f.now, 5_000.0);
        assert!(only_work.pinned.is_empty());
        assert_eq!(only_work.groups.len(), 1);
    }

    #[test]
    fn seen_bookkeeping() {
        let f = parse(&sample()).unwrap();
        let mut lola = f.sessions[3].clone();
        assert!(needs_seen(&lola));
        mark_seen(&mut lola, 10_000.0);
        assert!(!lola.unread);
        assert!(!needs_seen(&lola));
        let mut fer = f.sessions[0].clone();
        mark_seen(&mut fer, 10_000.0);
        assert!(fer.unread, "a waiting session stays marked until answered");
        assert!(!needs_seen(&f.sessions[4]));
    }

    fn sel(s: &str) -> Selector {
        Selector::parse(s).unwrap()
    }

    #[test]
    fn feed_html_structure() {
        let f = parse(&sample()).unwrap();
        let v = View {
            chip: Chip::All,
            day_start: 5_000.0,
            en: false,
            hl: "lola",
            active: "deep",
        };
        let doc = Html::parse_fragment(&feed_html(&f, &v));
        let chips: Vec<String> = doc
            .select(&sel(".fs-chip"))
            .map(|c| c.text().collect())
            .collect();
        assert_eq!(chips, ["Todas5", "Sin leer3", "Permisos1", "Trabajando1"]);
        assert_eq!(doc.select(&sel(".fs-chip.on")).count(), 1);
        let pin = doc.select(&sel(".fs-pin")).next().unwrap();
        let gh: String = pin.select(&sel(".fs-gh")).next().unwrap().text().collect();
        assert_eq!(gh, "Te espera · 2");
        assert_eq!(pin.select(&sel(".fs-row.fs-ask")).count(), 2);
        let heads: Vec<String> = doc
            .select(&sel(".fs-scroll .fs-gh"))
            .map(|c| c.text().collect())
            .collect();
        assert_eq!(heads, ["Ahora", "Última hora", "Antes"]);
        assert_eq!(
            doc.select(&sel(".fs-row[data-session=lola].hl.fs-un"))
                .count(),
            1
        );
        assert_eq!(
            doc.select(&sel(".fs-row[data-session=deep].on.fs-work"))
                .count(),
            1
        );
        // The quoted block shows the menu question while it waits on it, never buttons.
        let fer = doc
            .select(&sel(".fs-row[data-session=term-r26549]"))
            .next()
            .unwrap();
        let q: String = fer.select(&sel(".fs-q")).next().unwrap().text().collect();
        assert!(q.starts_with("Bash(git push origin main)"));
        assert_eq!(fer.select(&sel("button")).count(), 0);
        let you: String = fer.select(&sel(".fs-you")).next().unwrap().text().collect();
        assert_eq!(you, "Tú: pushea todo");
        assert_eq!(
            fer.select(&sel(".fs-av"))
                .next()
                .unwrap()
                .text()
                .collect::<String>(),
            "FE✻"
        );
        // No reply and idle: no quoted block.
        let old = doc
            .select(&sel(".fs-row[data-session=old]"))
            .next()
            .unwrap();
        assert_eq!(old.select(&sel(".fs-q")).count(), 0);
        assert_eq!(old.select(&sel(".fs-you")).count(), 0);
    }

    #[test]
    fn feed_html_escapes_foreign_text() {
        let f = parse(
            &json!({"now":1.0,"sessions":[{"session":"a\"b","label":"<img>","state":"idle",
            "ts":1.0,"reply":"<script>x</script>","prompt":"&"}]}),
        )
        .unwrap();
        let html = feed_html(
            &f,
            &View {
                chip: Chip::All,
                day_start: 0.0,
                en: true,
                hl: "",
                active: "",
            },
        );
        assert!(!html.contains("<img>"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("data-session=\"a&quot;b\""));
        assert!(html.contains("You:"));
    }

    #[test]
    fn empty_filter_says_so() {
        let f = parse(&json!({"now":1.0,"sessions":[]})).unwrap();
        let v = View {
            chip: Chip::Perm,
            day_start: 0.0,
            en: false,
            hl: "",
            active: "",
        };
        let doc = Html::parse_fragment(&feed_html(&f, &v));
        assert_eq!(doc.select(&sel(".fs-empty")).count(), 1);
        assert_eq!(doc.select(&sel(".fs-pin")).count(), 0);
    }

    #[test]
    fn preview_card_desktop_and_phone() {
        let f = parse(&sample()).unwrap();
        let fer = &f.sessions[0];
        let desk = Html::parse_fragment(&card_html(fer, f.now, false, false));
        let lines: Vec<String> = desk
            .select(&sel(".fs-ol"))
            .map(|c| c.text().collect())
            .collect();
        assert_eq!(lines[0], "1Yes");
        assert_eq!(lines[2], "3No <esc>");
        let foot: String = desk.select(&sel(".fs-ft")).next().unwrap().text().collect();
        assert_eq!(foot, "clicabrir123responder");
        assert_eq!(desk.select(&sel("button")).count(), 0);
        let phone = Html::parse_fragment(&card_html(fer, f.now, false, true));
        let keys: Vec<&str> = phone
            .select(&sel("button.fs-opt"))
            .filter_map(|b| b.value().attr("data-key"))
            .collect();
        assert_eq!(keys, ["1", "2", "3"]);
        assert_eq!(phone.select(&sel(".fs-ft")).count(), 0);
        // A reply card has no menu nor answer keys.
        let lola = Html::parse_fragment(&card_html(&f.sessions[3], f.now, false, false));
        assert_eq!(lola.select(&sel(".fs-opts")).count(), 0);
        let foot: String = lola.select(&sel(".fs-ft")).next().unwrap().text().collect();
        assert_eq!(foot, "clicabrir");
        let q: String = lola.select(&sel(".fs-q")).next().unwrap().text().collect();
        assert_eq!(q, "One correction");
    }

    /// Static page to eyeball the real CSS (no browser needed to produce it):
    /// `FS_FIXTURE_OUT=/abs/feed.html cargo test -p comandos-web-view session_feed::tests::fixture -- --ignored`
    #[test]
    #[ignore]
    fn fixture() {
        let Ok(out) = std::env::var("FS_FIXTURE_OUT") else {
            return;
        };
        let f = parse(&sample()).unwrap();
        let page_css = include_str!("index_page.css");
        let feed_css = include_str!("../../../dash/workspace.css");
        let tile = |e: &Entry, on: bool, hl: bool| {
            format!(
                "<div class=\"apptab {}{}{}\"><span class=\"dot\" hidden></span><span class=\"lbl\">{}</span><span class=\"mdl\">opus-5</span><span class=\"ws-count\" hidden></span><i class=\"fs-mk\"></i><span class=\"x\">×</span></div>",
                mark_class(mark(e)),
                if on { " on" } else { "" },
                if hl { " fs-hl" } else { "" },
                esc(&e.label)
            )
        };
        let tiles: String = f
            .sessions
            .iter()
            .enumerate()
            .map(|(i, e)| tile(e, i == 0, i == 3))
            .collect();
        let v = View {
            chip: Chip::All,
            day_start: 5_000.0,
            en: false,
            hl: "lola",
            active: "",
        };
        let page = format!(
            "<!doctype html><html data-feed=\"1\"><head><meta charset=\"utf-8\"><style>{page_css}</style><style>{feed_css}</style><style>body{{display:block;overflow:auto}}.fx{{display:flex;gap:16px;padding:16px;align-items:flex-start}}.fx>div{{position:relative}}.fs-peek{{position:static;width:320px}}</style></head><body class=\"app tabs-rows\"><div id=\"app-navigation\"><div id=\"tabbar\">{tiles}</div></div><div class=\"fx\"><div id=\"command-sidebar\" style=\"width:300px;height:640px;display:flex;flex-direction:column\"><div class=\"cs-tools\"><button type=\"button\" data-flat class=\"tb\"><span class=\"lb\">Archivos</span></button><button type=\"button\" data-flat class=\"tb on\"><span class=\"lb\">Sesiones</span><b class=\"fs-n\">2</b></button></div><div class=\"cs-empty-terms\" style=\"flex:1\"><div class=\"cs-feed\">{}</div></div></div><div class=\"fs-peek\">{}</div><div class=\"fs-peek\">{}</div><div class=\"fs-peek touch\">{}</div></div></body></html>",
            feed_html(&f, &v),
            card_html(&f.sessions[0], f.now, false, false),
            card_html(&f.sessions[3], f.now, false, false),
            card_html(&f.sessions[0], f.now, false, true),
        );
        std::fs::write(out, page).unwrap();
    }
}
