//! tmux control mode (`tmux -C attach`): the protocol only, no PTY, no GTK.
//!
//! In control mode tmux does not draw a screen. It sends each pane's raw
//! output (`%output %3 …`) and layout changes as text lines, and answers
//! commands between `%begin` and `%end`. The app then owns what is shown:
//! one engine per pane, its own history, scroll and selection.
//!
//! This module turns the byte stream into [`Event`]s and builds the commands
//! the app sends back. Checked against tmux 3.2a.
use std::collections::VecDeque;

/// A line longer than this is dropped (a pane cannot produce one: tmux splits
/// `%output` well below it).
const MAX_LINE: usize = 1 << 20;
/// Lines kept for one command reply.
const MAX_REPLY_LINES: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneRect {
    pub id: u32,
    pub left: u16,
    pub top: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub cols: u16,
    pub rows: u16,
    pub panes: Vec<PaneRect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Raw bytes a pane's program wrote, already unescaped.
    Output { pane: u32, data: Vec<u8> },
    /// The answer to the command with this number (`ok` false: `%error`).
    Reply { number: u64, ok: bool, lines: Vec<Vec<u8>> },
    /// The window's panes moved or changed size.
    LayoutChange { window: u32, layout: Layout },
    /// tmux stopped sending this pane's output (the client fell behind).
    Pause { pane: u32 },
    Continue { pane: u32 },
    WindowAdd { window: u32 },
    WindowClose { window: u32 },
    SessionChanged { session: u32, name: String },
    /// The client is detached; nothing follows.
    Exit { reason: String },
    /// A notification this client does not act on.
    Other(String),
    /// A line that is not protocol: tmux could not attach, or what the
    /// wrapper prints after the client ends.
    Text(String),
}

struct Block {
    number: u64,
    lines: Vec<Vec<u8>>,
}

/// Splits the control stream into events. Feed it whatever the pipe returns.
#[derive(Default)]
pub struct Parser {
    line: Vec<u8>,
    overflow: bool,
    block: Option<Block>,
}

impl Parser {
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Event>) {
        for &b in bytes {
            if b != b'\n' {
                if self.line.len() < MAX_LINE {
                    self.line.push(b);
                } else {
                    self.overflow = true;
                }
                continue;
            }
            let mut line = std::mem::take(&mut self.line);
            if std::mem::take(&mut self.overflow) {
                continue;
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.line_done(line, out);
        }
    }

    fn line_done(&mut self, line: Vec<u8>, out: &mut Vec<Event>) {
        if let Some(block) = &mut self.block {
            // Inside a reply only its own `%end`/`%error` is protocol; any
            // other line, even one starting with `%`, is command output.
            let closes = |word: &[u8]| {
                line.strip_prefix(word)
                    .and_then(guard_number)
                    .is_some_and(|n| n == block.number)
            };
            let (end, error) = (closes(b"%end "), closes(b"%error "));
            if end || error {
                let block = self.block.take().unwrap_or(Block { number: 0, lines: Vec::new() });
                out.push(Event::Reply { number: block.number, ok: end, lines: block.lines });
            } else if block.lines.len() < MAX_REPLY_LINES {
                block.lines.push(line);
            }
            return;
        }
        let begin = line.strip_prefix(b"\x1bP1000p").map_or(line.as_slice(), |rest| rest);
        if let Some(number) = begin.strip_prefix(b"%begin ").and_then(guard_number) {
            self.block = Some(Block { number, lines: Vec::new() });
            return;
        }
        // `-CC` wraps the stream in a DCS: `ESC P 1000 p` … `ESC \`.
        let line = line.strip_prefix(b"\x1bP1000p").map_or(line.as_slice(), |rest| rest);
        let line = line.strip_prefix(b"\x1b\\").unwrap_or(line);
        if let Some(event) = notification(line) {
            out.push(event);
        } else if !line.is_empty() {
            out.push(Event::Text(String::from_utf8_lossy(line).into_owned()));
        }
    }
}

/// `<time> <number> <flags>` → the command number.
fn guard_number(rest: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(rest).ok()?;
    text.split(' ').nth(1)?.parse().ok()
}

fn id(text: &str, mark: char) -> Option<u32> {
    text.strip_prefix(mark)?.parse().ok()
}

fn notification(line: &[u8]) -> Option<Event> {
    if !line.starts_with(b"%") {
        return None;
    }
    let space = line.iter().position(|&b| b == b' ').unwrap_or(line.len());
    let (word, rest) = line.split_at(space);
    let rest = rest.get(1..).unwrap_or_default();
    match word {
        b"%output" => {
            let cut = rest.iter().position(|&b| b == b' ').unwrap_or(rest.len());
            let pane = id(std::str::from_utf8(rest.get(..cut)?).ok()?, '%')?;
            let data = unescape(rest.get(cut + 1..).unwrap_or_default());
            Some(Event::Output { pane, data })
        }
        // `%extended-output %0 <age> … : <data>`
        b"%extended-output" => {
            let cut = rest.iter().position(|&b| b == b' ')?;
            let pane = id(std::str::from_utf8(rest.get(..cut)?).ok()?, '%')?;
            let at = rest.windows(3).position(|w| w == b" : ")?;
            Some(Event::Output { pane, data: unescape(rest.get(at + 3..)?) })
        }
        _ => {
            let text = String::from_utf8_lossy(rest);
            let mut fields = text.split(' ');
            let first = fields.next().unwrap_or_default();
            Some(match word {
                b"%layout-change" => Event::LayoutChange {
                    window: id(first, '@')?,
                    layout: parse_layout(fields.next()?)?,
                },
                b"%pause" => Event::Pause { pane: id(first, '%')? },
                b"%continue" => Event::Continue { pane: id(first, '%')? },
                b"%window-add" => Event::WindowAdd { window: id(first, '@')? },
                b"%window-close" | b"%unlinked-window-close" => Event::WindowClose { window: id(first, '@')? },
                b"%session-changed" => Event::SessionChanged {
                    session: id(first, '$')?,
                    name: fields.collect::<Vec<_>>().join(" "),
                },
                b"%exit" => Event::Exit { reason: text.into_owned() },
                _ => Event::Other(String::from_utf8_lossy(line).into_owned()),
            })
        }
    }
}

/// tmux writes bytes below space and the backslash as `\ooo`; the rest is raw.
pub fn unescape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while let Some(&byte) = data.get(i) {
        let octal = |k: usize| data.get(i + k).filter(|b| (b'0'..=b'7').contains(b)).map(|b| u32::from(b - b'0'));
        if byte == b'\\'
            && let (Some(a), Some(b), Some(c)) = (octal(1), octal(2), octal(3))
            && let Ok(byte) = u8::try_from(a * 64 + b * 8 + c)
        {
            out.push(byte);
            i += 4;
        } else {
            out.push(byte);
            i += 1;
        }
    }
    out
}

/// `f91d,120x40,0,0{60x40,0,0,0,59x40,61,0,1}` → the window size and its
/// panes. Containers (`{}` side by side, `[]` stacked) only carry geometry.
pub fn parse_layout(text: &str) -> Option<Layout> {
    let (_checksum, body) = text.split_once(',')?;
    let bytes = body.as_bytes();
    let mut at = 0;
    let mut panes = Vec::new();
    let (cols, rows) = layout_cell(bytes, &mut at, &mut panes, 0)?;
    (at == bytes.len() && !panes.is_empty()).then_some(Layout { cols, rows, panes })
}

fn layout_number(b: &[u8], at: &mut usize) -> Option<u32> {
    let start = *at;
    while b.get(*at).is_some_and(u8::is_ascii_digit) {
        *at += 1;
    }
    std::str::from_utf8(b.get(start..*at)?).ok()?.parse().ok()
}

