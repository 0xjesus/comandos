//! Plan de pintado por fila, sin cairo: rectángulos de fondo, tiras de texto,
//! glifos de caja (U+2500–U+259F y Powerline, de `comandos-term::glyphs`) y
//! líneas de subrayado/tachado, en píxeles del widget. T6 lo ejecuta.
use crate::term::engine::{CellMetrics, DrawOp, Palette, RowRender, RunKind, Underline, draw_ops};

pub fn hex_rgb(h: &str) -> Option<[u8; 3]> {
    let digits = h.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| {
        digits
            .get(i..i + 2)
            .and_then(|s| u8::from_str_radix(s, 16).ok())
    };
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// `set_colors(fg, bg, pal)` + `set_color_cursor(cursor)` de `make_term`.
pub fn palette_from_theme(fg: &str, bg: &str, cursor: &str, pal16: &[&str]) -> Option<Palette> {
    if pal16.len() != 16 {
        return None;
    }
    let (fg, bg, cursor) = (hex_rgb(fg)?, hex_rgb(bg)?, hex_rgb(cursor)?);
    let mut p = Palette::xterm_default(fg, bg, cursor, bg, fg);
    for (slot, hex) in p.ansi.iter_mut().zip(pal16.iter().take(16)) {
        *slot = hex_rgb(hex)?;
    }
    Some(p)
}

/// Atenuado de VTE (`rgb_from_index`): 2/3 de cada canal en 16 bits.
pub fn vte_dim(c: [u8; 3]) -> [u8; 3] {
    c.map(|v| {
        let wide = u32::from(v) * 257 * 2 / 3;
        u8::try_from(wide / 257).unwrap_or(u8::MAX)
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellGeom {
    pub cell_w: f64,
    pub cell_h: f64,
    /// Esquina superior izquierda de la celda (0, 0) dentro del widget (márgenes de `make_term`).
    pub origin_x: f64,
    pub origin_y: f64,
    pub dpr: f64,
    pub font_size: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Under(Underline),
    Strike,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaintOp {
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        rgb: [u8; 3],
    },
    Text {
        col: u16,
        x: f64,
        y: f64,
        cells: u16,
        text: String,
        rgb: [u8; 3],
        bold: bool,
        italic: bool,
    },
    Glyph {
        x: f64,
        y: f64,
        ops: Vec<DrawOp>,
        rgb: [u8; 3],
    },
    Line {
        x: f64,
        y: f64,
        w: f64,
        rgb: [u8; 3],
        kind: LineKind,
    },
}

pub fn plan_row(
    row: &RowRender,
    line: usize,
    g: &CellGeom,
    fg_is_rgb: &dyn Fn(u16) -> bool,
    out: &mut Vec<PaintOp>,
) {
    let y = g.origin_y + g.cell_h * f64::from(u32::try_from(line).unwrap_or(u32::MAX));
    let x_of = |col: u16| g.origin_x + g.cell_w * f64::from(col);
    for &(col, cells, rgb) in &row.bg_runs {
        out.push(PaintOp::Rect {
            x: x_of(col),
            y,
            w: g.cell_w * f64::from(cells),
            h: g.cell_h,
            rgb,
        });
    }
    let metrics = CellMetrics {
        cell_w: g.cell_w * g.dpr,
        cell_h: g.cell_h * g.dpr,
        dpr: g.dpr,
        font_size: g.font_size,
    };
    for run in &row.runs {
        if run.style.hidden {
            continue;
        }
        // xterm render runs merge equal resolved styles. VTE dim depends on
        // color provenance, so equal RGB/indexed cells must be separated here.
        let mut segments: Vec<(u16, u16, String, bool)> = Vec::new();
        if run.style.dim && run.kind == RunKind::Text && run.cells > 1 {
            for (col, text) in run.cell_texts() {
                let rgb = fg_is_rgb(col);
                if let Some(last) = segments
                    .last_mut()
                    .filter(|s| s.3 == rgb && s.0.saturating_add(s.1) == col)
                {
                    last.1 = last.1.saturating_add(1);
                    last.2.push_str(text);
                } else {
                    segments.push((col, 1, text.to_owned(), rgb));
                }
            }
        } else {
            segments.push((run.col, run.cells, run.text.clone(), fg_is_rgb(run.col)));
        }
        for (col, cells, text, direct_rgb) in segments {
            let base = run.style.dim_fg.unwrap_or(run.style.fg);
            let rgb = if run.style.dim && !direct_rgb {
                vte_dim(base)
            } else if run.style.dim {
                base
            } else {
                run.style.fg
            };
            let mut glyph = false;
            if let RunKind::Box(c) = run.kind {
                let mut ops = Vec::new();
                if draw_ops(c, &metrics, &mut ops) {
                    out.push(PaintOp::Glyph {
                        x: x_of(col),
                        y,
                        ops,
                        rgb,
                    });
                    glyph = true;
                }
            }
            if !glyph {
                out.push(PaintOp::Text {
                    col,
                    x: x_of(col),
                    y,
                    cells,
                    text,
                    rgb,
                    bold: run.style.bold,
                    italic: run.style.italic,
                });
            }
            let w = g.cell_w * f64::from(cells);
            let line_rgb = run.style.underline_color.unwrap_or(rgb);
            if run.style.underline != Underline::None {
                out.push(PaintOp::Line {
                    x: x_of(col),
                    y,
                    w,
                    rgb: line_rgb,
                    kind: LineKind::Under(run.style.underline),
                });
            }
            if run.style.strike {
                out.push(PaintOp::Line {
                    x: x_of(col),
                    y,
                    w,
                    rgb,
                    kind: LineKind::Strike,
                });
            }
        }
    }
}

/// Retained frame: incidental exposure/blink cannot reveal a partial DEC2026
/// update. Native tests exercise the same cache that GTK paints.
pub struct FrameCache {
    ops: Vec<PaintOp>,
    /// `ops[a..b]` belong to grid row `i`.
    row_ops: Vec<(usize, usize)>,
    /// What each grid row shows, for the noise filter.
    inks: Vec<crate::term::noise::RowInk>,
    /// tmux border cells, row-major, for the pane tiles.
    borders: Vec<crate::term::tiles::Cell>,
    cols: u16,
    cursor: crate::term::engine::CursorView,
}
impl Default for FrameCache {
    fn default() -> Self {
        Self {
            ops: Vec::new(),
            row_ops: Vec::new(),
            inks: Vec::new(),
            borders: Vec::new(),
            cols: 0,
            cursor: crate::term::engine::CursorView {
                line: 0,
                col: 0,
                shape: crate::term::engine::CursorShape::Block,
                visible: false,
                wide: false,
            },
        }
    }
}
/// Notes the tmux border cells of one rendered row.
fn mark_borders(borders: &mut [crate::term::tiles::Cell], row: &RowRender, line: usize, cols: u16) {
    for run in &row.runs {
        if !crate::term::tiles::is_border(run.style.fg) {
            continue;
        }
        let chars: Vec<char> = run.text.chars().collect();
        for i in 0..run.cells {
            let ch = chars.get(usize::from(i)).or(chars.last()).copied().unwrap_or(' ');
            let col = usize::from(run.col.saturating_add(i));
            if col < usize::from(cols)
                && let Some(cell) = borders.get_mut(line * usize::from(cols) + col)
            {
                *cell = crate::term::tiles::classify(ch, run.style.fg);
            }
        }
    }
}
impl FrameCache {
    pub fn refresh(&mut self, term: &crate::term::engine::TermEngine, geom: &CellGeom) -> bool {
        if term.next_deadline().is_some() {
            return false;
        }
        self.ops.clear();
        self.row_ops.clear();
        self.inks.clear();
        let (cols, rows) = term.size();
        self.cols = cols;
        self.borders.clear();
        self.borders
            .resize(usize::from(cols) * usize::from(rows), crate::term::tiles::Cell::Other);
        let normal = term.palette().fg;
        let mut row = RowRender::default();
        for line in 0..usize::from(term.size().1) {
            term.render_line(line, &mut row);
            self.inks.push(crate::term::noise::row_ink(&row, normal));
            mark_borders(&mut self.borders, &row, line, cols);
            let first = self.ops.len();
            plan_row(
                &row,
                line,
                geom,
                &|col| term.fg_is_rgb(line, col),
                &mut self.ops,
            );
            self.row_ops.push((first, self.ops.len()));
        }
        self.cursor = term.cursor();
        true
    }
    /// Same result as `refresh` when only `lines` changed since the last one:
    /// the other rows keep what was planned for them. Typing a letter or a
    /// spinner tick then costs one row, not the whole screen.
    pub fn refresh_lines(&mut self, term: &crate::term::engine::TermEngine, geom: &CellGeom, lines: &[usize]) -> bool {
        let (cols, rows) = term.size();
        let count = usize::from(rows);
        let mut lines: Vec<usize> = lines.iter().copied().filter(|l| *l < count).collect();
        lines.sort_unstable();
        lines.dedup();
        if self.cols != cols || self.row_ops.len() != count || self.inks.len() != count || lines.len() * 2 > count {
            return self.refresh(term, geom);
        }
        if term.next_deadline().is_some() {
            return false;
        }
        let normal = term.palette().fg;
        let width = usize::from(cols);
        let mut row = RowRender::default();
        let mut ops = Vec::new();
        for line in lines {
            term.render_line(line, &mut row);
            self.inks[line] = crate::term::noise::row_ink(&row, normal);
            if let Some(cells) = self.borders.get_mut(line * width..(line + 1) * width) {
                cells.fill(crate::term::tiles::Cell::Other);
            }
            mark_borders(&mut self.borders, &row, line, cols);
            plan_row(&row, line, geom, &|col| term.fg_is_rgb(line, col), &mut ops);
            let (a, b) = self.row_ops[line];
            let end = a + ops.len();
            self.ops.splice(a..b, ops.drain(..));
            self.row_ops[line] = (a, end);
            for later in &mut self.row_ops[line + 1..] {
                *later = (later.0 + end - b, later.1 + end - b);
            }
        }
        self.cursor = term.cursor();
        true
    }
    pub fn ops(&self) -> &[PaintOp] {
        &self.ops
    }
    pub fn row_ops(&self, line: usize) -> &[PaintOp] {
        self.row_ops
            .get(line)
            .and_then(|&(a, b)| self.ops.get(a..b))
            .unwrap_or(&[])
    }
    /// Temporary: what the border scan saw, for the lag log.
    pub fn border_debug(&self) -> String {
        let cols = usize::from(self.cols.max(1));
        let mut out = String::new();
        for (row, cells) in self.borders.chunks(cols).enumerate() {
            if cells.iter().all(|c| *c == crate::term::tiles::Cell::Other) {
                continue;
            }
            let line: String = cells
                .iter()
                .map(|c| match c {
                    crate::term::tiles::Cell::Other => '.',
                    crate::term::tiles::Cell::Post => '|',
                    crate::term::tiles::Cell::Rule { active: true } => '=',
                    crate::term::tiles::Cell::Rule { active: false } => '-',
                })
                .collect();
            out.push_str(&format!(" r{row}:{line}"));
            if out.len() > 900 {
                break;
            }
        }
        out
    }
    /// Panes drawn as tiles, from tmux's border cells on this frame.
    pub fn tiles(&self, bottom: u16) -> Vec<crate::term::tiles::Tile> {
        crate::term::tiles::detect(&self.borders, self.cols, bottom)
    }
    pub fn inks(&self) -> &[crate::term::noise::RowInk] {
        &self.inks
    }
    pub fn cursor(&self) -> crate::term::engine::CursorView {
        self.cursor
    }
}
