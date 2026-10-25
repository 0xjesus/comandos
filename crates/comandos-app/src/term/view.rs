//! GTK terminal over the shared engine and a private nonblocking PTY.
use super::{
    engine::*,
    keys::{drag_button, key_action, mouse_button},
    paint::{CellGeom, FrameCache, LineKind, PaintOp},
    pty::{DrainOutcome, PtyDrain, PtyError, PtySession},
    schedule::PaintSchedule,
    settle::{
        RESIZE_QUIET_MS, RESPAWN_MS, SETTLE_CAP_MS, SETTLE_QUIET_MS, Settle, fits_shared_window,
        initial_size,
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
/// The wheel went up at a cell; `true` when the host took it (the chat opens).
pub type WheelUpCallback = Rc<dyn Fn(u16, u16) -> bool>;
pub struct TermOptions {
    pub argv: Vec<String>,
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
    ssh_delta: f64,
    ssh_point: (u16, u16),
    ssh_at: Option<u64>,
    resize_at: Option<u64>,
}
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
    context_menu: RefCell<Option<ContextCallback>>,
    link_event: RefCell<Option<LinkCallback>>,
    clean_click: RefCell<Option<ContextCallback>>,
    primary_press: RefCell<Option<ContextCallback>>,
    replay: RefCell<crate::ui::clipboard::Replay<(gdk::EventKey, f64)>>,
    app_key: RefCell<Option<AppKeyCallback>>,
    key_observer: RefCell<Option<KeyObserver>>,
    wheel_up: RefCell<Option<WheelUpCallback>>,
    last_ctrl_c: Cell<Option<f64>>,
    cleanup_cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    clipboard: Rc<crate::ui::clipboard::Clipboard>,
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
                ssh_delta: 0.,
                ssh_point: (0, 0),
                ssh_at: None,
                resize_at: None,
            }),
            options,
            read_source: RefCell::new(None),
            write_source: RefCell::new(None),
            timer: RefCell::new(None),
            closed: Cell::new(false),
            context_menu: RefCell::new(None),
            link_event: RefCell::new(None),
            clean_click: RefCell::new(None),
            primary_press: RefCell::new(None),
            replay: RefCell::new(crate::ui::clipboard::Replay::default()),
            app_key: RefCell::new(None),
            key_observer: RefCell::new(None),
            wheel_up: RefCell::new(None),
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
        self.inner.wheel_up.borrow_mut().take();
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
    /// Plain wheel-up (no Shift, no selection) asks this first.
    pub fn on_wheel_up(&self, callback: WheelUpCallback) {
        *self.inner.wheel_up.borrow_mut() = Some(callback);
    }
    pub fn cell_geometry(&self) -> CellGeom {
        self.inner.model.borrow().geom
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
        self.inner.area.queue_draw();
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
        self.inner.area.queue_draw();
        self.inner.area.set_tooltip_text(Some(message));
    }
    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.inner.area
    }
    pub fn feed(&self, bytes: &[u8]) -> Result<(), PtyError> {
        self.inner.send(bytes)
    }
    pub fn paste(&self, text: &str) -> Result<(), PtyError> {
        let bytes = encode_paste(text, &self.inner.model.borrow().engine.modes());
        self.feed(&bytes)
    }
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.inner.resize(cols, rows)
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
        self.inner.area.queue_draw();
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
        m.geom.origin_x = m.padding + 2.;
        m.geom.origin_y = m.padding + 14.;
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
        self.area.queue_draw();
    }
    fn dimensions(&self) -> (u16, u16) {
        let m = self.model.borrow();
        initial_size(
            (self.area.allocated_width(), self.area.allocated_height()),
            m.geom.cell_w,
            m.geom.cell_h,
            (
                m.geom.origin_x + (m.padding - 4.).max(0.),
                m.geom.origin_y + (m.padding - 2.).max(0.),
            ),
            self.options.tmux_size,
        )
    }
    fn allocate(self: &Rc<Self>) {
        let now = self.ms();
        let mut m = self.model.borrow_mut();
        m.settle.on_alloc(now);
        if m.pty.is_some() {
            m.resize_at = Some(now.saturating_add(RESIZE_QUIET_MS));
        }
        drop(m);
        self.arm();
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
            return;
        }
        if self.model.borrow().engine.size() != (c, r) {
            let _ = self.resize(c, r);
        }
    }
    /// Seeing a tmux tab again takes its window back from any other client (phone,
    /// another view) so the text wraps at this view's width without pressing a key.
    fn claim_size(&self) {
        if self.options.mode == RunMode::Shadow || self.options.session.is_none() {
            return;
        }
        let m = self.model.borrow();
        if let Some(pty) = &m.pty {
            let (cols, rows) = m.engine.size();
            if fits_shared_window((cols, rows)) {
                let _ = pty.claim_size(cols, rows);
            }
        }
    }
    fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        if self.options.mode == RunMode::Shadow {
            return Ok(());
        }
        let mut m = self.model.borrow_mut();
        if let Some(p) = &m.pty {
            p.resize(cols, rows)?;
        }
        let px = (
            m.geom.cell_w.round().clamp(1., f64::from(u16::MAX)) as u16,
            m.geom.cell_h.round().clamp(1., f64::from(u16::MAX)) as u16,
        );
        m.engine.resize(cols, rows, px);
        self.area.queue_draw();
        Ok(())
    }
    fn spawn(self: &Rc<Self>) -> Result<(), PtyError> {
        if self.options.mode == RunMode::Shadow {
            return Err(PtyError::Spawn("Shadow cannot attach a PTY".into()));
        }
        if let Some(before) = &self.options.before_spawn {
            before().map_err(PtyError::Spawn)?;
        }
        let (mut cols, mut rows) = self.dimensions();
        if self.options.session.is_some() && !fits_shared_window((cols, rows)) {
            (cols, rows) = self.options.tmux_size.unwrap_or((80, 24));
        }
        let p = PtySession::spawn_with_env(
            &self.options.argv,
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
        }
        self.resize(cols, rows)?;
        if let Some(source) = self.read_source.borrow_mut().take() {
            source.remove();
        }
        let weak = Rc::downgrade(self);
        *self.read_source.borrow_mut() = Some(glib::source::unix_fd_add_local(
            fd,
            IOCondition::IN | IOCondition::HUP | IOCondition::ERR,
            move |_, condition| {
                let Some(inner) = weak.upgrade() else {
                    return ControlFlow::Break;
                };
                if inner.read(condition) {
                    ControlFlow::Continue
                } else {
                    inner.read_source.borrow_mut().take();
                    ControlFlow::Break
                }
            },
        ));
        Ok(())
    }
    fn send(self: &Rc<Self>, bytes: &[u8]) -> Result<(), PtyError> {
        if self.options.mode == RunMode::Shadow || self.closed.get() {
            return Err(PtyError::Io("terminal is read-only or closed".into()));
        }
        let mut m = self.model.borrow_mut();
        let Some(p) = m.pty.as_mut() else {
            return Err(PtyError::Io("PTY not attached".into()));
        };
        p.write(bytes)?;
        drop(m);
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
        let outcome = {
            let Model {
                pty, engine, drain, ..
            } = &mut *m;
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
        m.schedule.damage(self.ms(), &lines);
        if !lines.is_empty() {
            m.hovered_link = None;
            m.last_motion = None;
        }
        drop(m);
        if !drained.replies.is_empty() {
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
        if let Some(id) = self.timer.borrow_mut().take() {
            id.remove();
        }
        let now = self.ms();
        let mut m = self.model.borrow_mut();
        // Request this GTK frame directly: rearming the maintenance timeout on
        // every PTY read can otherwise postpone painting under continuous output.
        if m.schedule.queue_due_paint(now, self.area.is_mapped()) {
            self.area.queue_draw();
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
            self.area.queue_draw();
        }
        if m.schedule.queue_due_paint(now, mapped) {
            self.area.queue_draw();
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
                glib::Propagation::Stop
            }
            KeyAction::ScrollPage(direction) => {
                let mut m = self.model.borrow_mut();
                let rows = i32::from(m.engine.size().1);
                m.engine.scroll(-direction * rows);
                self.area.queue_draw();
                glib::Propagation::Stop
            }
            KeyAction::None => glib::Propagation::Proceed,
        }
    }
    fn selection_text(&self) -> Option<String> {
        let m = self.model.borrow();
        m.selection
            .map(|s| selected_text(m.engine.engine(), &s))
            .filter(|text| !text.is_empty())
    }
    fn point(&self, x: f64, y: f64) -> (u16, u16) {
        let m = self.model.borrow();
        let (cols, rows) = m.engine.size();
        let col = ((x - m.geom.origin_x).max(0.) / m.geom.cell_w).floor() as u16;
        let row = ((y - m.geom.origin_y).max(0.) / m.geom.cell_h).floor() as u16;
        (
            col.min(cols.saturating_sub(1)),
            row.min(rows.saturating_sub(1)),
        )
    }
    fn absolute(&self, col: u16, row: u16) -> Point {
        let offset =
            i32::try_from(self.model.borrow().engine.engine().display_offset()).unwrap_or(i32::MAX);
        (i32::from(row).saturating_sub(offset), col)
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
        self.wheel_up.get_mut().take();
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
            inner.claim_size();
            inner.area.queue_draw();
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
            inner.area.queue_draw();
        }
    });
    let weak = Rc::downgrade(inner);
    inner.im.connect_commit(move |_, text| {
        if let Some(inner) = weak.upgrade() {
            let bytes = {
                let mut m = inner.model.borrow_mut();
                m.preedit.clear();
                m.ime.end(text)
            };
            if let Some(bytes) = bytes {
                inner.send_event(&bytes);
            }
            inner.area.queue_draw();
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
                inner.area.queue_draw();
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
        let (col, row) = inner.point(x, y);
        if let Some(callback) = inner.primary_press.borrow().clone() {
            callback(event, (col, row));
        }
        if event.button() == 3
            && let Some(callback) = inner.context_menu.borrow().as_ref()
        {
            callback(event, (col, row));
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
            super::links::link_at(
                &inner.model.borrow().engine,
                inner.absolute(col, row),
                row,
                col,
            )
        } else {
            None
        };
        let link_gesture = link.is_some();
        if event.button() == 1 {
            inner.model.borrow_mut().link_gesture = link_gesture;
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
                callback(link, event, (col, row), true);
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
            inner.area.queue_draw();
        }
        glib::Propagation::Stop
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_motion_notify_event(move |area, event| {
        let Some(inner) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let (x, y) = event.position();
        let (col, row) = inner.point(x, y);
        let point = inner.absolute(col, row);
        let state = event.state();
        let mut m = inner.model.borrow_mut();
        if m.last_motion == Some((col, row, state)) {
            return glib::Propagation::Stop;
        }
        m.last_motion = Some((col, row, state));
        let button = drag_button(state);
        let linked = super::links::link_at(&m.engine, point, row, col).is_some();
        let hovered = (linked && button == Button::None)
            .then(|| word_at(m.engine.engine(), point))
            .flatten();
        if m.hovered_link != hovered {
            m.hovered_link = hovered;
            area.queue_draw();
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
            area.queue_draw();
        }
        drop(m);
        glib::Propagation::Stop
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_leave_notify_event(move |area, _| {
        if let Some(inner) = weak.upgrade() {
            let mut model = inner.model.borrow_mut();
            model.last_motion = None;
            if model.hovered_link.take().is_some() {
                area.queue_draw();
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
            callback(event, point);
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
                callback(&link, event, point, false);
            } else if let Some(callback) = &inner.options.on_link {
                callback(&link);
            }
        }
        inner.area.queue_draw();
        glib::Propagation::Stop
    });
    let weak = Rc::downgrade(inner);
    inner.area.connect_scroll_event(move |_, event| {
        let Some(inner) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
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
        if amount < 0.
            && !event.state().contains(gdk::ModifierType::SHIFT_MASK)
            && inner.model.borrow().selection.is_none()
        {
            let callback = inner.wheel_up.borrow().clone();
            if let Some(callback) = callback
                && callback(col, row)
            {
                inner.model.borrow_mut().wheel_delta = 0.;
                return glib::Propagation::Stop;
            }
        }
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
        match wheel(lines, col, row, modifiers(event.state()), &m.engine.modes()) {
            WheelAction::Mouse(bytes) | WheelAction::Arrows(bytes) => {
                drop(m);
                // La rueda deja el pane en copy-mode de tmux y la vista se congela:
                // la primera tecla lo saca, igual que tras seleccionar con el mouse.
                inner.clipboard.release_selection();
                inner.send_event(&bytes);
            }
            WheelAction::Scroll(lines) => {
                m.engine.scroll(-lines);
                inner.area.queue_draw();
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
        let mut m = self.model.borrow_mut();
        let palette = m.engine.palette();
        context.set_source_rgba(
            f64::from(palette.bg[0]) / 255.,
            f64::from(palette.bg[1]) / 255.,
            f64::from(palette.bg[2]) / 255.,
            m.opacity,
        );
        context.set_operator(cairo::Operator::Source);
        let _ = context.paint();
        context.set_operator(cairo::Operator::Over);
        let layout = self.area.create_pango_layout(None);
        let mut text_painter = TextPainter::new(&layout, &m.font);
        let (_, rows) = m.engine.size();
        {
            let Model {
                engine,
                frame,
                geom,
                ..
            } = &mut *m;
            frame.refresh(engine, geom);
        }
        for op in m.frame.ops() {
            paint_op(context, &mut text_painter, &m.geom, op);
        }
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
                let x = m.geom.origin_x + f64::from(col) * m.geom.cell_w;
                let y = m.geom.origin_y + f64::from(row + 1) * m.geom.cell_h - 1.5;
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
                context.rectangle(
                    m.geom.origin_x + f64::from(left) * m.geom.cell_w,
                    m.geom.origin_y + f64::from(row) * m.geom.cell_h,
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
        let cursor = m.frame.cursor();
        let blink = m
            .preferences
            .get("cursor_blink")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if cursor.visible && (!blink || !m.focused || m.blink_visible) {
            let x = m.geom.origin_x + f64::from(cursor.col) * m.geom.cell_w;
            let y = m.geom.origin_y + cursor.line as f64 * m.geom.cell_h;
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
                    CursorShape::Block => context.rectangle(x, y, width, m.geom.cell_h),
                    CursorShape::HollowBlock => {
                        context.rectangle(x + 0.5, y + 0.5, width - 1., m.geom.cell_h - 1.);
                        context.set_line_width(1.);
                        let _ = context.stroke();
                    }
                    CursorShape::Hidden => {}
                }
                let _ = context.fill();
                if shape == CursorShape::Block {
                    let _ = context.save();
                    context.rectangle(x, y, width, m.geom.cell_h);
                    context.clip();
                    for op in m.frame.ops() {
                        let mut op = op.clone();
                        match &mut op {
                            PaintOp::Text {
                                y: row_y,
                                col,
                                cells,
                                rgb,
                                ..
                            } if *row_y == y
                                && *col <= cursor.col
                                && cursor.col < col.saturating_add(*cells) =>
                            {
                                *rgb = m.engine.palette().cursor_accent;
                                paint_op(context, &mut text_painter, &m.geom, &op);
                            }
                            PaintOp::Glyph {
                                x: cell_x,
                                y: row_y,
                                rgb,
                                ..
                            } if *row_y == y && *cell_x == x => {
                                *rgb = m.engine.palette().cursor_accent;
                                paint_op(context, &mut text_painter, &m.geom, &op);
                            }
                            _ => {}
                        }
                    }
                    let _ = context.restore();
                }
            }
        }
        if !m.preedit.is_empty() {
            layout.set_font_description(Some(&m.font));
            layout.set_text(&m.preedit);
            color(context, m.engine.palette().fg);
            context.move_to(
                m.geom.origin_x + f64::from(cursor.col) * m.geom.cell_w,
                m.geom.origin_y + cursor.line as f64 * m.geom.cell_h,
            );
            pangocairo::functions::show_layout(context, &layout);
        }
        m.schedule.painted(self.ms());
    }
}

/// Reuse shaping within one frame while retaining the existing cell positions.
/// Frame-local ownership also makes font, scale and Pango context changes take
/// effect immediately. Unusual Unicode output cannot grow the cache unbounded.
struct TextPainter {
    template: pango::Layout,
    fonts: [pango::FontDescription; 4],
    layouts: [std::collections::HashMap<String, pango::Layout>; 4],
    entries: usize,
}
impl TextPainter {
    const MAX_LAYOUTS: usize = 512;

    fn new(template: &pango::Layout, font: &pango::FontDescription) -> Self {
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
        if let Some(layout) = self.layouts[style].get(text) {
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
        layout.set_font_description(Some(&self.fonts[style]));
        #[cfg(test)]
        native_paint_tests::TEXT_SHAPES.with(|count| count.set(count.get() + 1));
        layout.set_text(text);
        context.move_to(x, y);
        pangocairo::functions::show_layout(context, &layout);
        if retain {
            self.layouts[style].insert(text.to_string(), layout);
            self.entries += 1;
        }
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
            } else {
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
                assert_eq!(
                    render(false, font, scale),
                    render(true, font, scale),
                    "{font} at {scale}"
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