fn layout_cell(b: &[u8], at: &mut usize, panes: &mut Vec<PaneRect>, depth: u8) -> Option<(u16, u16)> {
    if depth > 32 {
        return None;
    }
    let number = |at: &mut usize, sep: u8| -> Option<u16> {
        let n = u16::try_from(layout_number(b, at)?).ok()?;
        if sep != 0 {
            (b.get(*at) == Some(&sep)).then_some(())?;
            *at += 1;
        }
        Some(n)
    };
    let width = number(at, b'x')?;
    let height = number(at, b',')?;
    let left = number(at, b',')?;
    let top = number(at, 0)?;
    match b.get(*at) {
        Some(b',') => {
            *at += 1;
            let id = layout_number(b, at)?;
            panes.push(PaneRect { id, left, top, width, height });
        }
        Some(&open @ (b'{' | b'[')) => {
            let close = if open == b'{' { b'}' } else { b']' };
            loop {
                *at += 1;
                layout_cell(b, at, panes, depth + 1)?;
                match b.get(*at) {
                    Some(b',') => {}
                    Some(&c) if c == close => {
                        *at += 1;
                        break;
                    }
                    _ => return None,
                }
            }
        }
        _ => return None,
    }
    Some((width, height))
}

/// What a queued command was for, returned with its reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// Sent for its effect; the reply only matters when it is an error.
    Plain,
    /// The four parts of a pane snapshot, read by tmux at one instant: the
    /// history above the screen with wrapped lines joined,
    Above { pane: u32 },
    /// the screen row by row with its colours,
    Rows { pane: u32 },
    /// the screen as bare text with wrapped lines joined (which rows wrap),
    Joined { pane: u32 },
    /// the same two readings of the screen a full-screen program covers
    /// (empty when none does),
    MainRows { pane: u32 },
    MainJoined { pane: u32 },
    /// and the pane's cursor, size and modes ([`PANES_FORMAT`]).
    State { pane: u32 },
    /// A check of what the engine shows: the screen,
    CheckRows { pane: u32 },
    /// and the cursor at that instant.
    CheckState { pane: u32 },
    /// `list-panes` with [`PANES_FORMAT`].
    Panes,
}

/// Commands in the order sent. tmux answers in that order, so each `Reply`
/// belongs to the oldest one waiting.
#[derive(Default)]
pub struct Commands {
    /// With each: whether the next one was sent on the same line.
    waiting: VecDeque<(Ask, bool)>,
    /// The reply tmux sends on attach, before any command of ours.
    greeted: bool,
}

impl Commands {
    /// Appends one command line to `wire` and remembers what it asked.
    fn push(&mut self, wire: &mut Vec<u8>, ask: Ask, line: &str) {
        debug_assert!(!line.contains('\n'));
        self.push_line(wire, &[ask], line);
    }

    /// One line of commands joined by ` ; `: tmux runs them without reading
    /// any pane in between and answers each one.
    fn push_line(&mut self, wire: &mut Vec<u8>, asks: &[Ask], line: &str) {
        debug_assert!(!line.contains('\n'));
        wire.extend_from_slice(line.as_bytes());
        wire.push(b'\n');
        for (i, ask) in asks.iter().enumerate() {
            self.waiting.push_back((ask.clone(), i + 1 < asks.len()));
        }
    }

    /// The command a reply answers (`None`: the attach greeting, or a reply
    /// nobody asked for). When a command fails tmux drops the rest of its
    /// line, so those are not waited for.
    pub fn answered(&mut self, ok: bool) -> Option<Ask> {
        if !std::mem::replace(&mut self.greeted, true) {
            return None;
        }
        let (ask, mut more) = self.waiting.pop_front()?;
        while !ok && more {
            more = self.waiting.pop_front().is_some_and(|(_, more)| more);
        }
        Some(ask)
    }

    pub fn pending(&self) -> usize {
        self.waiting.len()
    }

    /// The size this client offers for the window.
    pub fn resize(&mut self, wire: &mut Vec<u8>, cols: u16, rows: u16) {
        self.push(wire, Ask::Plain, &format!("refresh-client -C {cols},{rows}"));
    }

    /// Keys or pasted bytes for a pane, exactly as typed. Sent as hex so no
    /// byte is read as a key name; split so no line grows too long.
    pub fn send_bytes(&mut self, wire: &mut Vec<u8>, pane: u32, bytes: &[u8]) {
        for chunk in bytes.chunks(256) {
            let mut line = format!("send-keys -t %{pane} -H");
            for b in chunk {
                line.push_str(&format!(" {b:02x}"));
            }
            self.push(wire, Ask::Plain, &line);
        }
    }

    /// Everything needed to show a pane exactly as tmux holds it, read at
    /// one instant: up to `lines` of history, the screen, which of its rows
    /// wrap, and the cursor and modes.
    pub fn snapshot(&mut self, wire: &mut Vec<u8>, pane: u32, lines: u32) {
        let line = format!(
            "capture-pane -p -e -J -t %{pane} -S -{lines} -E -1 ; capture-pane -p -e -N -t %{pane} ; \
             capture-pane -p -J -t %{pane} ; capture-pane -p -e -N -a -q -t %{pane} ; \
             capture-pane -p -J -a -q -t %{pane} ; display-message -p -t %{pane} \"{PANES_FORMAT}\""
        );
        let asks = [
            Ask::Above { pane },
            Ask::Rows { pane },
            Ask::Joined { pane },
            Ask::MainRows { pane },
            Ask::MainJoined { pane },
            Ask::State { pane },
        ];
        self.push_line(wire, &asks, &line);
    }

    /// The pane's screen and its cursor, to compare with the engine.
    pub fn check(&mut self, wire: &mut Vec<u8>, pane: u32) {
        let line = format!("capture-pane -p -e -t %{pane} ; display-message -p -t %{pane} \"{PANES_FORMAT}\"");
        self.push_line(wire, &[Ask::CheckRows { pane }, Ask::CheckState { pane }], &line);
    }

    pub fn panes(&mut self, wire: &mut Vec<u8>) {
        self.push(wire, Ask::Panes, &format!("list-panes -F \"{PANES_FORMAT}\""));
    }

    /// tmux holds a pane's output once this client is `seconds` behind and
    /// says `%pause`, instead of growing its buffer without limit.
    pub fn pause_after(&mut self, wire: &mut Vec<u8>, seconds: u32) {
        self.push(wire, Ask::Plain, &format!("refresh-client -f pause-after={seconds}"));
    }

    /// Stop or resume a pane's output: a hidden tab costs nothing.
    pub fn set_pane_output(&mut self, wire: &mut Vec<u8>, pane: u32, on: bool) {
        let state = if on { "continue" } else { "pause" };
        self.push(wire, Ask::Plain, &format!("refresh-client -A %{pane}:{state}"));
    }

    pub fn select_pane(&mut self, wire: &mut Vec<u8>, pane: u32) {
        self.push(wire, Ask::Plain, &format!("select-pane -t %{pane}"));
    }
}

/// One pane of the window the session shows, in window cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneInfo {
    pub id: u32,
    pub left: u16,
    pub top: u16,
    pub width: u16,
    pub height: u16,
    /// The pane tmux sends typed keys to.
    pub active: bool,
}

/// What the view does with what the session read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The panes on screen now. Comes before the first bytes of a new pane.
    Layout(Vec<PaneInfo>),
    /// Bytes for a pane's engine.
    Feed { pane: u32, bytes: Vec<u8> },
    /// Bytes that leave the pane's engine, first set to `size`, showing
    /// exactly what tmux holds: on attach, after a resize, after a repair.
    Seed { pane: u32, size: (u16, u16), bytes: Vec<u8> },
    /// What tmux holds on the pane's screen right now, to compare with the
    /// engine ([`first_difference`]); [`Session::reseed`] repairs it.
    Check { pane: u32, size: (u16, u16), cursor: (u16, u16), rows: Vec<Vec<u8>> },
    /// Text that is not from a pane (tmux could not attach, the end notice).
    Notice(Vec<u8>),
    /// tmux ended the client (the session closed or was detached).
    Exited,
}

/// History lines read when a pane is first shown.
pub const SEED_LINES: u32 = 2000;
const MAX_PENDING_INPUT: usize = 64 << 10;

/// One line per pane: its size, cursor, the modes its program has set and
/// where it sits in the window.
pub const PANES_FORMAT: &str = "#{pane_id} #{pane_width} #{pane_height} #{cursor_x} #{cursor_y} #{alternate_on} #{keypad_cursor_flag} #{keypad_flag} #{mouse_standard_flag} #{mouse_button_flag} #{mouse_any_flag} #{mouse_sgr_flag} #{cursor_flag} #{pane_left} #{pane_top} #{pane_active} #{window_zoomed_flag} #{history_size} #{alternate_saved_x} #{alternate_saved_y}";

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
struct PaneState {
    id: u32,
    size: (u16, u16),
    cursor: (u16, u16),
    alternate: bool,
    flags: [bool; 7],
    at: (u16, u16),
    active: bool,
    zoomed: bool,
    /// Lines above the screen.
    history: u32,
    /// Where the cursor was left on the screen a full-screen program covers.
    saved: (u16, u16),
}

