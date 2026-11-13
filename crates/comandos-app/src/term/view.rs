//! GTK terminal over the shared engine and a private nonblocking PTY.
use super::{
    engine::*,
    keys::{drag_button, key_action, mouse_button},
    paint::{CellGeom, FrameCache, LineKind, PaintOp},
    pty::{DrainOutcome, PtyDrain, PtyError, PtySession},
    schedule::PaintSchedule,
    settle::{
        HIDDEN_RESIZE_MS, HIDDEN_RESIZE_STEP_MS, HIDDEN_RESIZE_TURNS, HOLD_CAP_MS, HOLD_IDLE_MS, HOLD_QUIET_MS,
        RESIZE_QUIET_MS, RESPAWN_MS, SETTLE_CAP_MS,
        SETTLE_QUIET_MS, Settle, fits_shared_window, initial_size,
    },
};
use crate::config::RunMode;
use glib::{ControlFlow, IOCondition, SourceId};
use gtk::prelude::*;
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

pub type TextCallback = Rc<dyn Fn(&str)>;
pub type ExitCallback = Rc<dyn Fn(i32)>;
pub type ScrollCallback = Rc<dyn Fn(&str, i32, u16, u16)>;
pub type AppKeyCallback = Rc<dyn Fn(&gdk::EventKey) -> bool>;
pub type KeyObserver = Rc<dyn Fn(&gdk::EventKey)>;
pub type LinkCallback = Rc<dyn Fn(&str, &gdk::EventButton, (u16, u16), bool)>;
pub type ContextCallback = Rc<dyn Fn(&gdk::EventButton, (u16, u16))>;
pub struct TermOptions {
    pub argv: Vec<String>,
    /// `tmux -CC attach` for the same session: the app draws the pane itself
    /// (own history, scroll and selection). `argv` stays as the classic way
    /// for windows control mode does not cover yet (see `control::Session`).
    pub control_argv: Option<Vec<String>>,
    pub cwd: PathBuf,
    pub session: Option<String>,
    pub palette: Palette,
    pub scrollback: usize,
    pub preferences: Value,
    pub mode: RunMode,
    pub environment: Vec<(String, String)>,
    pub clear_env: bool,
    pub tmux_size: Option<(u16, u16)>,
    pub shadow_tmux: Option<crate::tmux::TmuxCtl>,
    pub before_spawn: Option<Rc<dyn Fn() -> Result<(), String>>>,
    pub on_title: Option<TextCallback>,
    pub on_exit: Option<ExitCallback>,
    pub on_bell: Option<Rc<dyn Fn()>>,
    pub on_link: Option<TextCallback>,
    pub on_ssh_scroll: Option<ScrollCallback>,
}
#[derive(Debug)]
pub enum TermViewError {
    GtkUnavailable,
    UnsafeEnvironment,
    ShadowSource(String),
    Pty(PtyError),
}
struct Model {
    engine: TermEngine,
    frame: FrameCache,
    pty: Option<PtySession>,
    shadow: Option<super::shadow::ShadowReader>,
    shadow_frame: Option<super::shadow::ShadowFrame>,
    shadow_at: u64,
    drain: PtyDrain,
    geom: CellGeom,
    font: pango::FontDescription,
    font_scale: f64,
    opacity: f64,
    padding: f64,
    preferences: Value,
    settle: Settle,
    respawn_at: Option<u64>,
    respawn: super::lifecycle::RespawnPolicy,
    schedule: PaintSchedule,
    focused: bool,
    blink_visible: bool,
    selection: Option<Selection>,
    pressed: Option<Point>,
    primary_press: Option<super::links::PrimaryPress>,
    link_gesture: bool,
    last_motion: Option<(u16, u16, gdk::ModifierType)>,
    hovered_link: Option<(Point, Point)>,
    ime: Ime,
    preedit: String,
    cursor_shape: CursorShape,
    wheel_delta: f64,
    /// Lines the wheel asked for that the view has not moved yet (positive:
    /// toward older text), and the part of a line it is past its grid now.
    /// The view glides there over a few frames instead of jumping.
    glide: f64,
    glide_frac: f64,
    ssh_delta: f64,
    ssh_point: (u16, u16),
    ssh_at: Option<u64>,
    resize_at: Option<u64>,
    /// Lag log: last key sent and first output not yet painted.
    key_at: Option<Instant>,
    /// Panes that got text from tmux since the last read was accounted for.
    fed: Vec<u32>,
    /// Something reached every pane (a new layout, a notice of the app).
    fed_all: bool,
    /// The key whose echo is on its way to the screen (lag trace only).
    key_shown: Option<Instant>,
    /// Noise filter: on/off, the panes to filter and the last row maps.
    hide_noise: bool,
    /// The tmux control client behind this view, when it is pane-native.
    control: Option<super::control::Session>,
    /// The pane whose engine, picture and selection sit in the fields of
    /// this model right now: the one under the pointer, or the one with the
    /// keys while typing. The other panes of the window wait in `slots`.
    loaded: Option<super::control::PaneInfo>,
    slots: Vec<PaneSlot>,
    /// The pane tmux sends typed keys to.
    key_pane: Option<u32>,
    /// Panes whose picture changed since they were painted.
    stale: Vec<u32>,
    /// Left edge of the window's first column.
    base_x: f64,
    /// Cells of the tmux window the panes sit in (right and bottom edges).
    window_cells: (u16, u16),
    /// When the panes are next compared with tmux, the first output not
    /// compared yet, and how many times in a row each pane was found wrong.
    audit_at: Option<u64>,
    audit_from: Option<u64>,
    audit_wrong: Vec<(u32, u8)>,
    /// Lines the engine had scrolled off its top when the selection was
    /// last moved with them.
    scroll_seen: u64,
    regions: Vec<super::noise::Region>,
    noise_maps: Vec<(super::noise::Region, Vec<Option<u16>>)>,
    output_at: Option<Instant>,
    /// tmux pane id of each region, and the history that fills rows freed by
    /// hidden noise (fetched off the main thread, see `fetch_history`).
    pane_ids: Vec<String>,
    fills: std::collections::HashMap<String, Fill>,
    fill_wanted: Vec<(String, u16)>,
    fill_busy: std::collections::HashSet<String>,
    /// Claude panes the app scrolls back itself, by tmux pane id.
    scrolls: std::collections::HashMap<String, Scroll>,
    output_ms: u64,
    /// Last wheel scroll: tmux moved the text, so history fills are stale.
    scroll_ms: u64,
    /// Mouse selection done by the app inside one pane (see `pick.rs`).
    pick: Option<super::pick::Pick>,
    /// Per pane, how far tmux copy mode is scrolled back (this frame).
    copy_pos: Vec<(super::noise::Region, u32)>,
    /// Rows under the panes that belong to tmux's status line.
    status_rows: u16,
    /// Panes as tmux last reported them (late while resizing). `regions`
    /// holds the panes found on the current frame when tiles are visible.
    tmux_regions: Vec<super::noise::Region>,
    tmux_ids: Vec<String>,
    /// Last tiles found, kept for a frame caught halfway through a tmux
    /// redraw (no borders yet) so the tiles never blink.
    held_tiles: Option<(Vec<super::tiles::Tile>, Instant)>,
    /// The picture kept on screen while a new size settles.
    hold: Option<Hold>,
    resized_ms: u64,
    /// When this tab was last shown. A history fill read before that is drawn
    /// only once its text has been matched against the screen again.
    shown_ms: u64,
    /// Lag log: the tab was just shown and has not painted / got its history.
    shown_at: Option<Instant>,
    shown_fill: Option<Instant>,
    /// What the canvas shows, to repaint only the rows that change.
    painted: Option<FrameLook>,
    /// Screen row where the cursor was painted.
    painted_cursor: Option<u16>,
}

/// After a resize the old picture stays until tmux and the program have
/// drawn the new size, so the screen changes once instead of showing every
/// half-drawn step.
#[derive(Debug, Clone, Copy)]
struct Hold {
    since: u64,
    cap_at: u64,
    /// The new screen is drawn; only the history above it is still on its way.
    filling: bool,
}
impl Hold {
    fn due(&self, output_ms: u64) -> u64 {
        let quiet = if self.filling {
            self.cap_at
        } else if output_ms > self.since {
            output_ms.saturating_add(HOLD_QUIET_MS)
        } else {
            self.since.saturating_add(HOLD_IDLE_MS)
        };
        quiet.min(self.cap_at)
    }
}
/// A pane is compared with tmux this long after its last output, and at
/// least this often while it keeps writing.
const AUDIT_QUIET_MS: u64 = 500;
const AUDIT_BUSY_MS: u64 = 4000;
/// Repairs in a row that did not hold before a pane is left alone.
const AUDIT_GIVE_UP: u8 = 3;

/// A pane that is not loaded in the model: everything that is its own.
struct PaneSlot {
    info: super::control::PaneInfo,
    engine: TermEngine,
    frame: FrameCache,
    selection: Option<Selection>,
    /// Its own wheel glide (see `Model::glide`).
    glide: (f64, f64),
    /// See `Model::scroll_seen`.
    scroll_seen: u64,
}
/// A history fill as one frame paints it.
#[derive(Clone, PartialEq)]
struct FillDraw {
    id: String,
    region: super::noise::Region,
    holes: u16,
    /// Visible lines scrolled back by the app.
    skip: usize,
    count: usize,
    /// Rows of the fill that can be drawn: it moves as the pane scrolls.
    end: usize,
    asked_ms: u64,
}
/// Everything a picture depends on besides the text of each row. Two equal
/// looks differ only in the rows whose text changed.
#[derive(Clone, PartialEq)]
struct FrameLook {
    fresh: bool,
    /// No selection, composition or scrollback on top of the text.
    plain: bool,
    size: (u16, u16),
    area: (i32, i32),
    geom: CellGeom,
    opacity: f64,
    palette: Palette,
    tiles: Vec<super::tiles::Tile>,
    maps: Vec<(super::noise::Region, Vec<Option<u16>>)>,
    fills: Vec<FillDraw>,
    hovered_link: Option<(Point, Point)>,
    status_rows: u16,
}
/// A frame read at the start of a GTK frame and painted at its end.
struct Planned {
    look: FrameLook,
    /// Screen rows to paint again; `None` paints everything.
    rows: Option<Vec<bool>>,
}

