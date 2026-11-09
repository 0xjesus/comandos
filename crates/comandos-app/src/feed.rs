//! «Ficha viva» (design/ficha-viva-spec.md): `GET /session-feed` → K5 tile state,
//! the hover preview text, the digits a preview may answer and when to mark seen.
//! Pure logic; GTK lives in `ui::feed_preview` and `ui::app_feed`.
use serde_json::Value;
use std::collections::BTreeMap;

/// Every class the K5 tile can carry; removed before the current ones are added.
pub const K5_CLASSES: [&str; 4] = ["k5-ask", "k5-unread", "k5-work", "k5-quiet"];
/// Linked hover with the web feed row.
pub const HIGHLIGHT_CLASS: &str = "k5-hl";
/// The marker slot is always reserved, so a state change never moves the text.
pub const MARKER_SIZE: i32 = 9;
/// Hover preview geometry and timing (B4).
pub const PREVIEW_WIDTH: i32 = 320;
pub const PREVIEW_GAP: i32 = 6;
pub const PREVIEW_DELAY_MS: u64 = 350;
pub const PREVIEW_GRACE_MS: u64 = 150;
/// Same pause as `comandos_notifyd::actions::CHOOSE_DELAY` between digit and Enter.
pub const CHOOSE_ENTER_MS: u64 = 250;
/// One full turn of the working ring.
pub const RING_TURN_SECS: f64 = 1.1;
/// Consecutive failed polls after which the tiles go back to today's look.
pub const FEED_FAILURES_TO_CLEAR: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedState {
    Ask,
    Limit,
    Work,
    Idle,
}
impl FeedState {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "ask" => Self::Ask,
            "limit" => Self::Limit,
            "work" => Self::Work,
            _ => Self::Idle,
        }
    }
    /// «Te espera»: a numbered menu or a usage-limit text on screen.
    pub fn waits(self) -> bool {
        matches!(self, Self::Ask | Self::Limit)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskOption {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub question: String,
    pub options: Vec<AskOption>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeedEntry {
    pub session: String,
    pub label: String,
    pub state: FeedState,
    pub unread: bool,
    pub ts: f64,
    pub seen: f64,
    pub pane: String,
    pub panes: u64,
    pub agent: String,
    pub model: String,
    pub prompt: String,
    pub reply: String,
    pub ask: Option<Ask>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Feed {
    pub now: f64,
    pub sessions: BTreeMap<String, FeedEntry>,
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
fn number(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.)
}

fn parse_ask(value: &Value) -> Option<Ask> {
    let object = value.as_object()?;
    let options = object
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|option| {
            let key = option.get("key").and_then(Value::as_str)?;
            // Only single digits 1–9 are answerable (server `/key` allow-list).
            (key.len() == 1 && key.bytes().all(|b| (b'1'..=b'9').contains(&b))).then(|| AskOption {
                key: key.to_string(),
                label: text(option, "label"),
            })
        })
        .collect::<Vec<_>>();
    Some(Ask {
        question: object
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        options,
    })
}

/// `None` when the document is not a feed (404 body, `null`, an old server…).
pub fn parse_feed(value: &Value) -> Option<Feed> {
    let sessions = value.get("sessions")?.as_array()?;
    let mut out = Feed {
        now: number(value, "now"),
        sessions: BTreeMap::new(),
    };
    for item in sessions {
        let session = text(item, "session");
        if session.is_empty() || !comandos_desktop::validation::valid_session(&session) {
            continue;
        }
        let state = FeedState::parse(item.get("state").and_then(Value::as_str).unwrap_or(""));
        let entry = FeedEntry {
            label: text(item, "label"),
            state,
            unread: item.get("unread").and_then(Value::as_bool).unwrap_or(false),
            ts: number(item, "ts"),
            seen: number(item, "seen"),
            pane: text(item, "pane"),
            panes: item.get("panes").and_then(Value::as_u64).unwrap_or(1),
            agent: text(item, "agent"),
            model: text(item, "model"),
            prompt: text(item, "prompt"),
            reply: text(item, "reply"),
            ask: item.get("ask").and_then(parse_ask),
            session: session.clone(),
        };
        out.sessions.insert(session, entry);
    }
    Some(out)
}

/// What the 9 px slot at the end of a tile draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    None,
    /// Te espera: dark dot on the solid violet tile.
    Ask,
    /// Sin leer: 7 px violet dot.
    Dot,
    /// Trabajando: 9 px violet ring at 60 % with a gap at the top, rotating.
    Ring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileLook {
    pub class: Option<&'static str>,
    pub marker: Marker,
}
impl TileLook {
    /// Without a feed the tile looks exactly like today.
    pub const TODAY: Self = Self {
        class: None,
        marker: Marker::None,
    };
}

/// K5: one violet, more intense = more urgent. `current_seen` is true when the
/// session is the open tab of a focused window: its «unread» is being read now.
pub fn tile_look(feed: Option<&Feed>, session: &str, current_seen: bool) -> TileLook {
    let Some(feed) = feed else {
        return TileLook::TODAY;
    };
    let quiet = TileLook {
        class: Some("k5-quiet"),
        marker: Marker::None,
    };
    let Some(entry) = feed.sessions.get(session) else {
        return quiet;
    };
    match entry.state {
        FeedState::Ask | FeedState::Limit => TileLook {
            class: Some("k5-ask"),
            marker: Marker::Ask,
        },
        FeedState::Work => TileLook {
            class: Some("k5-work"),
            marker: Marker::Ring,
        },
        FeedState::Idle if entry.unread && !current_seen => TileLook {
            class: Some("k5-unread"),
            marker: Marker::Dot,
        },
        FeedState::Idle => quiet,
    }
}

/// Header verb of the preview (and of the web feed row).
pub fn verb(entry: &FeedEntry, english: bool) -> &'static str {
    match (entry.state, english) {
        (FeedState::Ask, false) => "pide permiso",
        (FeedState::Ask, true) => "asks permission",
        (FeedState::Limit, false) => "llegó al límite de uso",
        (FeedState::Limit, true) => "hit the usage limit",
        (FeedState::Work, false) => "está trabajando",
        (FeedState::Work, true) => "is working",
        (FeedState::Idle, false) if !entry.reply.is_empty() => "respondió",
        (FeedState::Idle, true) if !entry.reply.is_empty() => "replied",
        (FeedState::Idle, false) => "sin actividad",
        (FeedState::Idle, true) => "no activity",
    }
}

/// «hace cuánto»: `ahora`, `N min`, `N h`, `N d`; empty when the time is unknown.
pub fn time_ago(now: f64, ts: f64, english: bool) -> String {
    if ts <= 0. || !now.is_finite() || !ts.is_finite() {
        return String::new();
    }
    let seconds = (now - ts).max(0.);
    if seconds < 60. {
        if english { "now" } else { "ahora" }.into()
    } else if seconds < 3600. {
        format!("{} min", (seconds / 60.).floor() as u64)
    } else if seconds < 86400. {
        format!("{} h", (seconds / 3600.).round() as u64)
    } else {
        format!("{} d", (seconds / 86400.).round() as u64)
    }
}

/// Initials of the avatar: first letters of the first two words.
pub fn initials(name: &str) -> String {
    let words = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>();
    let out: String = match words.as_slice() {
        [] => String::new(),
        [one] => one.chars().take(2).collect(),
        [first, second, ..] => first
            .chars()
            .take(1)
            .chain(second.chars().take(1))
            .collect(),
    };
    out.to_uppercase()
}

/// Small corner glyph of the avatar: ✻ Claude, ◎ Codex, nothing otherwise.
pub fn agent_glyph(agent: &str) -> &'static str {
    match agent {
        "claude" => "✻",
        "codex" => "◎",
        _ => "",
    }
}