fn pane_state(line: &[u8]) -> Option<PaneState> {
    let text = std::str::from_utf8(line).ok()?;
    let mut f = text.split(' ');
    let id = id(f.next()?, '%')?;
    let mut n = || f.next()?.parse::<u16>().ok();
    let size = (n()?, n()?);
    let cursor = (n()?, n()?);
    let alternate = n()? != 0;
    let mut flags = [false; 7];
    for flag in &mut flags {
        *flag = n()? != 0;
    }
    let at = (n()?, n()?);
    let active = n()? != 0;
    let zoomed = n()? != 0;
    let history = f.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    let mut n = || f.next().and_then(|n| n.parse::<u16>().ok()).unwrap_or(0);
    let saved = (n(), n());
    Some(PaneState { id, size, cursor, alternate, flags, at, active, zoomed, history, saved })
}

/// Drops from a pane's bytes what only tmux understands: `ESC k name ESC \`,
/// the window name shells set under `TERM=tmux` or `screen` (oh-my-zsh does
/// on every prompt and command). tmux keeps the name and shows nothing; an
/// xterm-like engine would print it on the screen and shift what follows.
/// The name ends where tmux ends it: at the next ESC, BEL, CAN or SUB.
#[derive(Default)]
struct TmuxOnly {
    state: TmuxOnlyState,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum TmuxOnlyState {
    #[default]
    Ground,
    Escape,
    Name,
}

impl TmuxOnly {
    fn clean(&mut self, data: Vec<u8>) -> Vec<u8> {
        use TmuxOnlyState::{Escape, Ground, Name};
        if self.state == Ground && !data.contains(&0x1b) {
            return data;
        }
        let mut out = Vec::with_capacity(data.len());
        for b in data {
            match (self.state, b) {
                (Ground, 0x1b) | (Name, 0x1b) => self.state = Escape,
                (Ground, _) => out.push(b),
                (Escape, b'k') => self.state = Name,
                (Escape, 0x1b) => out.push(0x1b),
                (Escape, _) => {
                    out.extend_from_slice(&[0x1b, b]);
                    self.state = Ground;
                }
                (Name, 0x07 | 0x18 | 0x1a) => self.state = Ground,
                (Name, _) => {}
            }
        }
        out
    }
}

#[derive(Default)]
struct Track {
    seeded: bool,
    /// The size its engine was last seeded at.
    size: (u16, u16),
    /// A snapshot is on its way, and its parts as they arrive.
    capturing: bool,
    above: Vec<Vec<u8>>,
    rows: Vec<Vec<u8>>,
    joined: Vec<Vec<u8>>,
    main_rows: Vec<Vec<u8>>,
    main_joined: Vec<Vec<u8>>,
    /// Snapshots that failed in a row: a pane tmux cannot read is left alone.
    failed: u8,
    /// A check is on its way, and its screen.
    checking: bool,
    check_rows: Vec<Vec<u8>>,
    tmux_only: TmuxOnly,
}

/// One control client showing the panes of a session's current window:
/// reads the stream, keeps the window the size of the view and turns typed
/// bytes into commands. No IO: the caller moves `wire` to tmux and applies
/// the actions.
#[derive(Default)]
pub struct Session {
    parser: Parser,
    commands: Commands,
    events: Vec<Event>,
    greeted: bool,
    ended: bool,
    panes: std::collections::BTreeMap<u32, Track>,
    shown: Vec<PaneInfo>,
    /// The cells the view has for panes and the window size last asked for.
    want: Option<(u16, u16)>,
    asked: Option<(u16, u16)>,
    /// Window rows above the panes (the border line with their titles).
    extra: u16,
    pending_input: Vec<u8>,
    /// A pane query is already on its way.
    querying: bool,
}

impl Session {
    /// True once this pane's screen is in its engine and keys reach it.
    pub fn seeded(&self, pane: u32) -> bool {
        self.panes.get(&pane).is_some_and(|t| t.seeded)
    }

    /// True once any pane is on screen.
    pub fn ready(&self) -> bool {
        self.panes.values().any(|t| t.seeded)
    }

    fn ask_size(&mut self, wire: &mut Vec<u8>) -> bool {
        let Some((cols, rows)) = self.want else { return false };
        let window = (cols, rows.saturating_add(self.extra));
        if !self.greeted || self.ended || self.asked == Some(window) {
            return false;
        }
        self.asked = Some(window);
        self.commands.resize(wire, window.0, window.1);
        true
    }

    fn query(&mut self, wire: &mut Vec<u8>) {
        if self.greeted && !self.ended {
            self.querying = true;
            self.commands.panes(wire);
        }
    }

    /// The view's grid for panes changed (or is known for the first time).
    pub fn resize(&mut self, cols: u16, rows: u16, wire: &mut Vec<u8>) {
        self.want = Some((cols, rows));
        self.ask_size(wire);
    }

    /// Offers the size again, so tmux gives the window back to this client.
    pub fn claim(&mut self, wire: &mut Vec<u8>) {
        self.asked = None;
        self.ask_size(wire);
    }

    /// Typed or pasted bytes for a pane. Before its screen is read they wait.
    pub fn input(&mut self, pane: Option<u32>, bytes: &[u8], wire: &mut Vec<u8>) {
        match pane.filter(|p| self.seeded(*p) && !self.ended) {
            Some(pane) => self.commands.send_bytes(wire, pane, bytes),
            None if self.pending_input.len() + bytes.len() <= MAX_PENDING_INPUT => {
                self.pending_input.extend_from_slice(bytes);
            }
            None => {}
        }
    }

    /// Makes a pane the one tmux sends keys to.
    pub fn focus(&mut self, pane: u32, wire: &mut Vec<u8>) {
        if self.greeted && !self.ended {
            self.commands.select_pane(wire, pane);
        }
    }

    pub fn read(&mut self, bytes: &[u8], wire: &mut Vec<u8>, out: &mut Vec<Action>) {
        let mut events = std::mem::take(&mut self.events);
        self.parser.feed(bytes, &mut events);
        for event in events.drain(..) {
            self.event(event, wire, out);
        }
        self.events = events;
    }