/// tmux history above one pane, parsed and planned like the screen.
struct Fill {
    frame: FrameCache,
    /// Rows to show, top to bottom, noise already removed.
    picks: Vec<usize>,
    /// Whether each of `picks` is an empty row.
    blank: Vec<bool>,
    /// `picks[..end]` are above the live screen and can be drawn.
    end: usize,
    asked_ms: u64,
    /// Rows read.
    count: usize,
    /// Row `j` is tmux line `j - base` (0 = top of the live screen).
    base: usize,
    /// Rows of the live screen read after the history (0: a scroll copy).
    /// When the pane scrolls, lines leave the screen: they are already here,
    /// so `align` only moves `base` and nothing is missing until tmux is
    /// read again.
    screen: usize,
    /// Text marks of the rows from `marks_from` on (see `pick::row_mark`).
    marks: Vec<u64>,
    marks_from: usize,
    aligned: bool,
    /// Last time `align` found the screen's first rows in this fill.
    matched_ms: u64,
    /// The screen is far from where it was read, or was not found: read soon.
    loose: bool,
    /// The screen starts exactly where it did when this was read, and
    /// nowhere else: no line has left it, so the history is still this one.
    sure: bool,
    /// Kept to read the text of a filled row when it is selected.
    engine: TermEngine,
    lines: u16,
    /// tmux may hold more history than was asked for.
    more: bool,
    left: u16,
    width: u16,
    geom: CellGeom,
}
impl Fill {
    /// The rows drawn in `holes` rows above the first screen row shown,
    /// `skip` visible lines back from the newest.
    fn shown(&self, skip: usize, holes: usize) -> &[usize] {
        let end = self.end.saturating_sub(skip);
        &self.picks[end.saturating_sub(holes)..end]
    }
    /// Row `base` is the first row of the live screen.
    fn set_base(&mut self, base: usize) {
        self.base = base;
        let mut end = self.picks.partition_point(|&j| j < base);
        if self.screen == 0 {
            // A blank at the end of a scroll copy would just add an empty row.
            while end > 0 && self.blank[end - 1] {
                end -= 1;
            }
        } else if self.blank.get(end) == Some(&true) {
            // The pane drops a blank first row (`noise::keep_flags` starts as
            // if one came before): the row that separates it from its
            // history is drawn here, or the text would jump a row when that
            // blank scrolls into the history.
            end += 1;
        }
        self.end = end;
    }
    /// Finds where the live screen starts now from the text of its first
    /// rows (`top`), nearest to where it was read and forward first.
    fn align(&mut self, top: &[u64], now_ms: u64) {
        self.aligned = true;
        if self.screen == 0 || top.is_empty() {
            return;
        }
        let nominal = self.count.saturating_sub(self.screen);
        let hit = |at: usize| {
            let known = (self.count - at).min(top.len());
            known >= top.len().min(3)
                && top[..known].iter().any(|mark| *mark != 0)
                && (0..known).all(|i| self.marks.get(at + i - self.marks_from) == Some(&top[i]))
        };
        let found = (nominal..self.count).chain((self.marks_from..nominal).rev()).find(|&at| hit(at));
        self.sure = found == Some(nominal) && !(self.marks_from..self.count).any(|at| at != nominal && hit(at));
        match found {
            Some(at) => {
                self.matched_ms = now_ms;
                self.loose = at.abs_diff(nominal) * 2 > self.screen;
                if at != self.base {
                    self.set_base(at);
                }
            }
            None => self.loose = top.iter().any(|mark| *mark != 0),
        }
    }
    fn fits(&self, m: &Model, r: &super::noise::Region) -> bool {
        self.geom == m.geom && self.left == r.left && self.width == r.width
    }
    /// Read, or matched against the screen, since the tab was last shown: a
    /// copy kept from an earlier visit is never drawn on trust.
    fn current(&self, shown_ms: u64) -> bool {
        self.asked_ms >= shown_ms || self.matched_ms >= shown_ms
    }
}
fn next_turn() -> u64 {
    thread_local! {
        static TURN: Cell<u64> = const { Cell::new(0) };
    }
    TURN.with(|t| t.replace((t.get() + 1) % HIDDEN_RESIZE_TURNS))
}
const MAX_FILL_LINES: u16 = 500;
/// How long the PTY must stay silent before a frame that follows fresh
/// output is planned, and the most that wait may add to one frame.
const SETTLE_OUTPUT_QUIET: Duration = Duration::from_micros(1500);
const SETTLE_OUTPUT_CAP: Duration = Duration::from_millis(5);
/// Rows at the top of a pane compared with its history fill.
const ALIGN_ROWS: usize = 8;
/// Output that does not move the screen (`Fill::sure`) leaves the history as
/// it was read; it is still read again after this long, in case the rows
/// compared were repeated text.
const FILL_TRUST_MS: u64 = 3000;
/// Moves a pane's history fill to where its screen starts now.
fn align_fill(m: &mut Model, id: &str, r: &super::noise::Region, now_ms: u64) {
    let height = usize::from(r.height);
    let rows = ALIGN_ROWS.min((height / 3).max(3)).min(height);
    let mut render = RowRender::default();
    let top: Vec<u64> = (0..rows)
        .map(|i| {
            m.engine.render_line(usize::from(r.top) + i, &mut render);
            super::pick::row_mark(&render, r.left, r.width)
        })
        .collect();
    if let Some(fill) = m.fills.get_mut(id) {
        fill.align(&top, now_ms);
    }
}
/// With noise hidden, the wheel over a Claude Code pane does not go to tmux:
/// tmux would move raw lines (gray ones too) and show the gray text again.
/// The app shows a filtered copy of the history above the input box and
/// moves it by the lines the screen shows.
#[derive(Debug, Clone, Copy)]
struct Scroll {
    /// Visible lines back from the newest.
    back: usize,
    /// History lines asked from tmux for the copy.
    lines: u16,
    busy: bool,
}
const SCROLL_LINES: u16 = 800;
const MAX_SCROLL_LINES: u16 = 6400;
fn scroll_key(id: &str) -> String {
    format!("{id}\u{1}scroll")
}
/// `(tmux target, first line, last line, done)`: read pane lines from tmux
/// (0 = top of the live screen, negative = history).
pub type HistorySource = Rc<dyn Fn(String, i64, i64, Box<dyn FnOnce(Option<String>)>)>;
struct Inner {
    area: gtk::DrawingArea,
    im: gtk::IMMulticontext,
    options: TermOptions,
    epoch: Instant,
    model: RefCell<Model>,
    read_source: RefCell<Option<SourceId>>,
    write_source: RefCell<Option<SourceId>>,
    timer: RefCell<Option<SourceId>>,
    closed: Cell<bool>,
    /// Control mode gave up on this session's window: attach the classic way.
    classic: Cell<bool>,
    context_menu: RefCell<Option<ContextCallback>>,
    link_event: RefCell<Option<LinkCallback>>,
    clean_click: RefCell<Option<ContextCallback>>,
    primary_press: RefCell<Option<ContextCallback>>,
    replay: RefCell<crate::ui::clipboard::Replay<(gdk::EventKey, f64)>>,
    app_key: RefCell<Option<AppKeyCallback>>,
    key_observer: RefCell<Option<KeyObserver>>,
    last_ctrl_c: Cell<Option<f64>>,
    cleanup_cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    clipboard: Rc<crate::ui::clipboard::Clipboard>,
    /// Shaped text survives between frames; rebuilt when the font or scale changes.
    painter: RefCell<Option<TextPainter>>,
    /// Client-side frame buffer the terminal paints into before one copy.
    canvas: RefCell<Option<cairo::ImageSurface>>,
    history: RefCell<Option<HistorySource>>,
    /// Scrolls tmux while a selection is dragged past the pane's edge.
    pick_timer: RefCell<Option<SourceId>>,
    /// The next draw repaints everything.
    full: Cell<bool>,
    frame_asked: Cell<bool>,
    /// A frame callback is moving the view toward where the wheel left it.
    gliding: Cell<bool>,
    /// Lag log: when the last frame was asked for and when it started.
    asked_at: Cell<Option<Instant>>,
    ticked_at: Cell<Option<Instant>>,
    /// Lag log: when the main loop was first free after the frame was asked for.
    free_at: Rc<Cell<Option<Instant>>>,
    /// When the PTY was last read (see `settle_output`).
    read_at: Cell<Option<Instant>>,
    planned: RefCell<Option<Planned>>,
    /// Place in the queue of hidden tabs that resize one after another.
    turn: u64,
}
#[derive(Clone)]
pub struct TermView {
    inner: Rc<Inner>,
}
impl TermView {
    pub fn new(options: TermOptions) -> Result<Self, TermViewError> {
        if !gtk::is_initialized_main_thread() {
            return Err(TermViewError::GtkUnavailable);
        }
        if options.mode == RunMode::Sandbox && !options.clear_env {
            return Err(TermViewError::UnsafeEnvironment);
        }
        let shadow = if options.mode == RunMode::Shadow {
            let ctl = options.shadow_tmux.clone().ok_or_else(|| {
                TermViewError::ShadowSource("Shadow requires a readonly source".into())
            })?;
            let session = options.session.clone().ok_or_else(|| {
                TermViewError::ShadowSource("Shadow requires an existing session".into())
            })?;
            Some(
                super::shadow::ShadowReader::new(ctl, session)
                    .map_err(|e| TermViewError::ShadowSource(format!("{e:?}")))?,
            )
        } else {
            None
        };
        let area = gtk::DrawingArea::new();
        area.set_can_focus(true);
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.add_events(
            gdk::EventMask::KEY_PRESS_MASK
                | gdk::EventMask::KEY_RELEASE_MASK
                | gdk::EventMask::BUTTON_PRESS_MASK
                | gdk::EventMask::BUTTON_RELEASE_MASK
                | gdk::EventMask::POINTER_MOTION_MASK
                | gdk::EventMask::LEAVE_NOTIFY_MASK
                | gdk::EventMask::SCROLL_MASK
                | gdk::EventMask::SMOOTH_SCROLL_MASK
                | gdk::EventMask::FOCUS_CHANGE_MASK,
        );
        let epoch = Instant::now();
        let preferences = options.preferences.clone();
        let cleanup_cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let clipboard = crate::ui::clipboard::Clipboard::new(
            options.mode,
            Rc::new(crate::ui::clipboard::GtkClipboard),
            cleanup_cancelled.clone(),
        );
        let inner = Rc::new(Inner {
            area,
            im: gtk::IMMulticontext::new(),
            epoch,
            model: RefCell::new(Model {
                engine: TermEngine::new(
                    80,
                    24,
                    options.scrollback,
                    options.palette.clone(),
                    true,
                    epoch,
                ),
                frame: FrameCache::default(),
                pty: None,
                shadow,
                shadow_frame: None,
                shadow_at: 0,
                drain: PtyDrain::default(),
                geom: CellGeom {
                    cell_w: 8.,
                    cell_h: 19.,
                    origin_x: 12.,
                    origin_y: 10.,
                    dpr: 1.,
                    font_size: 13.,
                },
                font: pango::FontDescription::from_string("monospace 13"),
                font_scale: 1.,
                opacity: 1.,
                padding: 10.,
                preferences,
                settle: Settle::new(SETTLE_QUIET_MS, SETTLE_CAP_MS, 0),
                respawn_at: None,
                respawn: super::lifecycle::RespawnPolicy::default(),
                schedule: PaintSchedule::default(),
                focused: false,
                blink_visible: true,
                selection: None,
                pressed: None,
                primary_press: None,
                link_gesture: false,
                last_motion: None,
                hovered_link: None,
                ime: Ime::default(),
                preedit: String::new(),
                cursor_shape: CursorShape::Block,
                wheel_delta: 0.,
                glide: 0.,
                glide_frac: 0.,
                ssh_delta: 0.,
                ssh_point: (0, 0),
                ssh_at: None,
                resize_at: None,
                key_at: None,
                fed: Vec::new(),
                fed_all: false,
                key_shown: None,
                output_at: None,
                hide_noise: true,
                control: None,
                loaded: None,
                slots: Vec::new(),
                key_pane: None,
                stale: Vec::new(),
                base_x: 12.,
                window_cells: (0, 0),
                audit_at: None,
                audit_from: None,
                audit_wrong: Vec::new(),
                scroll_seen: 0,
                regions: Vec::new(),
                noise_maps: Vec::new(),
                pane_ids: Vec::new(),
                fills: std::collections::HashMap::new(),
                fill_wanted: Vec::new(),
                fill_busy: std::collections::HashSet::new(),
                scrolls: std::collections::HashMap::new(),
                output_ms: 0,
                scroll_ms: 0,
                pick: None,
                copy_pos: Vec::new(),
                status_rows: 1,
                tmux_regions: Vec::new(),
                tmux_ids: Vec::new(),
                held_tiles: None,
                hold: None,
                resized_ms: 0,
                shown_ms: 0,
                shown_at: None,
                shown_fill: None,
                painted: None,
                painted_cursor: None,
            }),
            options,
            painter: RefCell::new(None),
            canvas: RefCell::new(None),
            history: RefCell::new(None),
            pick_timer: RefCell::new(None),
            full: Cell::new(true),
            frame_asked: Cell::new(false),
            gliding: Cell::new(false),
            asked_at: Cell::new(None),
            ticked_at: Cell::new(None),
            free_at: Rc::new(Cell::new(None)),
            read_at: Cell::new(None),
            turn: next_turn(),
            planned: RefCell::new(None),
            read_source: RefCell::new(None),
            write_source: RefCell::new(None),
            timer: RefCell::new(None),
            closed: Cell::new(false),
            classic: Cell::new(false),
            context_menu: RefCell::new(None),
            link_event: RefCell::new(None),
            clean_click: RefCell::new(None),
            primary_press: RefCell::new(None),
            replay: RefCell::new(crate::ui::clipboard::Replay::default()),
            app_key: RefCell::new(None),
            key_observer: RefCell::new(None),
            last_ctrl_c: Cell::new(None),
            cleanup_cancelled,
            clipboard,
        });
        inner.configure_font();
        connect_events(&inner);
        inner.arm();
        // Standalone terminals start immediately; session clients wait for size.
        if inner.options.mode != RunMode::Shadow && inner.options.session.is_none() {
            inner.spawn().map_err(TermViewError::Pty)?;
        }
        Ok(Self { inner })
    }
    pub fn shutdown(&self) {
        self.inner.model.borrow_mut().respawn.shutdown();
        self.inner.closed.set(true);
        self.inner.replay.borrow_mut().cancel();
        self.inner.link_event.borrow_mut().take();
        self.inner.clean_click.borrow_mut().take();
        self.inner.primary_press.borrow_mut().take();
        self.inner.clipboard.invalidate();
        self.inner.last_ctrl_c.set(None);
        self.inner
            .cleanup_cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.inner.app_key.borrow_mut().take();
        self.inner.key_observer.borrow_mut().take();
        for source in [
            &self.inner.read_source,
            &self.inner.write_source,
            &self.inner.timer,
        ] {
            if let Some(id) = source.borrow_mut().take() {
                id.remove();
            }
        }
        self.inner.model.borrow_mut().pty.take();
        self.inner.model.borrow_mut().shadow.take();
    }
    pub fn on_app_key(&self, callback: AppKeyCallback) {
        *self.inner.app_key.borrow_mut() = Some(callback);
    }
    pub fn on_key_observer(&self, callback: KeyObserver) {
        *self.inner.key_observer.borrow_mut() = Some(callback);
    }
    /// Geometry in tmux window rows: row 0 is the border line the pane header
    /// is drawn over. A pane-native view keeps that row free above its text,
    /// so overlays placed by tmux coordinates land where they always did.
    pub fn cell_geometry(&self) -> CellGeom {
        let m = self.inner.model.borrow();
        let mut geom = m.geom;
        if self.inner.pane_native_wanted() {
            (geom.origin_x, geom.origin_y) = (m.base_x, m.padding + 14.);
        }
        geom
    }
    pub fn grid_size(&self) -> (u16, u16) {
        self.inner.model.borrow().engine.size()
    }
    /// El gesto pertenece a esta terminal, como _last_ctrl_c del objeto VTE original.
    pub fn ctrl_c_action(
        &self,
        selection: bool,
        copy_context: bool,
        now: f64,
    ) -> crate::agent_stop::CtrlCAction {
        use crate::agent_stop::{self, CtrlCAction};
        if self.inner.closed.get() {
            self.inner.last_ctrl_c.set(None);
            return CtrlCAction::SendInterrupt;
        }
        let action = agent_stop::ctrl_c_action(
            selection,
            copy_context,
            now,
            self.inner.last_ctrl_c.get().unwrap_or(f64::NAN),
            agent_stop::DOUBLE_TAP_SECONDS,
        );
        self.inner
            .last_ctrl_c
            .set(if action == CtrlCAction::SendInterrupt && !copy_context {
                Some(now)
            } else {
                None
            });
        action
    }
    pub fn cleanup_cancellation(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.inner.cleanup_cancelled.clone()
    }
    pub fn set_respawn(&self, enabled: bool) {
        self.inner.model.borrow_mut().respawn.set_enabled(enabled);
    }
    pub fn clipboard(&self) -> Rc<crate::ui::clipboard::Clipboard> {
        self.inner.clipboard.clone()
    }
    pub fn paste_clipboard(&self, primary: bool) {
        let weak = Rc::downgrade(&self.inner);
        self.inner.clipboard.request(
            if primary {
                crate::ui::clipboard::Target::Primary
            } else {
                crate::ui::clipboard::Target::Clipboard
            },
            Box::new(move |text| {
                if let Some(inner) = weak.upgrade().filter(|i| !i.closed.get()) {
                    let bytes = encode_paste(&text, &inner.model.borrow().engine.modes());
                    inner.send_event(&bytes);
                }
            }),
        );
    }
    pub fn on_primary_press(&self, callback: ContextCallback) {
        *self.inner.primary_press.borrow_mut() = Some(callback);
    }
    pub fn on_clean_click(&self, callback: ContextCallback) {
        *self.inner.clean_click.borrow_mut() = Some(callback);
    }
    pub fn clear_selection(&self) {
        self.inner.model.borrow_mut().selection = None;
        self.inner.redraw();
    }
    pub fn on_link_event(&self, callback: LinkCallback) {
        *self.inner.link_event.borrow_mut() = Some(callback);
    }
    pub fn key_pending(&self) -> bool {
        self.inner.replay.borrow().pending()
    }
    pub fn queue_key(
        &self,
        event: &gdk::EventKey,
        now: f64,
    ) -> Result<(), crate::ui::clipboard::ClipboardError> {
        let mut replay = self.inner.replay.borrow_mut();
        if replay.pending() {
            replay.push((event.clone(), now))
        } else {
            replay.begin((event.clone(), now))
        }
    }
    pub fn finish_keys(&self, context: bool) -> Vec<((gdk::EventKey, f64), bool)> {
        self.inner.replay.borrow_mut().finish(context)
    }
    pub fn forward_key(&self, event: &gdk::EventKey) -> glib::Propagation {
        if self.inner.closed.get() {
            return glib::Propagation::Stop;
        }
        self.inner.forward_key(event)
    }
    pub fn on_context_menu(&self, callback: ContextCallback) {
        *self.inner.context_menu.borrow_mut() = Some(callback);
    }
    pub fn show_finished(&self, message: &str) {
        self.shutdown();
        self.inner
            .model
            .borrow_mut()
            .engine
            .feed(format!("\r\n{message}\r\n").as_bytes(), Instant::now());
        self.inner.redraw();
        self.inner.area.set_tooltip_text(Some(message));
    }
    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.inner.area
    }
    /// Hide or show Claude Code's gray tool noise in this terminal.
    pub fn set_hide_noise(&self, on: bool) {
        let mut m = self.inner.model.borrow_mut();
        // Retired: cutting gray rows out of the picture tmux draws broke
        // split panes (text cut at the pane edge, stale history banners). It
        // returns as stable folding once every pane is drawn by the app.
        let _ = on;
        let on = false;
        if m.hide_noise != on {
            m.hide_noise = on;
            m.pick = None;
            m.scrolls.clear();
            drop(m);
            self.inner.redraw();
        }
    }
    /// The tmux panes inside this terminal (`left, width, top, height`), so
    /// each one is filtered on its own columns.
    pub fn set_regions(&self, regions: Vec<(String, u16, u16, u16, u16)>) {
        let (ids, regions): (Vec<_>, Vec<_>) = regions
            .into_iter()
            .map(|(id, left, width, top, height)| {
                (id, super::noise::Region { left, width, top, height })
            })
            .unzip();
        let mut m = self.inner.model.borrow_mut();
        if m.control.is_some() {
            // The whole view is the pane; tmux draws no borders in it.
            return;
        }
        if m.tmux_regions == regions && m.tmux_ids == ids {
            return;
        }
        if m.tmux_ids != ids {
            // Different panes: their history and any selection are gone.
            m.fills.clear();
            m.scrolls.clear();
            m.pick = None;
        }
        m.tmux_regions = regions;
        m.tmux_ids = ids;
        drop(m);
        self.inner.redraw();
    }
    /// Rows at the bottom that tmux uses for its status line.
    pub fn set_status_rows(&self, rows: u16) {
        self.inner.model.borrow_mut().status_rows = rows.min(2);
    }
    /// Where the filter reads tmux history to fill the rows it frees.
    pub fn set_history_source(&self, source: HistorySource) {
        *self.inner.history.borrow_mut() = Some(source);
    }
    /// Cell width and the horizontal widget pixels that are not cells.
    pub fn cell_layout(&self) -> (f64, f64) {
        let m = self.inner.model.borrow();
        (m.geom.cell_w, m.geom.origin_x + (m.padding - 4.).max(0.))
    }
    pub fn feed(&self, bytes: &[u8]) -> Result<(), PtyError> {
        self.inner.aim_keys();
        self.inner.send(bytes)
    }
    pub fn paste(&self, text: &str) -> Result<(), PtyError> {
        self.inner.aim_keys();
        let bytes = encode_paste(text, &self.inner.model.borrow().engine.modes());
        self.feed(&bytes)
    }
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.inner.resize(cols, rows)
    }
    /// Moves the view over its own history (positive: older lines).
    pub fn scroll_lines(&self, lines: i32) {
        self.inner.model.borrow_mut().glide += f64::from(lines);
        self.inner.glide();
    }
    /// True while a tmux control client feeds this view (pane-native).
    pub fn pane_native(&self) -> bool {
        self.inner.model.borrow().control.as_ref().is_some_and(super::control::Session::ready)
    }
    pub fn session(&self) -> Option<&str> {
        self.inner.options.session.as_deref()
    }
    pub fn child_tty(&self) -> Option<String> {
        self.inner
            .model
            .borrow()
            .pty
            .as_ref()
            .and_then(PtySession::child_tty)
    }
    pub fn selection_text(&self) -> Option<String> {
        self.inner.selection_text()
    }
    pub fn diagnostic_ready(&self) -> bool {
        let model = self.inner.model.borrow();
        (model.pty.is_some() || model.shadow_frame.is_some() || self.inner.closed.get())
            && model.engine.next_deadline().is_none()
    }
    pub fn diagnostic_snapshot(&self) -> Result<Value, String> {
        let model = self.inner.model.borrow();
        let cursor = model.engine.cursor();
        let shape = comandos_term::render::preferred_cursor_shape(
            cursor.shape,
            Some(model.cursor_shape),
            model.focused,
        );
        let blink = model
            .preferences
            .get("cursor_blink")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let font = crate::layout_dump::measured_font(self.widget().upcast_ref(), &model.font)?;
        Ok(
            serde_json::json!({"grid":model.engine.diagnostic_grid(100_000)?,"font":font,"font_scale":model.font_scale,"cell_metrics":{"width":model.geom.cell_w,"height":model.geom.cell_h,"origin_x":model.geom.origin_x,"origin_y":model.geom.origin_y,"dpr":model.geom.dpr,"font_size_px":model.geom.font_size},"opacity":model.opacity,"focused":model.focused,"closed":self.inner.closed.get(),"selection_text":self.inner.selection_text(),"preedit":model.preedit,"cursor":{"row":cursor.line,"col":cursor.col,"shape":format!("{shape:?}").to_lowercase(),"visible":cursor.visible,"painted":cursor.visible && (!blink || !model.focused || model.blink_visible),"wide":cursor.wide}}),
        )
    }
    pub fn font_scale(&self) -> f64 {
        self.inner.model.borrow().font_scale
    }
    pub fn set_font_scale(&self, scale: f64) {
        if scale.is_finite() {
            self.inner.model.borrow_mut().font_scale = scale.clamp(0.5, 3.);
            self.inner.configure_font();
            self.inner.allocate();
        }
    }
    pub fn set_preferences(&self, prefs: &Value) {
        self.inner.model.borrow_mut().preferences = prefs.clone();
        self.inner.configure_font();
        self.inner.allocate();
    }
    pub fn set_palette(&self, palette: Palette) {
        self.inner.model.borrow_mut().engine.set_palette(palette);
        self.inner.redraw();
    }
}
impl Inner {
    fn dispatch_app_key(&self, event: &gdk::EventKey) -> bool {
        if self.closed.get() {
            return false;
        }
        let callback = self.app_key.borrow().clone();
        callback.is_some_and(|callback| callback(event))
    }
    fn ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
    fn configure_font(&self) {
        let mut m = self.model.borrow_mut();
        let family = m
            .preferences
            .get("font_family")
            .and_then(Value::as_str)
            .unwrap_or("Ubuntu Sans Mono");
        let size = m
            .preferences
            .get("font_size")
            .and_then(Value::as_f64)
            .unwrap_or(13.)
            .clamp(6., 72.)
            * m.font_scale;
        let mut font = pango::FontDescription::new();
        font.set_family(&format!(
            "{family},JetBrainsMono Nerd Font,JetBrains Mono,DejaVu Sans Mono,Monospace"
        ));
        font.set_size((size * f64::from(pango::SCALE)).round() as i32);
        let layout = self.area.create_pango_layout(Some("M"));
        layout.set_font_description(Some(&font));
        let (width, height) = layout.pixel_size();
        let line_scale = 1.2; // cc-app TERM_LINE_SCALE, not a preference.
        m.font = font;
        self.painter.borrow_mut().take();
        m.geom.cell_w = f64::from(width.max(1));
        m.geom.cell_h = (f64::from(height.max(1)) * line_scale).ceil();
        m.geom.font_size = size * 96. / 72.;
        m.geom.dpr = f64::from(self.area.scale_factor().max(1));
        m.padding = m
            .preferences
            .get("terminal_padding")
            .and_then(Value::as_f64)
            .unwrap_or(8.)
            .clamp(0., 40.);
        m.base_x = m.padding + 2.;
        m.geom.origin_x = m.base_x;
        // A classic attach starts with tmux's border line, and the pane's
        // header is drawn over it. A pane-native view has no such line:
        // leave that row free so the header does not cover text.
        m.geom.origin_y = m.padding + 14.;
        if self.pane_native_wanted() {
            Self::place(&mut m);
        }
        m.opacity = m
            .preferences
            .get("terminal_opacity")
            .and_then(Value::as_f64)
            .unwrap_or(100.)
            .clamp(30., 100.)
            / 100.;
        m.cursor_shape = match m.preferences.get("cursor_shape").and_then(Value::as_str) {
            Some("ibeam") => CursorShape::Beam,
            Some("underline") => CursorShape::Underline,
            _ => CursorShape::Block,
        };
        self.redraw();
    }
    fn dimensions(&self) -> (u16, u16) {
        let m = self.model.borrow();
        initial_size(
            (self.area.allocated_width(), self.area.allocated_height()),
            m.geom.cell_w,
            m.geom.cell_h,
            if self.pane_native_wanted() {
                // The cells for panes: everything under the header row.
                (m.base_x + (m.padding - 4.).max(0.), m.padding + 14. + m.geom.cell_h + (m.padding - 2.).max(0.))
            } else {
                (m.geom.origin_x + (m.padding - 4.).max(0.), m.geom.origin_y + (m.padding - 2.).max(0.))
            },
            self.options.tmux_size,
        )
    }
    /// Repaints the whole terminal on the next frame. Output alone goes
    /// through `request_frame`, which repaints only the rows that changed.
    fn redraw(&self) {
        self.full.set(true);
        self.area.queue_draw();
    }
    fn request_frame(self: &Rc<Self>) {
        if self.frame_asked.replace(true) {
            return;
        }
        self.asked_at.set(Some(Instant::now()));
        // Runs before GTK's repaint but after every ready event source: tells
        // a busy main loop from a frame clock waiting for the compositor.
        self.free_at.set(None);
        let free = self.free_at.clone();
        glib::idle_add_local_full(glib::Priority::HIGH_IDLE, move || {
            if free.get().is_none() {
                free.set(Some(Instant::now()));
            }
            ControlFlow::Break
        });
        let weak = Rc::downgrade(self);
        self.area.add_tick_callback(move |_, _| {
            if let Some(inner) = weak.upgrade() {
                inner.ticked_at.set(Some(Instant::now()));
                inner.settle_output();
                inner.frame_asked.set(false);
                inner.plan_frame();
            }
            ControlFlow::Break
        });
    }
    /// A redraw reaches the app in pieces a fraction of a millisecond apart
    /// (tmux forwards the program's output 4 KB at a time). A frame that
    /// starts between two pieces would show half a redraw, so it waits for
    /// the PTY to go quiet: at most `SETTLE_OUTPUT_CAP`, and only when output
    /// arrived within the last `SETTLE_OUTPUT_QUIET`.
    fn settle_output(self: &Rc<Self>) {
        let started = Instant::now();
        while !self.closed.get() && self.area.is_mapped() {
            let Some(left) = self
                .read_at
                .get()
                .and_then(|at| SETTLE_OUTPUT_QUIET.checked_sub(at.elapsed()))
                .filter(|_| started.elapsed() < SETTLE_OUTPUT_CAP)
            else {
                return;
            };
            let more = self.model.borrow().pty.as_ref().is_some_and(|p| p.wait_readable(left));
            if !more {
                return;
            }
            self.read_at.set(Some(Instant::now()));
            if !self.read(IOCondition::IN) {
                return;
            }
        }
    }
    /// Keeps the current picture while tmux and the program redraw for `size`.
    fn hold_picture(&self, m: &mut Model, size: (u16, u16)) {
        let live = self.options.session.is_some() && m.pty.is_some() && self.area.is_mapped();
        if live && m.engine.size() != size && self.canvas.borrow().is_some() {
            let now = self.ms();
            m.hold = Some(Hold { since: now, cap_at: now.saturating_add(HOLD_CAP_MS), filling: false });
        }
    }
    fn release_hold(&self, now: u64) {
        let mut m = self.model.borrow_mut();
        let Some(hold) = m.hold else { return };
        if now < hold.due(m.output_ms) {
            return;
        }
        if !hold.filling && now < hold.cap_at && m.hide_noise && self.history.borrow().is_some() {
            // The new screen is read now and the history shown above the
            // filtered text is asked for before the picture changes, so it
            // does not pop in a moment later.
            m.resized_ms = now;
            self.prepare(&mut m, None);
            if !m.fill_wanted.is_empty() {
                m.hold = Some(Hold { filling: true, ..hold });
                return;
            }
        }
        self.end_hold(&mut m, now);
        drop(m);
        self.redraw();
    }
    fn end_hold(&self, m: &mut Model, now: u64) {
        if let Some(hold) = m.hold.take() {
            let (cols, rows) = m.engine.size();
            super::lag::note(&format!(
                "resize {} {cols}x{rows} held={}ms",
                self.options.session.as_deref().unwrap_or("-"),
                now.saturating_sub(hold.since)
            ));
        }
        m.resized_ms = now;
        m.output_at = None;
        m.schedule.reset_queued_paint();
    }
    /// Ends a hold that only waited for history once none is on its way.
    fn fill_arrived(&self) {
        let mut m = self.model.borrow_mut();
        if m.hold.is_some_and(|h| h.filling) && m.fill_busy.is_empty() {
            let now = self.ms();
            self.end_hold(&mut m, now);
            drop(m);
            self.redraw();
        }
    }
    fn hold_delay(&self, m: &Model, now: u64) -> Option<u64> {
        m.hold.map(|h| h.due(m.output_ms).saturating_sub(now))
    }
    fn allocate(self: &Rc<Self>) {
        let now = self.ms();
        let mut m = self.model.borrow_mut();
        m.settle.on_alloc(now);
        if m.pty.is_some() {
            // A hidden tab waits for its turn: when every tab tells tmux its
            // new size at once, every program redraws while the visible one
            // is still settling.
            let wait = if self.area.is_mapped() {
                RESIZE_QUIET_MS
            } else {
                HIDDEN_RESIZE_MS.saturating_add(self.turn.saturating_mul(HIDDEN_RESIZE_STEP_MS))
            };
            m.resize_at = Some(now.saturating_add(wait));
        }
        drop(m);
        self.arm();
    }
    /// A tab shown before its turn takes the new size now.
    fn resize_if_waiting(&self) {
        if self.model.borrow().resize_at.is_some() {
            self.resize_when_quiet(u64::MAX);
        }
    }
    fn resize_when_quiet(&self, now: u64) {
        let due = self.model.borrow().resize_at.is_some_and(|at| now >= at);
        if !due {
            return;
        }
        self.model.borrow_mut().resize_at = None;
        let (c, r) = self.dimensions();
        // Too small to read: keep the size the window already has and clip the
        // drawing, instead of shrinking the tmux window for every client.
        if self.options.session.is_some() && !fits_shared_window((c, r)) {
            self.redraw();
            return;
        }
        // Same grid, or a size the PTY refused: the picture kept while the
        // window moved is painted again for the new area.
        if self.model.borrow().engine.size() == (c, r) || self.resize(c, r).is_err() {
            self.redraw();
        }
    }
    /// Seeing a tmux tab again takes its window back from any other client (phone,
    /// another view) so the text wraps at this view's width without pressing a key.
    fn claim_size(&self) {
        if self.options.mode == RunMode::Shadow || self.options.session.is_none() {
            return;
        }
        let mut m = self.model.borrow_mut();
        let m = &mut *m;
        if let Some(pty) = &mut m.pty {
            let (cols, rows) = m.engine.size();
            if !fits_shared_window((cols, rows)) {
                return;
            }
            if let Some(control) = &mut m.control {
                let mut wire = Vec::new();
                control.claim(&mut wire);
                let _ = pty.write(&wire);
            } else {
                let _ = pty.claim_size(cols, rows);
            }
        }
    }
    fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        if self.options.mode == RunMode::Shadow {
            return Ok(());
        }
        let mut m = self.model.borrow_mut();
        let m = &mut *m;
        let wire = Self::native_resize(m, cols, rows);
        // Pane-native engines take their sizes from tmux's layout.
        let native = wire.is_some();
        match (&mut m.pty, wire) {
            (Some(p), Some(wire)) => p.write(&wire)?,
            (Some(p), None) => p.resize(cols, rows)?,
            _ => {}
        }
        let px = (
            m.geom.cell_w.round().clamp(1., f64::from(u16::MAX)) as u16,
            m.geom.cell_h.round().clamp(1., f64::from(u16::MAX)) as u16,
        );
        if !native {
            self.hold_picture(m, (cols, rows));
            m.engine.resize(cols, rows, px);
        }
        self.redraw();
        Ok(())
    }
    /// Moves the view over its history a part of the way each frame: fast at
    /// first, slowing as it arrives, and by fractions of a line in between.
    fn glide(self: &Rc<Self>) {
        if self.gliding.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        self.area.add_tick_callback(move |_, _| {
            let Some(inner) = weak.upgrade().filter(|i| !i.closed.get()) else {
                return ControlFlow::Break;
            };
            let mut m = inner.model.borrow_mut();
            let left = m.glide;
            // The last stretch lands the view exactly on a line.
            let step = if left.abs() < 0.06 { left } else { left * 0.3 };
            m.glide -= step;
            let total = m.glide_frac + step;
            let whole = total.floor();
            m.glide_frac = total - whole;
            let done = m.glide == 0.;
            if done {
                // Rounding left a hair of a line: settle on the grid.
                m.glide_frac = if m.glide_frac > 0.5 { 1. } else { 0. };
            }
            let mut lines = whole as i32;
            if m.glide_frac >= 1. {
                m.glide_frac = 0.;
                lines += 1;
            }
            let moved = if lines == 0 {
                true
            } else {
                let before = m.engine.engine().display_offset();
                m.engine.scroll(lines);
                m.engine.engine().display_offset() != before
            };
            if !moved {
                // The first or the last line: nothing further that way.
                (m.glide, m.glide_frac) = (0., 0.);
            }
            let end = m.glide == 0. && m.glide_frac == 0.;
            drop(m);
            if lines != 0 || end {
                inner.redraw();
            } else {
                inner.area.queue_draw();
            }
            if end {
                inner.gliding.set(false);
                ControlFlow::Break
            } else {
                ControlFlow::Continue
            }
        });
    }
    fn pane_native_wanted(&self) -> bool {
        self.options.control_argv.is_some() && !self.classic.get()
    }
    /// The control-mode command line, unless this view fell back to classic.
    fn control_argv(&self) -> Option<&Vec<String>> {
        self.options.control_argv.as_ref().filter(|_| !self.classic.get())
    }
    fn fit_header(&self) {
        let mut m = self.model.borrow_mut();
        m.geom.origin_y = m.padding + 14.;
        if self.pane_native_wanted() {
            Self::place(&mut m);
        }
    }
    /// Puts the cell grid where the loaded pane sits in the window. Row 0 of
    /// the window is the border line the pane header is drawn over, so a
    /// pane starts one row down (and that row stays free above its text).
    fn place(m: &mut Model) {
        let (left, top) = m.loaded.map_or((0, 1), |p| (p.left, p.top));
        m.geom.origin_x = m.base_x + f64::from(left) * m.geom.cell_w;
        m.geom.origin_y = m.padding + 14. + f64::from(top) * m.geom.cell_h;
    }
    /// Starts (or clears) the control session of a freshly spawned client.
    fn begin_native(model: &mut Model, on: bool) {
        model.control = on.then(super::control::Session::default);
        model.slots.clear();
        model.loaded = None;
        model.key_pane = None;
        model.stale.clear();
        model.scroll_seen = model.engine.engine().scrolled_up();
        if on {
            model.hide_noise = false;
            model.tmux_regions.clear();
            model.tmux_ids.clear();
            model.fills.clear();
            model.scrolls.clear();
            model.pick = None;
        }
    }
    /// Typed bytes as control commands (`None`: a classic client takes them raw).
    fn native_input(m: &mut Model, bytes: &[u8]) -> Option<Vec<u8>> {
        let pane = m.loaded.map(|p| p.id);
        let control = m.control.as_mut()?;
        let mut wire = Vec::new();
        control.input(pane, bytes, &mut wire);
        // Typing goes to the live screen, like in any terminal. Focus and
        // mouse reports are not typing: the view stays where it was read.
        let report = [&b"\x1b[I"[..], b"\x1b[O", b"\x1b[<", b"\x1b[M"].iter().any(|p| bytes.starts_with(p));
        if !report {
            m.engine.scroll(i32::MIN / 2);
            (m.glide, m.glide_frac) = (0., 0.);
        }
        Some(wire)
    }
    /// The new size as a control command: tmux sizes the window from what
    /// the client says, not from the PTY the control stream travels on.
    fn native_resize(m: &mut Model, cols: u16, rows: u16) -> Option<Vec<u8>> {
        let control = m.control.as_mut()?;
        let mut wire = Vec::new();
        control.resize(cols, rows, &mut wire);
        Some(wire)
    }
    /// Makes `id` the pane the model's own fields belong to.
    fn load_pane(m: &mut Model, id: u32) -> bool {
        if m.loaded.is_some_and(|p| p.id == id) {
            return true;
        }
        let Some(i) = m.slots.iter().position(|s| s.info.id == id) else {
            return false;
        };
        let Some(out) = m.loaded else { return false };
        let Some(slot) = m.slots.get_mut(i) else { return false };
        std::mem::swap(&mut m.engine, &mut slot.engine);
        std::mem::swap(&mut m.frame, &mut slot.frame);
        std::mem::swap(&mut m.selection, &mut slot.selection);
        std::mem::swap(&mut m.scroll_seen, &mut slot.scroll_seen);
        m.loaded = Some(std::mem::replace(&mut slot.info, out));
        (m.glide, m.glide_frac, slot.glide) = (slot.glide.0, slot.glide.1, (m.glide, m.glide_frac));
        m.painted = None;
        m.painted_cursor = None;
        Self::place(m);
        true
    }
    /// A cell of the loaded pane as a cell of the tmux window: what the app
    /// asks tmux about (which pane was clicked, what is under a link).
    fn window_cell(&self, (col, row): (u16, u16)) -> (u16, u16) {
        match self.model.borrow().loaded {
            Some(p) => (col.saturating_add(p.left), row.saturating_add(p.top)),
            None => (col, row),
        }
    }
    /// The pane with the keys takes the model before a key is encoded.
    fn aim_keys(&self) {
        let mut m = self.model.borrow_mut();
        if let Some(id) = m.key_pane.filter(|_| !m.slots.is_empty()) {
            Self::load_pane(&mut m, id);
        }
    }
    /// The pane under the pointer takes the model; a click also gives it the keys.
    fn aim_at(self: &Rc<Self>, x: f64, y: f64, click: bool) {
        let mut m = self.model.borrow_mut();
        if m.control.is_none() || m.loaded.is_none() {
            return;
        }
        let col = ((x - m.base_x) / m.geom.cell_w).floor();
        let row = ((y - (m.padding + 14.)) / m.geom.cell_h).floor();
        // The row above a pane is its header: it belongs to that pane too.
        let holds = |p: &super::control::PaneInfo| {
            col >= f64::from(p.left) - 1.
                && col <= f64::from(p.left) + f64::from(p.width)
                && row >= f64::from(p.top) - 1.
                && row < f64::from(p.top) + f64::from(p.height)
        };
        let under = m
            .loaded
            .filter(|p| holds(p))
            .or_else(|| m.slots.iter().map(|s| s.info).find(|p| holds(p)));
        let Some(pane) = under else { return };
        if m.loaded.is_some_and(|p| p.id != pane.id) {
            // What the pointer had started belongs to the pane it left.
            m.pressed = None;
            m.hovered_link = None;
            m.last_motion = None;
            Self::load_pane(&mut m, pane.id);
        }
        if click && m.key_pane != Some(pane.id) {
            m.key_pane = Some(pane.id);
            {
                let m = &mut *m;
                if let (Some(control), Some(pty)) = (&mut m.control, &mut m.pty) {
                    let mut wire = Vec::new();
                    control.focus(pane.id, &mut wire);
                    let _ = pty.write(&wire);
                }
            }
            drop(m);
            self.watch_write();
            self.redraw();
        }
    }
    /// Reads the control stream and applies what it says to the panes.
    fn native_read(&self, m: &mut Model) -> DrainOutcome {
        let (mut wire, mut actions) = (Vec::new(), Vec::new());
        let outcome = {
            let Model { pty, drain, control, .. } = &mut *m;
            match (pty.as_mut(), control.as_mut()) {
                (Some(pty), Some(control)) => {
                    let outcome = drain.read_into(pty, |bytes| control.read(bytes, &mut wire, &mut actions));
                    if !wire.is_empty() {
                        let _ = pty.write(&wire);
                    }
                    outcome
                }
                _ => DrainOutcome::Closed,
            }
        };
        let now = Instant::now();
        for action in actions {
            match action {
                super::control::Action::Layout(panes) => {
                    m.fed_all = true;
                    self.layout_panes(m, &panes);
                }
                super::control::Action::Feed { pane, bytes } => {
                    if !m.fed.contains(&pane) {
                        m.fed.push(pane);
                    }
                    if m.loaded.is_some_and(|p| p.id == pane) {
                        m.engine.feed(&bytes, now);
                    } else if let Some(slot) = m.slots.iter_mut().find(|s| s.info.id == pane) {
                        slot.engine.feed(&bytes, now);
                    }
                    // Compared with tmux once the pane goes quiet, and every
                    // few seconds while it does not.
                    let ms = self.ms();
                    let from = *m.audit_from.get_or_insert(ms);
                    m.audit_at = Some(ms.saturating_add(AUDIT_QUIET_MS).min(from.saturating_add(AUDIT_BUSY_MS)));
                }
                super::control::Action::Seed { pane, size, bytes } => {
                    if !m.fed.contains(&pane) {
                        m.fed.push(pane);
                    }
                    self.native_seed(m, pane, size, &bytes, now);
                }
                super::control::Action::Check { pane, size, cursor, rows } => {
                    self.native_check(m, pane, size, cursor, &rows);
                }
                super::control::Action::Notice(bytes) => {
                    m.fed_all = true;
                    m.engine.feed(&bytes, now);
                }
                super::control::Action::Exited => {
                    m.fed_all = true;
                    m.engine.feed(b"\x1b[0m\r\n", now);
                }
            }
        }
        outcome
    }
    /// A pane that held its picture (synchronized output) shows it now.
    fn tick_panes(&self, m: &mut Model) {
        for slot in &mut m.slots {
            if slot.engine.tick(Instant::now()) {
                self.full.set(true);
            }
        }
        if m.audit_at.is_some_and(|at| self.ms() >= at) {
            (m.audit_at, m.audit_from) = (None, None);
            Self::native_command(m, |control, wire| control.check(wire));
        }
    }
    /// While keys or text arrive the cursor stays lit: it only blinks once
    /// the terminal has been quiet for a moment, so it is never missing
    /// from the place being typed in.
    fn steady_cursor(&self, m: &mut Model) -> bool {
        let now = self.ms();
        m.schedule.blinked(now);
        if m.blink_visible {
            return false;
        }
        // It was in the dark half of a blink: its row is painted again.
        m.blink_visible = true;
        let row = m.frame.cursor().line;
        m.schedule.damage(now, &[row]);
        if !m.slots.is_empty()
            && let Some(id) = m.loaded.map(|p| p.id)
            && !m.stale.contains(&id)
        {
            m.stale.push(id);
        }
        true
    }
    /// Sends what a control session wants to say.
    fn native_command(m: &mut Model, say: impl FnOnce(&mut super::control::Session, &mut Vec<u8>)) {
        let Model { pty, control, .. } = m;
        if let (Some(pty), Some(control)) = (pty.as_mut(), control.as_mut()) {
            let mut wire = Vec::new();
            say(control, &mut wire);
            if !wire.is_empty() {
                let _ = pty.write(&wire);
            }
        }
    }
    /// A pane's engine is filled with exactly what tmux holds: on attach,
    /// when tmux gave the pane another size, or to repair it.
    fn native_seed(&self, m: &mut Model, pane: u32, size: (u16, u16), bytes: &[u8], now: Instant) {
        let px = (
            m.geom.cell_w.round().clamp(1., f64::from(u16::MAX)) as u16,
            m.geom.cell_h.round().clamp(1., f64::from(u16::MAX)) as u16,
        );
        let loaded = m.loaded.is_some_and(|p| p.id == pane);
        let (engine, selection, seen) = if loaded {
            (&mut m.engine, &mut m.selection, &mut m.scroll_seen)
        } else if let Some(slot) = m.slots.iter_mut().find(|s| s.info.id == pane) {
            (&mut slot.engine, &mut slot.selection, &mut slot.scroll_seen)
        } else {
            return;
        };
        // Whoever was reading back stays that far back.
        let back = engine.engine().display_offset();
        if engine.size() != size {
            engine.resize(size.0, size.1, px);
        }
        engine.feed(bytes, now);
        if back > 0 {
            engine.scroll(i32::try_from(back).unwrap_or(i32::MAX));
        }
        *selection = None;
        *seen = engine.engine().scrolled_up();
        if loaded {
            m.painted = None;
            m.painted_cursor = None;
        }
        self.full.set(true);
        // And it is compared right away: a seed is checked like any output.
        m.audit_at = Some(self.ms());
    }
    /// Compares a pane's engine with what tmux says the pane shows. tmux is
    /// the program's real terminal, so when they differ the engine is filled
    /// again from tmux (and the difference is noted in the lag log).
    fn native_check(&self, m: &mut Model, pane: u32, size: (u16, u16), cursor: (u16, u16), rows: &[Vec<u8>]) {
        let loaded = m.loaded.is_some_and(|p| p.id == pane);
        let (engine, selecting) = if loaded {
            (&m.engine, m.selection.is_some() || m.pressed.is_some())
        } else if let Some(slot) = m.slots.iter().find(|s| s.info.id == pane) {
            (&slot.engine, slot.selection.is_some())
        } else {
            return;
        };
        let later = Some(self.ms().saturating_add(AUDIT_BUSY_MS));
        // Mid-resize, or mid-update (a synchronized update is not on the
        // grid until the program ends it): compared the next time.
        if engine.size() != size || engine.next_deadline().is_some() {
            m.audit_at = m.audit_at.or(later);
            return;
        }
        let (ours, at) = engine.live_screen();
        let row = super::control::first_difference(&ours, rows);
        let times = m.audit_wrong.iter().find(|(id, _)| *id == pane).map_or(0, |(_, times)| *times);
        if row.is_none() && at == cursor {
            m.audit_wrong.retain(|(id, _)| *id != pane);
            if std::env::var_os("COMANDOS_AUDIT_TRACE").is_some() {
                super::lag::note(&format!(
                    "audit same {} %{pane} {}x{}",
                    self.options.session.as_deref().unwrap_or("-"),
                    size.0,
                    size.1
                ));
            }
            return;
        }
        // A pane the seed itself cannot reproduce is left as it is.
        if times >= AUDIT_GIVE_UP {
            return;
        }
        // Nobody's reading or selection is pulled from under them.
        if engine.engine().display_offset() > 0 || selecting {
            m.audit_at = m.audit_at.or(later);
            return;
        }
        let cut = |text: &str| text.chars().take(200).collect::<String>();
        let shown = row.map_or_else(String::new, |r| {
            let theirs = rows.get(r).map(|t| String::from_utf8_lossy(t).into_owned()).unwrap_or_default();
            format!(" row {r} ours {:?} tmux {:?}", cut(ours.get(r).map_or("", String::as_str)), cut(&theirs))
        });
        super::lag::note(&format!(
            "drift {} %{pane} {}x{} try {} cursor ours {at:?} tmux {cursor:?}{shown}",
            self.options.session.as_deref().unwrap_or("-"),
            size.0,
            size.1,
            times + 1,
        ));
        m.audit_wrong.retain(|(id, _)| *id != pane);
        m.audit_wrong.push((pane, times + 1));
        Self::native_command(m, |control, wire| control.reseed(pane, wire));
    }
    /// A selection stays on its text: the lines that left through the top
    /// of the screen took it up with them, as far as they went.
    fn follow_scroll(m: &mut Model) {
        fn moved(engine: &TermEngine, seen: &mut u64) -> i32 {
            let now = engine.engine().scrolled_up();
            i32::try_from(now.wrapping_sub(std::mem::replace(seen, now))).unwrap_or(0)
        }
        let lift = |selection: &mut Option<Selection>, up: i32| {
            if let Some(s) = selection {
                s.anchor.0 = s.anchor.0.saturating_sub(up);
                s.head.0 = s.head.0.saturating_sub(up);
            }
        };
        let up = moved(&m.engine, &mut m.scroll_seen);
        if up > 0 {
            lift(&mut m.selection, up);
            if let Some(pressed) = &mut m.pressed {
                pressed.0 = pressed.0.saturating_sub(up);
            }
        }
        for slot in &mut m.slots {
            let up = moved(&slot.engine, &mut slot.scroll_seen);
            if up > 0 {
                lift(&mut slot.selection, up);
            }
        }
    }
    /// Notes which panes changed, so only those are painted again.
    fn native_damage(&self, m: &mut Model, lines: &mut Vec<usize>) {
        Self::follow_scroll(m);
        // An engine names its cursor row as changed every time it is asked.
        // A read that brought a pane no text (tmux answering a command, or
        // text for another pane) must not paint it: for the pane being typed
        // in, that frame is the one the program's echo would have used.
        let fed = std::mem::take(&mut m.fed);
        let all = std::mem::take(&mut m.fed_all) || m.loaded.is_none();
        let got = |id: u32| all || fed.contains(&id);
        if lines.len() <= 1 && !m.loaded.is_none_or(|p| got(p.id)) {
            lines.clear();
        }
        if !lines.is_empty() {
            let _ = self.steady_cursor(m);
        }
        if m.slots.is_empty() {
            return;
        }
        if !lines.is_empty()
            && let Some(id) = m.loaded.map(|p| p.id)
            && !m.stale.contains(&id)
        {
            m.stale.push(id);
        }
        let mut scratch = Vec::new();
        let Model { slots, stale, .. } = &mut *m;
        for slot in slots.iter_mut().filter(|s| got(s.info.id)) {
            let _ = slot.engine.drain();
            scratch.clear();
            if slot.engine.take_dirty(&mut scratch) || !scratch.is_empty() {
                if !stale.contains(&slot.info.id) {
                    stale.push(slot.info.id);
                }
                if lines.is_empty() {
                    lines.push(0);
                }
            }
        }
    }
    /// The window's panes changed: one engine each, at the size tmux gave it.
    fn layout_panes(&self, m: &mut Model, panes: &[super::control::PaneInfo]) {
        let px = (
            m.geom.cell_w.round().clamp(1., f64::from(u16::MAX)) as u16,
            m.geom.cell_h.round().clamp(1., f64::from(u16::MAX)) as u16,
        );
        m.slots.retain(|s| panes.iter().any(|p| p.id == s.info.id));
        if m.loaded.is_some_and(|l| !panes.iter().any(|p| p.id == l.id)) {
            // Its pane is gone: another one takes the model's engine.
            match m.slots.pop() {
                Some(slot) => {
                    m.engine = slot.engine;
                    m.frame = slot.frame;
                    m.selection = slot.selection;
                    m.scroll_seen = slot.scroll_seen;
                    m.loaded = Some(slot.info);
                }
                None => {
                    m.loaded = None;
                    m.selection = None;
                    m.engine.feed(b"\x1bc", Instant::now());
                }
            }
        }
        for pane in panes {
            let size = (pane.width, pane.height);
            if m.loaded.is_some_and(|l| l.id == pane.id) {
                m.loaded = Some(*pane);
                if m.engine.size() != size {
                    m.engine.resize(size.0, size.1, px);
                }
            } else if let Some(slot) = m.slots.iter_mut().find(|s| s.info.id == pane.id) {
                slot.info = *pane;
                if slot.engine.size() != size {
                    slot.engine.resize(size.0, size.1, px);
                }
            } else if m.loaded.is_none() {
                m.loaded = Some(*pane);
                m.engine.resize(size.0, size.1, px);
            } else {
                m.slots.push(PaneSlot {
                    info: *pane,
                    engine: TermEngine::new(
                        size.0,
                        size.1,
                        self.options.scrollback,
                        m.engine.palette().clone(),
                        true,
                        self.epoch,
                    ),
                    frame: FrameCache::default(),
                    selection: None,
                    glide: (0., 0.),
                    scroll_seen: 0,
                });
            }
        }
        m.key_pane = panes.iter().find(|p| p.active).map(|p| p.id).or(m.key_pane);
        m.window_cells = (
            panes.iter().map(|p| p.left + p.width).max().unwrap_or(0),
            panes.iter().map(|p| p.top + p.height).max().unwrap_or(0),
        );
        m.painted = None;
        m.painted_cursor = None;
        m.stale.clear();
        Self::place(m);
        self.full.set(true);
        self.area.queue_draw();
    }
    fn spawn(self: &Rc<Self>) -> Result<(), PtyError> {
        if self.options.mode == RunMode::Shadow {
            return Err(PtyError::Spawn("Shadow cannot attach a PTY".into()));
        }
        if let Some(before) = &self.options.before_spawn {
            before().map_err(PtyError::Spawn)?;
        }
        self.fit_header();
        let (mut cols, mut rows) = self.dimensions();
        if self.options.session.is_some() && !fits_shared_window((cols, rows)) {
            (cols, rows) = self.options.tmux_size.unwrap_or((80, 24));
        }
        let control = self.control_argv();
        let p = PtySession::spawn_with_env(
            control.unwrap_or(&self.options.argv),
            cols,
            rows,
            &self.options.cwd,
            &self.options.environment,
            self.options.clear_env,
        )?;
        let fd = p.raw_fd();
        {
            let mut model = self.model.borrow_mut();
            model.pty = Some(p);
            model.drain = PtyDrain::default();
            Self::begin_native(&mut model, control.is_some());
        }
        self.resize(cols, rows)?;
        self.watch_read(fd);
        Ok(())
    }
    /// Reads the PTY when it has output. A tab on screen is read ahead of
    /// GTK's repaint; a hidden one only when the main loop has nothing else
    /// to do, so the output of every other tab cannot delay the visible one
    /// (tmux keeps what a slow client has not read).
    fn watch_read(self: &Rc<Self>, fd: i32) {
        if let Some(source) = self.read_source.borrow_mut().take() {
            source.remove();
        }
        let priority = if self.area.is_mapped() {
            glib::Priority::DEFAULT
        } else {
            glib::Priority::DEFAULT_IDLE
        };
        let weak = glib::thread_guard::ThreadGuard::new(Rc::downgrade(self));
        let source = glib::source::unix_fd_source_new(
            fd,
            IOCondition::IN | IOCondition::HUP | IOCondition::ERR,
            None,
            priority,
            move |_, condition| {
                let Some(inner) = weak.get_ref().upgrade() else {
                    return ControlFlow::Break;
                };
                inner.read_at.set(Some(Instant::now()));
                if inner.read(condition) {
                    ControlFlow::Continue
                } else {
                    inner.read_source.borrow_mut().take();
                    ControlFlow::Break
                }
            },
        );
        *self.read_source.borrow_mut() = Some(source.attach(None));
    }
    /// Moves the PTY read to the priority of the tab's new visibility.
    fn rewatch_read(self: &Rc<Self>) {
        if self.read_source.borrow().is_none() {
            return;
        }
        let fd = self.model.borrow().pty.as_ref().map(PtySession::raw_fd);
        if let Some(fd) = fd {
            self.watch_read(fd);
        }
    }
    fn send(self: &Rc<Self>, bytes: &[u8]) -> Result<(), PtyError> {
        if self.options.mode == RunMode::Shadow || self.closed.get() {
            return Err(PtyError::Io("terminal is read-only or closed".into()));
        }
        let mut guard = self.model.borrow_mut();
        let m = &mut *guard;
        if m.pty.is_none() {
            return Err(PtyError::Io("PTY not attached".into()));
        }
        let wire = Self::native_input(m, bytes);
        if let Some(p) = m.pty.as_mut() {
            p.write(wire.as_deref().unwrap_or(bytes))?;
        }
        drop(guard);
        self.watch_write();
        Ok(())
    }
    fn send_event(self: &Rc<Self>, bytes: &[u8]) {
        if self.options.mode == RunMode::Shadow || self.closed.get() {
            return;
        }
        if let Err(error) = self.send(bytes) {
            let message = format!("terminal input: {error:?}");
            if let Some(callback) = &self.options.on_title {
                callback(&message);
            } else {
                eprintln!("{message}");
            }
        }
    }
    fn watch_write(self: &Rc<Self>) {
        if self.write_source.borrow().is_some() {
            return;
        }
        let fd = self
            .model
            .borrow()
            .pty
            .as_ref()
            .filter(|p| p.has_pending())
            .map(PtySession::raw_fd);
        let Some(fd) = fd else {
            return;
        };
        let weak = Rc::downgrade(self);
        *self.write_source.borrow_mut() = Some(glib::source::unix_fd_add_local(
            fd,
            IOCondition::OUT | IOCondition::HUP | IOCondition::ERR,
            move |_, condition| {
                let Some(inner) = weak.upgrade() else {
                    return ControlFlow::Break;
                };
                let keep = if condition.intersects(IOCondition::HUP | IOCondition::ERR) {
                    false
                } else {
                    inner
                        .model
                        .borrow_mut()
                        .pty
                        .as_mut()
                        .is_some_and(|p| matches!(p.flush_pending(), Ok(false)))
                };
                if keep {
                    ControlFlow::Continue
                } else {
                    inner.write_source.borrow_mut().take();
                    ControlFlow::Break
                }
            },
        ));
    }
    fn read(self: &Rc<Self>, condition: IOCondition) -> bool {
        let mut m = self.model.borrow_mut();
        let native = m.control.is_some();
        let outcome = if native {
            self.native_read(&mut m)
        } else {
            let Model { pty, engine, drain, .. } = &mut *m;
            pty.as_mut().map_or(DrainOutcome::Closed, |pty| {
                drain.read_into(pty, |bytes| engine.feed(bytes, Instant::now()))
            })
        };
        let closed = outcome == DrainOutcome::Closed
            || (outcome == DrainOutcome::WouldBlock
                && condition.intersects(IOCondition::HUP | IOCondition::ERR));
        if closed {
            m.drain.close();
        }
        let drained = m.engine.drain();
        let mut lines = Vec::new();
        if m.engine.take_dirty(&mut lines) {
            lines.extend(0..usize::from(m.engine.size().1));
        }
        if native {
            self.native_damage(&mut m, &mut lines);
        }
        m.schedule.damage(self.ms(), &lines);
        if !lines.is_empty() {
            if let Some(key) = m.key_at.take() {
                let echo = super::lag::ms(key);
                if echo > super::lag::SLOW_ECHO_MS {
                    super::lag::note(&format!(
                        "echo {echo:.0}ms {}",
                        self.options.session.as_deref().unwrap_or("-")
                    ));
                }
            }
            m.output_ms = self.ms();
            if self.area.is_mapped() && m.output_at.is_none() {
                m.output_at = Some(Instant::now());
            }
            m.hovered_link = None;
            m.last_motion = None;
        }
        drop(m);
        if native {
            // Queued commands (a size, the first queries) leave now.
            self.watch_write();
        }
        // In control mode tmux is the program's terminal and has already
        // answered its queries; answering again would type garbage into it.
        if !drained.replies.is_empty() && !native {
            self.send_event(&drained.replies);
        }
        if let Some(title) = drained.title
            && let Some(callback) = &self.options.on_title
        {
            callback(&title);
        }
        if drained.bell
            && let Some(callback) = &self.options.on_bell
        {
            callback();
        }
        if let Some(text) = drained.clipboard {
            let target = if drained.clipboard_target == Some(ClipboardTarget::Selection) {
                crate::ui::clipboard::Target::Primary
            } else {
                crate::ui::clipboard::Target::Clipboard
            };
            self.clipboard.osc52(target, &text);
        }
        self.arm();
        !closed
    }
    fn arm(self: &Rc<Self>) {
        if self.closed.get() {
            return;
        }
        // A hidden tab paints nothing: its output must not replace the timer
        // on every read. The one pending (100 ms at most) does the rest.
        if !self.area.is_mapped() && self.timer.borrow().is_some() {
            return;
        }
        if let Some(id) = self.timer.borrow_mut().take() {
            id.remove();
        }
        let now = self.ms();
        let mut m = self.model.borrow_mut();
        // Request this GTK frame directly: rearming the maintenance timeout on
        // every PTY read can otherwise postpone painting under continuous output.
        if m.schedule.queue_due_paint(now, self.area.is_mapped()) {
            self.request_frame();
        }
        // fd readiness drives IO; the low-rate fallback observes exit status.
        let mut delay = if m.drain.has_exited() && m.drain.finished().is_none() {
            1
        } else {
            100
        };
        if let Some(paint) = m
            .schedule
            .next_delay_ms_if_mapped(now, self.area.is_mapped())
        {
            delay = delay.min(paint);
        }
        if self.options.mode != RunMode::Shadow && m.pty.is_none() {
            let at = m.respawn_at.or_else(|| m.settle.next_check_ms(now));
            if let Some(at) = at {
                delay = delay.min(at.saturating_sub(now));
            }
        }
        if let Some(at) = m.ssh_at {
            delay = delay.min(at.saturating_sub(now));
        }
        if let Some(at) = m.resize_at {
            delay = delay.min(at.saturating_sub(now));
        }
        if let Some(left) = self.hold_delay(&m, now) {
            delay = delay.min(left);
        }
        if let Some(at) = m.engine.next_deadline() {
            let ms = u64::try_from(at.saturating_duration_since(self.epoch).as_millis())
                .unwrap_or(u64::MAX);
            delay = delay.min(ms.saturating_sub(now));
        }
        drop(m);
        let weak = Rc::downgrade(self);
        *self.timer.borrow_mut() = Some(glib::timeout_add_local(
            Duration::from_millis(delay.max(1)),
            move || {
                if let Some(inner) = weak.upgrade() {
                    inner.timer.borrow_mut().take();
                    if !inner.closed.get() {
                        inner.tick();
                        inner.arm();
                    }
                }
                ControlFlow::Break
            },
        ));
    }
    fn tick(self: &Rc<Self>) {
        let now = self.ms();
        self.resize_when_quiet(now);
        self.release_hold(now);
        self.fetch_history(now);
        self.fill_arrived();
        let mut m = self.model.borrow_mut();
        let spawn = self.options.mode != RunMode::Shadow
            && m.respawn.may_start()
            && m.pty.is_none()
            && (m.respawn_at.is_some_and(|at| now >= at)
                || (m.respawn_at.is_none() && m.settle.due(now)));
        if spawn {
            m.settle.fired();
            m.respawn_at = None;
            drop(m);
            if let Err(error) = self.spawn() {
                if let Some(callback) = &self.options.on_title {
                    callback(&format!("terminal: {error:?}"));
                }
                self.model.borrow_mut().respawn_at = Some(now.saturating_add(RESPAWN_MS));
            }
            m = self.model.borrow_mut();
        }
        {
            let Model { pty, drain, .. } = &mut *m;
            if let Some(pty) = pty.as_mut() {
                drain.observe_exit(pty);
            }
        }
        if m.drain.has_exited() && m.drain.finished().is_none() {
            drop(m);
            if !self.read(IOCondition::empty())
                && let Some(source) = self.read_source.borrow_mut().take()
            {
                source.remove();
            }
            m = self.model.borrow_mut();
        }
        let exit = m.drain.finished();
        if let Some(code) = exit {
            m.pty.take();
            m.drain = PtyDrain::default();
            m.respawn_at = m.respawn.exited().then(|| now.saturating_add(RESPAWN_MS));
            drop(m);
            if let Some(source) = self.read_source.borrow_mut().take() {
                source.remove();
            }
            if let Some(source) = self.write_source.borrow_mut().take() {
                source.remove();
            }
            if let Some(callback) = &self.options.on_exit {
                callback(code);
            }
            m = self.model.borrow_mut();
        }
        self.poll_shadow(&mut m, now);
        // Damage queries always include Alacritty's cursor row. Only consume
        // timer damage when DEC2026 actually flushes; PTY bytes use read().
        self.tick_panes(&mut m);
        if m.engine.tick(Instant::now()) {
            let mut lines = Vec::new();
            if m.engine.take_dirty(&mut lines) {
                lines.extend(0..usize::from(m.engine.size().1));
            }
            m.schedule.damage(now, &lines);
        }
        let blink = m
            .preferences
            .get("cursor_blink")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let synchronized = m.engine.next_deadline().is_some();
        m.schedule.set_synchronized(synchronized);
        let mapped = self.area.is_mapped();
        if mapped && blink && m.schedule.blink_due(now) {
            m.blink_visible = !m.blink_visible;
            m.schedule.blinked(now);
            self.request_frame();
        }
        if m.schedule.queue_due_paint(now, mapped) {
            self.request_frame();
        }
        let ssh = if m.ssh_at.is_some_and(|at| now >= at) {
            m.ssh_at = None;
            let delta = m.ssh_delta;
            m.ssh_delta = 0.;
            let mut n = delta.round_ties_even() as i32;
            if n == 0 && delta.abs() >= 0.25 {
                n = if delta > 0. { 1 } else { -1 };
            }
            Some((n, m.ssh_point))
        } else {
            None
        };
        drop(m);
        if let Some((delta, (col, row))) = ssh
            && delta != 0
            && let (Some(callback), Some(session)) =
                (&self.options.on_ssh_scroll, self.options.session.as_deref())
        {
            callback(session, delta, col, row);
        }
    }
    fn poll_shadow(&self, m: &mut Model, now: u64) {
        if self.options.mode == RunMode::Shadow {
            let mapped = self.area.is_mapped();
            let selecting = m.selection.is_some();
            let frame = if let Some(reader) = &mut m.shadow {
                reader.set_visible(mapped && !selecting);
                reader.take()
            } else {
                None
            };
            if let Some(Err(_)) = &frame {
                self.area.set_tooltip_text(Some(
                    "Read-only snapshot unavailable; showing the last captured frame",
                ));
            }
            if let Some(Ok(frame)) = frame {
                self.area.set_tooltip_text(Some("Read-only snapshot"));
                if m.shadow_frame.as_ref() != Some(&frame) {
                    let px = (
                        m.geom.cell_w.round().max(1.) as u16,
                        m.geom.cell_h.round().max(1.) as u16,
                    );
                    m.engine.resize(frame.cols, frame.rows, px);
                    m.engine.feed(&frame.bytes, Instant::now());
                    // A captured audit frame never sends engine replies or OSC callbacks.
                    m.engine.drain();
                    let mut lines = Vec::new();
                    if m.engine.take_dirty(&mut lines) {
                        lines.extend(0..usize::from(m.engine.size().1));
                    }
                    m.schedule.damage(now, &lines);
                    m.shadow_frame = Some(frame);
                }
            }
            if now >= m.shadow_at {
                if let Some(reader) = &mut m.shadow {
                    reader.request();
                }
                m.shadow_at = now.saturating_add(super::shadow::REFRESH_MS);
            }
        }
    }
    fn forward_key(self: &Rc<Self>, event: &gdk::EventKey) -> glib::Propagation {
        self.aim_keys();
        if self.im.filter_keypress(event) {
            return glib::Propagation::Stop;
        }
        let mut m = self.model.borrow_mut();
        if m.ime.keydown_ignored(0) {
            return glib::Propagation::Stop;
        }
        let action = key_action(event.keyval(), event.state(), &m.engine.modes());
        m.ime.forget_commit();
        drop(m);
        match action {
            KeyAction::Send(bytes) => {
                self.send_event(&bytes);
                let mut m = self.model.borrow_mut();
                m.key_at = Some(Instant::now());
                m.key_shown = m.key_at.filter(|_| super::lag::trace());
                let lit = self.steady_cursor(&mut m);
                drop(m);
                if lit {
                    self.request_frame();
                }
                glib::Propagation::Stop
            }
            KeyAction::ScrollPage(direction) => {
                let mut m = self.model.borrow_mut();
                let rows = i32::from(m.engine.size().1);
                m.engine.scroll(-direction * rows);
                self.redraw();
                glib::Propagation::Stop
            }
            KeyAction::None => glib::Propagation::Proceed,
        }
    }
    fn selection_text(&self) -> Option<String> {
        let m = self.model.borrow();
        if let Some(pick) = m.pick.filter(|p| p.dragged) {
            return visible_text(&m, &pick).filter(|text| !text.is_empty());
        }
        m.selection
            .map(|s| selected_text(m.engine.engine(), &s))
            .map(|text| {
                // The spaces a program pads a line with are not what was read.
                let mut out = String::with_capacity(text.len());
                for (i, line) in text.split('\n').enumerate() {
                    if i > 0 {
                        out.push('\n');
                    }
                    out.push_str(line.trim_end_matches([' ', '\t']));
                }
                out
            })
            .filter(|text| !text.is_empty())
    }
    fn point(&self, x: f64, y: f64) -> (u16, u16) {
        let m = self.model.borrow();
        let (cols, rows) = m.engine.size();
        let col = ((x - m.geom.origin_x).max(0.) / m.geom.cell_w).floor() as u16;
        let row = ((y - m.geom.origin_y).max(0.) / m.geom.cell_h).floor() as u16;
        let (col, row) = (col.min(cols.saturating_sub(1)), row.min(rows.saturating_sub(1)));
        // A filtered pane shows grid rows somewhere else: find the one drawn here.
        let row = m
            .noise_maps
            .iter()
            .find(|(r, _)| {
                col >= r.left
                    && col < r.left.saturating_add(r.width)
                    && row >= r.top
                    && row < r.top.saturating_add(r.height)
            })
            .and_then(|(r, map)| {
                map.iter()
                    .position(|d| *d == Some(row))
                    .or_else(|| map.iter().position(Option::is_some))
                    .and_then(|i| u16::try_from(i).ok())
                    .map(|i| r.top.saturating_add(i))
            })
            .unwrap_or(row);
        (col, row)
    }
    fn absolute(&self, col: u16, row: u16) -> Point {
        let offset =
            i32::try_from(self.model.borrow().engine.engine().display_offset()).unwrap_or(i32::MAX);
        (i32::from(row).saturating_sub(offset), col)
    }
}
impl Inner {
    /// The tab is seen again. Its screen kept up while hidden, so the history
    /// above the filtered text is read from tmux now instead of once output
    /// goes quiet: no late text and no copy from the last visit.
    fn shown(self: &Rc<Self>) {
        let now = self.ms();
        {
            let mut m = self.model.borrow_mut();
            m.shown_ms = now;
            m.shown_at = Some(Instant::now());
            m.shown_fill = None;
            if !m.hide_noise || self.history.borrow().is_none() || m.pty.is_none() {
                return;
            }
            self.prepare(&mut m, None);
        }
        self.fetch_history(now);
        let mut m = self.model.borrow_mut();
        if !m.fill_busy.is_empty() {
            m.shown_fill = m.shown_at;
        }
    }
    /// Read the history above panes whose hidden noise left empty rows: right
    /// after output goes quiet, or at most once a second while it streams.
    fn fetch_history(self: &Rc<Self>, now: u64) {
        let Some(source) = self.history.borrow().clone() else {
            return;
        };
        let mut m = self.model.borrow_mut();
        if m.fill_wanted.is_empty() || !m.hide_noise {
            return;
        }
        // Right after a wheel scroll the fill is wrong: refresh as soon as
        // tmux has redrawn instead of waiting for output to settle.
        let recent = |at: u64| now.saturating_sub(at) < 1500;
        let settle = if recent(m.scroll_ms) || recent(m.resized_ms) || recent(m.shown_ms) { 40 } else { 200 };
        let quiet = now.saturating_sub(m.output_ms) >= settle;
        // The live screen is read too (see `Fill::screen`).
        let whole = m.engine.size().1.saturating_sub(m.status_rows);
        let jobs: Vec<(String, u16, u16)> = m
            .fill_wanted
            .iter()
            .filter(|(id, _)| {
                !m.fill_busy.contains(id)
                    && (quiet
                        || m.fills
                            .get(id)
                            .is_none_or(|f| now.saturating_sub(f.asked_ms) >= if f.loose { 250 } else { 1000 }))
            })
            .map(|(id, lines)| {
                let screen = if id.is_empty() {
                    whole
                } else {
                    m.pane_ids
                        .iter()
                        .position(|p| p == id)
                        .and_then(|i| m.regions.get(i))
                        .map_or(0, |r| r.height)
                };
                (id.clone(), *lines, screen)
            })
            .collect();
        m.fill_wanted.retain(|(id, _)| !jobs.iter().any(|j| j.0 == *id));
        for (id, ..) in &jobs {
            m.fill_busy.insert(id.clone());
        }
        drop(m);
        for (id, lines, screen) in jobs {
            let target = if id.is_empty() {
                match self.options.session.as_deref() {
                    Some(session) => format!("={session}:"),
                    None => continue,
                }
            } else {
                id.clone()
            };
            let weak = Rc::downgrade(self);
            source(
                target,
                -i64::from(lines),
                i64::from(screen) - 1,
                Box::new(move |text| {
                    if let Some(inner) = weak.upgrade() {
                        inner.store_history(id, lines, screen, now, text);
                        inner.fill_arrived();
                    }
                }),
            );
        }
    }
    fn store_history(&self, id: String, lines: u16, screen: u16, asked_ms: u64, text: Option<String>) {
        let mut m = self.model.borrow_mut();
        m.fill_busy.remove(&id);
        let (cols, rows) = m.engine.size();
        let region = if id.is_empty() {
            Some(super::noise::Region { left: 0, width: cols, top: 0, height: rows })
        } else {
            m.pane_ids
                .iter()
                .position(|p| *p == id)
                .and_then(|i| m.regions.get(i).copied())
        };
        let Some(r) = region else {
            return;
        };
        let fill = parse_fill(&m, &r, &text.unwrap_or_default(), lines, usize::from(screen), asked_ms, self.epoch);
        m.fills.insert(id, fill);
        if let Some(at) = m.shown_fill.take() {
            super::lag::note(&format!(
                "shown {} history={:.0}ms",
                self.options.session.as_deref().unwrap_or("-"),
                super::lag::ms(at)
            ));
        }
        drop(m);
        self.redraw();
    }
    /// Wheel over a pane: scroll it here when the filter owns it (see
    /// `Scroll`). False leaves the wheel to tmux.
    fn app_scroll(self: &Rc<Self>, x: f64, y: f64, lines: i32) -> bool {
        let mut m = self.model.borrow_mut();
        if !m.hide_noise || self.history.borrow().is_none() {
            return false;
        }
        let (col, row) = raw_cell(&m, x, y);
        let Some(r) = m
            .regions
            .iter()
            .find(|r| {
                col >= r.left
                    && col < r.left.saturating_add(r.width)
                    && row >= r.top
                    && row < r.top.saturating_add(r.height)
            })
            .copied()
        else {
            return false;
        };
        if m.regions.is_empty() || copy_pos(&m, &r) > 0 {
            return false;
        }
        let Some(id) = region_id(&m, &r).filter(|id| !id.is_empty()) else {
            return false;
        };
        let Some(input) = super::noise::input_top(m.frame.inks(), &r) else {
            m.scrolls.remove(&id);
            return false;
        };
        if lines == 0 {
            return true;
        }
        let key = scroll_key(&id);
        let snap = m
            .fills
            .get(&key)
            .filter(|f| f.fits(&m, &r))
            .map(|f| (f.picks.len().saturating_sub(usize::from(input)), f.more));
        let scroll = m.scrolls.entry(id.clone()).or_insert(Scroll { back: 0, lines: 0, busy: false });
        scroll.back = usize::try_from((scroll.back as i64 - i64::from(lines)).max(0)).unwrap_or(0);
        if let Some((room, _)) = snap {
            scroll.back = scroll.back.min(room);
        }
        if scroll.back == 0 {
            m.scrolls.remove(&id);
            m.fills.remove(&key);
            drop(m);
            self.redraw();
            return true;
        }
        // Read the history the first time, and more of it near the top.
        let ask = match snap {
            _ if scroll.busy => None,
            None if scroll.lines == 0 => Some(SCROLL_LINES),
            Some((room, true)) if scroll.back + usize::from(input) >= room && scroll.lines < MAX_SCROLL_LINES => {
                Some(scroll.lines.saturating_mul(2).min(MAX_SCROLL_LINES))
            }
            _ => None,
        };
        if let Some(lines) = ask {
            scroll.lines = lines;
            scroll.busy = true;
        }
        drop(m);
        if let Some(lines) = ask
            && let Some(source) = self.history.borrow().clone()
        {
            let weak = Rc::downgrade(self);
            let asked_ms = self.ms();
            source(
                id.clone(),
                -i64::from(lines),
                i64::from(input) - 1,
                Box::new(move |text| {
                    if let Some(inner) = weak.upgrade() {
                        inner.store_scroll(id, r, lines, input, asked_ms, text);
                    }
                }),
            );
        }
        self.redraw();
        true
    }
    fn store_scroll(
        &self,
        id: String,
        r: super::noise::Region,
        lines: u16,
        input: u16,
        asked_ms: u64,
        text: Option<String>,
    ) {
        let mut m = self.model.borrow_mut();
        let Some(scroll) = m.scrolls.get_mut(&id) else {
            return;
        };
        scroll.busy = false;
        let Some(text) = text else {
            return;
        };
        let mut fill = parse_fill(&m, &r, &text, lines, 0, asked_ms, self.epoch);
        fill.base = fill.count.saturating_sub(usize::from(input));
        let room = fill.picks.len().saturating_sub(usize::from(input));
        let back = m.scrolls.get_mut(&id).map_or(0, |s| {
            s.back = s.back.min(room);
            s.back
        });
        if back == 0 {
            m.scrolls.remove(&id);
        } else {
            m.fills.insert(scroll_key(&id), fill);
        }
        drop(m);
        self.redraw();
    }
}
/// tmux lines read for a pane, parsed and with the noise rows left out. The
/// last `screen` of them are the live screen (0: only lines to show).
fn parse_fill(
    m: &Model,
    r: &super::noise::Region,
    text: &str,
    lines: u16,
    screen: usize,
    asked_ms: u64,
    epoch: Instant,
) -> Fill {
    let captured: Vec<&str> = text.lines().collect();
    let count = u16::try_from(captured.len()).unwrap_or(u16::MAX).max(1);
    let mut engine = TermEngine::new(r.width, count, 0, m.engine.palette().clone(), false, epoch);
    engine.feed(captured.join("\r\n").as_bytes(), Instant::now());
    let mut geom = m.geom;
    geom.origin_x += f64::from(r.left) * geom.cell_w;
    let mut frame = FrameCache::default();
    frame.refresh(&engine, &geom);
    let (picks, blank): (Vec<usize>, Vec<bool>) = if captured.is_empty() {
        Default::default()
    } else {
        super::noise::kept_rows(frame.inks(), r.width).into_iter().unzip()
    };
    let total = captured.len();
    let screen = screen.min(total);
    let marks_from = total.saturating_sub(screen * 2);
    let mut marks = Vec::new();
    if screen > 0 {
        let mut row = RowRender::default();
        for line in marks_from..total {
            engine.render_line(line, &mut row);
            marks.push(super::pick::row_mark(&row, 0, r.width));
        }
    }
    let mut fill = Fill {
        frame,
        engine,
        picks,
        blank,
        end: 0,
        asked_ms,
        count: total,
        base: 0,
        screen,
        marks,
        marks_from,
        aligned: screen == 0,
        matched_ms: 0,
        loose: false,
        sure: false,
        lines,
        more: total - screen >= usize::from(lines),
        left: r.left,
        width: r.width,
        geom: m.geom,
    };
    fill.set_base(total - screen);
    if screen == 0 {
        fill.picks.truncate(fill.end);
        fill.blank.truncate(fill.end);
    }
    fill
}
/// The cell under a pointer, in display rows (no noise mapping).
fn raw_cell(m: &Model, x: f64, y: f64) -> (u16, u16) {
    let (col, row) = raw_point(m, x, y);
    let (_, rows) = m.engine.size();
    (col, u16::try_from(row.max(0)).unwrap_or(0).min(rows.saturating_sub(1)))
}
/// Like `raw_cell`, but the row may fall above or below the terminal.
fn raw_point(m: &Model, x: f64, y: f64) -> (u16, i32) {
    let (cols, _) = m.engine.size();
    let col = ((x - m.geom.origin_x).max(0.) / m.geom.cell_w).floor() as u16;
    let row = ((y - m.geom.origin_y) / m.geom.cell_h).floor() as i32;
    (col.min(cols.saturating_sub(1)), row)
}

