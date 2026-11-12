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
                    .and_then(|rest| guard_number(rest))
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
            let pane = id(std::str::from_utf8(&rest[..cut]).ok()?, '%')?;
            let data = unescape(rest.get(cut + 1..).unwrap_or_default());
            Some(Event::Output { pane, data })
        }
        // `%extended-output %0 <age> … : <data>`
        b"%extended-output" => {
            let cut = rest.iter().position(|&b| b == b' ')?;
            let pane = id(std::str::from_utf8(&rest[..cut]).ok()?, '%')?;
            let at = rest.windows(3).position(|w| w == b" : ")?;
            Some(Event::Output { pane, data: unescape(&rest[at + 3..]) })
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
    while i < data.len() {
        let octal = |k: usize| data.get(i + k).filter(|b| (b'0'..=b'7').contains(b)).map(|b| u32::from(b - b'0'));
        if data[i] == b'\\'
            && let (Some(a), Some(b), Some(c)) = (octal(1), octal(2), octal(3))
            && let Ok(byte) = u8::try_from(a * 64 + b * 8 + c)
        {
            out.push(byte);
            i += 4;
        } else {
            out.push(data[i]);
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
    std::str::from_utf8(&b[start..*at]).ok()?.parse().ok()
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
    /// `capture-pane` of this pane: history first, then the screen.
    History { pane: u32 },
    /// `list-panes` with [`PANES_FORMAT`].
    Panes,
}

/// One line per pane: its size, cursor and the modes its program has set.
pub const PANES_FORMAT: &str = "#{pane_id} #{pane_width} #{pane_height} #{cursor_x} #{cursor_y} #{alternate_on} #{keypad_cursor_flag} #{keypad_flag} #{mouse_standard_flag} #{mouse_button_flag} #{mouse_any_flag} #{mouse_sgr_flag} #{cursor_flag}";

/// Commands in the order sent. tmux answers in that order, so each `Reply`
/// belongs to the oldest one waiting.
#[derive(Default)]
pub struct Commands {
    waiting: VecDeque<Ask>,
    /// The reply tmux sends on attach, before any command of ours.
    greeted: bool,
}

impl Commands {
    /// Appends one command line to `wire` and remembers what it asked.
    fn push(&mut self, wire: &mut Vec<u8>, ask: Ask, line: &str) {
        debug_assert!(!line.contains('\n'));
        wire.extend_from_slice(line.as_bytes());
        wire.push(b'\n');
        self.waiting.push_back(ask);
    }

    /// The command a reply answers (`None`: the attach greeting, or a reply
    /// nobody asked for).
    pub fn answered(&mut self) -> Option<Ask> {
        if !std::mem::replace(&mut self.greeted, true) {
            return None;
        }
        self.waiting.pop_front()
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

    /// The pane's last `lines` of history and its screen, with colours and
    /// wrapped lines joined (0: only the screen, row by row).
    pub fn history(&mut self, wire: &mut Vec<u8>, pane: u32, lines: u32) {
        let range = if lines == 0 { String::new() } else { format!(" -J -S -{lines}") };
        self.push(wire, Ask::History { pane }, &format!("capture-pane -p -e -t %{pane}{range}"));
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

/// What the view does with what the session read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Bytes for the pane's engine.
    Feed(Vec<u8>),
    /// This window cannot be shown pane-native yet (splits, another window):
    /// attach the classic way.
    Fallback(&'static str),
    /// tmux ended the client (the session closed or was detached).
    Exited,
}

/// History lines read when the pane is first shown.
pub const SEED_LINES: u32 = 2000;
const MAX_PENDING_INPUT: usize = 64 << 10;

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
struct PaneState {
    id: u32,
    size: (u16, u16),
    cursor: (u16, u16),
    alternate: bool,
    flags: [bool; 7],
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
    Some(PaneState { id, size, cursor, alternate, flags })
}

/// One control client showing the single pane of a session's window: reads
/// the stream, keeps the pane the size of the view and turns typed bytes
/// into commands. No IO: the caller moves `wire` to tmux and applies actions.
#[derive(Default)]
pub struct Session {
    parser: Parser,
    commands: Commands,
    events: Vec<Event>,
    greeted: bool,
    pane: Option<PaneState>,
    history: Option<Vec<Vec<u8>>>,
    seeded: bool,
    ended: bool,
    /// The pane size the view wants and the window size last asked for.
    want: Option<(u16, u16)>,
    asked: Option<(u16, u16)>,
    /// Window cells that are not the pane (the border line with its title).
    extra: (u16, u16),
    pending_input: Vec<u8>,
    /// Times the pane was read again while waiting for its new size.
    size_tries: u8,
    /// The screen has been asked for and is on its way.
    capturing: bool,
}

impl Session {
    /// True once the pane's screen is in the engine and keys reach it.
    pub fn seeded(&self) -> bool {
        self.seeded
    }

    pub fn pane(&self) -> Option<u32> {
        self.pane.map(|p| p.id)
    }

    fn ask_size(&mut self, wire: &mut Vec<u8>) {
        let Some((cols, rows)) = self.want else { return };
        let window = (cols.saturating_add(self.extra.0), rows.saturating_add(self.extra.1));
        if self.greeted && !self.ended && self.asked != Some(window) {
            self.asked = Some(window);
            self.commands.resize(wire, window.0, window.1);
        }
    }

    /// The view's grid changed (or is known for the first time).
    pub fn resize(&mut self, cols: u16, rows: u16, wire: &mut Vec<u8>) {
        self.want = Some((cols, rows));
        self.ask_size(wire);
    }

    /// Offers the size again, so tmux gives the window back to this client.
    pub fn claim(&mut self, wire: &mut Vec<u8>) {
        self.asked = None;
        self.ask_size(wire);
    }

    /// Typed or pasted bytes. Before the pane is known they wait.
    pub fn input(&mut self, bytes: &[u8], wire: &mut Vec<u8>) {
        match self.pane.filter(|_| self.seeded && !self.ended) {
            Some(pane) => self.commands.send_bytes(wire, pane.id, bytes),
            None if self.pending_input.len() + bytes.len() <= MAX_PENDING_INPUT => {
                self.pending_input.extend_from_slice(bytes);
            }
            None => {}
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
                // Output from before the screen was read is already in it.
                if self.seeded && self.pane.is_some_and(|p| p.id == pane) {
                    out.push(Action::Feed(data));
                }
            }
            Event::Reply { ok, lines, .. } => {
                if !self.greeted {
                    self.greeted = true;
                    self.commands.answered();
                    self.ask_size(wire);
                    self.commands.panes(wire);
                    return;
                }
                match self.commands.answered() {
                    Some(Ask::Panes) if ok => self.panes(&lines, wire, out),
                    Some(Ask::History { .. }) if ok => self.history = Some(lines),
                    Some(Ask::Panes | Ask::History { .. }) => out.push(Action::Fallback("tmux refused a query")),
                    _ => {}
                }
            }
            Event::LayoutChange { layout, .. } => {
                let [only] = layout.panes.as_slice() else {
                    // Another window of the session changing is not ours.
                    if self.pane.is_some_and(|pane| layout.panes.iter().any(|p| p.id == pane.id)) {
                        out.push(Action::Fallback("the window was split"));
                    }
                    return;
                };
                if self.pane.is_some_and(|pane| pane.id != only.id) {
                    return;
                }
                // The layout counts the border line as part of the pane; its
                // real size comes from the pane itself.
                if self.pane.is_some() {
                    self.commands.panes(wire);
                }
            }
            Event::Other(text) if text.starts_with("%session-window-changed") => {
                if self.pane.is_some() {
                    out.push(Action::Fallback("the session moved to another window"));
                }
            }
            Event::Exit { .. } => {
                self.ended = true;
                out.push(Action::Exited);
            }
            Event::Text(text) if !self.greeted || self.ended => {
                let mut bytes = text.into_bytes();
                bytes.extend_from_slice(b"\r\n");
                out.push(Action::Feed(bytes));
            }
            _ => {}
        }
    }

    /// Asks for a taller window when the pane came out smaller than the view
    /// by the rows tmux keeps for itself (the border line). True if it asked.
    /// A pane cut by another client's smaller screen is left alone.
    fn fit(&mut self, pane: (u16, u16), wire: &mut Vec<u8>) -> bool {
        let (Some(want), Some(asked)) = (self.want, self.asked) else { return false };
        let extra = (asked.0.saturating_sub(pane.0), asked.1.saturating_sub(pane.1));
        if pane == want || extra.0 != 0 || extra.1 > 2 || extra == self.extra {
            return false;
        }
        self.extra = extra;
        self.ask_size(wire);
        true
    }

    fn panes(&mut self, lines: &[Vec<u8>], wire: &mut Vec<u8>, out: &mut Vec<Action>) {
        let [line] = lines else {
            out.push(Action::Fallback("the window has several panes"));
            return;
        };
        let Some(state) = pane_state(line) else {
            out.push(Action::Fallback("unreadable pane state"));
            return;
        };
        if self.seeded {
            self.fit(state.size, wire);
            return;
        }
        match (self.pane, self.history.take()) {
            // The state read right after the screen: both go into the engine.
            (Some(first), Some(history)) if first.id == state.id => {
                self.pane = Some(state);
                self.seeded = true;
                out.push(Action::Feed(seed(&state, &history)));
                let waiting = std::mem::take(&mut self.pending_input);
                if !waiting.is_empty() {
                    self.commands.send_bytes(wire, state.id, &waiting);
                }
            }
            // A pane read for another reason while the screen is on its way.
            _ if self.capturing => {}
            _ => {
                self.pane = Some(state);
                // The screen is read at the size it will be shown at: a row
                // taken by the border line is asked for first, and the pane
                // is read again once tmux has resized it.
                if self.size_tries < 3 && self.fit(state.size, wire) {
                    self.size_tries += 1;
                    self.commands.panes(wire);
                    return;
                }
                let lines = if state.alternate { 0 } else { SEED_LINES };
                self.capturing = true;
                self.commands.history(wire, state.id, lines);
                self.commands.panes(wire);
            }
        }
    }
}

/// The bytes that leave a fresh engine showing what the pane shows: history,
/// screen, cursor and the modes its program had already set.
fn seed(state: &PaneState, lines: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"\x1bc".to_vec();
    if state.alternate {
        out.extend_from_slice(b"\x1b[?1049h");
        for (row, line) in lines.iter().take(usize::from(state.size.1)).enumerate() {
            out.extend_from_slice(format!("\x1b[{};1H", row + 1).as_bytes());
            out.extend_from_slice(line);
        }
    } else {
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                out.extend_from_slice(b"\r\n");
            }
            out.extend_from_slice(line);
        }
    }
    out.extend_from_slice(b"\x1b[0m");
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
        commands.history(&mut wire, 3, 500);
        commands.send_bytes(&mut wire, 3, b"\x1b[A\xc3\xa9");
        assert_eq!(
            String::from_utf8(wire).unwrap(),
            "refresh-client -C 120,40\ncapture-pane -p -e -t %3 -J -S -500\nsend-keys -t %3 -H 1b 5b 41 c3 a9\n"
        );
        assert_eq!(commands.answered(), None);
        assert_eq!(commands.answered(), Some(Ask::Plain));
        assert_eq!(commands.answered(), Some(Ask::History { pane: 3 }));
        assert_eq!(commands.pending(), 1);
    }

    #[test]
    fn long_input_is_split_across_commands() {
        let mut commands = Commands::default();
        let mut wire = Vec::new();
        commands.send_bytes(&mut wire, 1, &[b'a'; 600]);
        assert_eq!(commands.pending(), 3);
        assert!(wire.split(|&b| b == b'\n').all(|line| line.len() < 1024));
    }

    const STATE: &str = "%4 80 24 10 2 0 1 0 0 0 0 0 1";

    fn reply(number: u32, body: &str) -> Vec<u8> {
        format!("%begin 1 {number} 1\r\n{body}%end 1 {number} 1\r\n").into_bytes()
    }

    fn text(wire: &mut Vec<u8>) -> String {
        String::from_utf8(std::mem::take(wire)).unwrap()
    }

    fn seeded_session() -> (Session, Vec<u8>, Vec<Action>) {
        let (mut s, mut wire, mut out) = (Session::default(), Vec::new(), Vec::new());
        s.resize(80, 24, &mut wire);
        s.input(b"early", &mut wire);
        assert!(wire.is_empty(), "nothing is sent before tmux greets");
        s.read(b"\x1bP1000p%begin 1 1 0\r\n%end 1 1 0\r\n", &mut wire, &mut out);
        assert_eq!(text(&mut wire), format!("refresh-client -C 80,24\nlist-panes -F \"{PANES_FORMAT}\"\n"));
        // The border line takes a row: the window is asked one row taller.
        s.read(&reply(2, ""), &mut wire, &mut out);
        s.read(&reply(3, "%4 80 23 0 0 0 0 0 0 0 0 0 1\r\n"), &mut wire, &mut out);
        assert_eq!(text(&mut wire), format!("refresh-client -C 80,25\nlist-panes -F \"{PANES_FORMAT}\"\n"));
        s.read(&reply(31, ""), &mut wire, &mut out);
        s.read(b"%layout-change @1 b65d,80x25,0,0,4 x *\r\n", &mut wire, &mut out);
        assert_eq!(text(&mut wire), format!("list-panes -F \"{PANES_FORMAT}\"\n"));
        s.read(&reply(32, "%4 80 24 0 0 0 0 0 0 0 0 0 1\r\n"), &mut wire, &mut out);
        assert_eq!(
            text(&mut wire),
            format!("capture-pane -p -e -t %4 -J -S -2000\nlist-panes -F \"{PANES_FORMAT}\"\n")
        );
        s.read(b"%output %4 dropped: already in the capture\r\n", &mut wire, &mut out);
        s.read(&reply(33, "%4 80 24 0 0 0 0 0 0 0 0 0 1\r\n"), &mut wire, &mut out);
        assert!(text(&mut wire).is_empty(), "the answer to the layout change adds nothing");
        assert!(out.is_empty());
        s.read(&reply(34, "old\r\n\x1b[31mprompt$ \r\n"), &mut wire, &mut out);
        s.read(&reply(35, &format!("{STATE}\r\n")), &mut wire, &mut out);
        (s, wire, out)
    }

    #[test]
    fn a_session_seeds_the_engine_then_streams_output_and_input() {
        let (mut s, mut wire, mut out) = seeded_session();
        assert!(s.seeded());
        assert_eq!(
            out,
            vec![Action::Feed(b"\x1bcold\r\n\x1b[31mprompt$ \x1b[0m\x1b[?1h\x1b[?2004h\x1b[3;11H".to_vec())]
        );
        // What was typed while attaching goes to the pane now.
        assert_eq!(text(&mut wire), "send-keys -t %4 -H 65 61 72 6c 79\n");
        out.clear();
        s.read(b"%output %4 hi\\015\r\n%output %9 other pane\r\n", &mut wire, &mut out);
        assert_eq!(out, vec![Action::Feed(b"hi\r".to_vec())]);
        s.input(b"\r", &mut wire);
        assert_eq!(text(&mut wire), "send-keys -t %4 -H 0d\n");
    }

    #[test]
    fn the_window_is_asked_larger_by_what_is_not_the_pane() {
        let (mut s, mut wire, mut out) = seeded_session();
        out.clear();
        text(&mut wire);
        s.resize(100, 30, &mut wire);
        assert_eq!(text(&mut wire), "refresh-client -C 100,31\n");
        s.claim(&mut wire);
        assert_eq!(text(&mut wire), "refresh-client -C 100,31\n");
        // A phone holds the window smaller: the pane is not chased.
        s.read(b"%layout-change @1 b65d,60x11,0,0,4 x *\r\n", &mut wire, &mut out);
        assert_eq!(text(&mut wire), format!("list-panes -F \"{PANES_FORMAT}\"\n"));
        s.read(&reply(40, ""), &mut wire, &mut out);
        s.read(&reply(41, ""), &mut wire, &mut out);
        s.read(&reply(42, "%4 60 10 0 0 0 0 0 0 0 0 0 1\r\n"), &mut wire, &mut out);
        assert!(text(&mut wire).is_empty());
        assert!(out.is_empty());
    }

    #[test]
    fn splits_other_windows_and_the_end_are_reported() {
        let (mut s, mut wire, mut out) = seeded_session();
        out.clear();
        s.read(b"%layout-change @7 b65d,80x24,0,0{40x24,0,0,8,39x24,41,0,9} x *\r\n", &mut wire, &mut out);
        assert!(out.is_empty(), "a window without our pane is not ours");
        s.read(b"%layout-change @1 b65d,80x24,0,0{40x24,0,0,4,39x24,41,0,9} x *\r\n", &mut wire, &mut out);
        assert_eq!(out, vec![Action::Fallback("the window was split")]);
        out.clear();
        s.read(b"%session-window-changed $1 @7\r\n%exit\r\n\x1b\\[closed]\r\n", &mut wire, &mut out);
        assert_eq!(
            out,
            vec![
                Action::Fallback("the session moved to another window"),
                Action::Exited,
                Action::Feed(b"[closed]\r\n".to_vec()),
            ]
        );

        let (mut s, mut wire, mut out) = (Session::default(), Vec::new(), Vec::new());
        s.read(b"can't find session: x\r\n", &mut wire, &mut out);
        assert_eq!(out, vec![Action::Feed(b"can't find session: x\r\n".to_vec())]);
        out.clear();
        s.read(b"%begin 1 1 0\r\n%end 1 1 0\r\n", &mut wire, &mut out);
        s.read(&reply(2, &format!("{STATE}\r\n%5 80 24 0 0 0 0 0 0 0 0 0 1\r\n")), &mut wire, &mut out);
        assert_eq!(out, vec![Action::Fallback("the window has several panes")]);
    }

    #[test]
    fn a_full_screen_program_is_seeded_row_by_row_on_the_alternate_screen() {
        let (mut s, mut wire, mut out) = (Session::default(), Vec::new(), Vec::new());
        s.read(b"%begin 1 1 0\r\n%end 1 1 0\r\n", &mut wire, &mut out);
        let alt = "%4 80 2 3 1 1 0 0 1 0 0 1 0";
        s.read(&reply(2, &format!("{alt}\r\n")), &mut wire, &mut out);
        assert!(text(&mut wire).contains("capture-pane -p -e -t %4\n"));
        s.read(&reply(3, "top\r\nbottom\r\n"), &mut wire, &mut out);
        s.read(&reply(4, &format!("{alt}\r\n")), &mut wire, &mut out);
        assert_eq!(
            out,
            vec![Action::Feed(
                b"\x1bc\x1b[?1049h\x1b[1;1Htop\x1b[2;1Hbottom\x1b[0m\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[?25l\x1b[2;4H".to_vec()
            )]
        );
    }
}