    fn event(&mut self, event: Event, wire: &mut Vec<u8>, out: &mut Vec<Action>) {
        match event {
            Event::Output { pane, data } => {
                let Some(track) = self.panes.get_mut(&pane) else { return };
                // Read even before the screen is: a name may be half sent.
                let bytes = track.tmux_only.clean(data);
                // Output from before the screen was read is already in it.
                if track.seeded && !bytes.is_empty() {
                    out.push(Action::Feed { pane, bytes });
                }
            }
            Event::Reply { ok, lines, .. } => {
                if !self.greeted {
                    self.greeted = true;
                    self.commands.answered(ok);
                    // The panes are read before any size is asked for: a
                    // window that already fits is not resized, and its
                    // programs do not redraw just because the app attached.
                    self.query(wire);
                    return;
                }
                let Some(ask) = self.commands.answered(ok) else { return };
                if ask == Ask::Panes {
                    self.querying = false;
                    if ok {
                        self.listed(&lines, wire, out);
                    }
                    return;
                }
                let (Ask::Above { pane }
                | Ask::Rows { pane }
                | Ask::Joined { pane }
                | Ask::MainRows { pane }
                | Ask::MainJoined { pane }
                | Ask::State { pane }
                | Ask::CheckRows { pane }
                | Ask::CheckState { pane }) = ask
                else {
                    return;
                };
                let Some(track) = self.panes.get_mut(&pane) else { return };
                let check = matches!(ask, Ask::CheckRows { .. } | Ask::CheckState { .. });
                if !ok {
                    // The rest of the line was dropped with it.
                    if check {
                        track.checking = false;
                    } else {
                        track.capturing = false;
                        track.failed = track.failed.saturating_add(1);
                        if !self.querying {
                            self.query(wire);
                        }
                    }
                    return;
                }
                match ask {
                    Ask::Above { .. } => track.above = lines,
                    Ask::Rows { .. } => track.rows = lines,
                    Ask::Joined { .. } => track.joined = lines,
                    Ask::MainRows { .. } => track.main_rows = lines,
                    Ask::MainJoined { .. } => track.main_joined = lines,
                    Ask::CheckRows { .. } => track.check_rows = lines,
                    Ask::State { .. } => {
                        track.capturing = false;
                        let screen = Screen {
                            above: std::mem::take(&mut track.above),
                            rows: std::mem::take(&mut track.rows),
                            joined: std::mem::take(&mut track.joined),
                            main_rows: std::mem::take(&mut track.main_rows),
                            main_joined: std::mem::take(&mut track.main_joined),
                        };
                        let Some(state) = lines.first().and_then(|l| pane_state(l)) else { return };
                        track.seeded = true;
                        track.failed = 0;
                        track.size = state.size;
                        let bytes = seed(&state, &screen);
                        out.push(Action::Seed { pane, size: state.size, bytes });
                        self.flush_input(wire);
                    }
                    Ask::CheckState { .. } => {
                        track.checking = false;
                        let rows = std::mem::take(&mut track.check_rows);
                        // A snapshot on its way replaces the screen anyway.
                        if !track.capturing
                            && let Some(state) = lines.first().and_then(|l| pane_state(l))
                        {
                            out.push(Action::Check { pane, size: state.size, cursor: state.cursor, rows });
                        }
                    }
                    Ask::Plain | Ask::Panes => {}
                }
            }
            // Panes moved, another window was chosen or another pane took
            // the keys: read them again. The layout text counts the border
            // line as pane, so sizes come from the panes themselves.
            Event::LayoutChange { .. } => self.query(wire),
            Event::Other(text)
                if text.starts_with("%session-window-changed") || text.starts_with("%window-pane-changed") =>
            {
                self.query(wire);
            }
            Event::Exit { .. } => {
                self.ended = true;
                out.push(Action::Exited);
            }
            Event::Text(text) if !self.greeted || self.ended => {
                let mut bytes = text.into_bytes();
                bytes.extend_from_slice(b"\r\n");
                out.push(Action::Notice(bytes));
            }
            _ => {}
        }
    }

    fn listed(&mut self, lines: &[Vec<u8>], wire: &mut Vec<u8>, out: &mut Vec<Action>) {
        let mut states: Vec<PaneState> = lines.iter().filter_map(|l| pane_state(l)).collect();
        if states.is_empty() {
            return;
        }
        // A zoomed pane fills the window; the others are not on screen.
        if states.iter().any(|s| s.zoomed) {
            states.retain(|s| s.active);
        }
        // Rows tmux keeps above the panes: ask for a window that much taller,
        // and read the panes again once it has resized them.
        self.extra = states.iter().map(|s| s.at.1).min().unwrap_or(0).min(2);
        if self.ask_size(wire) {
            self.query(wire);
            return;
        }
        self.panes.retain(|id, _| states.iter().any(|s| s.id == *id));
        let shown: Vec<PaneInfo> = states
            .iter()
            .map(|s| PaneInfo {
                id: s.id,
                left: s.at.0,
                top: s.at.1,
                width: s.size.0,
                height: s.size.1,
                active: s.active,
            })
            .collect();
        if shown != self.shown {
            self.shown.clone_from(&shown);
            out.push(Action::Layout(shown));
        }
        for state in &states {
            let track = self.panes.entry(state.id).or_default();
            // A pane not shown yet, or one tmux gave another size: tmux and
            // the engine rewrap text differently, so the engine is not asked
            // to; it is filled again with what tmux holds at the new size.
            let stale = !track.seeded || track.size != state.size;
            if stale && !track.capturing && track.failed < 3 {
                track.capturing = true;
                self.commands.snapshot(wire, state.id, SEED_LINES);
            }
        }
        self.flush_input(wire);
    }

    /// What was typed while attaching goes to the pane with the keys.
    fn flush_input(&mut self, wire: &mut Vec<u8>) {
        if self.pending_input.is_empty() {
            return;
        }
        if let Some(active) = self.shown.iter().find(|p| p.active && self.seeded(p.id)).map(|p| p.id) {
            let waiting = std::mem::take(&mut self.pending_input);
            self.commands.send_bytes(wire, active, &waiting);
        }
    }

    /// Asks tmux what every pane shows, to compare with the engines
    /// ([`Action::Check`]). Nothing is asked of a pane being read or checked.
    pub fn check(&mut self, wire: &mut Vec<u8>) {
        if !self.greeted || self.ended {
            return;
        }
        for pane in &self.shown {
            if let Some(track) = self.panes.get_mut(&pane.id)
                && track.seeded
                && !track.capturing
                && !track.checking
            {
                track.checking = true;
                self.commands.check(wire, pane.id);
            }
        }
    }