fn region_id(m: &Model, r: &super::noise::Region) -> Option<String> {
    if m.regions.is_empty() {
        return Some(String::new());
    }
    m.regions.iter().position(|x| x == r).and_then(|i| m.pane_ids.get(i).cloned())
}

/// History drawn above this pane's screen rows, and how many visible lines
/// back it is scrolled.
fn usable_fill<'a>(m: &'a Model, r: &super::noise::Region) -> Option<(&'a Fill, usize)> {
    let id = region_id(m, r)?;
    if let Some(scroll) = m.scrolls.get(&id)
        && let Some(snap) = m.fills.get(&scroll_key(&id)).filter(|f| f.fits(m, r))
    {
        return Some((snap, scroll.back));
    }
    m.fills
        .get(&id)
        .filter(|f| f.fits(m, r) && f.asked_ms >= m.scroll_ms && f.current(m.shown_ms))
        .map(|f| (f, 0))
}
/// The input box row of a pane the app has scrolled back, when its copy of
/// the history is ready.
fn scrolled_input(m: &Model, r: &super::noise::Region) -> Option<u16> {
    let id = region_id(m, r)?;
    m.scrolls.get(&id)?;
    m.fills.get(&scroll_key(&id)).filter(|f| f.fits(m, r))?;
    super::noise::input_top(m.frame.inks(), r)
}