/// The quoted block: the menu question when the agent asks, else its reply.
pub fn quote(entry: &FeedEntry) -> &str {
    match (&entry.ask, entry.state) {
        (Some(ask), FeedState::Ask) if !ask.question.is_empty() => &ask.question,
        _ => &entry.reply,
    }
}

/// Options shown under the question (only while the agent asks).
pub fn menu(entry: &FeedEntry) -> &[AskOption] {
    match (&entry.ask, entry.state) {
        (Some(ask), FeedState::Ask) => &ask.options,
        _ => &[],
    }
}

/// The keys of the footer hint «1 2 3 responder»; empty without a menu.
pub fn answer_keys(entry: &FeedEntry) -> Vec<&str> {
    menu(entry).iter().map(|o| o.key.as_str()).collect()
}

/// A typed digit answers the preview's session only when that digit is one of
/// the menu options on screen. Every other key passes through untouched.
pub fn answer_digit(entry: Option<&FeedEntry>, digit: char) -> Option<String> {
    let entry = entry?;
    if entry.state != FeedState::Ask {
        return None;
    }
    let digit = digit.to_string();
    menu(entry).iter().any(|o| o.key == digit).then_some(digit)
}

/// The two `POST /key` bodies of an answer: the digit, then Enter 250 ms later.
pub fn answer_payloads(entry: &FeedEntry, digit: &str) -> [Value; 2] {
    let body = |key: &str| {
        let mut map = serde_json::Map::new();
        map.insert("session".into(), Value::String(entry.session.clone()));
        map.insert("key".into(), Value::String(key.into()));
        if valid_pane(&entry.pane) {
            map.insert("pane".into(), Value::String(entry.pane.clone()));
        }
        Value::Object(map)
    };
    [body(digit), body("Enter")]
}