    /// Fills a pane's engine again from tmux: it was found showing something
    /// else.
    pub fn reseed(&mut self, pane: u32, wire: &mut Vec<u8>) {
        if !self.greeted || self.ended {
            return;
        }
        if let Some(track) = self.panes.get_mut(&pane)
            && track.seeded
            && !track.capturing
            && track.failed < 3
        {
            track.capturing = true;
            self.commands.snapshot(wire, pane, SEED_LINES);
        }
    }
}

/// The parts of a pane snapshot ([`Commands::snapshot`]).
#[derive(Default)]
struct Screen {
    above: Vec<Vec<u8>>,
    rows: Vec<Vec<u8>>,
    joined: Vec<Vec<u8>>,
    main_rows: Vec<Vec<u8>>,
    main_joined: Vec<Vec<u8>>,
}

/// Writes screen rows each at its own place, so the engine's screen is
/// tmux's cell for cell whatever it would have made of the same text. A row
/// tmux holds as wrapped into the next (`joined` tells) is followed by it
/// without a jump: the engine wraps it itself and knows they are one line,
/// so a copy of it has no break.
fn place_rows(out: &mut Vec<u8>, rows: &[Vec<u8>], joined: Option<&[Vec<u8>]>, size: (u16, u16)) {
    let rows = rows.get(..usize::from(size.1)).unwrap_or(rows);
    let wraps = joined.map_or_else(Vec::new, |joined| wrapped_rows(rows, joined, size.0));
    let mut jump = true;
    for (row, line) in rows.iter().enumerate() {
        if jump {
            out.extend_from_slice(format!("\x1b[{};1H", row + 1).as_bytes());
        }
        out.extend_from_slice(line);
        jump = !wraps.get(row).copied().unwrap_or(false);
    }
    out.extend_from_slice(b"\x0f\x1b[0m");
}

/// The bytes that leave an engine showing what the pane shows: history,
/// screen, cursor and the modes its program had already set. Under a
/// full-screen program the screen it covers is written first, so leaving the
/// program uncovers what tmux will uncover.
fn seed(state: &PaneState, screen: &Screen) -> Vec<u8> {
    // G1 is line drawing, as tmux assumes of its own terminal: captured
    // cells of that set come between shift-out and shift-in.
    let mut out = b"\x1bc\x1b)0".to_vec();
    // With no history tmux answers the range with the first screen row.
    if state.history > 0 {
        let mut background = None;
        for line in &screen.above {
            out.extend_from_slice(line);
            // A new line is born with the background in use: end the line
            // without one, then set it again for what follows.
            track_background(line, &mut background);
            match &background {
                Some(set) => {
                    out.extend_from_slice(b"\x1b[49m\r\n");
                    out.extend_from_slice(set);
                }
                None => out.extend_from_slice(b"\r\n"),
            }
        }
    }
    // The history leaves the screen, which is then written in place.
    out.extend_from_slice(b"\x1b[0m");
    out.extend(std::iter::repeat_n(b'\n', usize::from(state.size.1).saturating_sub(1)));
    if state.alternate {
        place_rows(&mut out, &screen.main_rows, Some(&screen.main_joined), state.size);
        out.extend_from_slice(format!("\x1b[{};{}H\x1b[?1049h", state.saved.1 + 1, state.saved.0 + 1).as_bytes());
        place_rows(&mut out, &screen.rows, None, state.size);
    } else {
        place_rows(&mut out, &screen.rows, Some(&screen.joined), state.size);
    }
    let modes: [&[u8]; 7] =
        [b"\x1b[?1h", b"\x1b=", b"\x1b[?1000h", b"\x1b[?1002h", b"\x1b[?1003h", b"\x1b[?1006h", b""];
    for (on, bytes) in state.flags.iter().zip(modes) {
        if *on {
            out.extend_from_slice(bytes);
        }
    }
    // tmux 3.2a does not say whether the program asked for bracketed paste.
    // Claude Code, Codex and current shells do; assume it until they say.
    out.extend_from_slice(b"\x1b[?2004h");
    if !state.flags[6] {
        out.extend_from_slice(b"\x1b[?25l");
    }
    out.extend_from_slice(format!("\x1b[{};{}H", state.cursor.1 + 1, state.cursor.0 + 1).as_bytes());
    out
}

/// A captured line without its escape sequences and charset shifts.
fn plain(line: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len());
    let mut i = 0;
    while let Some(&b) = line.get(i) {
        match (b, line.get(i + 1)) {
            (0x1b, Some(b'[')) => {
                i += 2;
                while line.get(i).is_some_and(|b| !(0x40..=0x7e).contains(b)) {
                    i += 1;
                }
                i += 1;
            }
            (0x1b, Some(b']')) => {
                i += 2;
                while line.get(i).is_some_and(|&b| b != 0x07 && b != 0x1b) {
                    i += 1;
                }
                i += if line.get(i) == Some(&0x1b) { 2 } else { 1 };
            }
            (0x0e | 0x0f, _) => i += 1,
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// Follows the background colour through a captured line: `background` ends
/// as the sequence that sets the one left in use (`None`: the default).
fn track_background(line: &[u8], background: &mut Option<Vec<u8>>) {
    let mut i = 0;
    while let Some(at) = line.get(i..).and_then(|rest| rest.windows(2).position(|w| w == b"\x1b[")) {
        let start = i + at + 2;
        let len = line.get(start..).map_or(0, |rest| rest.iter().take_while(|b| !(0x40..=0x7e).contains(*b)).count());
        i = start + len + 1;
        if line.get(start + len) != Some(&b'm') {
            continue;
        }
        let Some(Ok(text)) = line.get(start..start + len).map(std::str::from_utf8) else { continue };
        let mut params = text.split(';').map(|p| p.parse::<u16>().unwrap_or(0));
        while let Some(p) = params.next() {
            match p {
                0 | 49 => *background = None,
                40..=47 | 100..=107 => *background = Some(format!("\x1b[{p}m").into_bytes()),
                38 | 48 => {
                    // `5;n` or `2;r;g;b`.
                    let kind = params.next().unwrap_or(0);
                    let spec: Vec<String> =
                        params.by_ref().take(if kind == 2 { 3 } else { 1 }).map(|n| n.to_string()).collect();
                    if p == 48 {
                        *background = Some(format!("\x1b[48;{kind};{}m", spec.join(";")).into_bytes());
                    }
                }
                _ => {}
            }
        }
    }
}

fn cells(text: &[u8]) -> usize {
    String::from_utf8_lossy(text).chars().map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)).sum()
}

/// Which screen rows tmux holds as wrapped into the next one: `joined` is the
/// same screen with those rows already joined. A row counts only if it fills
/// the pane, so the engine wraps where tmux did; when the two readings do not
/// add up no row does, and every row is simply written at its place.
fn wrapped_rows(rows: &[Vec<u8>], joined: &[Vec<u8>], width: u16) -> Vec<bool> {
    let none = vec![false; rows.len()];
    let bare: Vec<Vec<u8>> = rows.iter().map(|row| plain(row)).collect();
    let mut wraps = none.clone();
    let mut next = 0;
    for line in joined {
        let (first, mut used) = (next, 0);
        loop {
            let Some(row) = bare.get(next) else { return none };
            if line.get(used..used + row.len()) != Some(row.as_slice()) {
                return none;
            }
            used += row.len();
            next += 1;
            if used == line.len() {
                break;
            }
        }
        for wrap in wraps.iter_mut().take(next - 1).skip(first) {
            *wrap = true;
        }
    }
    if next != rows.len() {
        return none;
    }
    let width = usize::from(width);
    for (row, wrap) in wraps.iter_mut().enumerate() {
        if *wrap {
            let wide_next = bare
                .get(row + 1)
                .and_then(|next| String::from_utf8_lossy(next).chars().next())
                .is_some_and(|c| unicode_width::UnicodeWidthChar::width(c) == Some(2));
            let filled = bare.get(row).map_or(0, |text| cells(text));
            *wrap = filled == width || (filled + 1 == width && wide_next);
        }
    }
    wraps
}

/// What a terminal shows for a line-drawing cell (`ESC ( 0`), which tmux
/// keeps as the letter the program wrote.
fn line_drawing(b: u8) -> Option<char> {
    const SHOWN: [char; 32] = [
        ' ', '◆', '▒', '␉', '␌', '␍', '␊', '°', '±', '␤', '␋', '┘', '┐', '┌', '└', '┼', '⎺', '⎻', '─', '⎼', '⎽', '├',
        '┤', '┴', '┬', '│', '≤', '≥', 'π', '≠', '£', '·',
    ];
    SHOWN.get(usize::from(b.checked_sub(0x5f)?)).copied()
}

/// The first row where the engine's screen (`ours`, one text per row) is not
/// what tmux holds (`theirs`, a capture with its escape sequences). Ends of
/// rows are not compared (tmux trims them), nor marks that take no cell
/// (each side keeps them its own way).
pub fn first_difference(ours: &[String], theirs: &[Vec<u8>]) -> Option<usize> {
    let tidy = |text: &str| -> String {
        let kept: String = text.chars().filter(|c| unicode_width::UnicodeWidthChar::width(*c) != Some(0)).collect();
        kept.trim_end().to_string()
    };
    // Line-drawing cells come between shift-out and shift-in, and a run of
    // them goes on across rows.
    let mut drawing = false;
    let mut shown = |row: &[u8]| -> String {
        let mut text = Vec::with_capacity(row.len());
        let mut i = 0;
        while let Some(&b) = row.get(i) {
            match (b, row.get(i + 1)) {
                (0x1b, Some(b'[')) => {
                    i += 2;
                    while row.get(i).is_some_and(|b| !(0x40..=0x7e).contains(b)) {
                        i += 1;
                    }
                }
                (0x1b, Some(b']')) => {
                    i += 2;
                    while row.get(i).is_some_and(|&b| b != 0x07 && b != 0x1b) {
                        i += 1;
                    }
                    i += usize::from(row.get(i) == Some(&0x1b));
                }
                (0x0e | 0x0f, _) => drawing = b == 0x0e,
                _ => match line_drawing(b).filter(|_| drawing) {
                    Some(c) => text.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
                    None => text.push(b),
                },
            }
            i += 1;
        }
        tidy(&String::from_utf8_lossy(&text))
    };
    let theirs: Vec<String> = theirs.iter().map(|row| shown(row)).collect();
    (0..ours.len().max(theirs.len())).find(|&row| {
        let our = ours.get(row).map(|t| tidy(t)).unwrap_or_default();
        our != theirs.get(row).map_or("", String::as_str)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(stream: &[u8]) -> Vec<Event> {
        let mut out = Vec::new();
        Parser::default().feed(stream, &mut out);
        out
    }

    #[test]
    fn output_is_unescaped_and_keeps_raw_utf8() {
        let got = events(b"%output %3 \\033[31mh\xc3\xa9\\134n\\015\\012\n");
        assert_eq!(got, vec![Event::Output { pane: 3, data: b"\x1b[31mh\xc3\xa9\\n\r\n".to_vec() }]);
    }

    #[test]
    fn extended_output_drops_its_age_field() {
        let got = events(b"%extended-output %0 1250 : a : b\\015\n");
        assert_eq!(got, vec![Event::Output { pane: 0, data: b"a : b\r".to_vec() }]);
    }

    #[test]
    fn a_stream_cut_anywhere_gives_the_same_events() {
        let stream = b"%begin 1 7 0\n%0 0 0 60 40\n%end 1 7 0\n%output %1 x\\033y\n%exit\n";
        let whole = events(stream);
        for cut in 0..stream.len() {
            let mut parser = Parser::default();
            let mut out = Vec::new();
            parser.feed(&stream[..cut], &mut out);
            parser.feed(&stream[cut..], &mut out);
            assert_eq!(out, whole, "cut at {cut}");
        }
        assert_eq!(whole.len(), 3);
    }

    #[test]
    fn reply_lines_that_look_like_notifications_stay_in_the_reply() {
        let got = events(b"%begin 5 9 1\n%output %1 fake\n%end 5 8 1\ntext\n%error 5 9 1\n");
        assert_eq!(
            got,
            vec![Event::Reply {
                number: 9,
                ok: false,
                lines: vec![b"%output %1 fake".to_vec(), b"%end 5 8 1".to_vec(), b"text".to_vec()],
            }]
        );
    }

    #[test]
    fn layouts_with_nested_splits_list_every_pane() {
        let layout = parse_layout("f91d,120x40,0,0{60x40,0,0,0,59x40,61,0[59x20,61,0,1,59x19,61,21,4]}").unwrap();
        assert_eq!((layout.cols, layout.rows), (120, 40));
        assert_eq!(
            layout.panes,
            vec![
                PaneRect { id: 0, left: 0, top: 0, width: 60, height: 40 },
                PaneRect { id: 1, left: 61, top: 0, width: 59, height: 20 },
                PaneRect { id: 4, left: 61, top: 21, width: 59, height: 19 },
            ]
        );
        let single = parse_layout("b25d,80x24,0,0,7").unwrap();
        assert_eq!(single.panes, vec![PaneRect { id: 7, left: 0, top: 0, width: 80, height: 24 }]);
        for bad in ["", "x", "f91d,120x40,0,0", "f91d,120x40,0,0{60x40,0,0,0", "f91d,120x40,0,0,1junk"] {
            assert_eq!(parse_layout(bad), None, "{bad}");
        }
    }

    #[test]
    fn layout_change_and_other_notifications() {
        let got = events(
            b"%layout-change @2 b25d,80x24,0,0,7 b25d,80x24,0,0,7 *\n%pause %7\n%continue %7\n%session-changed $1 my name\n%window-renamed @2 zsh\nnoise\n",
        );
        assert_eq!(got.len(), 6);
        assert_eq!(got[5], Event::Text("noise".into()));
        assert!(matches!(&got[0], Event::LayoutChange { window: 2, layout } if layout.panes[0].id == 7));
        assert_eq!(got[1], Event::Pause { pane: 7 });
        assert_eq!(got[2], Event::Continue { pane: 7 });
        assert_eq!(got[3], Event::SessionChanged { session: 1, name: "my name".into() });
        assert_eq!(got[4], Event::Other("%window-renamed @2 zsh".into()));
    }

    #[test]
    fn an_oversized_line_is_dropped_and_the_stream_recovers() {
        let mut stream = vec![b'x'; MAX_LINE + 10];
        stream.extend_from_slice(b"\n%pause %1\n");
        assert_eq!(events(&stream), vec![Event::Pause { pane: 1 }]);
    }

    #[test]
    fn commands_are_matched_to_replies_in_order_after_the_greeting() {
        let mut commands = Commands::default();
        let mut wire = Vec::new();
        commands.resize(&mut wire, 120, 40);
        commands.send_bytes(&mut wire, 3, b"\x1b[A\xc3\xa9");
        assert_eq!(
            String::from_utf8(wire).unwrap(),
            "refresh-client -C 120,40\nsend-keys -t %3 -H 1b 5b 41 c3 a9\n"
        );
        assert_eq!(commands.answered(true), None);
        assert_eq!(commands.answered(true), Some(Ask::Plain));
        assert_eq!(commands.pending(), 1);
    }

    #[test]
    fn a_failed_command_takes_the_rest_of_its_line_with_it() {
        let mut commands = Commands { greeted: true, ..Commands::default() };
        let mut wire = Vec::new();
        commands.snapshot(&mut wire, 3, 500);
        commands.check(&mut wire, 3);
        commands.panes(&mut wire);
        let sent = String::from_utf8(wire).unwrap();
        let lines: Vec<&str> = sent.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(
            lines[0].starts_with(
                "capture-pane -p -e -J -t %3 -S -500 -E -1 ; capture-pane -p -e -N -t %3 ; capture-pane -p -J -t %3 ; capture-pane -p -e -N -a -q -t %3 ; capture-pane -p -J -a -q -t %3 ; display-message -p -t %3 \"#{pane_id} "
            ),
            "{sent}"
        );
        assert!(lines[1].starts_with("capture-pane -p -e -t %3 ; display-message -p -t %3 \"#{pane_id} "), "{sent}");
        // The second command of the snapshot fails: the ones after it are
        // never answered, and the next reply belongs to the check.
        assert_eq!(commands.answered(true), Some(Ask::Above { pane: 3 }));
        assert_eq!(commands.answered(false), Some(Ask::Rows { pane: 3 }));
        assert_eq!(commands.answered(false), Some(Ask::CheckRows { pane: 3 }));
        assert_eq!(commands.answered(true), Some(Ask::Panes));
        assert_eq!(commands.pending(), 0);
    }

    #[test]
    fn long_input_is_split_across_commands() {
        let mut commands = Commands::default();
        let mut wire = Vec::new();
        commands.send_bytes(&mut wire, 1, &[b'a'; 600]);
        assert_eq!(commands.pending(), 3);
        assert!(wire.split(|&b| b == b'\n').all(|line| line.len() < 1024));
    }

    #[test]
    fn a_window_name_never_reaches_the_engine() {
        let mut filter = TmuxOnly::default();
        assert_eq!(filter.clean(b"a\x1bkls -la\x1b\\b".to_vec()), b"a\x1b\\b");
        // Cut anywhere, including inside the name and between ESC and `k`.
        let stream = b"one\x1b[31m\x1bkname with spaces\x1b\\two\x1b]0;title\x07\x1bkx\x07three";
        let whole = TmuxOnly::default().clean(stream.to_vec());
        assert_eq!(whole, b"one\x1b[31m\x1b\\two\x1b]0;title\x07three");
        for cut in 0..stream.len() {
            let mut filter = TmuxOnly::default();
            let mut got = filter.clean(stream[..cut].to_vec());
            got.extend(filter.clean(stream[cut..].to_vec()));
            assert_eq!(got, whole, "cut at {cut}");
        }
    }

    #[test]
    fn rows_tmux_wrapped_are_found_from_the_joined_reading() {
        let rows = vec![b"\x1b[1m$ echo 12".to_vec(), b"34\x1b[0m".to_vec(), b"".to_vec(), b"abcdefghij".to_vec(), b"k".to_vec()];
        let joined = vec![b"$ echo 1234".to_vec(), b"".to_vec(), b"abcdefghijk".to_vec()];
        // The first row is short of the 10 columns: it is not let run on.
        assert_eq!(wrapped_rows(&rows, &joined, 10), vec![false, false, false, true, false]);
        assert_eq!(wrapped_rows(&rows, &joined, 9), vec![true, false, false, false, false]);
        // A wide character that did not fit left the last cell empty.
        let rows = vec!["12345678\u{65e5}".as_bytes().to_vec(), "\u{672c}x".as_bytes().to_vec()];
        let joined = vec!["12345678\u{65e5}\u{672c}x".as_bytes().to_vec()];
        assert_eq!(wrapped_rows(&rows, &joined, 11), vec![true, false]);
        assert_eq!(wrapped_rows(&rows, &joined, 12), vec![false, false]);
        // Readings that do not add up wrap nothing.
        assert_eq!(wrapped_rows(&rows, &[b"other".to_vec()], 11), vec![false, false]);
    }

    #[test]
    fn the_background_left_by_a_line_is_followed() {
        let mut bg = None;
        track_background(b"\x1b[1m\x1b[38;5;196m\x1b[48;5;237mx", &mut bg);
        assert_eq!(bg.as_deref(), Some(&b"\x1b[48;5;237m"[..]));
        track_background(b"plain \x1b[38;2;1;2;3mtext", &mut bg);
        assert_eq!(bg.as_deref(), Some(&b"\x1b[48;5;237m"[..]));
        track_background(b"\x1b[48;2;10;20;30m\x1b[K", &mut bg);
        assert_eq!(bg.as_deref(), Some(&b"\x1b[48;2;10;20;30m"[..]));
        track_background(b"\x1b[1;44m", &mut bg);
        assert_eq!(bg.as_deref(), Some(&b"\x1b[44m"[..]));
        track_background(b"\x1b[49m", &mut bg);
        assert_eq!(bg, None);
        track_background(b"\x1b[101m\x1b[0m", &mut bg);
        assert_eq!(bg, None);
    }

    #[test]
    fn screens_are_compared_without_row_ends_or_marks_that_take_no_cell() {
        let ours = vec!["a  b   ".to_string(), "e".to_string(), String::new()];
        let same = vec![b"a  b".to_vec(), "e\u{301}".as_bytes().to_vec()];
        assert_eq!(first_difference(&ours, &same), None);
        assert_eq!(first_difference(&ours, &[b"a  b".to_vec(), b"x".to_vec()]), Some(1));
        assert_eq!(first_difference(&ours, &[b"a  b".to_vec(), b"e".to_vec(), b"".to_vec(), b"more".to_vec()]), Some(3));
        // Colours are not text, and line drawing is compared as it is shown,
        // also when the run goes on in the next row.
        let ours = vec!["x ┌─┐ q".to_string(), "│ │".to_string(), "x".to_string()];
        let theirs = vec![
            b"\x1b[1m\x1b[38;5;4mx\x1b[0m \x0elqk\x0f q".to_vec(),
            b"\x0ex x".to_vec(),
            b"\x0fx\x1b]8;;http://a\x1b\\".to_vec(),
        ];
        assert_eq!(first_difference(&ours, &theirs), None);
    }

    /// One `list-panes` line; the cursor is at column 3 of row 1 and one
    /// line of history is above the screen.
    fn pane(id: u32, size: (u16, u16), at: (u16, u16), active: bool) -> String {
        format!("%{id} {} {} 3 1 0 1 0 0 0 0 0 1 {} {} {} 0 1\r\n", size.0, size.1, at.0, at.1, u8::from(active))
    }

    fn reply(number: u32, body: &str) -> Vec<u8> {
        format!("%begin 1 {number} 1\r\n{body}%end 1 {number} 1\r\n").into_bytes()
    }

    /// The replies of a pane snapshot: history, rows, joined rows and the
    /// state, with no full-screen program covering another screen.
    fn snapshot(s: &mut Session, wire: &mut Vec<u8>, out: &mut Vec<Action>, parts: [&str; 4]) {
        let [above, rows, joined, state] = parts;
        for part in [above, rows, joined, "", "", state] {
            s.read(&reply(50, part), wire, out);
        }
    }

    fn text(wire: &mut Vec<u8>) -> String {
        String::from_utf8(std::mem::take(wire)).unwrap()
    }

    const LIST: &str = "list-panes -F";

    /// The seed of a 24-row pane with `above` in its history and two rows.
    fn seeded_with(above: &str, first: &str, second: &str) -> Vec<u8> {
        let down = "\n".repeat(23);
        format!("\x1bc\x1b)0{above}\x1b[0m{down}\x1b[1;1H{first}\x1b[2;1H{second}\x0f\x1b[0m\x1b[?1h\x1b[?2004h\x1b[2;4H")
            .into_bytes()
    }

    /// A session attached to a window of two panes side by side, both seeded.
    fn split_session() -> (Session, Vec<u8>, Vec<Action>) {
        let (mut s, mut wire, mut out) = (Session::default(), Vec::new(), Vec::new());
        s.resize(81, 24, &mut wire);
        s.input(Some(4), b"early", &mut wire);
        assert!(wire.is_empty(), "nothing is sent before tmux greets");
        s.read(b"\x1bP1000p%begin 1 1 0\r\n%end 1 1 0\r\n", &mut wire, &mut out);
        // Nothing is resized before the panes are known.
        let sent = text(&mut wire);
        assert!(sent.starts_with(LIST), "{sent}");
        // The border line takes a row: the window is asked one row taller
        // and the panes are read again before any screen is.
        let short = pane(4, (40, 23), (0, 1), true) + &pane(5, (40, 23), (41, 1), false);
        s.read(&reply(3, &short), &mut wire, &mut out);
        let sent = text(&mut wire);
        assert!(sent.starts_with("refresh-client -C 81,25\nlist-panes -F"), "{sent}");
        assert!(out.is_empty());
        s.read(&reply(4, ""), &mut wire, &mut out);
        let full = pane(4, (40, 24), (0, 1), true) + &pane(5, (40, 24), (41, 1), false);
        s.read(&reply(5, &full), &mut wire, &mut out);
        let sent = text(&mut wire);
        let lines: Vec<&str> = sent.lines().collect();
        assert_eq!(lines.len(), 2, "{sent}");
        assert!(lines[0].starts_with("capture-pane -p -e -J -t %4 -S -2000 -E -1 ; "), "{sent}");
        assert!(lines[1].starts_with("capture-pane -p -e -J -t %5 -S -2000 -E -1 ; "), "{sent}");
        assert_eq!(
            out,
            vec![Action::Layout(vec![
                PaneInfo { id: 4, left: 0, top: 1, width: 40, height: 24, active: true },
                PaneInfo { id: 5, left: 41, top: 1, width: 40, height: 24, active: false },
            ])]
        );
        out.clear();
        s.read(b"%output %4 dropped: already in the snapshot\r\n", &mut wire, &mut out);
        let state = pane(4, (40, 24), (0, 1), true);
        snapshot(&mut s, &mut wire, &mut out, ["old\r\n", "\x1b[31mleft$ \r\n\r\n", "left$ \r\n\r\n", &state]);
        assert_eq!(out.len(), 1);
        let state = pane(5, (40, 24), (41, 1), false);
        snapshot(&mut s, &mut wire, &mut out, ["\x1b[44mblue\r\n", "right$ \r\n\r\n", "right$ \r\n\r\n", &state]);
        (s, wire, out)
    }

    #[test]
    fn every_pane_is_seeded_then_streams_its_own_output_and_input() {
        let (mut s, mut wire, mut out) = split_session();
        assert!(s.seeded(4) && s.seeded(5) && s.ready());
        assert_eq!(
            out,
            vec![
                Action::Seed { pane: 4, size: (40, 24), bytes: seeded_with("old\r\n", "\x1b[31mleft$ ", "") },
                // The line born after a coloured one does not take its colour.
                Action::Seed {
                    pane: 5,
                    size: (40, 24),
                    bytes: seeded_with("\x1b[44mblue\x1b[49m\r\n\x1b[44m", "right$ ", "")
                },
            ]
        );
        // What was typed while attaching goes to the pane with the keys.
        assert_eq!(text(&mut wire), "send-keys -t %4 -H 65 61 72 6c 79\n");
        out.clear();
        s.read(b"%output %5 hi\\015\r\n%output %9 not shown\r\n", &mut wire, &mut out);
        assert_eq!(out, vec![Action::Feed { pane: 5, bytes: b"hi\r".to_vec() }]);
        // The name a shell gives its window is tmux's business, not text.
        s.read(b"%output %5 \\033kzsh\\033\\134$ \r\n", &mut wire, &mut out);
        assert_eq!(out[1], Action::Feed { pane: 5, bytes: b"\x1b\\$ ".to_vec() });
        s.input(Some(5), b"\r", &mut wire);
        s.focus(5, &mut wire);
        assert_eq!(text(&mut wire), "send-keys -t %5 -H 0d\nselect-pane -t %5\n");
    }

    #[test]
    fn layout_changes_are_read_from_the_panes_and_new_panes_are_seeded() {
        let (mut s, mut wire, mut out) = split_session();
        out.clear();
        text(&mut wire);
        s.read(&reply(9, ""), &mut wire, &mut out);
        s.read(b"%layout-change @1 b65d,81x25,0,0{40x25,0,0,4,40x25,41,0,5} x *\r\n", &mut wire, &mut out);
        assert!(text(&mut wire).starts_with(LIST));
        // The right pane was split: the new one is read, and the one that
        // changed size is read again instead of rewrapped.
        let three = pane(4, (40, 24), (0, 1), false)
            + &pane(5, (40, 11), (41, 1), false)
            + &pane(6, (40, 12), (41, 13), true);
        s.read(&reply(10, &three), &mut wire, &mut out);
        let sent = text(&mut wire);
        let lines: Vec<&str> = sent.lines().collect();
        assert_eq!(lines.len(), 2, "{sent}");
        assert!(lines[0].starts_with("capture-pane -p -e -J -t %5 ") && lines[1].starts_with("capture-pane -p -e -J -t %6 "));
        assert!(matches!(&out[..], [Action::Layout(panes)] if panes.len() == 3 && panes[2].active));
        out.clear();
        // Output of the pane being read again still reaches its engine.
        s.read(b"%output %5 meanwhile\r\n", &mut wire, &mut out);
        assert_eq!(out, vec![Action::Feed { pane: 5, bytes: b"meanwhile".to_vec() }]);
        out.clear();
        let state = pane(5, (40, 11), (41, 1), false);
        snapshot(&mut s, &mut wire, &mut out, ["", "r$ \r\n", "r$ \r\n", &state]);
        let state = pane(6, (40, 12), (41, 13), true);
        snapshot(&mut s, &mut wire, &mut out, ["", "new$ \r\n", "new$ \r\n", &state]);
        assert!(
            matches!(&out[..], [Action::Seed { pane: 5, size: (40, 11), .. }, Action::Seed { pane: 6, size: (40, 12), .. }]),
            "{out:?}"
        );
        // Nothing is read twice while the sizes stay.
        s.read(b"%window-pane-changed @1 %6\r\n", &mut wire, &mut out);
        text(&mut wire);
        s.read(&reply(11, &three), &mut wire, &mut out);
        assert!(text(&mut wire).is_empty());
        // A pane is closed: it leaves the layout and its output is ignored.
        out.clear();
        s.read(b"%window-pane-changed @1 %4\r\n", &mut wire, &mut out);
        let two = pane(4, (40, 24), (0, 1), true) + &pane(6, (40, 24), (41, 1), false);
        s.read(&reply(13, &two), &mut wire, &mut out);
        s.read(b"%output %5 gone\r\n", &mut wire, &mut out);
        assert!(matches!(&out[..], [Action::Layout(panes)] if panes.len() == 2), "{out:?}");
        assert!(!s.seeded(5));
    }

    #[test]
    fn a_zoomed_pane_is_the_only_one_shown_and_sizes_are_asked_once() {
        let (mut s, mut wire, mut out) = split_session();
        out.clear();
        text(&mut wire);
        s.read(&reply(9, ""), &mut wire, &mut out);
        s.read(b"%layout-change @1 x x *\r\n", &mut wire, &mut out);
        s.read(b"%layout-change @1 b65d,81x25,0,0,5 x *Z\r\n", &mut wire, &mut out);
        let zoomed = "%4 40 24 3 1 0 1 0 0 0 0 0 1 0 1 0 1\r\n%5 81 24 3 1 0 1 0 0 0 0 0 1 0 1 1 1\r\n";
        s.read(&reply(10, zoomed), &mut wire, &mut out);
        assert_eq!(
            out,
            vec![Action::Layout(vec![PaneInfo { id: 5, left: 0, top: 1, width: 81, height: 24, active: true }])]
        );
        text(&mut wire);
        s.resize(100, 30, &mut wire);
        assert_eq!(text(&mut wire), "refresh-client -C 100,31\n");
        s.resize(100, 30, &mut wire);
        assert!(text(&mut wire).is_empty());
        s.claim(&mut wire);
        assert_eq!(text(&mut wire), "refresh-client -C 100,31\n");
    }

    #[test]
    fn a_pane_is_checked_against_tmux_and_filled_again_when_asked() {
        let (mut s, mut wire, mut out) = split_session();
        out.clear();
        text(&mut wire);
        s.read(&reply(9, ""), &mut wire, &mut out);
        s.check(&mut wire);
        s.check(&mut wire);
        let sent = text(&mut wire);
        let lines: Vec<&str> = sent.lines().collect();
        assert_eq!(lines.len(), 2, "one check per pane at a time: {sent}");
        assert!(lines[0].starts_with("capture-pane -p -e -t %4 ; display-message -p -t %4 "), "{sent}");
        s.read(&reply(10, "left$ \r\n"), &mut wire, &mut out);
        s.read(&reply(11, &pane(4, (40, 24), (0, 1), true)), &mut wire, &mut out);
        assert_eq!(
            out,
            vec![Action::Check { pane: 4, size: (40, 24), cursor: (3, 1), rows: vec![b"left$ ".to_vec()] }]
        );
        // The engine showed something else: the pane is read again, and a
        // check that crosses with that reading is not reported.
        s.reseed(4, &mut wire);
        s.reseed(4, &mut wire);
        let sent = text(&mut wire);
        assert!(sent.starts_with("capture-pane -p -e -J -t %4 ") && sent.lines().count() == 1, "{sent}");
        out.clear();
        s.read(&reply(12, "right$ \r\n"), &mut wire, &mut out);
        s.read(&reply(13, &pane(5, (40, 24), (41, 1), false)), &mut wire, &mut out);
        assert!(matches!(&out[..], [Action::Check { pane: 5, .. }]), "{out:?}");
        out.clear();
        let state = pane(4, (40, 24), (0, 1), true);
        snapshot(&mut s, &mut wire, &mut out, ["old\r\n", "left$ \r\n", "left$ \r\n", &state]);
        assert!(matches!(&out[..], [Action::Seed { pane: 4, .. }]), "{out:?}");
        // A pane tmux cannot read is not asked for ever.
        for _ in 0..5 {
            s.reseed(4, &mut wire);
            let asked = !text(&mut wire).is_empty();
            if asked {
                s.read(b"%begin 1 60 1\r\ncan't find pane\r\n%error 1 60 1\r\n", &mut wire, &mut out);
                // It asks for the panes again; they still list it.
                s.read(&reply(61, &(pane(4, (40, 24), (0, 1), true) + &pane(5, (40, 24), (41, 1), false))), &mut wire, &mut out);
            }
        }
        assert!(text(&mut wire).is_empty());
    }

    #[test]
    fn the_end_and_attach_errors_are_reported() {
        let (mut s, mut wire, mut out) = split_session();
        out.clear();
        s.read(b"%exit\r\n\x1b\\[closed]\r\n", &mut wire, &mut out);
        assert_eq!(out, vec![Action::Exited, Action::Notice(b"[closed]\r\n".to_vec())]);
        let (mut s, mut out) = (Session::default(), Vec::new());
        s.read(b"can't find session: x\r\n", &mut wire, &mut out);
        assert_eq!(out, vec![Action::Notice(b"can't find session: x\r\n".to_vec())]);
    }

    #[test]
    fn a_full_screen_program_is_seeded_over_the_screen_it_covers() {
        let (mut s, mut wire, mut out) = (Session::default(), Vec::new(), Vec::new());
        s.read(b"%begin 1 1 0\r\n%end 1 1 0\r\n", &mut wire, &mut out);
        // Two rows, one line of history, the cursor left at column 2 of
        // row 0 of the covered screen.
        let alt = "%4 80 2 3 1 1 0 0 1 0 0 1 0 0 0 1 0 1 2 0\r\n";
        s.read(&reply(2, alt), &mut wire, &mut out);
        assert!(text(&mut wire).contains("capture-pane -p -e -N -a -q -t %4 ;"));
        for part in ["before\r\n", "top\r\n\x0elqk\x0f\r\n", "top\r\nlqk\r\n", "$ vi\r\n\r\n", "$ vi\r\n\r\n", alt] {
            s.read(&reply(3, part), &mut wire, &mut out);
        }
        assert_eq!(
            out[1],
            Action::Seed {
                pane: 4,
                size: (80, 2),
                bytes: b"\x1bc\x1b)0before\r\n\x1b[0m\n\x1b[1;1H$ vi\x1b[2;1H\x0f\x1b[0m\x1b[1;3H\x1b[?1049h\x1b[1;1Htop\x1b[2;1H\x0elqk\x0f\x0f\x1b[0m\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[?25l\x1b[2;4H"
                    .to_vec()
            }
        );
    }
}