fn copy_pos(m: &Model, r: &super::noise::Region) -> u32 {
    m.copy_pos.iter().find(|(x, _)| x == r).map_or(0, |(_, p)| *p)
}

/// The tmux line shown on each row of a pane (None: an empty row).
fn region_lines(m: &Model, r: &super::noise::Region) -> Vec<Option<i64>> {
    let height = usize::from(r.height);
    let pos = i64::from(copy_pos(m, r));
    let Some((_, map)) = m.noise_maps.iter().find(|(x, _)| x == r) else {
        return (0..height).map(|i| Some(i as i64 - pos)).collect();
    };
    let mut out = vec![None; height];
    for (i, shown) in map.iter().enumerate() {
        if let Some(shown) = shown
            && let Some(slot) = out.get_mut(usize::from(shown.saturating_sub(r.top)))
        {
            *slot = Some(i as i64);
        }
    }
    let holes = usize::from(super::noise::holes(map, r));
    if let Some((fill, skip)) = usable_fill(m, r) {
        let shown = fill.shown(skip, holes);
        let first = holes - shown.len();
        for (k, &j) in shown.iter().enumerate() {
            if let Some(slot) = out.get_mut(first + k) {
                *slot = Some(j as i64 - fill.base as i64);
            }
        }
    }
    out
}