/// `%<digits>` like the notifier's pane check.
pub fn valid_pane(pane: &str) -> bool {
    pane.strip_prefix('%')
        .is_some_and(|n| !n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// On a feed poll: mark the open tab seen while the window has the focus and
/// there is something newer than the last mark (the server's `unread` rule).
pub fn seen_on_poll(feed: &Feed, current: Option<&str>, focused: bool) -> Option<String> {
    let current = current?;
    if !focused {
        return None;
    }
    let entry = feed.sessions.get(current)?;
    (entry.unread || entry.ts > entry.seen).then(|| current.to_string())
}

/// On a tab switch: the newly opened tab is seen when the window has the focus.
pub fn seen_on_switch(
    previous: Option<&str>,
    current: Option<&str>,
    focused: bool,
) -> Option<String> {
    let current = current?;
    (focused && previous != Some(current) && comandos_desktop::validation::valid_session(current))
        .then(|| current.to_string())
}

/// Where the preview opens: under the tile, aligned to its left edge, or to its
/// right edge when it would overflow the window. Never over the tile itself.
pub fn preview_position(
    tile: (i32, i32, i32, i32),
    bounds: (i32, i32),
    width: i32,
    gap: i32,
) -> (i32, i32) {
    let (x, y, w, h) = tile;
    let (left, right) = bounds;
    let mut px = x;
    if px + width > right {
        px = x + w - width;
    }
    if px < left {
        px = left;
    }
    (px, y + h + gap)
}

/// `{"feedHover": "<session>" | ""}` from the web feed. `Some("")` clears.
pub fn feed_hover_message(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let hover = value.as_object()?.get("feedHover")?;
    match hover {
        Value::Null => Some(String::new()),
        Value::String(s) if s.is_empty() => Some(String::new()),
        Value::String(s) if comandos_desktop::validation::valid_session(s) => Some(s.clone()),
        _ => None,
    }
}

/// The quiet call into the web feed; the argument is serialized, never interpolated.
pub fn hover_script(session: &str) -> String {
    let arg = serde_json::to_string(session).unwrap_or_else(|_| "\"\"".into());
    format!(
        "try{{window.ComandosFeed&&typeof window.ComandosFeed.hover==='function'&&window.ComandosFeed.hover({arg})}}catch(e){{}}void 0;"
    )
}

/// Ring arc in radians for `seconds` since start (static when animations are off).
pub fn ring_angle(seconds: f64, animate: bool) -> f64 {
    if !animate {
        return 0.;
    }
    (seconds / RING_TURN_SECS).fract() * std::f64::consts::TAU
}

/// Collapse whitespace so one-line labels never break.
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
