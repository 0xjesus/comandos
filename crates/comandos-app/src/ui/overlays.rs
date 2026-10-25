//! Logical coordinates from the terminal's Pango geometry. DPR is already applied there.
use crate::term::paint::CellGeom;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
pub const HEADER_EXTRA: f64 = 14.0;
#[derive(Debug, Default)]
pub struct GeometryGate {
    flight: Option<u64>,
    pending: bool,
    serial: u64,
    closed: bool,
}
impl GeometryGate {
    pub fn request(&mut self) -> Option<u64> {
        if self.closed {
            return None;
        }
        if self.flight.is_some() {
            self.pending = true;
            return None;
        }
        self.serial = self.serial.wrapping_add(1);
        self.flight = Some(self.serial);
        Some(self.serial)
    }
    pub fn finish(&mut self, ticket: u64) -> (bool, Option<u64>) {
        if self.flight != Some(ticket) {
            return (false, None);
        }
        self.flight = None;
        if self.closed {
            return (false, None);
        }
        let pending = std::mem::take(&mut self.pending);
        (!pending, if pending { self.request() } else { None })
    }
    pub fn pause(&mut self) {
        self.flight = None;
        self.pending = false;
    }
    pub fn close(&mut self) {
        self.closed = true;
        self.pause();
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneGeometry {
    pub id: String,
    pub left: u16,
    pub top: u16,
    pub width: u16,
    pub height: u16,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneRecord {
    pub geometry: PaneGeometry,
    pub command: String,
    pub active: bool,
}
pub const PANE_FORMAT: &str = "#{pane_id} #{pane_left} #{pane_top} #{pane_width} #{pane_current_command} #{pane_height} #{pane_active} #{pane_pid}";
pub fn parse_geometry(raw: &str) -> Result<Vec<PaneRecord>, String> {
    if raw.len() > 1024 * 1024 {
        return Err("pane geometry exceeds 1 MiB".into());
    }
    let mut panes = vec![];
    let mut seen = BTreeSet::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let id = fields.first().copied().unwrap_or_default();
        if fields.len() < 7
            || !super::app_commands::valid_pane(id)
            || !seen.insert(id.to_string())
            || panes.len() >= 512
        {
            return Err("invalid pane geometry".into());
        }
        let number = |index: usize| {
            fields
                .get(index)
                .ok_or("missing geometry coordinate")
                .and_then(|s| s.parse::<u16>().map_err(|_| "invalid geometry coordinate"))
        };
        panes.push(PaneRecord {
            geometry: PaneGeometry {
                id: id.into(),
                left: number(1)?,
                top: number(2)?,
                width: number(3)?,
                height: number(5)?,
            },
            // `sh -lc "claude …"` restored panes: name what runs in the foreground.
            command: match fields.get(7).and_then(|p| p.parse::<i64>().ok()) {
                Some(pid) => comandos_runtime::agent_procs::foreground_command(
                    std::path::Path::new("/proc"),
                    pid,
                    fields.get(4).copied().unwrap_or_default(),
                ),
                None => fields.get(4).copied().unwrap_or_default().to_string(),
            },
            active: fields.get(6).copied() == Some("1"),
        });
    }
    Ok(panes)
}
pub fn is_shell(command: &str) -> bool {
    matches!(
        command,
        "zsh" | "bash" | "sh" | "fish" | "dash" | "ksh" | "tcsh" | "csh" | "nu"
    )
}
#[allow(clippy::too_many_arguments)]
pub fn resize_neighbor_when(
    tmux: &impl super::clipboard::TmuxIo,
    session: &str,
    pane: &str,
    orientation: &str,
    neighbor: &str,
    size: u16,
    allowed: impl Fn() -> bool,
) -> Result<(), String> {
    if tmux.mode() == crate::config::RunMode::Shadow
        || !matches!(orientation, "v" | "h")
        || !allowed()
    {
        return Err("Pane resize refused".into());
    }
    let socket = tmux.socket().to_path_buf();
    let identity = super::accounts::pane_identity_when(tmux, session, pane, None, &allowed)?;
    let adjacent = super::accounts::pane_identity_when(tmux, session, neighbor, None, &allowed)?;
    let target = format!("={session}");
    let layout = tmux
        .read(&["list-panes", "-t", &target, "-F", PANE_FORMAT])
        .map_err(|_| "Pane geometry unavailable".to_string())?;
    if !layout.ok() || !allowed() || tmux.socket() != socket {
        return Err("Pane resize cancelled".into());
    }
    let records = parse_geometry(&layout.stdout)?;
    let panes = records.into_iter().map(|p| p.geometry).collect::<Vec<_>>();
    if gutter_neighbor(&panes, pane, orientation) != Some(neighbor) {
        return Err("Adjacent split changed".into());
    }
    let p = panes.iter().find(|p| p.id == pane).ok_or("Pane removed")?;
    let cell = f64::from(if orientation == "v" { p.left } else { p.top }) + f64::from(size);
    let n = gutter_target(&panes, pane, orientation, cell)
        .and_then(|n| n.as_f64())
        .ok_or("Invalid pane size")?
        .trunc()
        .to_string();
    super::accounts::pane_identity_when(tmux, session, pane, Some(&identity), &allowed)?;
    super::accounts::pane_identity_when(tmux, session, neighbor, Some(&adjacent), &allowed)?;
    if !allowed() || tmux.socket() != socket {
        return Err("Pane resize cancelled".into());
    }
    let out = tmux
        .mutate(
            &[
                "resize-pane",
                "-t",
                pane,
                if orientation == "v" { "-x" } else { "-y" },
                &n,
            ],
            None,
        )
        .map_err(|_| "Pane resize failed".to_string())?;
    if out.ok() {
        Ok(())
    } else {
        Err("Pane resize failed".into())
    }
}
pub fn read_geometry_when(
    tmux: &impl super::clipboard::TmuxIo,
    session: &str,
    allowed: impl Fn() -> bool,
) -> Result<Vec<PaneRecord>, String> {
    if !allowed() {
        return Err("Pane geometry cancelled".into());
    }
    let socket = tmux.socket().to_path_buf();
    let target = format!("={session}");
    // tmux 3.2a dereferences NULL when a time format (session_created,
    // *_activity…) meets a target that no longer exists, killing the server
    // (24-oct-2026). Identity uses the session id digits instead; see
    // `tests/tmux_time_formats.rs`.
    let identity_target = format!("={session}:");
    let identity_format = "#{pid}|#{session_id}|#{s/[$]//:session_id}|#{session_name}";
    let pin = tmux
        .read(&[
            "display-message",
            "-p",
            "-t",
            &identity_target,
            identity_format,
        ])
        .map_err(|_| "Session unavailable".to_string())?;
    let fields = pin.stdout.trim().split('|').collect::<Vec<_>>();
    if !pin.ok()
        || fields.len() != 4
        || fields.get(3).copied() != Some(session)
        || fields
            .first()
            .is_none_or(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
        || fields.get(1).is_none_or(|s| {
            s.strip_prefix('$')
                .is_none_or(|id| id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()))
        })
        || fields
            .get(2)
            .is_none_or(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
        || !allowed()
        || tmux.socket() != socket
    {
        return Err("Session changed".into());
    }
    let raw = tmux
        .read(&["list-panes", "-t", &target, "-F", PANE_FORMAT])
        .map_err(|_| "Pane geometry unavailable".to_string())?;
    if !raw.ok() || !allowed() || tmux.socket() != socket {
        return Err("Pane geometry cancelled".into());
    }
    let now = tmux
        .read(&[
            "display-message",
            "-p",
            "-t",
            &identity_target,
            identity_format,
        ])
        .map_err(|_| "Session unavailable".to_string())?;
    if !now.ok() || pin.stdout.trim() != now.stdout.trim() || !allowed() || tmux.socket() != socket
    {
        return Err("Session changed".into());
    }
    parse_geometry(&raw.stdout)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gutter {
    pub orientation: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub pane: String,
}
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FrameLayout {
    pub frames: Vec<Value>,
    pub rows: BTreeMap<String, (f64, f64, f64, f64)>,
    pub gutters: Vec<Gutter>,
}
impl FrameLayout {
    pub fn frame_at(&self, x: f64, y: f64) -> Option<usize> {
        self.frames.iter().position(|frame| {
            let n = |i| frame.get(i).and_then(Value::as_f64).unwrap_or(0.0);
            x >= n(0) && x < n(0) + n(2) && y >= n(1) && y < n(1) + n(3)
        })
    }
}
pub fn frame_layout(
    panes: &[PaneGeometry],
    g: &CellGeom,
    grid: (u16, u16),
    active: &BTreeSet<String>,
    focused: bool,
) -> FrameLayout {
    let mut layout = FrameLayout::default();
    for p in panes {
        if p.height == 0 || p.width < 8 {
            continue;
        }
        let left = f64::from(p.left);
        let top = f64::from(p.top);
        let width = f64::from(p.width);
        let height = f64::from(p.height);
        let rail = g.origin_y
            + if p.top >= 1 {
                (top - 1.0) * g.cell_h
            } else {
                0.0
            };
        let x0 = g.origin_x + left * g.cell_w - if p.left == 0 { 5.0 } else { 3.0 };
        let x1 = g.origin_x + (left + width) * g.cell_w + 3.0;
        let y0 = if p.top <= 1 {
            rail - (HEADER_EXTRA + 3.0)
        } else {
            rail + 3.0
        };
        let bottom = g.origin_y + (top + height) * g.cell_h;
        let y1 = bottom
            + if grid.1 != 0 && u32::from(p.top) + u32::from(p.height) >= u32::from(grid.1) {
                3.0
            } else {
                -2.0
            };
        layout.frames.push(json!([
            x0,
            y0,
            x1 - x0,
            y1 - y0,
            focused && active.contains(&p.id)
        ]));
        layout.rows.insert(p.id.clone(), (x0, rail, x1 - x0, y0));
        if grid.0 != 0 && u32::from(p.left) + u32::from(p.width) < u32::from(grid.0) {
            layout.gutters.push(Gutter {
                orientation: "v".into(),
                x: g.origin_x + (left + width) * g.cell_w,
                y: y0 + 6.0,
                width: g.cell_w,
                height: (y1 - y0 - 12.0).max(0.0),
                pane: p.id.clone(),
            });
        }
        if grid.1 != 0 && u32::from(p.top) + u32::from(p.height) < u32::from(grid.1) {
            layout.gutters.push(Gutter {
                orientation: "h".into(),
                x: x0 + 10.0,
                y: bottom - 2.0,
                width: (x1 - x0 - 20.0).max(0.0),
                height: 5.0,
                pane: p.id.clone(),
            });
        }
    }
    layout
}
/// Without runtime pane-active/grid information, produce the frame coordinates only.
pub fn pane_frames(rows: &[PaneGeometry], geom: &CellGeom, focused: bool) -> Vec<Value> {
    frame_layout(
        rows,
        geom,
        (0, 0),
        &rows.iter().map(|p| p.id.clone()).collect(),
        focused,
    )
    .frames
}
pub fn gutter_neighbor<'a>(
    panes: &'a [PaneGeometry],
    pane: &str,
    orientation: &str,
) -> Option<&'a str> {
    let p = panes.iter().find(|p| p.id == pane)?;
    let mut best = None;
    let mut overlap = 0i32;
    for q in panes.iter().filter(|q| q.id != pane) {
        let next = if orientation == "v"
            && u32::from(q.left) == u32::from(p.left) + u32::from(p.width) + 1
        {
            (i32::from(p.top) + i32::from(p.height)).min(i32::from(q.top) + i32::from(q.height))
                - i32::from(p.top).max(i32::from(q.top))
        } else if orientation == "h"
            && u32::from(q.top) == u32::from(p.top) + u32::from(p.height) + 1
        {
            (i32::from(p.left) + i32::from(p.width)).min(i32::from(q.left) + i32::from(q.width))
                - i32::from(p.left).max(i32::from(q.left))
        } else {
            continue;
        };
        if next > overlap {
            overlap = next;
            best = Some(q.id.as_str());
        }
    }
    best
}
pub fn gutter_target(
    panes: &[PaneGeometry],
    pane: &str,
    orientation: &str,
    cell: f64,
) -> Option<Value> {
    if !cell.is_finite() || !matches!(orientation, "v" | "h") {
        return None;
    }
    let p = panes.iter().find(|p| p.id == pane)?;
    let (start, size) = if orientation == "v" {
        (p.left, p.width)
    } else {
        (p.top, p.height)
    };
    let wanted = cell - f64::from(start);
    let bound = gutter_neighbor(panes, pane, orientation)
        .and_then(|id| panes.iter().find(|p| p.id == id))
        .map(|q| {
            f64::from(size)
                + f64::from(if orientation == "v" {
                    q.width
                } else {
                    q.height
                })
                - 2.0
        });
    let wanted = bound.map_or(wanted, |bound| wanted.min(bound)).max(2.0);
    Some(json!(wanted))
}
pub fn gutter_half(panes: &[PaneGeometry], pane: &str, orientation: &str) -> Option<u16> {
    let p = panes.iter().find(|p| p.id == pane)?;
    let other = gutter_neighbor(panes, pane, orientation)?;
    let q = panes.iter().find(|p| p.id == other)?;
    Some(
        ((u32::from(if orientation == "v" {
            p.width
        } else {
            p.height
        }) + u32::from(if orientation == "v" {
            q.width
        } else {
            q.height
        })) / 2)
            .max(2)
            .min(u32::from(u16::MAX)) as u16,
    )
}
pub fn shell_pill_xy(
    rows: &BTreeMap<String, (f64, f64, f64, f64)>,
    pane_id: &str,
    margins: (f64, f64),
    pane_origin: (u16, u16),
    cell: (f64, f64),
) -> (i32, i32) {
    if let Some((fx, rail, _, fy)) = rows.get(pane_id) {
        let top = *fy as i32 + 1;
        return (
            *fx as i32 + 8,
            top + ((*rail + cell.1) as i32 - top - 22).div_euclid(2).max(0),
        );
    }
    (
        (margins.0 + f64::from(pane_origin.0) * cell.0 + cell.0).max(0.0) as i32,
        (margins.1 + f64::from(pane_origin.1.saturating_sub(1)) * cell.1).max(0.0) as i32,
    )
}
pub fn card_rect(row: (f64, f64, f64, f64), cell_h: f64) -> (i32, i32, i32, i32) {
    let (fx, rail, width, fy) = row;
    let top = fy as i32 + 1;
    (
        fx as i32 + 1,
        top,
        (width as i32 - 2).max(120),
        ((rail + cell_h) as i32 - top).max(18),
    )
}
/// How the right-hand buttons of a pane header (Cuenta, MCPs · Skills) fit next to
/// the card's own text: with labels, as icons only, or not at all. In a narrow pane
/// they used to be drawn on top of the model name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeysFit {
    Full,
    Icons,
    Hidden,
}
pub fn keys_fit(width: i32, card_text: i32, full: i32, icons: i32) -> KeysFit {
    let room = width - card_text - 16;
    if full <= room {
        KeysFit::Full
    } else if icons <= room {
        KeysFit::Icons
    } else {
        KeysFit::Hidden
    }
}
/// Fits a header's button row (natural width `full`) into a pane `width` px wide whose
/// card text takes `card_text`: drops the labels when needed. The width it uses, or
/// `None` when not even the icons fit.
pub fn fit_keys(keys: &gtk::Widget, width: i32, card_text: i32, full: i32) -> Option<i32> {
    set_key_labels(keys, false);
    let icons = keys.preferred_width().1;
    let fit = keys_fit(width, card_text, full, icons);
    set_key_labels(keys, fit == KeysFit::Full);
    // A later show_all() on the pane must not bring back what did not fit.
    keys.set_no_show_all(fit == KeysFit::Hidden);
    keys.set_visible(fit != KeysFit::Hidden);
    match fit {
        KeysFit::Full => Some(full),
        KeysFit::Icons => Some(icons),
        KeysFit::Hidden => None,
    }
}
/// Shows or hides the text of every button in a header's button row.
fn set_key_labels(keys: &gtk::Widget, visible: bool) {
    let Some(row) = keys.downcast_ref::<gtk::Box>() else { return };
    for button in row.children() {
        let Some(content) = button
            .downcast_ref::<gtk::Bin>()
            .and_then(gtk::prelude::BinExt::child)
            .and_then(|c| c.downcast::<gtk::Box>().ok())
        else {
            continue;
        };
        for part in content.children() {
            if part.is::<gtk::Label>() {
                part.set_no_show_all(!visible);
                part.set_visible(visible);
            }
        }
    }
}
pub fn grip_rect(g: &Gutter) -> (i32, i32, i32, i32) {
    if g.orientation == "v" {
        (
            (g.x - 8.0) as i32,
            g.y as i32,
            (g.width + 16.0) as i32,
            g.height as i32,
        )
    } else {
        (
            g.x as i32,
            (g.y - 8.0) as i32,
            g.width as i32,
            (g.height + 16.0) as i32,
        )
    }
}
pub fn gutter_at(gutters: &[Gutter], x: f64, y: f64, slack: f64) -> Option<usize> {
    gutters.iter().position(|g| {
        g.x - slack <= x
            && x <= g.x + g.width + slack
            && g.y - slack <= y
            && y <= g.y + g.height + slack
    })
}
pub fn gutter_measure(panes: &[PaneGeometry], g: &Gutter) -> String {
    let Some(p) = panes.iter().find(|p| p.id == g.pane) else {
        return String::new();
    };
    let q = gutter_neighbor(panes, &g.pane, &g.orientation)
        .and_then(|id| panes.iter().find(|p| p.id == id));
    if g.orientation == "v" {
        q.map_or_else(
            || format!("{} col", p.width),
            |q| format!("{} | {} col", p.width, q.width),
        )
    } else {
        q.map_or_else(
            || format!("{} filas", p.height),
            |q| format!("{} / {} filas", p.height, q.height),
        )
    }
}

use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
pub(crate) struct PaneWidget {
    pub pane: String,
    pub kind: &'static str,
    pub widget: gtk::Widget,
    pub width: i32,
}
#[derive(Debug, Clone)]
pub(crate) struct Drag {
    pub pane: String,
    pub orientation: String,
    pub neighbor: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResizeWish {
    pub pane: String,
    pub orientation: String,
    pub neighbor: Option<String>,
    pub size: u16,
}
/// Every input that can change placement or the overlay's own painted frame.
#[derive(PartialEq)]
struct RepositionState {
    layout: FrameLayout,
    geom: CellGeom,
    panes: Vec<PaneGeometry>,
    pills: Vec<(gtk::Widget, String, &'static str, i32)>,
    grips: Vec<gtk::EventBox>,
    colors: (String, String, String),
}
pub(crate) struct PaneOverlay {
    pub widget: gtk::Overlay,
    pub session: String,
    pub term: crate::term::view::TermView,
    pub scope: super::snippets::Scope,
    pub gate: RefCell<GeometryGate>,
    pub panes: RefCell<Vec<PaneRecord>>,
    pub layout: RefCell<FrameLayout>,
    repositioned: RefCell<Option<RepositionState>>,
    pub pills: RefCell<Vec<PaneWidget>>,
    pub grips: RefCell<Vec<gtk::EventBox>>,
    pub signature: RefCell<String>,
    pub timer: RefCell<Option<glib::SourceId>>,
    pub prefix_timer: RefCell<Option<glib::SourceId>>,
    pub prefix_until: Cell<f64>,
    pub hot: Cell<Option<usize>>,
    pub hovered_frame: Cell<Option<usize>>,
    pub drag: RefCell<Option<Drag>>,
    pub wanted: RefCell<Option<ResizeWish>>,
    pub sent: RefCell<Option<ResizeWish>>,
    pub resize_busy: Cell<bool>,
    pub ai: RefCell<Vec<gtk::Image>>,
    pub cache: RefCell<super::marks::IndicatorCache>,
    pub ai_frame: RefCell<String>,
    pub colors: RefCell<(String, String, String)>,
    pub cursors: RefCell<BTreeMap<String, gdk::Cursor>>,
}
impl PaneOverlay {
    pub fn new(session: &str, term: &crate::term::view::TermView) -> Rc<Self> {
        let widget = gtk::Overlay::new();
        widget.add(term.widget());
        // GtkWorkspace focuses its registered page. Forward that focus to the original terminal.
        widget.set_can_focus(true);
        let terminal = term.widget().downgrade();
        widget.connect_focus_in_event(move |_, _| {
            if let Some(terminal) = terminal.upgrade() {
                terminal.grab_focus();
            }
            glib::Propagation::Proceed
        });
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        Rc::new(Self {
            widget,
            session: session.into(),
            term: term.clone(),
            scope: super::snippets::Scope::default(),
            gate: RefCell::new(GeometryGate::default()),
            panes: RefCell::new(vec![]),
            layout: RefCell::new(FrameLayout::default()),
            repositioned: RefCell::new(None),
            pills: RefCell::new(vec![]),
            grips: RefCell::new(vec![]),
            signature: RefCell::new(String::new()),
            timer: RefCell::new(None),
            prefix_timer: RefCell::new(None),
            prefix_until: Cell::new(0.0),
            hot: Cell::new(None),
            hovered_frame: Cell::new(None),
            drag: RefCell::new(None),
            wanted: RefCell::new(None),
            sent: RefCell::new(None),
            resize_busy: Cell::new(false),
            ai: RefCell::new(vec![]),
            cache: RefCell::new(super::marks::IndicatorCache::default()),
            ai_frame: RefCell::new(String::new()),
            cursors: RefCell::new(BTreeMap::new()),
            colors: RefCell::new(("#223044".into(), "#8B7CF6".into(), "#0A0D13".into())),
        })
    }
    pub fn cancel(&self) {
        self.cursors.borrow_mut().clear();
        self.scope.close();
        self.gate.borrow_mut().close();
        self.drag.borrow_mut().take();
        self.wanted.borrow_mut().take();
        for slot in [&self.timer, &self.prefix_timer] {
            if let Some(source) = slot.borrow_mut().take() {
                source.remove();
            }
        }
    }
    pub fn cursor(&self, widget: &gtk::EventBox, name: &str) {
        if !matches!(name, "grab" | "grabbing") {
            return;
        }
        let Some(window) = widget.window() else {
            return;
        };
        if !self.cursors.borrow().contains_key(name)
            && let Some(cursor) = gdk::Cursor::from_name(&widget.display(), name)
        {
            self.cursors.borrow_mut().insert(name.into(), cursor);
        }
        window.set_cursor(self.cursors.borrow().get(name));
    }
    pub fn geometries(&self) -> Vec<PaneGeometry> {
        self.panes
            .borrow()
            .iter()
            .map(|p| p.geometry.clone())
            .collect()
    }
    pub fn reposition(&self, focused: bool) {
        let records = self.panes.borrow();
        let panes = records
            .iter()
            .map(|p| p.geometry.clone())
            .collect::<Vec<_>>();
        let active = records
            .iter()
            .filter(|p| p.active)
            .map(|p| p.geometry.id.clone())
            .collect();
        let geom = self.term.cell_geometry();
        let layout = frame_layout(&panes, &geom, self.term.grid_size(), &active, focused);
        let state = RepositionState {
            layout: layout.clone(),
            geom,
            panes: panes.clone(),
            pills: self
                .pills
                .borrow()
                .iter()
                .map(|p| (p.widget.clone(), p.pane.clone(), p.kind, p.width))
                .collect(),
            grips: self.grips.borrow().clone(),
            colors: self.colors.borrow().clone(),
        };
        if self.repositioned.borrow().as_ref() == Some(&state) {
            return;
        }
        *self.repositioned.borrow_mut() = Some(state);

        for pill in self.pills.borrow().iter() {
            let Some(row) = layout.rows.get(&pill.pane) else {
                pill.widget.hide();
                continue;
            };
            let (x, y, w, h) = card_rect(*row, geom.cell_h);
            match pill.kind {
                "card" => {
                    pill.widget.set_size_request(w, h);
                    pill.widget.set_margin_start(x.max(0));
                    pill.widget.set_margin_top(y.max(0));
                }
                "keys" => {
                    let card_text = self
                        .pills
                        .borrow()
                        .iter()
                        .find(|p| p.kind == "card" && p.pane == pill.pane)
                        .map_or(0, |p| p.width);
                    let Some(used) = fit_keys(&pill.widget, w, card_text, pill.width) else {
                        pill.widget.hide();
                        continue;
                    };
                    pill.widget
                        .set_margin_start(x.max(x + w - used - 8).max(0));
                    pill.widget
                        .set_margin_top((y + (h - 22).max(0) / 2 - 1).max(0));
                }
                _ => {
                    let Some(p) = panes.iter().find(|p| p.id == pill.pane) else {
                        continue;
                    };
                    let (x, y) = shell_pill_xy(
                        &layout.rows,
                        &pill.pane,
                        (geom.origin_x, geom.origin_y),
                        (p.left, p.top),
                        (geom.cell_w, geom.cell_h),
                    );
                    pill.widget.set_margin_start(x.max(0));
                    pill.widget.set_margin_top(y.max(0));
                }
            }
            pill.widget.show();
        }
        for (grip, g) in self.grips.borrow().iter().zip(layout.gutters.iter()) {
            let (x, y, w, h) = grip_rect(g);
            grip.set_margin_start(x.max(0));
            grip.set_margin_top(y.max(0));
            grip.set_size_request(w.max(1), h.max(1));
        }
        *self.layout.borrow_mut() = layout;
        self.widget.queue_draw();
    }
    /// Hover only changes paint; it never selects a tmux pane or moves keyboard focus.
    pub fn hover_at(&self, point: Option<(f64, f64)>) {
        let hovered = point.and_then(|(x, y)| self.layout.borrow().frame_at(x, y));
        if self.hovered_frame.replace(hovered) != hovered {
            self.widget.queue_draw();
        }
    }
    pub fn paint_ai(&self, state: &str, seconds: f64, english: bool) {
        use comandos_core::work_marks as marks;
        let state = marks::ai_status(state);
        let scale = self.widget.scale_factor().max(1);
        let frame = marks::ai_frame_index(state, seconds).unwrap_or(0);
        let signature = format!("{state}:{scale}:{frame}:{english}");
        if *self.ai_frame.borrow() == signature {
            return;
        }
        *self.ai_frame.borrow_mut() = signature;
        if let Some(pb) =
            self.cache
                .borrow_mut()
                .pixbuf(&format!("ai:{state}"), None, frame, scale, None)
        {
            for image in self.ai.borrow().iter() {
                if let Some(surface) = pb.create_surface(scale, image.window().as_ref()) {
                    image.set_from_surface(Some(&surface));
                } else {
                    image.set_from_pixbuf(Some(&pb));
                }
                image.set_tooltip_text(Some(&format!("IA: {}", marks::ai_label(state, english))));
            }
        }
    }
    #[allow(clippy::approx_constant)] // Exact original Cairo angles, compared by source oracle.
    pub fn draw(&self, cr: &cairo::Context) {
        let (line, brand, bg) = self.colors.borrow().clone();
        let color = |name: &str, fallback: &str| {
            gdk::RGBA::parse(name)
                .or_else(|_| gdk::RGBA::parse(fallback))
                .ok()
        };
        let (Some(line), Some(brand), Some(bg)) = (
            color(&line, "#223044"),
            color(&brand, "#8B7CF6"),
            color(&bg, "#0A0D13"),
        ) else {
            return;
        };
        let layout = self.layout.borrow();
        for (index, frame) in layout.frames.iter().enumerate() {
            let get = |index| frame.get(index).and_then(Value::as_f64).unwrap_or(0.0);
            let active = self.hovered_frame.get().map_or_else(
                || frame.get(4).and_then(Value::as_bool).unwrap_or(false),
                |hovered| hovered == index,
            );
            let c = if active { brand } else { line };
            cr.set_source_rgba(c.red(), c.green(), c.blue(), if active { 0.9 } else { 1.0 });
            cr.set_line_width(1.0);
            rounded(
                cr,
                get(0) + 0.5,
                get(1) + 0.5,
                (get(2) - 1.0).max(0.0),
                (get(3) - 1.0).max(0.0),
                10.0,
            );
            let _ = cr.stroke();
        }
        let Some(gutter) = self.hot.get().and_then(|index| layout.gutters.get(index)) else {
            return;
        };
        let dragging = self.drag.borrow().is_some();
        let vertical = gutter.orientation == "v";
        let cx = gutter.x + gutter.width / 2.0;
        let cy = gutter.y + gutter.height / 2.0;
        cr.set_source_rgba(
            brand.red(),
            brand.green(),
            brand.blue(),
            if dragging { 1.0 } else { 0.55 },
        );
        if vertical {
            cr.rectangle(cx - 1.0, gutter.y, 2.0, gutter.height);
        } else {
            cr.rectangle(gutter.x, cy - 1.0, gutter.width, 2.0);
        }
        let _ = cr.fill();
        let (long, thick) = if dragging { (44.0, 8.0) } else { (34.0, 6.0) };
        let (pw, ph) = if vertical {
            (thick, long)
        } else {
            (long, thick)
        };
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
        rounded(cr, cx - pw / 2.0, cy - ph / 2.0 + 1.0, pw, ph, thick / 2.0);
        let _ = cr.fill();
        cr.set_source_rgba(brand.red(), brand.green(), brand.blue(), 1.0);
        rounded(cr, cx - pw / 2.0, cy - ph / 2.0, pw, ph, thick / 2.0);
        let _ = cr.fill();
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
        for offset in [-7.0, 0.0, 7.0] {
            cr.arc(
                if vertical { cx } else { cx + offset },
                if vertical { cy + offset } else { cy },
                1.3,
                0.0,
                6.2832,
            );
            let _ = cr.fill();
        }
        if !dragging {
            return;
        }
        let label = gutter_measure(&self.geometries(), gutter);
        if label.is_empty() {
            return;
        }
        cr.select_font_face(
            "Ubuntu Sans Mono",
            cairo::FontSlant::Normal,
            cairo::FontWeight::Bold,
        );
        cr.set_font_size(11.0);
        let Ok(ext) = cr.text_extents(&label) else {
            return;
        };
        let bw = ext.width() + 16.0;
        let bh = 20.0;
        let bx = (cx - bw / 2.0).max(gutter.x - 200.0);
        let by = if vertical {
            cy - long / 2.0 - bh - 6.0
        } else {
            cy - bh - 10.0
        };
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
        rounded(cr, bx, by + 1.0, bw, bh, 5.0);
        let _ = cr.fill();
        cr.set_source_rgba(brand.red(), brand.green(), brand.blue(), 1.0);
        rounded(cr, bx, by, bw, bh, 5.0);
        let _ = cr.fill();
        cr.set_source_rgba(bg.red(), bg.green(), bg.blue(), 1.0);
        cr.move_to(
            bx + 8.0 - ext.x_bearing(),
            by + bh / 2.0 - ext.height() / 2.0 - ext.y_bearing(),
        );
        let _ = cr.show_text(&label);
    }
}
impl Drop for PaneOverlay {
    fn drop(&mut self) {
        self.cancel();
    }
}
#[allow(clippy::approx_constant)]
fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -1.5708, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, 1.5708);
    cr.arc(x + r, y + h - r, r, 1.5708, 3.1416);
    cr.arc(x + r, y + r, r, 3.1416, 4.7124);
    cr.close_path();
}