/// Start an app-side selection when the press lands inside a tmux pane.
fn start_pick(m: &mut Model, x: f64, y: f64, kind: gdk::EventType, mods: (bool, bool, bool)) -> bool {
    let (col, row) = raw_cell(m, x, y);
    let Some(region) = m
        .regions
        .iter()
        .find(|r| {
            col >= r.left
                && col < r.left.saturating_add(r.width)
                && row >= r.top
                && row < r.top.saturating_add(r.height)
        })
        .copied()
    else {
        return false;
    };
    let lines = region_lines(m, &region);
    let mut pick = super::pick::Pick::new(region, (0, col), (col, i32::from(row)), mods);
    let Some(at) = pick.spot_at_pointer(&lines) else {
        return false;
    };
    pick.anchor = at;
    pick.head = at;
    match kind {
        gdk::EventType::DoubleButtonPress => {
            let cells = display_cells(m, &region, row);
            let (from, to) = super::pick::word_bounds(&cells, usize::from(col - region.left));
            let at_col = |i: usize| region.left.saturating_add(u16::try_from(i).unwrap_or(0));
            pick.anchor = (at.0, at_col(from));
            pick.head = (at.0, at_col(to));
            pick.dragged = true;
        }
        gdk::EventType::TripleButtonPress => {
            pick.anchor = (at.0, region.left);
            pick.head = (at.0, pick.right());
            pick.dragged = true;
        }
        _ => {}
    }
    m.pick = Some(pick);
    true
}

/// The link under the pointer. A row the noise filter filled with history
/// (or a pane scrolled here) is not in the grid: its link is read from the
/// text drawn there, not from the grid row `point` falls back to.
fn link_under(m: &Model, x: f64, y: f64, point: Point, (col, row): (u16, u16)) -> Option<String> {
    let (raw_col, raw_row) = raw_cell(m, x, y);
    let Some(r) = history_region(m, raw_col, raw_row) else {
        return super::links::link_at(&m.engine, point, row, col);
    };
    let first = raw_row.saturating_sub(3).max(r.top);
    let last = raw_row.saturating_add(5).min(r.top.saturating_add(r.height));
    let rows: Vec<Vec<String>> = (first..last).map(|line| display_cells(m, r, line)).collect();
    if rows.iter().all(|cells| cells.iter().all(|c| c.trim().is_empty())) {
        return None;
    }
    super::links::link_in_cells(
        &rows,
        usize::from(raw_row - first),
        usize::from(raw_col - r.left),
        None,
        |_| false,
    )
}

/// The pane whose display row `row` is not a grid row (history or blank).
fn history_region(m: &Model, col: u16, row: u16) -> Option<&super::noise::Region> {
    m.noise_maps
        .iter()
        .find(|(r, map)| {
            col >= r.left
                && col < r.left.saturating_add(r.width)
                && row >= r.top
                && row < r.top.saturating_add(r.height)
                && !map.contains(&Some(row))
        })
        .map(|(r, _)| r)
}

/// The cells shown on display row `row` of a pane: a grid row, a row of
/// history filled in above it, or nothing.
fn display_cells(m: &Model, r: &super::noise::Region, row: u16) -> Vec<String> {
    let mut render = RowRender::default();
    let Some((_, map)) = m.noise_maps.iter().find(|(x, _)| x == r) else {
        m.engine.render_line(usize::from(row), &mut render);
        return super::pick::cells_of(&render, r.left, r.width);
    };
    if let Some(i) = map.iter().position(|d| *d == Some(row)) {
        m.engine.render_line(usize::from(r.top) + i, &mut render);
        return super::pick::cells_of(&render, r.left, r.width);
    }
    let holes = super::noise::holes(map, r);
    let first = r.top.saturating_add(holes);
    if let Some((fill, skip)) = usable_fill(m, r) {
        let shown = fill.shown(skip, usize::from(holes));
        let top = usize::from(first).saturating_sub(shown.len());
        if usize::from(row) >= top && row < first {
            if let Some(&j) = shown.get(usize::from(row) - top) {
                fill.engine.render_line(j, &mut render);
                return super::pick::cells_of(&render, 0, r.width);
            }
        }
    }
    vec![String::from(" "); usize::from(r.width)]
}

