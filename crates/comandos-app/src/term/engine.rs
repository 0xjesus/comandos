//! Única puerta a `comandos-term` (F22): el resto del crate importa de aquí. Si la
//! Fase 3 cambia una firma, solo cambia este archivo y quien use lo cambiado.
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Line;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color, NamedColor};
pub use comandos_term::engine::{
    ClipboardTarget, Damage, Drained, Engine, GridSize, Modes, MouseMode, Palette,
};
pub use comandos_term::glyphs::{CellMetrics, DrawOp, draw_ops};
pub use comandos_term::input::{
    Button, Ime, KeyAction, KeyInput, MouseKind, WheelAction, encode_key, encode_mouse,
    encode_paste, focus, wheel,
};
pub use comandos_term::render::{
    CursorShape, CursorView, RenderOpts, RowRender, Run, RunKind, Style, Underline, cursor,
    render_row_into,
};
pub use comandos_term::select::{
    Point, SelectMode, Selection, UrlSpan, find_urls, selected_text, selection_bounds, word_at,
};
use std::time::{Duration, Instant};

/// `set_scrollback_lines(10000)` de `make_term` (795).
pub const SCROLLBACK: usize = 10_000;
/// Celdas del mosaico (`_mosaic_term`).
pub const MOSAIC_SCROLLBACK: usize = 400;
/// VTE: la negrita no aclara el color; sin ajuste de contraste.
pub const VTE_OPTS: RenderOpts = RenderOpts {
    bold_is_bright: false,
    min_contrast: 1.0,
};

pub struct TermEngine {
    engine: Engine,
    epoch: Instant,
    palette: Palette,
    cols: u16,
    rows: u16,
}

impl TermEngine {
    pub fn new(
        cols: u16,
        rows: u16,
        scrollback: usize,
        palette: Palette,
        blink: bool,
        epoch: Instant,
    ) -> TermEngine {
        let (cols, rows) = (cols.max(2), rows.max(1));
        let engine =
            Engine::with_cursor_blink(GridSize { cols, rows }, scrollback, palette.clone(), blink);
        TermEngine {
            engine,
            epoch,
            palette,
            cols,
            rows,
        }
    }

    fn ms(&self, now: Instant) -> f64 {
        now.saturating_duration_since(self.epoch).as_secs_f64() * 1000.0
    }

    pub fn feed(&mut self, bytes: &[u8], now: Instant) {
        let ms = self.ms(now);
        self.engine.advance(bytes, ms);
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let ms = self.ms(now);
        self.engine.tick(ms)
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        let ms = self.engine.next_deadline_ms()?;
        Some(self.epoch + Duration::from_secs_f64((ms / 1000.0).max(0.0)))
    }

    pub fn take_dirty(&mut self, lines: &mut Vec<usize>) -> bool {
        if self.engine.next_deadline_ms().is_some() {
            lines.clear();
            return false;
        }
        self.engine.take_damage_into(lines)
    }

    pub fn render_line(&self, line: usize, out: &mut RowRender) {
        render_row_into(&self.engine, line, &self.palette, &VTE_OPTS, out);
        // xterm drops backgrounds equal to the theme. Desktop opacity still
        // requires an opaque rectangle for explicit colors, including spaces.
        let Some(cells) = self.row_cells(line) else {
            return;
        };
        let mut covered = vec![false; cells.len()];
        for (start, count, _) in &out.bg_runs {
            let start = usize::from(*start);
            let end = start.saturating_add(usize::from(*count)).min(covered.len());
            if let Some(span) = covered.get_mut(start..end) {
                span.fill(true);
            }
        }
        for (column, cell) in cells.iter().enumerate() {
            let Ok(col) = u16::try_from(column) else {
                continue;
            };
            let explicit = cell.bg != Color::Named(NamedColor::Background)
                || cell.flags.contains(Flags::INVERSE);
            if explicit && !covered.get(column).copied().unwrap_or(false) {
                out.bg_runs.push((col, 1, self.palette.bg));
            }
        }
        out.bg_runs.sort_by_key(|run| run.0);
        out.bg_runs.dedup_by(|next, previous| {
            if previous.2 == next.2 && previous.0.saturating_add(previous.1) == next.0 {
                previous.1 = previous.1.saturating_add(next.1);
                true
            } else {
                false
            }
        });
    }