/// The selected text when both ends are on screen, as the screen shows it
/// (rows the noise filter hides are left out). None when part of it is not.
fn visible_text(m: &Model, pick: &super::pick::Pick) -> Option<String> {
    let r = pick.region;
    let lines = region_lines(m, &r);
    let (a, b) = pick.ordered();
    if !lines.contains(&Some(a.0)) || !lines.contains(&Some(b.0)) {
        return None;
    }
    Some(
        pick.spans(&lines)
            .into_iter()
            .map(|(row, from, to)| {
                let cells = display_cells(m, &r, row);
                super::pick::span_text(&cells, usize::from(from - r.left), usize::from(to - r.left))
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Text of tmux lines read for a pick that left the screen.
fn captured_text(text: &str, pick: &super::pick::Pick, palette: &Palette, epoch: Instant) -> String {
    let r = pick.region;
    let rows: Vec<&str> = text.lines().collect();
    let count = u16::try_from(rows.len()).unwrap_or(u16::MAX).max(1);
    let mut engine = TermEngine::new(r.width, count, 0, palette.clone(), false, epoch);
    engine.feed(rows.join("\r\n").as_bytes(), Instant::now());
    let (a, b) = pick.ordered();
    let last = rows.len().saturating_sub(1);
    let mut render = RowRender::default();
    (0..rows.len())
        .map(|k| {
            engine.render_line(k, &mut render);
            let cells = super::pick::cells_of(&render, 0, r.width);
            let from = if k == 0 { usize::from(a.1 - r.left) } else { 0 };
            let to = if k == last { usize::from(b.1 - r.left) } else { usize::from(r.width) - 1 };
            super::pick::span_text(&cells, from, to)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Inner {
    /// Copy a finished pick: from the screen when it is all visible, else
    /// read the lines from tmux off the main thread.
    fn copy_pick(self: &Rc<Self>, pick: super::pick::Pick) {
        let m = self.model.borrow();
        if let Some(text) = visible_text(&m, &pick) {
            drop(m);
            self.copy_text(&text);
            return;
        }
        let (Some(id), Some(source)) = (region_id(&m, &pick.region), self.history.borrow().clone())
        else {
            return;
        };
        let target = if id.is_empty() {
            match self.options.session.as_deref() {
                Some(session) => format!("={session}:"),
                None => return,
            }
        } else {
            id
        };
        let palette = m.engine.palette().clone();
        drop(m);
        let (a, b) = pick.ordered();
        let weak = Rc::downgrade(self);
        let epoch = self.epoch;
        source(
            target,
            a.0,
            b.0,
            Box::new(move |text| {
                if let (Some(inner), Some(text)) = (weak.upgrade(), text) {
                    inner.copy_text(&captured_text(&text, &pick, &palette, epoch));
                }
            }),
        );
    }
    fn copy_text(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.clipboard
            .copy_selection(text, glib::monotonic_time() as f64 / 1_000_000.);
        self.clipboard.copy(crate::ui::clipboard::Target::Primary, text);
    }
    /// While a drag sits above or below its pane, scroll tmux that way.
    fn pick_autoscroll(self: &Rc<Self>) {
        if self.pick_timer.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local(Duration::from_millis(70), move || {
            let Some(inner) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            let m = inner.model.borrow();
            let Some(pick) = m.pick.filter(|p| p.held && p.edge() != 0) else {
                drop(m);
                inner.pick_timer.borrow_mut().take();
                return ControlFlow::Break;
            };
            let edge = pick.edge();
            if edge > 0 && copy_pos(&m, &pick.region) == 0 {
                // Already at the bottom: nothing below to scroll to.
                return ControlFlow::Continue;
            }
            let action = wheel(edge, pick.region.left, pick.region.top, (false, false, false), &m.engine.modes());
            drop(m);
            if let WheelAction::Mouse(bytes) | WheelAction::Arrows(bytes) = action {
                inner.model.borrow_mut().scroll_ms = inner.ms();
                inner.send_event(&bytes);
            }
            ControlFlow::Continue
        });
        *self.pick_timer.borrow_mut() = Some(id);
    }
    fn stop_autoscroll(&self) {
        if let Some(id) = self.pick_timer.borrow_mut().take() {
            id.remove();
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.closed.set(true);
        self.replay.borrow_mut().cancel();
        self.link_event.borrow_mut().take();
        self.clean_click.borrow_mut().take();
        self.primary_press.borrow_mut().take();
        self.last_ctrl_c.set(None);
        self.cleanup_cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.app_key.get_mut().take();
        self.key_observer.get_mut().take();
        for source in [
            &mut self.read_source,
            &mut self.write_source,
            &mut self.timer,
        ] {
            if let Some(id) = source.get_mut().take() {
                id.remove();
            }
        }
        self.model.get_mut().pty.take();
        self.model.get_mut().shadow.take();
    }
}

fn modifiers(state: gdk::ModifierType) -> (bool, bool, bool) {
    (
        state.contains(gdk::ModifierType::CONTROL_MASK),
        state.contains(gdk::ModifierType::MOD1_MASK),
        state.contains(gdk::ModifierType::SHIFT_MASK),
    )
}
fn connect_events(inner: &Rc<Inner>) {
    let weak = Rc::downgrade(inner);
    inner.area.connect_destroy(move |_| {
        if let Some(inner) = weak.upgrade() {
            inner.closed.set(true);
            inner.last_ctrl_c.set(None);
            inner.replay.borrow_mut().cancel();
            inner.link_event.borrow_mut().take();
            inner.clean_click.borrow_mut().take();
            inner.primary_press.borrow_mut().take();
            inner
                .cleanup_cancelled
                .store(true, std::sync::atomic::Ordering::Release);
            inner.app_key.borrow_mut().take();
            inner.key_observer.borrow_mut().take();
            for source in [&inner.read_source, &inner.write_source, &inner.timer] {
                if let Some(id) = source.borrow_mut().take() {
                    id.remove();
                }
            }
            inner.model.borrow_mut().pty.take();
            inner.model.borrow_mut().shadow.take();
        }
    });
    let weak = Rc::downgrade(inner);
    inner.im.connect_preedit_end(move |_| {
        if let Some(inner) = weak.upgrade() {
            let mut m = inner.model.borrow_mut();
            m.ime.end("");
            m.preedit.clear();
        }
    });
    let targets = gtk::TargetList::new(&[]);
    targets.add_uri_targets(0);
    targets.add_text_targets(1);
    inner
        .area
        .drag_dest_set(gtk::DestDefaults::ALL, &[], gdk::DragAction::COPY);
    inner.area.drag_dest_set_target_list(Some(&targets));
    let weak = Rc::downgrade(inner);
    inner
        .area
        .connect_drag_data_received(move |_, context, _, _, data, _, time| {
            let mut success = false;
            if let Some(inner) = weak.upgrade() {
                let uris = data.uris();
                let text = if uris.is_empty() {
                    data.text().map(|s| s.to_string())
                } else {
                    Some(
                        uris.iter()
                            .map(|uri| {
                                let raw = if uri.starts_with("file:") {
                                    glib::filename_from_uri(uri)
                                        .ok()
                                        .map(|(p, _)| p.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| uri.to_string())
                                } else {
                                    uri.to_string()
                                };
                                format!("'{}'", raw.replace('\'', "'\\''"))
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                            + " ",
                    )
                };
                if let Some(text) = text {
                    let bytes = encode_paste(&text, &inner.model.borrow().engine.modes());
                    success = inner.send(&bytes).is_ok();
                }
            }
            context.drag_finish(success, false, time);
        });

    let weak = Rc::downgrade(inner);
    inner.area.connect_realize(move |area| {
        if let Some(inner) = weak.upgrade() {
            inner.im.set_client_window(area.window().as_ref());
            inner.configure_font();
        }
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_unrealize(move |_| {
        if let Some(inner) = weak.upgrade() {
            inner.im.set_client_window(None);
        }
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_size_allocate(move |_, _| {
        if let Some(inner) = weak.upgrade() {
            inner.allocate();
        }
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_draw(move |_, context| {
        if let Some(inner) = weak.upgrade() {
            inner.draw(context);
        }
        glib::Propagation::Stop
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_map(move |_| {
        if let Some(inner) = weak.upgrade().filter(|inner| !inner.closed.get()) {
            inner.model.borrow_mut().schedule.reset_queued_paint();
            inner.rewatch_read();
            inner.resize_if_waiting();
            inner.claim_size();
            inner.shown();
            inner.redraw();
            inner.arm();
        }
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_unmap(move |_| {
        if let Some(inner) = weak.upgrade().filter(|inner| !inner.closed.get()) {
            inner.model.borrow_mut().schedule.reset_queued_paint();
            if let Some(reader) = &mut inner.model.borrow_mut().shadow {
                reader.set_visible(false);
            }
            inner.rewatch_read();
            inner.arm();
        }
    });
    let weak = Rc::downgrade(inner);
    inner.im.connect_preedit_start(move |_| {
        if let Some(inner) = weak.upgrade() {
            inner.model.borrow_mut().ime.start();
        }
    });
    let weak = Rc::downgrade(inner);
    inner.im.connect_preedit_changed(move |im| {
        if let Some(inner) = weak.upgrade() {
            inner.model.borrow_mut().preedit = im.preedit_string().0.to_string();
            inner.redraw();
        }
    });
    let weak = Rc::downgrade(inner);
    inner.im.connect_commit(move |_, text| {
        if let Some(inner) = weak.upgrade() {
            inner.aim_keys();
            // Only a composition that was on screen needs painting over: a
            // plain letter changes nothing until the program echoes it.
            // Typing also brings a view scrolled back down to the live screen.
            let (bytes, composed) = {
                let mut m = inner.model.borrow_mut();
                let composed = !m.preedit.is_empty()
                    || m.engine.engine().display_offset() != 0
                    || m.glide_frac != 0.;
                m.preedit.clear();
                (m.ime.end(text), composed)
            };
            if let Some(bytes) = bytes {
                inner.send_event(&bytes);
                let mut m = inner.model.borrow_mut();
                m.key_at = Some(Instant::now());
                m.key_shown = m.key_at.filter(|_| super::lag::trace());
                let lit = inner.steady_cursor(&mut m);
                drop(m);
                if lit {
                    inner.request_frame();
                }
            }
            if composed {
                inner.redraw();
            }
        }
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_key_press_event(move |_, event| {
        let Some(inner) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let observer = inner.key_observer.borrow().clone();
        if let Some(observer) = observer {
            observer(event);
        }
        if inner.dispatch_app_key(event) {
            return glib::Propagation::Stop;
        }
        if !event.is_modifier() {
            let mut m = inner.model.borrow_mut();
            // Typing brings panes the app scrolled back to the live bottom.
            let back = !m.scrolls.is_empty();
            m.scrolls.clear();
            if m.pick.take().is_some() || back {
                drop(m);
                inner.redraw();
            }
        }
        inner.forward_key(event)
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_key_release_event(move |_, event| {
        weak.upgrade().map_or(glib::Propagation::Proceed, |inner| {
            if inner.im.filter_keypress(event) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        })
    });
    for on in [true, false] {
        let weak = Rc::downgrade(inner);
        let callback = move |_: &gtk::DrawingArea, _: &gdk::EventFocus| {
            if let Some(inner) = weak.upgrade() {
                if on {
                    inner.im.focus_in();
                    inner.claim_size();
                } else {
                    inner.im.focus_out();
                }
                let mut m = inner.model.borrow_mut();
                m.focused = on;
                m.blink_visible = true;
                m.schedule.set_focused(on, inner.ms());
                let bytes = focus(on, &m.engine.modes());
                drop(m);
                if let Some(bytes) = bytes {
                    inner.send_event(bytes);
                }
                inner.redraw();
            }
            glib::Propagation::Proceed
        };
        if on {
            inner.area.connect_focus_in_event(callback);
        } else {
            inner.area.connect_focus_out_event(callback);
        }
    }
    let weak = Rc::downgrade(inner);
    inner.area.connect_button_press_event(move |area, event| {
        let Some(inner) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        area.grab_focus();
        let (x, y) = event.position();
        inner.aim_at(x, y, true);
        let (col, row) = inner.point(x, y);
        if let Some(callback) = inner.primary_press.borrow().clone() {
            callback(event, inner.window_cell((col, row)));
        }
        if event.button() == 3
            && let Some(callback) = inner.context_menu.borrow().as_ref()
        {
            callback(event, inner.window_cell((col, row)));
            return glib::Propagation::Stop;
        }
        let mods = modifiers(event.state());
        if event.button() == 2
            && (mods.2 || inner.model.borrow().engine.modes().mouse == MouseMode::Off)
        {
            let weak = Rc::downgrade(&inner);
            inner.clipboard.request(
                crate::ui::clipboard::Target::Primary,
                Box::new(move |text| {
                    if let Some(inner) = weak.upgrade().filter(|i| !i.closed.get()) {
                        let bytes = encode_paste(&text, &inner.model.borrow().engine.modes());
                        inner.send_event(&bytes);
                    }
                }),
            );
            return glib::Propagation::Stop;
        }
        let button = mouse_button(event.button());
        let link = if event.button() == 1 {
            link_under(&inner.model.borrow(), x, y, inner.absolute(col, row), (col, row))
        } else {
            None
        };
        let link_gesture = link.is_some();
        if event.button() == 1 {
            inner.model.borrow_mut().link_gesture = link_gesture;
        }
        {
            let mut m = inner.model.borrow_mut();
            let had = m.pick.take().is_some();
            if event.button() == 1 && !mods.0 && !link_gesture && start_pick(&mut m, x, y, event.event_type(), mods) {
                drop(m);
                inner.redraw();
                return glib::Propagation::Stop;
            }
            if had {
                inner.redraw();
            }
        }
        let bytes = encode_mouse(
            button,
            MouseKind::Press,
            col,
            row,
            mods,
            &inner.model.borrow().engine.modes(),
        );
        if let Some(bytes) = bytes.filter(|_| super::links::report_mouse(mods.2, link_gesture)) {
            inner.send_event(&bytes);
            return glib::Propagation::Stop;
        }
        if event.button() == 1 && mods.0 {
            inner.model.borrow_mut().primary_press.take();
            if let Some(link) = link.as_ref()
                && let Some(callback) = inner.link_event.borrow().clone()
            {
                callback(link, event, inner.window_cell((col, row)), true);
                return glib::Propagation::Stop;
            }
        }
        if event.button() == 1 {
            let point = inner.absolute(col, row);
            let mut m = inner.model.borrow_mut();
            let mode = match event.event_type() {
                gdk::EventType::DoubleButtonPress => SelectMode::Word,
                gdk::EventType::TripleButtonPress => SelectMode::Line,
                _ => {
                    if mods.1 {
                        SelectMode::Block
                    } else {
                        SelectMode::Simple
                    }
                }
            };
            m.pressed = Some(point);
            m.primary_press = Some(super::links::PrimaryPress::new((x, y), (col, row), link));
            m.selection = if mode == SelectMode::Simple {
                None
            } else {
                Some(Selection {
                    anchor: point,
                    head: point,
                    mode,
                })
            };
            drop(m);
            inner.redraw();
        }
        glib::Propagation::Stop
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_motion_notify_event(move |area, event| {
        let Some(inner) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let (x, y) = event.position();
        if !event.state().contains(gdk::ModifierType::BUTTON1_MASK) {
            inner.aim_at(x, y, false);
            // The release went elsewhere (a menu, another window): end the drag.
            let mut m = inner.model.borrow_mut();
            if let Some(pick) = m.pick.as_mut().filter(|p| p.held) {
                pick.held = false;
                if !pick.dragged {
                    m.pick = None;
                }
            }
        }
        if event.state().contains(gdk::ModifierType::BUTTON1_MASK) {
            let mut m = inner.model.borrow_mut();
            let pointer = raw_point(&m, x, y);
            if let Some(pick) = m.pick.filter(|p| p.held) {
                if pointer != pick.pointer {
                    let lines = region_lines(&m, &pick.region);
                    let mut next = pick;
                    next.pointer = pointer;
                    if let Some(head) = next.spot_at_pointer(&lines) {
                        next.head = head;
                    }
                    next.dragged |= next.head != next.anchor || next.edge() != 0;
                    m.pick = Some(next);
                    drop(m);
                    if next.edge() != 0 {
                        inner.pick_autoscroll();
                    }
                    inner.redraw();
                }
                return glib::Propagation::Stop;
            }
        }
        let (col, row) = inner.point(x, y);
        let point = inner.absolute(col, row);
        let state = event.state();
        let mut m = inner.model.borrow_mut();
        if m.last_motion == Some((col, row, state)) {
            return glib::Propagation::Stop;
        }
        m.last_motion = Some((col, row, state));
        let button = drag_button(state);
        let linked = link_under(&m, x, y, point, (col, row)).is_some();
        // The underline is drawn on grid cells; a history row has none.
        let (raw_col, raw_row) = raw_cell(&m, x, y);
        let on_grid = history_region(&m, raw_col, raw_row).is_none();
        let hovered = (linked && on_grid && button == Button::None)
            .then(|| word_at(m.engine.engine(), point))
            .flatten();
        if m.hovered_link != hovered {
            m.hovered_link = hovered;
            inner.redraw();
        }
        if let Some(window) = area.window() {
            window.set_cursor(
                gdk::Cursor::from_name(&window.display(), if linked { "pointer" } else { "text" })
                    .as_ref(),
            );
        }
        let link_gesture = m.link_gesture || (button == Button::None && linked);
        let bytes = encode_mouse(
            button,
            MouseKind::Move,
            col,
            row,
            modifiers(state),
            &m.engine.modes(),
        );
        if let Some(bytes) = bytes.filter(|_| {
            super::links::report_mouse(state.contains(gdk::ModifierType::SHIFT_MASK), link_gesture)
        }) {
            drop(m);
            inner.send_event(&bytes);
            return glib::Propagation::Stop;
        }
        if state.contains(gdk::ModifierType::BUTTON1_MASK) {
            if m.selection.is_none()
                && let Some(anchor) = m.pressed
            {
                m.selection = Some(Selection {
                    anchor,
                    head: point,
                    mode: SelectMode::Simple,
                });
            }
            if let Some(selection) = m.selection.as_mut() {
                selection.head = point;
            }
            inner.redraw();
        }
        drop(m);
        glib::Propagation::Stop
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_leave_notify_event(move |_, _| {
        if let Some(inner) = weak.upgrade() {
            let mut model = inner.model.borrow_mut();
            model.last_motion = None;
            if model.hovered_link.take().is_some() {
                inner.redraw();
            }
        }
        glib::Propagation::Proceed
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_button_release_event(move |_, event| {
        let Some(inner) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        inner.clipboard.release_selection();
        let (x, y) = event.position();
        let held = inner.model.borrow().pick.filter(|p| p.held);
        if event.button() == 1
            && let Some(pick) = held
        {
            inner.stop_autoscroll();
            let mut m = inner.model.borrow_mut();
            if pick.dragged {
                let mut done = m.pick.unwrap_or(pick);
                done.held = false;
                m.pick = Some(done);
                drop(m);
                inner.copy_pick(done);
                inner.redraw();
                return glib::Propagation::Stop;
            }
            // A plain click still reaches tmux (focus a pane, click a prompt).
            m.pick = None;
            let modes = m.engine.modes();
            drop(m);
            let (col, row) = inner.point(x, y);
            for kind in [MouseKind::Press, MouseKind::Release] {
                if let Some(bytes) = encode_mouse(mouse_button(1), kind, col, row, pick.mods, &modes) {
                    inner.send_event(&bytes);
                }
            }
            inner.redraw();
            return glib::Propagation::Stop;
        }
        let (col, row) = inner.point(x, y);
        let mods = modifiers(event.state());
        let mut m = inner.model.borrow_mut();
        let link_gesture = std::mem::take(&mut m.link_gesture);
        let press = m
            .primary_press
            .take()
            .and_then(|p| p.release((x, y), event.button()));
        let bytes = encode_mouse(
            mouse_button(event.button()),
            MouseKind::Release,
            col,
            row,
            mods,
            &m.engine.modes(),
        );
        if let Some(bytes) = bytes.filter(|_| super::links::report_mouse(mods.2, link_gesture)) {
            drop(m);
            inner.send_event(&bytes);
            return glib::Propagation::Stop;
        }
        m.pressed.take();
        let clicked = press.is_some();
        let point = press.as_ref().map_or((col, row), |p| p.point);
        let link = press.and_then(|p| p.link);
        drop(m);
        if clicked && let Some(callback) = inner.clean_click.borrow().clone() {
            callback(event, inner.window_cell(point));
        }
        if let Some(text) = inner.selection_text() {
            inner
                .clipboard
                .copy_selection(&text, glib::monotonic_time() as f64 / 1_000_000.);
            inner
                .clipboard
                .copy(crate::ui::clipboard::Target::Primary, &text);
        }
        if let Some(link) = link {
            if let Some(callback) = inner.link_event.borrow().clone() {
                callback(&link, event, inner.window_cell(point), false);
            } else if let Some(callback) = &inner.options.on_link {
                callback(&link);
            }
        }
        inner.redraw();
        glib::Propagation::Stop
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_scroll_event(move |_, event| {
        let Some(inner) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let dragging = event.state().contains(gdk::ModifierType::BUTTON1_MASK);
        if !dragging {
            let (x, y) = event.position();
            inner.aim_at(x, y, false);
        }
        {
            let mut m = inner.model.borrow_mut();
            // Mid-drag the wheel scrolls and the selection keeps growing; a
            // finished selection stays on its text too (it is anchored to lines).
            if !dragging && m.pick.is_some_and(|p| p.held) {
                m.pick = None;
                inner.redraw();
            }
        }
        let (x, y) = event.position();
        let (col, row) = inner.point(x, y);
        let amount = match event.direction() {
            gdk::ScrollDirection::Up => -3.,
            gdk::ScrollDirection::Down => 3.,
            gdk::ScrollDirection::Smooth => event
                .scroll_deltas()
                .map_or(0., |(_, dy)| (dy * 2.).clamp(-8., 8.)),
            _ => 0.,
        };
        let mut m = inner.model.borrow_mut();
        let ssh = inner
            .options
            .session
            .as_deref()
            .is_some_and(|s| s.starts_with("ssh-") || s.starts_with("sshtab-"));
        if inner.options.mode != RunMode::Shadow
            && ssh
            && m.selection.is_none()
            && inner.options.on_ssh_scroll.is_some()
        {
            m.scroll_ms = inner.ms();
            m.ssh_delta = (m.ssh_delta + amount).clamp(-24., 24.);
            m.ssh_point = (col, row);
            if m.ssh_at.is_none() {
                m.ssh_at = Some(inner.ms().saturating_add(45));
            }
            return glib::Propagation::Stop;
        }
        m.wheel_delta += amount;
        let lines = m.wheel_delta.trunc() as i32;
        m.wheel_delta -= f64::from(lines);
        drop(m);
        if !dragging && inner.app_scroll(x, y, lines) {
            return glib::Propagation::Stop;
        }
        let mut m = inner.model.borrow_mut();
        // tmux moves the text: history fills read before are stale.
        m.scroll_ms = inner.ms();
        match wheel(lines, col, row, modifiers(event.state()), &m.engine.modes()) {
            WheelAction::Mouse(bytes) | WheelAction::Arrows(bytes) => {
                drop(m);
                // La rueda deja el pane en copy-mode de tmux y la vista se congela:
                // la primera tecla lo saca, igual que tras seleccionar con el mouse.
                inner.clipboard.release_selection();
                inner.send_event(&bytes);
            }
            WheelAction::Scroll(lines) => {
                // Turning the wheel the other way stops what was left at once.
                if m.glide * f64::from(-lines) < 0. {
                    m.glide = 0.;
                }
                m.glide = (m.glide + f64::from(-lines)).clamp(-400., 400.);
                drop(m);
                inner.glide();
            }
        }
        glib::Propagation::Stop
    });
}
/// Paints a terminal into a PNG the way the widget does (same cell geometry, painter
/// and cursor), without a window. Used to check by eye what the app would show.
pub fn rasterize_png(
    term: &TermEngine,
    family: &str,
    size_pt: f64,
    padding: f64,
    path: &std::path::Path,
) -> Result<(), String> {
    let (cols, rows) = term.size();
    let probe = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1).map_err(|e| e.to_string())?;
    let layout = pangocairo::functions::create_layout(&cairo::Context::new(&probe).map_err(|e| e.to_string())?);
    let mut font = pango::FontDescription::new();
    font.set_family(&format!(
        "{family},JetBrainsMono Nerd Font,JetBrains Mono,DejaVu Sans Mono,Monospace"
    ));
    font.set_size((size_pt * f64::from(pango::SCALE)).round() as i32);
    layout.set_font_description(Some(&font));
    layout.set_text("M");
    let (width, height) = layout.pixel_size();
    let geom = CellGeom {
        cell_w: f64::from(width.max(1)),
        cell_h: (f64::from(height.max(1)) * 1.2).ceil(),
        origin_x: padding + 2.,
        origin_y: padding + 14.,
        dpr: 1.,
        font_size: size_pt * 96. / 72.,
    };
    let w = (geom.origin_x * 2. + geom.cell_w * f64::from(cols)).ceil() as i32;
    let h = (geom.origin_y + padding + geom.cell_h * f64::from(rows)).ceil() as i32;
    let mut surface = cairo::ImageSurface::create(cairo::Format::Rgb24, w, h).map_err(|e| e.to_string())?;
    let context = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
    color(&context, term.palette().bg);
    let _ = context.paint();
    let layout = pangocairo::functions::create_layout(&context);
    let mut text_painter = TextPainter::new(&layout, &font);
    let mut frame = FrameCache::default();
    frame.refresh(term, &geom);
    for op in frame.ops() {
        paint_op(&context, &mut text_painter, &geom, op);
    }
    let cursor = frame.cursor();
    if cursor.visible {
        color(&context, term.palette().cursor);
        context.rectangle(
            geom.origin_x + f64::from(cursor.col) * geom.cell_w,
            geom.origin_y + cursor.line as f64 * geom.cell_h,
            1.5,
            geom.cell_h,
        );
        let _ = context.fill();
    }
    drop(text_painter);
    drop(layout);
    drop(context);
    surface.flush();
    let stride = usize::try_from(surface.stride()).map_err(|e| e.to_string())?;
    let (wu, hu) = (w as usize, h as usize);
    let data = surface.data().map_err(|e| e.to_string())?;
    let mut rgb = Vec::with_capacity(wu * hu * 3);
    for row in data.chunks(stride).take(hu) {
        // Rgb24 is native-endian xRGB: B, G, R, x on little endian.
        for p in row.chunks_exact(4).take(wu) {
            if let &[b, g, r, _] = p {
                rgb.extend_from_slice(&[r, g, b]);
            }
        }
    }
    let pixbuf = gdk_pixbuf::Pixbuf::from_mut_slice(
        rgb,
        gdk_pixbuf::Colorspace::Rgb,
        false,
        8,
        w,
        h,
        w * 3,
    );
    pixbuf.savev(path, "png", &[]).map_err(|e| e.to_string())
}

fn color(context: &cairo::Context, rgb: [u8; 3]) {
    context.set_source_rgb(
        f64::from(rgb[0]) / 255.,
        f64::from(rgb[1]) / 255.,
        f64::from(rgb[2]) / 255.,
    );
}
impl Inner {
    fn draw(&self, context: &cairo::Context) {
        let started = Instant::now();
        // Paint into a client-side image and copy it in one operation: on X11 the
        // window surface lives in the X server, where every text run is a request.
        // The image always holds the last complete picture, so an exposure (a
        // window dragged over, an overlay) is only a copy.
        let scale = self.area.scale_factor().max(1);
        let (w, h) = (self.area.allocated_width().max(1), self.area.allocated_height().max(1));
        let mut canvas = self.canvas.borrow_mut();
        let fits = canvas
            .as_ref()
            .is_some_and(|c| c.width() == w * scale && c.height() == h * scale);
        let plan = self.planned.borrow_mut().take();
        let mut m = self.model.borrow_mut();
        // While the window changes size, and until tmux and the program have
        // drawn the new size, the last picture stays as it is (see `Hold`).
        let keep = m.hold.is_some() || (!fits && m.resize_at.is_some() && plan.is_none() && !self.full.get());
        if keep && let Some(image) = canvas.as_ref() {
            let bg = m.engine.palette().bg;
            context.set_source_rgba(
                f64::from(bg[0]) / 255.,
                f64::from(bg[1]) / 255.,
                f64::from(bg[2]) / 255.,
                m.opacity,
            );
            context.set_operator(cairo::Operator::Source);
            let _ = context.paint();
            if context.set_source_surface(image, 0., 0.).is_ok() {
                context.rectangle(0., 0., f64::from(image.width() / scale), f64::from(image.height() / scale));
                let _ = context.fill();
            }
            context.set_operator(cairo::Operator::Over);
            return;
        }
        let split = !m.slots.is_empty();
        let paint = !fits || self.full.get() || plan.is_some() || m.painted.is_none() || !m.stale.is_empty();
        let mut direct = false;
        if paint && split {
            if !fits {
                *canvas = cairo::ImageSurface::create(cairo::Format::ARgb32, w * scale, h * scale)
                    .ok()
                    .inspect(|c| c.set_device_scale(f64::from(scale), f64::from(scale)));
            }
            let image = canvas.as_ref().and_then(|c| cairo::Context::new(c).ok());
            direct = image.is_none();
            let all = !fits || self.full.get();
            self.draw_panes(&mut m, image.as_ref().unwrap_or(context), all, (f64::from(w), f64::from(h)));
            let now = self.ms();
            m.schedule.painted(now);
            self.full.set(false);
        } else if paint {
            if !fits {
                *canvas = cairo::ImageSurface::create(cairo::Format::ARgb32, w * scale, h * scale)
                    .ok()
                    .inspect(|c| c.set_device_scale(f64::from(scale), f64::from(scale)));
                m.painted = None;
            }
            // A plan made for this frame already read the screen and took its
            // changed lines; anything else repaints the whole picture.
            let plan = plan.filter(|_| fits && !self.full.get());
            let own = plan.is_none();
            let (look, rows) = match plan {
                Some(plan) => (plan.look, plan.rows),
                None => (self.prepare(&mut m, None), None),
            };
            let image = canvas.as_ref().and_then(|c| cairo::Context::new(c).ok());
            direct = image.is_none();
            self.draw_frame(&mut m, image.as_ref().unwrap_or(context), look, rows.as_deref());
            if own {
                let now = self.ms();
                m.schedule.painted(now);
            }
            self.full.set(false);
        }
        let blit_at = Instant::now();
        if !direct && let Some(image) = canvas.as_ref() {
            context.set_operator(cairo::Operator::Source);
            if context.set_source_surface(image, 0., 0.).is_ok() {
                let _ = context.paint();
            }
            // Mid-glide the text sits a part of a line lower than its grid.
            let shift = (m.glide_frac * m.geom.cell_h).round();
            if shift > 0. {
                let (top, bg) = (m.geom.origin_y, m.engine.palette().bg);
                let _ = context.save();
                // Among several panes only the one gliding moves.
                let (left, wide, tall) = match m.loaded.filter(|_| !m.slots.is_empty()) {
                    Some(p) => (
                        if p.left == 0 { 0. } else { m.geom.origin_x },
                        (f64::from(p.width) + if p.left == 0 { 1. } else { 0. }) * m.geom.cell_w + m.base_x,
                        f64::from(p.height) * m.geom.cell_h,
                    ),
                    None => (0., f64::from(w), f64::from(h) - top),
                };
                context.rectangle(left, top, wide, tall);
                context.clip();
                // The strip the text leaves behind is tile, like the rest.
                let bg = m.loaded.filter(|_| m.control.is_some()).map_or(bg, |p| Self::tile_rgb(&m, p.id));
                let alpha = if m.control.is_some() { 1. } else { m.opacity };
                context.set_source_rgba(
                    f64::from(bg[0]) / 255.,
                    f64::from(bg[1]) / 255.,
                    f64::from(bg[2]) / 255.,
                    alpha,
                );
                let _ = context.paint();
                if context.set_source_surface(image, 0., shift).is_ok() {
                    let _ = context.paint();
                }
                let _ = context.restore();
            }
            context.set_operator(cairo::Operator::Over);
        }
        drop(canvas);
        if !paint {
            return;
        }
        super::lag::phase("blit", blit_at);
        let took = super::lag::ms(started);
        let session = self.options.session.as_deref().unwrap_or("-");
        let phases = super::lag::take_phases();
        if super::lag::trace() {
            // A key counts as shown by the first paint of output that came after it.
            let echoed = m.key_shown.zip(m.output_at).filter(|(key, output)| output >= key);
            let (key, echo) = echoed.map_or((-1., -1.), |(key, output)| {
                (super::lag::ms(key), output.duration_since(key).as_secs_f64() * 1000.)
            });
            if echoed.is_some() {
                m.key_shown = None;
            }
            let wait = m.output_at.map_or(-1., super::lag::ms);
            super::lag::note(&format!(
                "trace paint {took:.1}ms key={key:.1} echo={echo:.1} output={wait:.1} {phases}"
            ));
        }
        if took > super::lag::SLOW_PAINT_MS {
            let (cols, rows) = m.engine.size();
            super::lag::note(&format!("paint {took:.1}ms {session} {cols}x{rows} {phases}"));
        }
        if let Some(at) = m.shown_at.take() {
            super::lag::note(&format!("shown {session} paint={:.0}ms", super::lag::ms(at)));
        }
        if let Some(output) = m.output_at.take() {
            let wait = super::lag::ms(output);
            if wait > super::lag::SLOW_FRAME_MS {
                // Where the wait went: until the frame was asked for and until
                // GTK started it (-1: before this output).
                let after = |at: Option<Instant>| {
                    at.and_then(|t| t.checked_duration_since(output))
                        .map_or(-1., |d| d.as_secs_f64() * 1000.)
                };
                let (ask, tick) = (after(self.asked_at.get()), after(self.ticked_at.get()));
                // `loop` near `tick`: the main loop was busy. `loop` near
                // `ask` with a late `tick`: the frame clock waited for the
                // compositor (`clock_gap` is the time since its last frame).
                let free = after(self.free_at.get());
                let gap = self.area.frame_clock().map_or(0., |c| {
                    let now = c.frame_counter();
                    let at = |n: i64| c.timings(n).map(|t| t.frame_time());
                    match (at(now), at(now - 1)) {
                        (Some(a), Some(b)) => (a - b) as f64 / 1000.,
                        _ => 0.,
                    }
                });
                super::lag::note(&format!(
                    "frame {wait:.0}ms after output {session} ask={ask:.0} loop={free:.0} tick={tick:.0} clock_gap={gap:.0}"
                ));
            }
        }
    }
    /// The solid tile behind the loaded pane: its header row and its text,
    /// with the same margins a tile found on a tmux picture gets.
    fn native_tile(m: &Model, width: f64) -> Option<((f64, f64, f64, f64), [u8; 3])> {
        let p = m.loaded.filter(|_| m.control.is_some())?;
        let (cell_w, cell_h, ox, oy) = (m.geom.cell_w, m.geom.cell_h, m.geom.origin_x, m.geom.origin_y);
        let (x0, x1) = (ox, ox + f64::from(p.width) * cell_w);
        let (y0, y1) = (oy - cell_h, oy + f64::from(p.height) * cell_h);
        let l = if p.left == 0 { x0 - 4. } else { x0 - cell_w / 2. + 3. };
        let r = if p.left + p.width >= m.window_cells.0 { x1 + 4. } else { x1 + cell_w / 2. - 3. };
        let top = if p.top <= 1 { y0 - 2. } else { y0 + 3. };
        let bot = if p.top + p.height >= m.window_cells.1 { y1 + 2. } else { y1 - 3. };
        let (l, r, top) = (l.max(2.), r.min(width - 2.), top.max(2.));
        let rgb = Self::tile_rgb(m, p.id);
        Some(((l, top, (r - l).max(0.), (bot - top).max(0.)), rgb))
    }
    /// The pane with the keys is a shade lighter than the others.
    fn tile_rgb(m: &Model, pane: u32) -> [u8; 3] {
        let (fg, bg) = (m.engine.palette().fg, m.engine.palette().bg);
        let active = m.slots.is_empty() || m.key_pane == Some(pane);
        mix(bg, fg, if active { 0.07 } else { 0.035 })
    }
    /// Paints a window of several panes: each one with its own engine, inside
    /// its own rectangle, and thin lines where tmux keeps a column between two.
    fn draw_panes(&self, m: &mut Model, context: &cairo::Context, all: bool, (w, h): (f64, f64)) {
        let Some(keep) = m.loaded.map(|p| p.id) else { return };
        let mut ids: Vec<u32> = std::iter::once(keep).chain(m.slots.iter().map(|s| s.info.id)).collect();
        if !all {
            ids.retain(|id| m.stale.contains(id));
        }
        let (focused, key) = (m.focused, m.key_pane);
        // Among several panes the cursor does not blink: a pane painted in
        // the dark half would keep it missing until its next output.
        let blink = std::mem::replace(&mut m.blink_visible, true);
        if all {
            let bg = m.engine.palette().bg;
            context.set_source_rgba(
                f64::from(bg[0]) / 255.,
                f64::from(bg[1]) / 255.,
                f64::from(bg[2]) / 255.,
                m.opacity,
            );
            context.set_operator(cairo::Operator::Source);
            let _ = context.paint();
            context.set_operator(cairo::Operator::Over);
        }
        for id in ids {
            if !Self::load_pane(m, id) {
                continue;
            }
            let Some(p) = m.loaded else { continue };
            let (x0, y0, x1, y1) = Self::pane_rect(m, &p, (w, h));
            let _ = context.save();
            context.rectangle(x0, y0, x1 - x0, y1 - y0);
            context.clip();
            // Only the pane with the keys shows a solid cursor.
            m.focused = focused && key == Some(id);
            let look = self.prepare(m, None);
            self.draw_frame(m, context, look, None);
            let _ = context.restore();
        }
        m.focused = focused;
        m.blink_visible = blink;
        Self::load_pane(m, keep);
        m.stale.clear();
    }
    /// The part of the widget that belongs to a pane: its text, the header
    /// row above it and half of each gap to its neighbours.
    fn pane_rect(m: &Model, p: &super::control::PaneInfo, (w, h): (f64, f64)) -> (f64, f64, f64, f64) {
        let (cell_w, cell_h, top, base_x) = (m.geom.cell_w, m.geom.cell_h, m.padding + 14., m.base_x);
        let (right_edge, bottom_edge) = m.window_cells;
        let x0 = if p.left == 0 { 0. } else { base_x + (f64::from(p.left) - 0.5) * cell_w };
        let x1 = if p.left + p.width >= right_edge { w } else { base_x + (f64::from(p.left + p.width) + 0.5) * cell_w };
        let y0 = if p.top <= 1 { 0. } else { top + f64::from(p.top - 1) * cell_h };
        let y1 = if p.top + p.height >= bottom_edge { h } else { top + f64::from(p.top + p.height) * cell_h };
        (x0, y0, x1, y1)
    }
    /// Runs at the start of a GTK frame, before it paints: reads the screen
    /// and asks GTK to repaint only the rows that changed. Typing, a spinner
    /// or the cursor blink then cost a few rows instead of the whole terminal.
    fn plan_frame(&self) {
        if self.closed.get() {
            return;
        }
        let mut m = self.model.borrow_mut();
        if !self.area.is_mapped() {
            m.schedule.reset_queued_paint();
            return;
        }
        if m.hold.is_some() {
            // The held picture is replaced in one go by `release_hold`.
            return;
        }
        if !m.slots.is_empty() {
            // Several panes: `draw` paints the ones that changed, and only
            // their part of the window is sent to the display again.
            let now = self.ms();
            m.schedule.painted(now);
            let size = (f64::from(self.area.allocated_width()), f64::from(self.area.allocated_height()));
            let parts: Vec<(f64, f64, f64, f64)> = if self.full.get() || m.glide_frac != 0. {
                Vec::new()
            } else {
                m.loaded
                    .iter()
                    .chain(m.slots.iter().map(|s| &s.info))
                    .filter(|p| m.stale.contains(&p.id))
                    .map(|p| Self::pane_rect(&m, p, size))
                    .collect()
            };
            drop(m);
            if parts.is_empty() {
                self.area.queue_draw();
            }
            for (x0, y0, x1, y1) in parts {
                let (x, y) = (x0.floor() as i32, y0.floor() as i32);
                self.area.queue_draw_area(x, y, x1.ceil() as i32 - x, y1.ceil() as i32 - y);
            }
            return;
        }
        let scale = self.area.scale_factor().max(1);
        let (w, h) = (self.area.allocated_width().max(1), self.area.allocated_height().max(1));
        let fits = self
            .canvas
            .borrow()
            .as_ref()
            .is_some_and(|c| c.width() == w * scale && c.height() == h * scale);
        if !fits || self.full.get() || self.planned.borrow().is_some() {
            drop(m);
            self.redraw();
            return;
        }
        let lines = m.schedule.pending_lines();
        // The frame cache matches the last picture: only the changed lines are read again.
        let known = m.painted.as_ref().is_some_and(|p| {
            p.fresh && p.size == m.engine.size() && p.geom == m.geom && p.palette == *m.engine.palette()
        });
        let look = self.prepare(&mut m, known.then_some(lines.as_slice()));
        let rows = changed_rows(&m, &look, &lines);
        let now = self.ms();
        m.schedule.painted(now);
        match &rows {
            Some(only) => {
                for (y, h) in row_bands(only, &m.geom) {
                    self.area.queue_draw_area(0, y, w, h);
                }
            }
            None => self.area.queue_draw(),
        }
        *self.planned.borrow_mut() = Some(Planned { look, rows });
    }
    /// Reads the screen into the frame cache (`lines`: only those changed) and
    /// works out everything the picture depends on besides the text of each row.
    fn prepare(&self, m: &mut Model, lines: Option<&[usize]>) -> FrameLook {
        let refresh_at = Instant::now();
        let fresh = {
            let Model {
                engine,
                frame,
                geom,
                ..
            } = &mut *m;
            match lines {
                Some(lines) => frame.refresh_lines(engine, geom, lines),
                None => frame.refresh(engine, geom),
            }
        };
        super::lag::phase("refresh", refresh_at);
        let look_at = Instant::now();
        // Panes as this frame shows them. tmux's own report arrives later
        // while the window or a division moves, so it only names them.
        let tiles = {
            let rows = m.engine.size().1;
            let found = m.frame.tiles(rows.saturating_sub(m.status_rows));
            if !found.is_empty() {
                m.held_tiles = Some((found.clone(), Instant::now()));
                found
            } else {
                match &m.held_tiles {
                    Some((held, at)) if !m.tmux_regions.is_empty() && at.elapsed() < Duration::from_millis(500) => {
                        held.clone()
                    }
                    _ => found,
                }
            }
        };
        if tiles.is_empty() {
            if m.regions != m.tmux_regions {
                m.regions = m.tmux_regions.clone();
                m.pane_ids = m.tmux_ids.clone();
            }
        } else {
            let live: Vec<super::noise::Region> = tiles
                .iter()
                .filter(|t| t.height > 1)
                .map(|t| super::noise::Region {
                    left: t.left,
                    width: t.width,
                    top: t.top.saturating_add(1),
                    height: t.height - 1,
                })
                .collect();
            let ids: Vec<String> = live
                .iter()
                .map(|r| {
                    let overlap = |o: &super::noise::Region| {
                        let w = (r.left + r.width).min(o.left + o.width).saturating_sub(r.left.max(o.left));
                        let h = (r.top + r.height).min(o.top + o.height).saturating_sub(r.top.max(o.top));
                        u32::from(w) * u32::from(h)
                    };
                    m.tmux_regions
                        .iter()
                        .enumerate()
                        .max_by_key(|(_, o)| overlap(o))
                        .and_then(|(i, _)| m.tmux_ids.get(i).cloned())
                        .unwrap_or_default()
                })
                .collect();
            m.regions = live;
            m.pane_ids = ids;
        }
        // A pane scrolled back in tmux copy mode is shown as tmux draws it, so
        // scrolling feels normal; only the live bottom is filtered.
        let copy_positions: Vec<(super::noise::Region, u32)> = {
            let mut render = RowRender::default();
            m.regions
                .iter()
                .filter_map(|r| {
                    m.engine.render_line(usize::from(r.top), &mut render);
                    let top = super::pick::cells_of(&render, r.left, r.width);
                    super::pick::copy_position(&top).filter(|p| *p > 0).map(|p| (*r, p))
                })
                .collect()
        };
        // A scrolled pane that lost its input box (Claude exited, the pane
        // moved) goes back to what tmux shows.
        if !m.scrolls.is_empty() {
            let lost: Vec<String> = m
                .scrolls
                .keys()
                .filter(|id| {
                    let region = m.pane_ids.iter().position(|p| p == *id).and_then(|i| m.regions.get(i));
                    region.is_none_or(|r| {
                        copy_positions.iter().any(|(x, _)| x == r)
                            || super::noise::input_top(m.frame.inks(), r).is_none()
                    })
                })
                .cloned()
                .collect();
            for id in lost {
                m.scrolls.remove(&id);
                m.fills.remove(&scroll_key(&id));
            }
        }
        let maps: Vec<(super::noise::Region, Vec<Option<u16>>)> = if m.hide_noise {
            let (cols, rows) = m.engine.size();
            let whole = [super::noise::Region { left: 0, width: cols, top: 0, height: rows }];
            let regions: &[super::noise::Region] = if m.regions.is_empty() { &whole } else { &m.regions };
            regions
                .iter()
                .filter(|r| !copy_positions.iter().any(|(x, _)| x == *r))
                .filter_map(|r| match scrolled_input(m, r) {
                    Some(input) => Some((*r, super::noise::scrolled_map(r, input))),
                    None => super::noise::compact(m.frame.inks(), r).map(|map| (*r, map)),
                })
                .collect()
        } else {
            Vec::new()
        };
        m.copy_pos = copy_positions;
        // Rows freed by hidden noise show earlier history instead of nothing.
        let mut wanted = Vec::new();
        let mut fills = Vec::new();
        for (r, map) in &maps {
            let holes = super::noise::holes(map, r);
            if holes == 0 {
                continue;
            }
            let id = if m.regions.is_empty() {
                Some(String::new())
            } else {
                m.regions
                    .iter()
                    .position(|x| x == r)
                    .and_then(|i| m.pane_ids.get(i).cloned())
            };
            let Some(id) = id else { continue };
            if let Some(scroll) = m.scrolls.get(&id)
                && let Some(snap) = m.fills.get(&scroll_key(&id)).filter(|f| f.fits(m, r))
            {
                fills.push(FillDraw {
                    id: scroll_key(&id),
                    region: *r,
                    holes,
                    skip: scroll.back,
                    count: snap.shown(scroll.back, usize::from(holes)).len(),
                    end: snap.end,
                    asked_ms: snap.asked_ms,
                });
                continue;
            }
            let want = (holes.saturating_mul(3).saturating_add(40)).min(MAX_FILL_LINES);
            // The pane scrolled since its fill was read: the lines that left
            // the screen are in the fill, which only has to move with it.
            let top = usize::from(r.top);
            let moved = lines.is_none_or(|lines| lines.iter().any(|l| (top..top + ALIGN_ROWS).contains(l)));
            if fresh
                && m.fills
                    .get(&id)
                    .is_some_and(|f| f.screen > 0 && f.fits(m, r) && (moved || !f.aligned || !f.current(m.shown_ms)))
            {
                align_fill(m, &id, r, self.ms());
            }
            let Some(fill) = m
                .fills
                .get(&id)
                .filter(|f| f.geom == m.geom && f.left == r.left && f.width == r.width)
            else {
                wanted.push((id, want));
                continue;
            };
            if fill.asked_ms < m.scroll_ms || !fill.current(m.shown_ms) {
                // tmux scrolled since this was read, or it is a copy from the
                // last time this tab was open that no longer matches the
                // screen: better empty than repeated or wrong.
                wanted.push((id.clone(), want.max(fill.lines)));
                continue;
            }
            let short = fill.end < usize::from(holes) && fill.more && fill.lines < MAX_FILL_LINES;
            if short {
                wanted.push((id.clone(), fill.lines.saturating_mul(2).min(MAX_FILL_LINES)));
            } else if fill.asked_ms < m.output_ms && !(fill.sure && fill.asked_ms + FILL_TRUST_MS >= m.output_ms) {
                wanted.push((id.clone(), want.max(fill.lines)));
            }
            fills.push(FillDraw {
                id,
                region: *r,
                holes,
                skip: 0,
                count: fill.end.min(usize::from(holes)),
                end: fill.end,
                asked_ms: fill.asked_ms,
            });
        }
        m.fill_wanted = wanted;
        super::lag::phase("look", look_at);
        FrameLook {
            fresh,
            plain: m.selection.is_none()
                && m.pick.is_none()
                && m.preedit.is_empty()
                && m.engine.engine().display_offset() == 0,
            size: m.engine.size(),
            area: (self.area.allocated_width(), self.area.allocated_height()),
            geom: m.geom,
            opacity: m.opacity,
            palette: m.engine.palette().clone(),
            tiles,
            maps,
            fills,
            hovered_link: m.hovered_link,
            status_rows: m.status_rows,
        }
    }
    /// Paints `look` from the frame cache. With `only`, just those screen
    /// rows are painted again and the rest of the picture is left as it is.
    fn draw_frame(&self, m: &mut Model, context: &cairo::Context, look: FrameLook, only: Option<&[bool]>) {
        let mut painter_slot = self.painter.borrow_mut();
        if painter_slot
            .as_ref()
            .is_none_or(|p| p.dpr != m.geom.dpr)
        {
            let layout = self.area.create_pango_layout(None);
            let mut painter = TextPainter::new(&layout, &m.font);
            painter.dpr = m.geom.dpr;
            *painter_slot = Some(painter);
        }
        let Some(text_painter) = painter_slot.as_mut() else {
            return;
        };
        let (_, rows) = m.engine.size();
        let (cell_w, cell_h, ox, oy) = (m.geom.cell_w, m.geom.cell_h, m.geom.origin_x, m.geom.origin_y);
        let width = f64::from(self.area.allocated_width());
        let height = f64::from(self.area.allocated_height());
        let ops_at = Instant::now();
        let _ = context.save();
        if let Some(only) = only {
            for (y, h) in row_bands(only, &m.geom) {
                context.rectangle(0., f64::from(y), width, f64::from(h));
            }
            context.clip();
        }
        // Rows inside the repainted bands, and rows whose glyphs may reach into them.
        let in_band = |row: usize| only.is_none_or(|o| o.get(row).copied().unwrap_or(false));
        let near_band = |row: usize| in_band(row) || in_band(row + 1) || row.checked_sub(1).is_some_and(in_band);
        let bg = m.engine.palette().bg;
        context.set_source_rgba(
            f64::from(bg[0]) / 255.,
            f64::from(bg[1]) / 255.,
            f64::from(bg[2]) / 255.,
            m.opacity,
        );
        context.set_operator(cairo::Operator::Source);
        let _ = context.paint();
        context.set_operator(cairo::Operator::Over);
        let maps = &look.maps;
        let rect_of = |r: &super::noise::Region| {
            let x0 = if r.left == 0 { 0. } else { ox + f64::from(r.left) * cell_w };
            let x1 = ox + f64::from(r.left.saturating_add(r.width)) * cell_w;
            (x0, oy + f64::from(r.top) * cell_h, x1 - x0, f64::from(r.height) * cell_h)
        };
        // Losas: one tile per tmux pane, found on this very frame.
        let tile_rects: Vec<((f64, f64, f64, f64), [u8; 3])> = {
            let (cols, rows) = m.engine.size();
            let bottom = rows.saturating_sub(m.status_rows);
            let (fg, bg) = (m.engine.palette().fg, m.engine.palette().bg);
            look.tiles
                .iter()
                .map(|t| {
                    let x0 = ox + f64::from(t.left) * cell_w;
                    let x1 = ox + f64::from(t.left.saturating_add(t.width)) * cell_w;
                    let y0 = oy + f64::from(t.top) * cell_h;
                    let y1 = oy + f64::from(t.top.saturating_add(t.height)) * cell_h;
                    let l = if t.left == 0 { x0 - 4. } else { x0 - cell_w / 2. + 3. };
                    let r = if t.left.saturating_add(t.width) >= cols { x1 + 4. } else { x1 + cell_w / 2. - 3. };
                    let top = if t.top == 0 { y0 - 2. } else { y0 + 3. };
                    let bot = if t.top.saturating_add(t.height) >= bottom { y1 + 2. } else { y1 - 3. };
                    let (l, r) = (l.max(2.), r.min(width - 2.));
                    let rgb = mix(bg, fg, if t.active { 0.07 } else { 0.035 });
                    ((l, top.max(2.), (r - l).max(0.), (bot - top.max(2.)).max(0.)), rgb)
                })
                .collect()
        };
        // A pane the app draws itself has no border cells to find: its
        // tile comes from where tmux says the pane is.
        let mut tile_rects = tile_rects;
        if let Some((rect, rgb)) = Self::native_tile(m, width) {
            tile_rects.push((rect, rgb));
        }
        // Tiles are solid: letters never show the window behind, and empty
        // rows look the same as rows with text (no stripes that move with
        // the output). The window's transparency stays in the gaps.
        for ((x, y, w, h), rgb) in &tile_rects {
            rounded_rect(context, *x, *y, *w, *h, 7.);
            color(context, *rgb);
            let _ = context.fill();
        }
        let hide_borders = !look.tiles.is_empty();
        // Without tiles, rows moved by the noise filter need their own solid
        // background (see the text bands below).
        let solid_rows = m.opacity < 1. && tile_rects.is_empty();
        let _ = context.save();
        if !maps.is_empty() {
            // Everything except the filtered panes is painted as usual.
            context.rectangle(0., 0., width, height);
            for (r, _) in maps {
                let (x, y, w, h) = rect_of(r);
                context.rectangle(x, y, w, h);
            }
            context.set_fill_rule(cairo::FillRule::EvenOdd);
            context.clip();
        }
        if solid_rows {
            // The window behind must never show through letters: rows that
            // carry text get a solid band; empty rows keep the transparency.
            color(context, m.engine.palette().bg);
            for (y, h) in &text_bands(m.frame.ops(), cell_h) {
                context.rectangle(0., *y, width, *h);
            }
            let _ = context.fill();
        }
        for row in (0..usize::from(rows)).filter(|row| near_band(*row)) {
            for op in m.frame.row_ops(row) {
                if hide_borders && is_border_op(op) {
                    continue;
                }
                paint_op(context, text_painter, &m.geom, op);
            }
        }
        let _ = context.restore();
        for (r, map) in maps {
            let (x, _, w, _) = rect_of(r);
            for (i, shown) in map.iter().enumerate() {
                let Some(shown) = *shown else { continue };
                if !in_band(usize::from(shown)) {
                    continue;
                }
                let grid = usize::from(r.top) + i;
                let row_ops = m.frame.row_ops(grid);
                let _ = context.save();
                context.rectangle(x, oy + f64::from(shown) * cell_h, w, cell_h);
                context.clip();
                if solid_rows && !row_ops.is_empty() {
                    color(context, m.engine.palette().bg);
                    let _ = context.paint();
                }
                context.translate(0., (f64::from(shown) - grid as f64) * cell_h);
                for op in row_ops {
                    paint_op(context, text_painter, &m.geom, op);
                }
                let _ = context.restore();
            }
        }
        // Rows freed by hidden noise show earlier history instead of nothing.
        for draw in &look.fills {
            let Some(fill) = m.fills.get(&draw.id) else { continue };
            let (x, _, w, _) = rect_of(&draw.region);
            let shown = fill.shown(draw.skip, usize::from(draw.holes));
            let count = shown.len();
            let first = usize::from(draw.region.top.saturating_add(draw.holes));
            for (k, &j) in shown.iter().enumerate() {
                let Some(shown) = first.checked_sub(count - k).filter(|row| in_band(*row)) else {
                    continue;
                };
                let row_ops = fill.frame.row_ops(j);
                let _ = context.save();
                context.rectangle(x, oy + shown as f64 * cell_h, w, cell_h);
                context.clip();
                if solid_rows && !row_ops.is_empty() {
                    color(context, m.engine.palette().bg);
                    let _ = context.paint();
                }
                context.translate(0., (shown as f64 - j as f64) * cell_h);
                for op in row_ops {
                    paint_op(context, text_painter, &m.geom, op);
                }
                let _ = context.restore();
            }
        }
        // A pane scrolled back by the app shows no new output: say so on the
        // row right above its input box.
        for draw in look.fills.iter().filter(|d| d.skip > 0 && d.holes > 0) {
            let row = usize::from(draw.region.top.saturating_add(draw.holes - 1));
            if !in_band(row) {
                continue;
            }
            let text = format!(" historial · {} líneas atrás · cualquier tecla vuelve ", draw.skip);
            let (x, _, w, _) = rect_of(&draw.region);
            let badge_w = (text.chars().count() as f64 * cell_w).min(w);
            let (badge_x, badge_y) = (x + w - badge_w, oy + row as f64 * cell_h);
            let _ = context.save();
            context.rectangle(badge_x, badge_y, badge_w, cell_h);
            context.clip();
            color(context, m.engine.palette().selection);
            let _ = context.paint();
            color(context, m.engine.palette().fg);
            text_painter.show(context, &text, false, false, badge_x, badge_y);
            let _ = context.restore();
        }
        super::lag::phase("ops", ops_at);
        let over_at = Instant::now();
        // Where a grid row ended up on screen (hidden rows: None).
        let shown_row = |col: u16, row: u16| -> Option<f64> {
            match maps
                .iter()
                .find(|(r, _)| col >= r.left && col < r.left.saturating_add(r.width) && row >= r.top)
            {
                Some((r, map)) => map
                    .get(usize::from(row - r.top))
                    .copied()
                    .unwrap_or(Some(row))
                    .map(f64::from),
                None => Some(f64::from(row)),
            }
        };
        if let Some((start, end)) = m.hovered_link {
            let offset = i32::try_from(m.engine.engine().display_offset()).unwrap_or(i32::MAX);
            let (cols, rows) = m.engine.size();
            let rgb = m.engine.palette().fg;
            context.set_source_rgba(
                f64::from(rgb[0]) / 255.,
                f64::from(rgb[1]) / 255.,
                f64::from(rgb[2]) / 255.,
                0.65,
            );
            context.set_line_width(1.);
            for (row, col, cells) in super::hover::lines(start, end, cols, rows, offset) {
                let Some(row) = shown_row(col, row) else { continue };
                let x = m.geom.origin_x + f64::from(col) * m.geom.cell_w;
                let y = m.geom.origin_y + (row + 1.) * m.geom.cell_h - 1.5;
                context.move_to(x, y);
                context.line_to(x + f64::from(cells) * m.geom.cell_w, y);
            }
            let _ = context.stroke();
        }
        if let Some(selection) = m.selection
            && let Some((start, end)) = selection_bounds(m.engine.engine(), &selection)
        {
            let offset = i32::try_from(m.engine.engine().display_offset()).unwrap_or(i32::MAX);
            color(context, m.engine.palette().selection);
            for row in 0..rows {
                let absolute = i32::from(row).saturating_sub(offset);
                if absolute < start.0 || absolute > end.0 {
                    continue;
                }
                let (cols, _) = m.engine.size();
                let (left, right) = if selection.mode == SelectMode::Block {
                    (start.1, end.1)
                } else {
                    (
                        if absolute == start.0 { start.1 } else { 0 },
                        if absolute == end.0 {
                            end.1
                        } else {
                            cols.saturating_sub(1)
                        },
                    )
                };
                let Some(shown) = shown_row(left, row) else { continue };
                context.rectangle(
                    m.geom.origin_x + f64::from(left) * m.geom.cell_w,
                    m.geom.origin_y + shown * m.geom.cell_h,
                    f64::from(right.saturating_sub(left).saturating_add(1)) * m.geom.cell_w,
                    m.geom.cell_h,
                );
                context.set_source_rgba(
                    f64::from(m.engine.palette().selection[0]) / 255.,
                    f64::from(m.engine.palette().selection[1]) / 255.,
                    f64::from(m.engine.palette().selection[2]) / 255.,
                    0.35,
                );
                let _ = context.fill();
            }
        }
        m.noise_maps = maps.clone();
        if let Some(mut pick) = m.pick {
            let lines = region_lines(m, &pick.region);
            if pick.held
                && let Some(head) = pick.spot_at_pointer(&lines)
            {
                // tmux scrolled under a held drag: the pointer now covers other text.
                pick.head = head;
                pick.dragged |= head != pick.anchor;
                m.pick = Some(pick);
            }
        }
        if let Some(pick) = m.pick.filter(|p| p.dragged) {
            let lines = region_lines(m, &pick.region);
            let sel = m.engine.palette().selection;
            context.set_source_rgba(
                f64::from(sel[0]) / 255.,
                f64::from(sel[1]) / 255.,
                f64::from(sel[2]) / 255.,
                0.35,
            );
            for (row, from, to) in pick.spans(&lines) {
                context.rectangle(
                    ox + f64::from(from) * cell_w,
                    oy + f64::from(row) * cell_h,
                    f64::from(to.saturating_sub(from).saturating_add(1)) * cell_w,
                    cell_h,
                );
            }
            let _ = context.fill();
        }
        let cursor = m.frame.cursor();
        let blink = m
            .preferences
            .get("cursor_blink")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let mut painted_cursor = None;
        if cursor.visible && (!blink || !m.focused || m.blink_visible) {
            let x = m.geom.origin_x + f64::from(cursor.col) * m.geom.cell_w;
            let line = u16::try_from(cursor.line).unwrap_or(u16::MAX);
            let shown = shown_row(cursor.col, line).unwrap_or(f64::from(line));
            painted_cursor = Some(shown as u16);
            let y = m.geom.origin_y + shown * m.geom.cell_h;
            let width = m.geom.cell_w * if cursor.wide { 2. } else { 1. };
            color(context, m.engine.palette().cursor);
            let shape = comandos_term::render::preferred_cursor_shape(
                cursor.shape,
                Some(m.cursor_shape),
                m.focused,
            );
            {
                match shape {
                    CursorShape::Beam => context.rectangle(x, y, 1., m.geom.cell_h),
                    CursorShape::Underline => {
                        context.rectangle(x, y + m.geom.cell_h - 2., width, 2.)
                    }
                    CursorShape::Block => cursor_block(context, x, y, width, m.geom.cell_h),
                    CursorShape::HollowBlock => {
                        context.rectangle(x + 0.5, y + 0.5, width - 1., m.geom.cell_h - 1.);
                        context.set_line_width(1.);
                        let _ = context.stroke();
                    }
                    CursorShape::Hidden => {}
                }
                let _ = context.fill();
                if shape == CursorShape::Block {
                    // The letter under the cursor, in the cursor's contrast color.
                    let _ = context.save();
                    context.rectangle(x, y, width, m.geom.cell_h);
                    context.clip();
                    context.translate(0., (shown - f64::from(line)) * m.geom.cell_h);
                    for op in m.frame.row_ops(cursor.line) {
                        let mut op = op.clone();
                        match &mut op {
                            PaintOp::Text {
                                col,
                                cells,
                                rgb,
                                ..
                            } if *col <= cursor.col && cursor.col < col.saturating_add(*cells) => {
                                *rgb = m.engine.palette().cursor_accent;
                                paint_op(context, text_painter, &m.geom, &op);
                            }
                            PaintOp::Glyph {
                                x: cell_x,
                                rgb,
                                ..
                            } if *cell_x == x => {
                                *rgb = m.engine.palette().cursor_accent;
                                paint_op(context, text_painter, &m.geom, &op);
                            }
                            _ => {}
                        }
                    }
                    let _ = context.restore();
                }
            }
        }
        // A program that hides the terminal's cursor and draws its own as one
        // cell in reverse video gets the same cursor as everything else.
        if !cursor.visible
            && m.focused
            && let Some((col, row, wide)) = m.engine.program_cursor()
            && let Some(shown) = shown_row(col, row)
        {
            let (x, y) = (ox + f64::from(col) * cell_w, oy + shown * cell_h);
            let width = cell_w * if wide { 2. } else { 1. };
            let under = tile_rects
                .iter()
                .find(|((tx, ty, tw, th), _)| x >= *tx && x < tx + tw && y >= *ty && y < ty + th)
                .map_or(m.engine.palette().bg, |(_, rgb)| *rgb);
            let _ = context.save();
            context.rectangle(x, y, width, cell_h);
            context.clip();
            color(context, under);
            let _ = context.paint();
            color(context, m.engine.palette().cursor);
            cursor_block(context, x, y, width, cell_h);
            let _ = context.fill();
            context.translate(0., (shown - f64::from(row)) * cell_h);
            for op in m.frame.row_ops(usize::from(row)) {
                let mut op = op.clone();
                match &mut op {
                    PaintOp::Text { col: from, cells, rgb, .. } if *from <= col && col < from.saturating_add(*cells) => {
                        *rgb = m.engine.palette().cursor_accent;
                        paint_op(context, text_painter, &m.geom, &op);
                    }
                    PaintOp::Glyph { x: cell_x, rgb, .. } if *cell_x == x => {
                        *rgb = m.engine.palette().cursor_accent;
                        paint_op(context, text_painter, &m.geom, &op);
                    }
                    _ => {}
                }
            }
            let _ = context.restore();
        }
        if !m.preedit.is_empty() {
            let layout = self.area.create_pango_layout(None);
            layout.set_font_description(Some(&m.font));
            layout.set_text(&m.preedit);
            color(context, m.engine.palette().fg);
            context.move_to(
                m.geom.origin_x + f64::from(cursor.col) * m.geom.cell_w,
                m.geom.origin_y + cursor.line as f64 * m.geom.cell_h,
            );
            pangocairo::functions::show_layout(context, &layout);
        }
        let _ = context.restore();
        super::lag::phase("over", over_at);
        m.painted_cursor = painted_cursor;
        m.painted = Some(look);
    }
}

/// Screen rows to paint again when only `lines` changed since the picture
/// `m.painted` was made; `None` when the whole picture has to be painted.
fn changed_rows(m: &Model, look: &FrameLook, lines: &[usize]) -> Option<Vec<bool>> {
    if !(look.plain && look.fresh) || m.painted.as_ref() != Some(look) {
        return None;
    }
    let rows = usize::from(look.size.1);
    let mut dirty = vec![false; rows];
    let cursor = m.frame.cursor();
    for &line in lines.iter().chain([&cursor.line]) {
        if let Some(row) = dirty.get_mut(line) {
            *row = true;
        }
        // The noise filter may show the line on another row of its pane.
        for (r, map) in &look.maps {
            let shown = line
                .checked_sub(usize::from(r.top))
                .and_then(|i| map.get(i).copied().flatten());
            if let Some(row) = shown.and_then(|row| dirty.get_mut(usize::from(row))) {
                *row = true;
            }
        }
    }
    if let Some(row) = m.painted_cursor.and_then(|row| dirty.get_mut(usize::from(row))) {
        *row = true;
    }
    // Glyphs may reach a little into the rows next to theirs.
    let only: Vec<bool> = (0..rows)
        .map(|i| dirty[i] || (i > 0 && dirty[i - 1]) || dirty.get(i + 1).copied().unwrap_or(false))
        .collect();
    (only.iter().filter(|on| **on).count() * 5 <= rows * 3).then_some(only)
}

/// Pixel bands `(y, height)` that cover the marked rows, joined when they touch.
fn row_bands(rows: &[bool], g: &CellGeom) -> Vec<(i32, i32)> {
    let mut bands: Vec<(i32, i32)> = Vec::new();
    for (i, _) in rows.iter().enumerate().filter(|(_, on)| **on) {
        let top = (g.origin_y + i as f64 * g.cell_h).floor() as i32;
        let bottom = (g.origin_y + (i + 1) as f64 * g.cell_h).ceil() as i32;
        match bands.last_mut() {
            Some((y, h)) if *y + *h >= top => *h = bottom - *y,
            _ => bands.push((top, bottom - top)),
        }
    }
    bands
}

/// Reuse shaping within one frame while retaining the existing cell positions.
/// Frame-local ownership also makes font, scale and Pango context changes take
/// effect immediately. Unusual Unicode output cannot grow the cache unbounded.
/// Vertical bands (y, height) of rows that contain visible text.
/// `a` moved `t` of the way toward `b`.
fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let lerp = |x: u8, y: u8| (f64::from(x) + (f64::from(y) - f64::from(x)) * t).round().clamp(0., 255.) as u8;
    [lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2])]
}

/// The block cursor: a little shorter than its row, with soft corners.
fn cursor_block(cr: &cairo::Context, x: f64, y: f64, w: f64, cell_h: f64) {
    let inset = (cell_h * 0.1).floor();
    rounded_rect(cr, x, y + inset, w, cell_h - 2. * inset, 2.5);
}

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.).min(h / 2.).max(0.);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.);
    cr.arc(x + w - r, y + h - r, r, 0., std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
}

/// tmux's own border lines and title text: the tiles replace them.
fn is_border_op(op: &PaintOp) -> bool {
    match op {
        PaintOp::Text { rgb, .. } | PaintOp::Glyph { rgb, .. } => super::tiles::is_border(*rgb),
        _ => false,
    }
}

fn text_bands(ops: &[PaintOp], cell_h: f64) -> Vec<(f64, f64)> {
    let mut rows: Vec<f64> = ops
        .iter()
        .filter_map(|op| match op {
            PaintOp::Text { y, text, .. } if !text.trim().is_empty() => Some(*y),
            PaintOp::Glyph { y, .. } => Some(*y),
            _ => None,
        })
        .collect();
    rows.sort_by(f64::total_cmp);
    rows.dedup();
    let mut bands: Vec<(f64, f64)> = Vec::new();
    for y in rows {
        match bands.last_mut() {
            Some((top, h)) if (*top + *h - y).abs() < 0.5 => *h += cell_h,
            _ => bands.push((y, cell_h)),
        }
    }
    bands
}

/// One run of text already shaped into font glyphs. Each glyph keeps its
/// column so the run stays on the cell grid whatever `cell_w` is.
struct ShapedRun {
    pieces: Vec<(cairo::ScaledFont, Vec<GridGlyph>)>,
}
/// Glyph index, column, x offset inside the cell and y from the cell top.
type GridGlyph = (u64, u32, f64, f64);

struct TextPainter {
    template: pango::Layout,
    fonts: [pango::FontDescription; 4],
    layouts: [std::collections::HashMap<String, pango::Layout>; 4],
    entries: usize,
    /// Grid runs: `None` marks text the fast path cannot draw (missing glyphs).
    runs: [std::collections::HashMap<String, Option<Rc<ShapedRun>>>; 4],
    run_entries: usize,
    attrs: pango::AttrList,
    glyphs: Vec<cairo::Glyph>,
    dpr: f64,
}
impl TextPainter {
    const MAX_LAYOUTS: usize = 512;
    const MAX_RUNS: usize = 8192;

    fn new(template: &pango::Layout, font: &pango::FontDescription) -> Self {
        // Per-cell drawing never formed ligatures; keep that look.
        let attrs = pango::AttrList::new();
        attrs.insert(pango::AttrFontFeatures::new("liga 0, calt 0, clig 0, dlig 0"));
        Self {
            template: template.clone(),
            fonts: std::array::from_fn(|style| {
                let mut font = font.clone();
                font.set_weight(if style & 1 != 0 {
                    pango::Weight::Bold
                } else {
                    pango::Weight::Normal
                });
                font.set_style(if style & 2 != 0 {
                    pango::Style::Italic
                } else {
                    pango::Style::Normal
                });
                font
            }),
            layouts: std::array::from_fn(|_| std::collections::HashMap::new()),
            entries: 0,
            runs: std::array::from_fn(|_| std::collections::HashMap::new()),
            run_entries: 0,
            attrs,
            glyphs: Vec::new(),
            dpr: 1.,
        }
    }

    fn show(
        &mut self,
        context: &cairo::Context,
        text: &str,
        bold: bool,
        italic: bool,
        x: f64,
        y: f64,
    ) {
        let style = usize::from(bold) | (usize::from(italic) << 1);
        if let Some(layout) = self.layouts.get(style).and_then(|cache| cache.get(text)) {
            context.move_to(x, y);
            pangocairo::functions::show_layout(context, layout);
            return;
        }
        let retain = self.entries < Self::MAX_LAYOUTS;
        let layout = if retain {
            self.template.copy()
        } else {
            self.template.clone()
        };
        layout.set_font_description(self.fonts.get(style));
        #[cfg(test)]
        native_paint_tests::TEXT_SHAPES.with(|count| count.set(count.get() + 1));
        layout.set_text(text);
        context.move_to(x, y);
        pangocairo::functions::show_layout(context, &layout);
        if retain {
            if let Some(cache) = self.layouts.get_mut(style) {
                cache.insert(text.to_string(), layout);
            }
            self.entries += 1;
        }
    }

    /// Shape `text` (one char per cell) once, then reuse it every frame.
    fn shape_grid(&self, text: &str, style: usize) -> Option<ShapedRun> {
        use glib::prelude::Cast;
        use pangocairo::prelude::PangoCairoFontExt as _;
        let layout = self.template.copy();
        layout.set_font_description(self.fonts.get(style));
        layout.set_attributes(Some(&self.attrs));
        layout.set_text(text);
        #[cfg(test)]
        native_paint_tests::TEXT_SHAPES.with(|count| count.set(count.get() + 1));
        if layout.line_count() != 1 {
            return None;
        }
        let scale = f64::from(pango::SCALE);
        let baseline = f64::from(layout.baseline()) / scale;
        let mut column = vec![0u32; text.len() + 1];
        for (index, (byte, _)) in text.char_indices().enumerate() {
            *column.get_mut(byte)? = u32::try_from(index).ok()?;
        }
        let line = layout.line_readonly(0)?;
        let mut pieces = Vec::new();
        for run in line.runs() {
            let item = run.item();
            let font = item.analysis().font();
            let scaled = font.dynamic_cast_ref::<pangocairo::Font>()?.scaled_font()?;
            let glyphs = run.glyph_string();
            let mut out = Vec::with_capacity(glyphs.glyph_info().len());
            let mut cluster = -1;
            let mut advance = 0.;
            for (info, &start) in glyphs.glyph_info().iter().zip(glyphs.log_clusters()) {
                if start != cluster {
                    cluster = start;
                    advance = 0.;
                }
                let glyph = info.glyph();
                let geometry = info.geometry();
                if glyph & pango::GLYPH_UNKNOWN_FLAG != 0 {
                    return None;
                }
                if glyph != pango::GLYPH_EMPTY {
                    let byte = usize::try_from(item.offset() + start).ok()?;
                    out.push((
                        u64::from(glyph),
                        *column.get(byte)?,
                        advance + f64::from(geometry.x_offset()) / scale,
                        baseline + f64::from(geometry.y_offset()) / scale,
                    ));
                }
                advance += f64::from(geometry.width()) / scale;
            }
            pieces.push((scaled, out));
        }
        Some(ShapedRun { pieces })
    }

    /// Draw a run whose chars each take one cell, in one cairo call per font.
    #[allow(clippy::too_many_arguments)]
    fn show_grid(
        &mut self,
        context: &cairo::Context,
        text: &str,
        bold: bool,
        italic: bool,
        x: f64,
        y: f64,
        cell_w: f64,
    ) -> bool {
        self.show_grid_at(context, text, bold, italic, x, y, cell_w)
            .unwrap_or(false)
    }

    #[allow(clippy::too_many_arguments)]
    fn show_grid_at(
        &mut self,
        context: &cairo::Context,
        text: &str,
        bold: bool,
        italic: bool,
        x: f64,
        y: f64,
        cell_w: f64,
    ) -> Option<bool> {
        let style = usize::from(bold) | (usize::from(italic) << 1);
        let shaped = match self.runs.get(style)?.get(text) {
            Some(found) => found.clone(),
            None => {
                if self.run_entries >= Self::MAX_RUNS {
                    self.runs.iter_mut().for_each(std::collections::HashMap::clear);
                    self.run_entries = 0;
                }
                let shaped = self.shape_grid(text, style).map(Rc::new);
                if let Some(cache) = self.runs.get_mut(style) {
                    cache.insert(text.to_string(), shaped.clone());
                }
                self.run_entries += 1;
                shaped
            }
        };
        let Some(shaped) = shaped else {
            return Some(false);
        };
        for (font, glyphs) in &shaped.pieces {
            self.glyphs.clear();
            self.glyphs.extend(glyphs.iter().map(|&(index, col, dx, dy)| {
                cairo::Glyph::new(index as _, x + f64::from(col) * cell_w + dx, y + dy)
            }));
            context.set_scaled_font(font);
            let _ = context.show_glyphs(&self.glyphs);
        }
        Some(true)
    }
}

fn paint_op(context: &cairo::Context, text_painter: &mut TextPainter, g: &CellGeom, op: &PaintOp) {
    match op {
        PaintOp::Rect { x, y, w, h, rgb } => {
            color(context, *rgb);
            context.rectangle(*x, *y, *w, *h);
            let _ = context.fill();
        }
        PaintOp::Text {
            x,
            y,
            text,
            cells,
            bold,
            italic,
            rgb,
            ..
        } => {
            color(context, *rgb);
            if *cells <= 2 && text.chars().count() != usize::from(*cells) {
                text_painter.show(context, text, *bold, *italic, *x, *y);
            } else if text.chars().count() == usize::from(*cells)
                && text_painter.show_grid(context, text, *bold, *italic, *x, *y, g.cell_w)
            {
            } else {
                super::lag::count_slow(text.chars().count());
                for (index, character) in text.chars().enumerate() {
                    text_painter.show(
                        context,
                        &character.to_string(),
                        *bold,
                        *italic,
                        *x + index as f64 * g.cell_w,
                        *y,
                    );
                }
            }
        }
        PaintOp::Line { x, y, w, rgb, kind } => {
            color(context, *rgb);
            let y = match kind {
                LineKind::Strike => *y + g.cell_h * 0.5,
                _ => *y + g.cell_h - 2.,
            };
            context.set_line_width(1.);
            let dash = match kind {
                LineKind::Under(Underline::Dotted) => vec![1., 2.],
                LineKind::Under(Underline::Dashed) => vec![4., 3.],
                _ => Vec::new(),
            };
            context.set_dash(&dash, 0.);
            context.move_to(*x, y);
            if *kind == LineKind::Under(Underline::Curly) {
                let mut at = *x;
                let step = (g.cell_w / 2.).max(2.);
                let mut sign = 1.;
                while at < *x + *w {
                    let end = (at + step).min(*x + *w);
                    context.curve_to(
                        at + (end - at) / 3.,
                        y + sign * 2.,
                        at + 2. * (end - at) / 3.,
                        y + sign * 2.,
                        end,
                        y,
                    );
                    at = end;
                    sign = -sign;
                }
            } else {
                context.line_to(*x + *w, y);
            }
            let _ = context.stroke();
            if *kind == LineKind::Under(Underline::Double) {
                context.move_to(*x, y - 2.);
                context.line_to(*x + *w, y - 2.);
                let _ = context.stroke();
            }
            context.set_dash(&[], 0.);
        }
        PaintOp::Glyph { x, y, ops, rgb } => {
            let _ = context.save();
            context.translate(*x, *y);
            context.scale(1. / g.dpr, 1. / g.dpr);
            color(context, *rgb);
            for op in ops {
                match op {
                    DrawOp::FillRect { x, y, w, h } => {
                        context.rectangle(*x, *y, *w, *h);
                        let _ = context.fill();
                    }
                    DrawOp::FillPattern { mask } => {
                        for y in 0..(g.cell_h * g.dpr).ceil() as usize {
                            for x in 0..(g.cell_w * g.dpr).ceil() as usize {
                                let on = mask
                                    .get(y % mask.len().max(1))
                                    .and_then(|row| row.get(x % row.len().max(1)))
                                    .copied()
                                    .unwrap_or(0);
                                if on != 0 {
                                    context.rectangle(x as f64, y as f64, 1., 1.);
                                }
                            }
                        }
                        let _ = context.fill();
                    }
                    DrawOp::ClipCell => {
                        context.rectangle(0., 0., g.cell_w * g.dpr, g.cell_h * g.dpr);
                        context.clip();
                    }
                    DrawOp::BeginPath => context.new_path(),
                    DrawOp::MoveTo { x, y } => context.move_to(*x, *y),
                    DrawOp::LineTo { x, y } => context.line_to(*x, *y),
                    DrawOp::CurveTo {
                        x1,
                        y1,
                        x2,
                        y2,
                        x,
                        y,
                    } => context.curve_to(*x1, *y1, *x2, *y2, *x, *y),
                    DrawOp::Stroke { line_width } => {
                        context.set_line_width(*line_width);
                        let _ = context.stroke();
                    }
                    DrawOp::Fill => {
                        let _ = context.fill();
                    }
                }
            }
            let _ = context.restore();
        }
    }
}

#[cfg(test)]
mod native_paint_tests {
    use super::*;
    thread_local! {
        pub(super) static TEXT_SHAPES: Cell<usize> = const { Cell::new(0) };
    }
    #[test]
    fn repeated_terminal_cells_are_shaped_once_per_frame() {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1200, 800).unwrap();
        let context = cairo::Context::new(&surface).unwrap();
        let layout = pangocairo::functions::create_layout(&context);
        let font = pango::FontDescription::from_string("monospace 13");
        let mut text_painter = TextPainter::new(&layout, &font);
        let geom = CellGeom {
            cell_w: 8.,
            cell_h: 19.,
            origin_x: 0.,
            origin_y: 0.,
            dpr: 1.,
            font_size: 13.,
        };
        let text = "terminal renders text during every keypress and cursor blink. 1234567890 ";
        TEXT_SHAPES.with(|count| count.set(0));
        for row in 0..40 {
            paint_op(
                &context,
                &mut text_painter,
                &geom,
                &PaintOp::Text {
                    col: 0,
                    x: 0.,
                    y: f64::from(row) * geom.cell_h,
                    cells: text.len() as u16,
                    text: text.into(),
                    rgb: [255; 3],
                    bold: false,
                    italic: false,
                },
            );
        }
        let unique = text.chars().collect::<std::collections::HashSet<_>>().len();
        assert!(
            TEXT_SHAPES.with(Cell::get) <= unique,
            "a frame must reuse identical glyph layouts instead of reshaping every terminal cell"
        );
    }
    /// `cargo test -p comandos-app --lib paint_speed -- --ignored --nocapture`
    #[test]
    #[ignore = "timing probe"]
    fn paint_speed() {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1700, 1100).unwrap();
        let context = cairo::Context::new(&surface).unwrap();
        let layout = pangocairo::functions::create_layout(&context);
        let font = pango::FontDescription::from_string("JetBrainsMono Nerd Font Mono 11");
        let geom = CellGeom { cell_w: 8., cell_h: 19., origin_x: 0., origin_y: 0., dpr: 1., font_size: 13. };
        let line = "  Lo armé enfocado en profundizar el lado Solana + adopción LatAm (que es justo lo que Superteam financia): Solana Desk v2, onboarding en español ";
        let ops: Vec<PaintOp> = (0..55)
            .map(|row| PaintOp::Text { col: 0, x: 0., y: f64::from(row) * 19., cells: line.chars().count() as u16, text: line.into(), rgb: [220; 3], bold: row % 7 == 0, italic: false })
            .collect();
        let mut painter = TextPainter::new(&layout, &font);
        let start = Instant::now();
        for _ in 0..30 {
            for op in &ops {
                paint_op(&context, &mut painter, &geom, op);
            }
        }
        let fast = start.elapsed() / 30;
        let start = Instant::now();
        for _ in 0..30 {
            let mut painter = TextPainter::new(&layout, &font);
            for op in &ops {
                if let PaintOp::Text { x, y, text, bold, italic, .. } = op {
                    for (i, c) in text.chars().enumerate() {
                        painter.show(&context, &c.to_string(), *bold, *italic, x + i as f64 * geom.cell_w, *y);
                    }
                }
            }
        }
        let old = start.elapsed() / 30;
        eprintln!("frame 150x55: new {fast:?}  old {old:?}");
    }
    #[test]
    fn text_layout_cache_is_bounded_for_diverse_unicode() {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 64, 32).unwrap();
        let context = cairo::Context::new(&surface).unwrap();
        let layout = pangocairo::functions::create_layout(&context);
        let mut painter = TextPainter::new(
            &layout,
            &pango::FontDescription::from_string("monospace 13"),
        );
        for value in 0x4e00..0x4e00 + 2 * TextPainter::MAX_LAYOUTS {
            let text = char::from_u32(value as u32).unwrap().to_string();
            painter.show(&context, &text, false, false, 0., 0.);
        }
        assert_eq!(painter.entries, TextPainter::MAX_LAYOUTS);
        assert_eq!(
            painter
                .layouts
                .iter()
                .map(|cache| cache.len())
                .sum::<usize>(),
            TextPainter::MAX_LAYOUTS
        );
    }
    #[test]
    fn terminal_text_raster_preserves_styles_unicode_and_scale() {
        fn render(reference: bool, font: &str, scale: f64) -> Vec<u8> {
            let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 800, 480).unwrap();
            let context = cairo::Context::new(&surface).unwrap();
            context.scale(scale, scale);
            let layout = pangocairo::functions::create_layout(&context);
            let font = pango::FontDescription::from_string(font);
            let mut text_painter = TextPainter::new(&layout, &font);
            let geom = CellGeom {
                cell_w: 8.25,
                cell_h: 22.,
                origin_x: 0.,
                origin_y: 0.,
                dpr: scale,
                font_size: 13.,
            };
            let cases = [
                ("ASCII repeated aa 11", 20),
                ("áñé→", 4),
                ("界", 2),
                ("e\u{301}", 1),
                ("👩\u{200d}💻", 2),
            ];
            for (style, (bold, italic)) in
                [(false, false), (true, false), (false, true), (true, true)]
                    .into_iter()
                    .enumerate()
            {
                for (row, (text, cells)) in cases.into_iter().enumerate() {
                    let x = 2.25 + style as f64 * 90.;
                    let y = 3.5 + row as f64 * geom.cell_h;
                    if reference {
                        let mut font = font.clone();
                        font.set_weight(if bold {
                            pango::Weight::Bold
                        } else {
                            pango::Weight::Normal
                        });
                        font.set_style(if italic {
                            pango::Style::Italic
                        } else {
                            pango::Style::Normal
                        });
                        layout.set_font_description(Some(&font));
                        color(&context, [211, 234, 245]);
                        if cells <= 2 && text.chars().count() != usize::from(cells) {
                            layout.set_text(text);
                            context.move_to(x, y);
                            pangocairo::functions::show_layout(&context, &layout);
                        } else {
                            for (column, character) in text.chars().enumerate() {
                                layout.set_text(&character.to_string());
                                context.move_to(x + column as f64 * geom.cell_w, y);
                                pangocairo::functions::show_layout(&context, &layout);
                            }
                        }
                    } else {
                        paint_op(
                            &context,
                            &mut text_painter,
                            &geom,
                            &PaintOp::Text {
                                col: 0,
                                x,
                                y,
                                cells,
                                text: text.into(),
                                rgb: [211, 234, 245],
                                bold,
                                italic,
                            },
                        );
                    }
                }
            }
            drop(text_painter);
            drop(layout);
            drop(context);
            surface.flush();
            surface.data().unwrap().to_vec()
        }
        for font in ["monospace 13", "monospace 16"] {
            for scale in [1., 2.] {
                // Glyph runs are positioned in exact doubles, Pango layouts in
                // 1/1024 px: only anti-aliasing at glyph edges may differ.
                let (fast, reference) = (render(false, font, scale), render(true, font, scale));
                let inked = reference.iter().filter(|&&v| v != 0).count();
                let differ = fast.iter().zip(&reference).filter(|(a, b)| a != b).count();
                let worst = fast
                    .iter()
                    .zip(&reference)
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap_or(0);
                assert!(
                    worst <= 96 && differ * 20 <= inked,
                    "{font} at {scale}: {differ} of {inked} inked bytes differ, worst {worst}"
                );
            }
        }
    }
    #[test]
    fn glyph_device_pixels_convert_to_widget_pixels_once() {
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 24, 24).unwrap();
        let context = cairo::Context::new(&surface).unwrap();
        let layout = pangocairo::functions::create_layout(&context);
        let font = pango::FontDescription::from_string("monospace 13");
        let mut text_painter = TextPainter::new(&layout, &font);
        let geom = CellGeom {
            cell_w: 8.,
            cell_h: 10.,
            origin_x: 0.,
            origin_y: 0.,
            dpr: 2.,
            font_size: 13.,
        };
        let op = PaintOp::Glyph {
            x: 0.,
            y: 0.,
            rgb: [255, 0, 0],
            ops: vec![DrawOp::FillRect {
                x: 0.,
                y: 0.,
                w: 16.,
                h: 20.,
            }],
        };
        paint_op(&context, &mut text_painter, &geom, &op);
        drop(context);
        drop(text_painter);
        drop(layout);
        surface.flush();
        let stride = usize::try_from(surface.stride()).unwrap();
        let data = surface.data().unwrap();
        let alpha = |x: usize, y: usize| data[y * stride + x * 4 + 3];
        assert_eq!(alpha(7, 9), 255);
        assert_eq!(alpha(8, 9), 0);
        assert_eq!(alpha(7, 10), 0);
    }
    #[test]
    fn curly_underline_has_a_different_raster_from_single() {
        let raster = |kind| {
            let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 40, 24).unwrap();
            let context = cairo::Context::new(&surface).unwrap();
            let layout = pangocairo::functions::create_layout(&context);
            let mut text_painter = TextPainter::new(
                &layout,
                &pango::FontDescription::from_string("monospace 13"),
            );
            let geom = CellGeom {
                cell_w: 8.,
                cell_h: 19.,
                origin_x: 0.,
                origin_y: 0.,
                dpr: 1.,
                font_size: 13.,
            };
            paint_op(
                &context,
                &mut text_painter,
                &geom,
                &PaintOp::Line {
                    x: 0.,
                    y: 0.,
                    w: 32.,
                    rgb: [255; 3],
                    kind,
                },
            );
            drop(context);
            drop(text_painter);
            drop(layout);
            surface.flush();
            surface.data().unwrap().to_vec()
        };
        assert_ne!(
            raster(LineKind::Under(Underline::Single)),
            raster(LineKind::Under(Underline::Curly))
        );
    }
}