    pub fn cursor(&self) -> CursorView {
        cursor(&self.engine)
    }

    pub fn modes(&self) -> Modes {
        self.engine.modes()
    }

    pub fn drain(&mut self) -> Drained {
        self.engine.drain()
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cell_px: (u16, u16)) {
        let (cols, rows) = (cols.max(2), rows.max(1));
        self.cols = cols;
        self.rows = rows;
        self.engine.resize(GridSize { cols, rows }, cell_px);
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.engine.set_palette(palette.clone());
        self.palette = palette;
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    pub fn scroll(&mut self, lines: i32) {
        self.engine.scroll_display(lines);
    }

    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Todas las celdas de la vista, incluidos blancos y spacers; jamás se trunca.
    pub fn diagnostic_grid(&self, max_cells: usize) -> Result<serde_json::Value, String> {
        use serde_json::json;
        if usize::from(self.cols).saturating_mul(usize::from(self.rows)) > max_cells {
            return Err("terminal grid exceeds diagnostic limit".into());
        }
        let mut lines = Vec::new();
        for line in 0..usize::from(self.rows) {
            let cells = self.row_cells(line).ok_or("terminal row unavailable")?;
            let mut out = Vec::new();
            for cell in cells {
                let flags = cell.flags;
                let spacer =
                    flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
                let text = if spacer {
                    String::new()
                } else {
                    std::iter::once(if matches!(cell.c, '\0' | '\t') {
                        ' '
                    } else {
                        cell.c
                    })
                    .chain(cell.zerowidth().unwrap_or_default().iter().copied())
                    .collect()
                };
                let (foreground, background) = if flags.contains(Flags::INVERSE) {
                    (cell.bg, cell.fg)
                } else {
                    (cell.fg, cell.bg)
                };
                let mut fg = self.diagnostic_rgb(foreground);
                if flags.contains(Flags::DIM) && !matches!(foreground, Color::Spec(_)) {
                    fg = crate::term::paint::vte_dim(fg);
                }
                let bg = self.diagnostic_rgb(background);
                let underline = if flags.contains(Flags::DOUBLE_UNDERLINE) {
                    "double"
                } else if flags.contains(Flags::UNDERCURL) {
                    "curly"
                } else if flags.contains(Flags::DOTTED_UNDERLINE) {
                    "dotted"
                } else if flags.contains(Flags::DASHED_UNDERLINE) {
                    "dashed"
                } else if flags.contains(Flags::UNDERLINE) {
                    "single"
                } else {
                    "none"
                };
                out.push(json!({"text":text,"width":if spacer{0}else if flags.contains(Flags::WIDE_CHAR){2}else{1},"fg":fg,"bg":bg,"bold":flags.contains(Flags::BOLD),"italic":flags.contains(Flags::ITALIC),"dim":flags.contains(Flags::DIM),"hidden":flags.contains(Flags::HIDDEN),"inverse":flags.contains(Flags::INVERSE),"strike":flags.contains(Flags::STRIKEOUT),"underline":underline,"underline_color":cell.underline_color().map(|color|self.diagnostic_rgb(color)),"wrap":flags.contains(Flags::WRAPLINE),"hyperlink":cell.hyperlink().map(|link|link.uri().to_string())}));
            }
            lines.push(out);
        }
        Ok(
            json!({"cols":self.cols,"rows":self.rows,"display_offset":self.engine.display_offset(),"cells":lines}),
        )
    }

    fn diagnostic_rgb(&self, color: Color) -> [u8; 3] {
        let (index, fallback) = match color {
            Color::Spec(rgb) => return [rgb.r, rgb.g, rgb.b],
            Color::Indexed(index) => (
                usize::from(index),
                self.palette
                    .ansi
                    .get(usize::from(index))
                    .copied()
                    .unwrap_or(self.palette.fg),
            ),
            Color::Named(name) if (name as usize) < 16 => (
                name as usize,
                self.palette
                    .ansi
                    .get(name as usize)
                    .copied()
                    .unwrap_or(self.palette.fg),
            ),
            Color::Named(NamedColor::Background) => {
                (NamedColor::Background as usize, self.palette.bg)
            }
            Color::Named(NamedColor::Cursor) => (NamedColor::Cursor as usize, self.palette.cursor),
            Color::Named(
                name @ (NamedColor::DimBlack
                | NamedColor::DimRed
                | NamedColor::DimGreen
                | NamedColor::DimYellow
                | NamedColor::DimBlue
                | NamedColor::DimMagenta
                | NamedColor::DimCyan
                | NamedColor::DimWhite),
            ) => {
                let index = (name as usize).saturating_sub(NamedColor::DimBlack as usize);
                (
                    index,
                    self.palette
                        .ansi
                        .get(index)
                        .copied()
                        .unwrap_or(self.palette.fg),
                )
            }
            Color::Named(_) => (NamedColor::Foreground as usize, self.palette.fg),
        };
        self.diagnostic_changed_color(index).unwrap_or(fallback)
    }
    #[allow(clippy::indexing_slicing)]
    fn diagnostic_changed_color(&self, index: usize) -> Option<[u8; 3]> {
        // Colors sólo ofrece Index; el mismo límite del renderer se comprueba antes.
        if index >= alacritty_terminal::term::color::COUNT {
            return None;
        }
        self.engine.term().colors()[index].map(|rgb| [rgb.r, rgb.g, rgb.b])
    }

    fn visible_line(&self, line: usize) -> Option<Line> {
        let grid = self.engine.term().grid();
        if line >= grid.screen_lines() {
            return None;
        }
        let line = i32::try_from(line).ok()?;
        let offset = i32::try_from(self.engine.display_offset()).ok()?;
        Some(Line(line.checked_sub(offset)?))
    }
    // Grid exposes Index only. Validate both viewport and storage bounds before
    // accessing one row; callers then use checked slice access for columns.
    #[allow(clippy::indexing_slicing)]
    fn row_cells(&self, line: usize) -> Option<&[Cell]> {
        let grid = self.engine.term().grid();
        let point = self.visible_line(line)?;
        if point < grid.topmost_line() || point > grid.bottommost_line() {
            return None;
        }
        Some(&grid[point][..])
    }

    pub fn hyperlink_at(&self, row: u16, col: u16) -> Option<String> {
        self.row_cells(usize::from(row))?
            .get(usize::from(col))?
            .hyperlink()
            .map(|link| link.uri().to_string())
            .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
    }

    /// The live screen as text, one string per row, and the cursor's cell
    /// (column, row), wherever the view is scrolled to: what is compared
    /// with tmux's own screen of the pane.
    #[allow(clippy::indexing_slicing)]
    pub fn live_screen(&self) -> (Vec<String>, (u16, u16)) {
        let grid = self.engine.term().grid();
        let spacer = Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER;
        let rows = (0..grid.screen_lines())
            .map(|row| {
                let line = &grid[Line(i32::try_from(row).unwrap_or(0))];
                (0..grid.columns())
                    .map(|col| &line[alacritty_terminal::index::Column(col)])
                    .filter(|cell| !cell.flags.intersects(spacer))
                    .map(|cell| if cell.c == '\t' { ' ' } else { cell.c })
                    .collect()
            })
            .collect();
        let at = grid.cursor.point;
        let cell = (u16::try_from(at.column.0).unwrap_or(0), u16::try_from(at.line.0).unwrap_or(0));
        (rows, cell)
    }

    pub fn row_wraps(&self, row: usize) -> bool {
        self.row_cells(row)
            .and_then(|cells| cells.last())
            .is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE))
    }

    /// El color de primer plano de la celda es RGB directo (VTE no lo atenúa).
    pub fn fg_is_rgb(&self, line: usize, col: u16) -> bool {
        self.row_cells(line)
            .and_then(|cells| cells.get(usize::from(col)))
            .is_some_and(|cell| {
                let color = if cell.flags.contains(Flags::INVERSE) {
                    cell.bg
                } else {
                    cell.fg
                };
                matches!(color, Color::Spec(_))
            })
    }
}
