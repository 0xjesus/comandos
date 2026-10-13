//! Selección, copia, enlaces y ligaduras: la parte pura (sin DOM) de lo que
//! xterm.js 5.5.0 y `dash/term.html` hacen sobre la rejilla. A6 pinta y
//! cablea el ratón; aquí solo se decide qué texto, qué enlaces y qué
//! secuencias.
//!
//! Las filas son **absolutas**: `0` es la primera fila de pantalla, la
//! historia es negativa (`-1` la fila justo encima) y no depende del
//! desplazamiento de la vista (`display_offset`).
//!
//! ## Diferencias con xterm.js / `term.html` (documentadas, no accidentales)
//!
//! - **Espacios finales.** Como `getTrimmedLength` de xterm.js: solo se
//!   recortan las celdas nunca escritas; un espacio que la aplicación escribió
//!   es contenido (`"abc   "` se copia entero). alacritty no lo distinguía de
//!   una celda borrada, así que el vendor marca las celdas escritas con
//!   `Flags::WRITTEN` (ver `vendor/alacritty_terminal/COMANDOS-PATCH.md`). Una
//!   fila con `WRAPLINE` en su última celda nunca se recorta. Los tabuladores
//!   (alacritty guarda `'\t'` en la primera celda del salto) cuentan como
//!   celdas vacías y se copian como espacios, como en xterm.js.
//! - **Cabeza inclusiva.** xterm.js guarda el final de la selección como
//!   exclusivo; [`Selection::head`] es la última celda seleccionada. Un clic
//!   sin arrastre no debe crear una [`Selection`] (xterm.js tampoco la
//!   considera selección).
//! - **Separadores de palabra** (`wordSeparator` por defecto de xterm.js 5.5):
//!   espacio y `()[]{}',"```. Salen del bundle, no de la lista del plan
//!   (que olvidaba la coma y el acento grave).
//! - **Enlaces.** Tres fuentes, con la precedencia de xterm.js (OSC 8,
//!   `WebLinksAddon`, proveedor de `term.html`) y una excepción documentada
//!   en [`find_urls`] (la URL con wrap duro de tmux no se abre truncada). El
//!   proveedor de tmux recorre como mucho 64 filas llenas a cada lado
//!   (`term.html` no tiene tope). La validación `new URL()` de
//!   `WebLinksAddon` se reduce a «anfitrión ASCII no vacío» (los IDN se
//!   rechazan como en xterm.js; `new URL` acepta anfitriones que empiezan por
//!   `_`, y aquí también). OSC 8 solo admite `http://` y `https://` (xterm.js
//!   acepta además cualquier URL cuyo protocolo analizado sea http/https,
//!   p. ej. `http:a.mx`; rechazar de más es lo seguro). `regex` no se usa: los
//!   dos patrones se evalúan a mano.
//! - **Alt+clic** ([`move_to_cell`]): copia de `moveToCellSequence` con sus
//!   rarezas (en la pantalla alternativa, hacia la izquierda en la misma fila
//!   no manda nada, y `horizontalDirection` recibe la columna destino como
//!   fila). Donde xterm.js podría recorrer la rejilla sin terminar, aquí el
//!   recorrido tiene tope y no se manda nada.
//! - **Ligaduras.** Los rangos son índices de carácter (`char`), no de
//!   UTF-16 ni de byte; el addon usa `m.index` (UTF-16) como columna, que
//!   coincide mientras no haya caracteres anchos ni astrales antes.
//!
//! ## Indexación
//!
//! `Grid` solo ofrece `Index` por `Line`: el único acceso está en [`cells`],
//! tras comprobar el rango (excepción a la regla de no indexar, como en
//! `render`).
use crate::engine::Engine;
use alacritty_terminal::{
    grid::Dimensions,
    index::Line,
    term::cell::{Cell, Flags},
};

/// Punto de la rejilla: (fila absoluta, columna).
pub type Point = (i32, u16);

/// Cómo se interpreta una [`Selection`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectMode {
    /// Arrastre: de `anchor` a `head`, ambas celdas incluidas.
    Simple,
    /// Doble clic (y arrastre): crece a palabras enteras en los dos extremos.
    Word,
    /// Triple clic: líneas lógicas enteras (con todas sus filas envueltas).
    Line,
    /// Alt+arrastre (`shouldColumnSelect`, modo 3 de xterm.js): el
    /// rectángulo de filas y columnas entre `anchor` y `head`, ambas celdas
    /// incluidas; cada fila se copia aparte, sin unir las envueltas.
    Block,
}

/// Selección en coordenadas absolutas. `anchor` y `head` pueden venir en
/// cualquier orden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Point,
    pub head: Point,
    pub mode: SelectMode,
}

/// Enlace encontrado: `start` y `end` son la primera y la última celda
/// (inclusivas); pueden estar en filas distintas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlSpan {
    pub start: Point,
    pub end: Point,
    pub url: String,
}

/// `wordSeparator` por defecto de xterm.js 5.5.0.
const WORD_SEPARATORS: &str = " ()[]{}',\"`";
/// Tope de caracteres a cada lado de la ventana de `WebLinksAddon`.
const WINDOW_LIMIT: usize = 2048;

// --- acceso a la rejilla ---------------------------------------------------

/// Celdas de la fila absoluta `row`, si existe (pantalla o historia).
fn cells(engine: &Engine, row: i32) -> Option<&[Cell]> {
    let grid = engine.term().grid();
    let line = Line(row);
    if line < grid.topmost_line() || line > grid.bottommost_line() {
        return None;
    }
    // `Grid` solo ofrece `Index`; el rango se comprobó arriba.
    Some(&grid[line][..])
}

fn first_row(engine: &Engine) -> i32 {
    engine.term().grid().topmost_line().0
}

fn last_row(engine: &Engine) -> i32 {
    engine.term().grid().bottommost_line().0
}

fn columns(engine: &Engine) -> usize {
    engine.term().grid().columns()
}

fn col16(col: usize) -> u16 {
    u16::try_from(col).unwrap_or(u16::MAX)
}

/// `isWrapped` de xterm.js: la fila anterior continúa en esta (alacritty
/// marca `WRAPLINE` en la última celda de la fila que se desborda).
fn is_wrapped(engine: &Engine, row: i32) -> bool {
    row.checked_sub(1)
        .and_then(|prev| cells(engine, prev))
        .and_then(<[Cell]>::last)
        .is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE))
}

fn is_spacer(cell: &Cell) -> bool {
    cell.flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
}

/// Carácter de la celda como lo ve `translateToString`: las celdas sin
/// escribir (y las que dejó un tabulador, que en xterm.js son NULL) son
/// espacios.
fn cell_char(cell: &Cell) -> char {
    match cell.c {
        '\t' | '\0' => ' ',
        c => c,
    }
}

/// Contenido de la celda según `BufferLine.getTrimmedLength` de xterm.js: un
/// carácter, o un espacio que escribió la aplicación (`Flags::WRITTEN`, del
/// parche del vendor). Las celdas borradas y las que deja un tabulador no
/// cuentan. Las mitades de relleno de un carácter ancho tampoco: ya las
/// cubre el ancho de su primera mitad.
fn has_content(cell: &Cell) -> bool {
    !is_spacer(cell)
        && (cell.flags.contains(Flags::WRITTEN)
            || !matches!(cell.c, ' ' | '\t' | '\0')
            || cell.zerowidth().is_some_and(|marks| !marks.is_empty()))
}

/// Espacio «de verdad» (lo que xterm.js ve como `' '` en la fila): ni mitad
/// de un carácter ancho ni otro carácter.
fn is_space(cell: &Cell) -> bool {
    !is_spacer(cell) && cell_char(cell) == ' '
}

/// `getTrimmedLength`: columnas hasta la última celda con contenido (un
/// carácter ancho ocupa dos). Una fila cuya última celda lleva `WRAPLINE`
/// nunca se recorta (como `LineLength::line_length` de alacritty): la fila
/// siguiente continúa su texto, y un espacio en el borde es parte de él.
fn trimmed_len(row: &[Cell]) -> usize {
    if row
        .last()
        .is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE))
    {
        return row.len();
    }
    row.iter().rposition(has_content).map_or(0, |index| {
        let wide = row
            .get(index)
            .is_some_and(|cell| cell.flags.contains(Flags::WIDE_CHAR));
        (index + 1 + usize::from(wide)).min(row.len())
    })
}

/// `translateToString(true, start, end)`: texto de las columnas
/// `start..end` recortado por la derecha; la segunda mitad de un carácter
/// ancho no aporta nada.
fn row_text(row: &[Cell], start: usize, end: Option<usize>) -> String {
    let limit = trimmed_len(row).min(end.unwrap_or(usize::MAX));
    let mut out = String::new();
    for cell in row.get(start..limit).unwrap_or_default() {
        if is_spacer(cell) {
            continue;
        }
        out.push(cell_char(cell));
        for mark in cell.zerowidth().unwrap_or_default() {
            out.push(*mark);
        }
    }
    out
}

// --- selección --------------------------------------------------------------

/// Extremos de la selección ya ordenados, recortados a la rejilla y
/// expandidos según el modo; `None` si la rejilla no tiene filas.
pub fn selection_bounds(engine: &Engine, s: &Selection) -> Option<(Point, Point)> {
    let (first, last) = (first_row(engine), last_row(engine));
    let max_col = col16(columns(engine).saturating_sub(1));
    let clamp = |(row, col): Point| (row.clamp(first, last), col.min(max_col));
    let (a, b) = (clamp(s.anchor), clamp(s.head));
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    Some(match s.mode {
        SelectMode::Simple => (lo, hi),
        SelectMode::Word => {
            let start = word_extent(engine, lo).map_or(lo, |(start, _)| start);
            let end = word_extent(engine, hi).map_or(hi, |(_, end)| end);
            (start, end)
        }
        SelectMode::Line => {
            let mut top = lo.0;
            while top > first && is_wrapped(engine, top) {
                top -= 1;
            }
            let mut bottom = hi.0;
            while bottom < last && is_wrapped(engine, bottom + 1) {
                bottom += 1;
            }
            ((top, 0), (bottom, max_col))
        }
        SelectMode::Block => ((lo.0, a.1.min(b.1)), (hi.0, a.1.max(b.1))),
    })
}

/// Texto seleccionado como lo copia xterm.js: las filas envueltas se unen sin
/// salto, el resto se separa con `\n`, cada fila va sin espacios finales y
/// el espacio de no separación se copia como espacio normal.
pub fn selected_text(engine: &Engine, s: &Selection) -> String {
    let Some(((row0, col0), (row1, col1))) = selection_bounds(engine, s) else {
        return String::new();
    };
    let mut parts: Vec<String> = Vec::new();
    let piece = |row: i32, from: usize, to: Option<usize>| {
        cells(engine, row)
            .map(|line| row_text(line, from, to))
            .unwrap_or_default()
    };
    let end_col = usize::from(col1) + 1;
    if s.mode == SelectMode::Block {
        // `selectionText` en modo columna: cada fila recortada a las
        // columnas, sin unir las envueltas.
        for row in row0..=row1 {
            parts.push(piece(row, usize::from(col0), Some(end_col)));
        }
        return parts.join("\n").replace('\u{a0}', " ");
    }
    parts.push(piece(
        row0,
        usize::from(col0),
        (row0 == row1).then_some(end_col),
    ));
    let push = |row: i32, text: String, parts: &mut Vec<String>| match parts.last_mut() {
        Some(last) if is_wrapped(engine, row) => last.push_str(&text),
        _ => parts.push(text),
    };
    let mut row = row0.saturating_add(1);
    while row < row1 {
        push(row, piece(row, 0, None), &mut parts);
        row += 1;
    }
    if row0 != row1 {
        push(row1, piece(row1, 0, Some(end_col)), &mut parts);
    }
    parts.join("\n").replace('\u{a0}', " ")
}

/// Palabra bajo `point` como la elige el doble clic de xterm.js
/// (`_getWordAt` con `allowWhitespaceOnlySelection`): sobre una racha de
/// espacios es la racha entera. Devuelve la primera y la última celda de la
/// palabra, que sigue por las filas envueltas; `None` solo fuera de la
/// rejilla.
pub fn word_at(engine: &Engine, point: Point) -> Option<(Point, Point)> {
    word_extent(engine, point)
}

fn is_separator(cell: &Cell) -> bool {
    !is_spacer(cell)
        && cell.zerowidth().is_none_or(<[char]>::is_empty)
        && WORD_SEPARATORS.contains(cell_char(cell))
}

/// Columnas `(inicio, fin)` de la palabra de `col` dentro de una fila.
fn word_cols(row: &[Cell], col: usize) -> Option<(usize, usize)> {
    let last = row.len().checked_sub(1)?;
    let mut col = col.min(last);
    // Mitad derecha de un carácter ancho: se parte de su primera mitad.
    if col > 0 && row.get(col).is_some_and(is_spacer) {
        col -= 1;
    }
    let at = |index: usize| row.get(index);
    if at(col).is_some_and(is_space) {
        let mut start = col;
        while start > 0 && at(start - 1).is_some_and(is_space) {
            start -= 1;
        }
        let mut end = col;
        while at(end + 1).is_some_and(is_space) {
            end += 1;
        }
        return Some((start, end));
    }
    let mut start = col;
    while start > 0 && at(start - 1).is_some_and(|cell| !is_separator(cell)) {
        start -= 1;
    }
    // Si se paró sobre la mitad derecha de un separador ancho, no es suya.
    if at(start).is_some_and(is_spacer) && start < col {
        start += 1;
    }
    let mut end = col;
    while at(end + 1).is_some_and(|cell| !is_separator(cell)) {
        end += 1;
    }
    Some((start, end))
}

/// Palabra con la extensión por filas envueltas de `_getWordAt`: si llega a
/// un borde y la fila vecina continúa sin espacio, la palabra sigue allí.
fn word_extent(engine: &Engine, point: Point) -> Option<(Point, Point)> {
    let (row, col) = point;
    let line = cells(engine, row)?;
    let (s, e) = word_cols(line, usize::from(col))?;
    let mut start = (row, col16(s));
    let mut end = (row, col16(e));
    let width = line.len();
    // Hacia atrás.
    let (mut cur, mut cur_start) = (row, s);
    while cur_start == 0
        && cells(engine, cur)
            .and_then(<[Cell]>::first)
            .is_some_and(|c| !is_space(c))
        && is_wrapped(engine, cur)
    {
        let prev = cur - 1;
        let Some(prev_cells) = cells(engine, prev) else {
            break;
        };
        if prev_cells.last().is_none_or(is_space) {
            break;
        }
        let Some((ps, _)) = word_cols(prev_cells, width) else {
            break;
        };
        start = (prev, col16(ps));
        cur = prev;
        cur_start = ps;
    }
    // Hacia delante.
    let (mut cur, mut cur_end) = (row, e);
    while cur_end + 1 == width
        && cells(engine, cur)
            .and_then(<[Cell]>::last)
            .is_some_and(|c| !is_space(c))
    {
        let next = cur + 1;
        let Some(next_cells) = cells(engine, next) else {
            break;
        };
        if !is_wrapped(engine, next) || next_cells.first().is_none_or(is_space) {
            break;
        }
        let Some((_, ne)) = word_cols(next_cells, 0) else {
            break;
        };
        end = (next, col16(ne));
        cur = next;
        cur_end = ne;
    }
    Some((start, end))
}

/// La celda `point` es la mitad derecha de un carácter ancho (`hasWidth`
/// 0 de xterm.js): un clic que cae en su borde izquierdo pasa a su borde
/// derecho (`_handleSingleClick`/`_handleMouseMove`).
pub fn is_wide_char_tail(engine: &Engine, point: Point) -> bool {
    cells(engine, point.0)
        .and_then(|row| row.get(usize::from(point.1)))
        .is_some_and(|cell| cell.flags.contains(Flags::WIDE_CHAR_SPACER))
}

// --- Alt+clic ------------------------------------------------------------------

/// `translateBufferLineToString(row, false, start, end).length`: unidades
/// UTF-16 de las columnas `start..end`, sin recortar; un carácter ancho salta
/// sus dos columnas y su mitad derecha suelta cuenta como un espacio. Una
/// fila que no existe no aporta nada.
fn js_row_len(engine: &Engine, row: i64, start: i64, end: i64) -> i64 {
    let Some(line) = i32::try_from(row).ok().and_then(|r| cells(engine, r)) else {
        return 0;
    };
    let end = end.min(i64::try_from(line.len()).unwrap_or(i64::MAX));
    let (mut col, mut len) = (start.max(0), 0i64);
    while col < end {
        let Some(cell) = usize::try_from(col).ok().and_then(|c| line.get(c)) else {
            break;
        };
        if is_spacer(cell) {
            len += 1;
            col += 1;
            continue;
        }
        let marks: usize = cell
            .zerowidth()
            .unwrap_or_default()
            .iter()
            .map(|m| m.len_utf16())
            .sum();
        len += i64::try_from(cell_char(cell).len_utf16() + marks).unwrap_or(1);
        col += if cell.flags.contains(Flags::WIDE_CHAR) {
            2
        } else {
            1
        };
    }
    len
}

/// Datos de la rejilla que usa `moveToCellSequence`.
struct MoveGrid<'a> {
    engine: &'a Engine,
    cols: i64,
    rows: i64,
}

impl MoveGrid<'_> {
    fn wrapped(&self, row: i64) -> bool {
        i32::try_from(row).is_ok_and(|r| is_wrapped(self.engine, r))
    }

    /// `wrappedRowsForRow`: filas envueltas encima de `row`.
    fn wrapped_rows_for_row(&self, mut row: i64) -> i64 {
        let mut count = 0;
        let mut wrapped = self.wrapped(row);
        while wrapped && row >= 0 && row < self.rows {
            count += 1;
            row -= 1;
            wrapped = self.wrapped(row);
        }
        count
    }

    /// `wrappedRowsCount`: filas envueltas entre las dos filas lógicas.
    fn wrapped_rows_between(&self, start_y: i64, target_y: i64) -> i64 {
        let start_row = start_y - self.wrapped_rows_for_row(start_y);
        let end_row = target_y - self.wrapped_rows_for_row(target_y);
        let dir = if start_y > target_y { -1 } else { 1 };
        (0..(start_row - end_row).abs())
            .filter(|n| self.wrapped(start_row + dir * n))
            .count() as i64
    }

    /// `moveToRequestedRow`: cuántas flechas verticales y en qué sentido.
    fn requested_row(&self, start_y: i64, target_y: i64) -> (i64, u8) {
        let start_row = start_y - self.wrapped_rows_for_row(start_y);
        let end_row = target_y - self.wrapped_rows_for_row(target_y);
        let n = (start_row - end_row).abs() - self.wrapped_rows_between(start_y, target_y);
        (n.max(0), if start_y > target_y { b'A' } else { b'B' })
    }

    /// `bufferLine(...).length`: celdas recorridas de `(start_x, start_y)` a
    /// `(end_x, end_y)`. xterm.js puede no terminar si el destino queda en
    /// el sentido contrario; aquí el recorrido tiene tope y entonces no se
    /// mueve nada.
    fn walk_len(&self, start: (i64, i64), end: (i64, i64), forward: bool) -> i64 {
        let ((mut from, mut row), (end_x, end_y)) = (start, end);
        let mut col = from;
        let mut len = 0;
        let mut budget = self.cols.saturating_mul(self.rows.saturating_add(2));
        while col != end_x || row != end_y {
            budget -= 1;
            if budget < 0 {
                return 0;
            }
            col += if forward { 1 } else { -1 };
            if forward && col > self.cols - 1 {
                len += js_row_len(self.engine, row, from, col);
                col = 0;
                from = 0;
                row += 1;
            } else if !forward && col < 0 {
                len += js_row_len(self.engine, row, 0, from + 1);
                col = self.cols - 1;
                from = col;
                row -= 1;
            }
        }
        len + js_row_len(self.engine, row, from, col)
    }
}

/// `moveToCellSequence` de xterm.js 5.5.0 (Alt+clic con
/// `altClickMovesCursor`): las flechas que llevan el cursor a `target`
/// (columna, fila de la pantalla). Sin historia (pantalla alternativa)
/// recorre las celdas de la rejilla como xterm.js, con su error de pasar la
/// columna destino como fila en `horizontalDirection`; con historia cuenta
/// celdas en línea recta.
pub fn move_to_cell(
    engine: &Engine,
    target: (u16, u16),
    app_cursor: bool,
    has_scrollback: bool,
) -> Vec<u8> {
    let grid = engine.term().grid();
    let g = MoveGrid {
        engine,
        cols: i64::try_from(grid.columns()).unwrap_or(0),
        rows: i64::try_from(grid.screen_lines()).unwrap_or(0),
    };
    let start_x = i64::try_from(grid.cursor.point.column.0).unwrap_or(0);
    let start_y = i64::from(grid.cursor.point.line.0);
    let (target_x, target_y) = (i64::from(target.0), i64::from(target.1));
    let mut out = Vec::new();
    let mut push = |n: i64, dir: u8| {
        for _ in 0..n.max(0) {
            out.extend_from_slice(&[0x1b, if app_cursor { b'O' } else { b'[' }, dir]);
        }
    };
    if !has_scrollback {
        // resetStartingRow
        let (rows, vertical) = g.requested_row(start_y, target_y);
        if rows > 0 {
            let back = start_y - g.wrapped_rows_for_row(start_y);
            push(g.walk_len((start_x, start_y), (start_x, back), false), b'D');
        }
        // moveToRequestedRow
        push(rows, vertical);
        // moveToRequestedCol
        let start_row = if rows > 0 {
            target_y - g.wrapped_rows_for_row(target_y)
        } else {
            start_y
        };
        let probe = if g.requested_row(target_x, target_y).0 > 0 {
            target_y - g.wrapped_rows_for_row(target_y)
        } else {
            start_y
        };
        let right =
            (start_x < target_x && probe <= target_y) || (start_x >= target_x && probe < target_y);
        let n = g.walk_len((start_x, start_row), (target_x, target_y), right);
        push(n, if right { b'C' } else { b'D' });
        return out;
    }
    if start_y == target_y {
        push(
            (start_x - target_x).abs(),
            if start_x > target_x { b'D' } else { b'C' },
        );
        return out;
    }
    let up = start_y > target_y;
    let rows = (start_y - target_y).abs();
    let n = (g.cols - if up { target_x } else { start_x })
        + (rows - 1) * g.cols
        + 1
        + ((if up { start_x } else { target_x }) - 1);
    push(n, if up { b'D' } else { b'C' });
    out
}

// --- enlaces -----------------------------------------------------------------

/// Un carácter de una fila con la celda donde está.
#[derive(Clone, Copy)]
struct Glyph {
    ch: char,
    row: i32,
    col: u16,
}

/// Caracteres de la fila recortada, uno por carácter (las marcas combinantes
/// comparten la columna de su base).
fn row_glyphs(row_cells: &[Cell], row: i32) -> Vec<Glyph> {
    let mut out = Vec::new();
    for (col, cell) in row_cells.iter().take(trimmed_len(row_cells)).enumerate() {
        if is_spacer(cell) {
            continue;
        }
        let col = col16(col);
        out.push(Glyph {
            ch: cell_char(cell),
            row,
            col,
        });
        for mark in cell.zerowidth().unwrap_or_default() {
            out.push(Glyph {
                ch: *mark,
                row,
                col,
            });
        }
    }
    out
}

/// Longitud en UTF-16 del texto de la fila (`lineTxt(i).length` de term.html).
fn utf16_len(glyphs: &[Glyph]) -> usize {
    glyphs.iter().map(|g| g.ch.len_utf16()).sum()
}

/// `\s` de JavaScript.
fn js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// `https?://` (o `HTTPS?://` si `upper_ok`) al comienzo de `text`; devuelve su
/// longitud en caracteres.
fn scheme_len(text: &[Glyph], upper_ok: bool) -> Option<usize> {
    let starts = |wanted: &str| {
        wanted
            .chars()
            .enumerate()
            .all(|(i, w)| text.get(i).is_some_and(|g| g.ch == w))
    };
    let variants: &[&str] = if upper_ok {
        &["https://", "http://", "HTTPS://", "HTTP://"]
    } else {
        &["https://", "http://"]
    };
    variants
        .iter()
        .find(|wanted| starts(wanted))
        .map(|wanted| wanted.len())
}

/// `/https?:\/\/[^\s'"<>\])]+/g` de `term.html`: rangos `[inicio, fin)` en
/// caracteres.
fn provider_matches(text: &[Glyph]) -> Vec<(usize, usize)> {
    let class = |c: char| !js_space(c) && !matches!(c, '\'' | '"' | '<' | '>' | ']' | ')');
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let Some(rest) = text.get(i..) else { break };
        if let Some(head) = scheme_len(rest, false) {
            let body = rest.get(head..).unwrap_or_default();
            let mut run = body.iter().take_while(|g| class(g.ch)).count();
            // Like `dash/term.html`: the sentence's closing period or comma is
            // not part of the link.
            while run > 0
                && body
                    .get(run - 1)
                    .is_some_and(|g| matches!(g.ch, '.' | ',' | ';' | ':' | '!' | '?'))
            {
                run -= 1;
            }
            if run > 0 {
                out.push((i, i + head + run));
                i += head + run;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Expresión de `WebLinksAddon` 0.11.0:
/// `/(https?|HTTPS?):[/]{2}[^\s"'!*(){}|\\\^<>`]*[^\s"':,.!?{}|\\\^~\[\]`()<>]/`.
fn weblink_matches(text: &[Glyph]) -> Vec<(usize, usize)> {
    let body = |c: char| {
        !js_space(c)
            && !matches!(
                c,
                '"' | '\'' | '!' | '*' | '(' | ')' | '{' | '}' | '|' | '\\' | '^' | '<' | '>' | '`'
            )
    };
    let last = |c: char| {
        !js_space(c)
            && !matches!(
                c,
                '"' | '\''
                    | ':'
                    | ','
                    | '.'
                    | '!'
                    | '?'
                    | '{'
                    | '}'
                    | '|'
                    | '\\'
                    | '^'
                    | '~'
                    | '['
                    | ']'
                    | '`'
                    | '('
                    | ')'
                    | '<'
                    | '>'
            )
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let Some(rest) = text.get(i..) else { break };
        let found = scheme_len(rest, true).and_then(|head| {
            let tail = rest.get(head..).unwrap_or_default();
            let run = tail.iter().take_while(|g| body(g.ch)).count();
            // Retroceso de `[^…]*` hasta que el siguiente carácter sirva de final.
            (0..=run)
                .rev()
                .find(|k| tail.get(*k).is_some_and(|g| last(g.ch)))
                .map(|k| head + k + 1)
        });
        match found {
            Some(len) => {
                let candidate: String = rest.iter().take(len).map(|g| g.ch).collect();
                // `RegExp.exec` con `g` reanuda tras la coincidencia aunque la
                // validación posterior la descarte.
                if valid_web_url(&candidate) {
                    out.push((i, i + len));
                }
                i += len;
            }
            None => i += 1,
        }
    }
    out
}

/// Reducción de la validación `new URL(..)` de `WebLinksAddon` (que exige
/// que `protocol//host` sea prefijo de la URL): tras `://` debe haber un
/// anfitrión ASCII no vacío que empiece por letra, cifra, `_`, `-` o `[`. Un
/// anfitrión IDN se rechaza, como en xterm.js (el `host` sale en punycode y
/// el prefijo no coincide); `new URL` acepta `_` y `-` iniciales.
fn valid_web_url(url: &str) -> bool {
    let Some((_, rest)) = url.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    host.is_ascii()
        && host
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '[' | '_' | '-'))
}

/// Tramo de glifos `[from, to)` como `UrlSpan` (extremos inclusivos).
fn span_of(glyphs: &[Glyph], from: usize, to: usize) -> Option<UrlSpan> {
    let first = glyphs.get(from)?;
    let last = glyphs.get(to.checked_sub(1)?)?;
    Some(UrlSpan {
        start: (first.row, first.col),
        end: (last.row, last.col),
        url: glyphs.get(from..to)?.iter().map(|g| g.ch).collect(),
    })
}

/// Tope de filas llenas que el proveedor de tmux recorre a cada lado de la
/// pedida. `term.html` no pone ninguno (recorre toda la historia en cada
/// hover); 64 filas son más de 5 000 caracteres con 80 columnas, bastante más
/// que cualquier URL, y acotan el trabajo de `find_urls` por llamada.
const MAX_FULL_ROWS: usize = 64;

/// Proveedor de URL envueltas por el wrap duro de tmux de `term.html`: junta
/// el bloque de filas llenas contiguas que toca `line` (hasta
/// [`MAX_FULL_ROWS`] a cada lado) y devuelve las URL que cruzan filas. Los
/// glifos de cada fila se calculan una sola vez.
fn tmux_wrapped(engine: &Engine, line: i32) -> Vec<UrlSpan> {
    let width = columns(engine);
    let (first, last) = (first_row(engine), last_row(engine));
    let glyphs_of = |row: i32| {
        cells(engine, row)
            .map(|c| row_glyphs(c, row))
            .unwrap_or_default()
    };
    let full = |glyphs: &[Glyph]| utf16_len(glyphs) >= width;
    let own = glyphs_of(line);
    // Hacia arriba mientras la fila anterior esté llena.
    let mut above: Vec<Vec<Glyph>> = Vec::new();
    let mut row = line;
    while row > first && above.len() < MAX_FULL_ROWS {
        let glyphs = glyphs_of(row - 1);
        if !full(&glyphs) {
            break;
        }
        above.push(glyphs);
        row -= 1;
    }
    // Hacia abajo mientras la fila actual esté llena (la primera no llena se
    // incluye, como `bot` de term.html).
    let mut below: Vec<Vec<Glyph>> = Vec::new();
    let mut row = line;
    let mut current_full = full(&own);
    while current_full && row < last && below.len() < MAX_FULL_ROWS {
        row += 1;
        let glyphs = glyphs_of(row);
        current_full = full(&glyphs);
        below.push(glyphs);
    }
    if above.is_empty() && below.is_empty() {
        return Vec::new();
    }
    let mut joined: Vec<Glyph> = above.into_iter().rev().flatten().collect();
    joined.extend(own);
    joined.extend(below.into_iter().flatten());
    provider_matches(&joined)
        .into_iter()
        .filter_map(|(s, e)| span_of(&joined, s, e))
        .filter(|span| span.start.0 != span.end.0)
        .filter(|span| span.start.0 <= line && line <= span.end.0)
        .collect()
}

/// Ventana de `WebLinksAddon` (`_getWindowedLineStrings`): la fila pedida
/// más las envueltas que la continúan por arriba y por abajo.
fn weblink_window(engine: &Engine, line: i32) -> Vec<Glyph> {
    let glyphs_of = |row: i32| {
        cells(engine, row)
            .map(|c| row_glyphs(c, row))
            .unwrap_or_default()
    };
    let has_space = |glyphs: &[Glyph]| glyphs.iter().any(|g| g.ch == ' ');
    let own = glyphs_of(line);
    let mut above: Vec<Vec<Glyph>> = Vec::new();
    if is_wrapped(engine, line) && own.first().is_some_and(|g| g.ch != ' ') {
        let (mut row, mut count) = (line, 0_usize);
        loop {
            row -= 1;
            if cells(engine, row).is_none() || count >= WINDOW_LIMIT {
                break;
            }
            let glyphs = glyphs_of(row);
            count += utf16_len(&glyphs);
            let keep_going = is_wrapped(engine, row) && !has_space(&glyphs);
            above.push(glyphs);
            if !keep_going {
                break;
            }
        }
    }
    let mut window: Vec<Glyph> = above.into_iter().rev().flatten().collect();
    window.extend(own);
    let (mut row, mut count) = (line, 0_usize);
    loop {
        row += 1;
        if cells(engine, row).is_none() || !is_wrapped(engine, row) || count >= WINDOW_LIMIT {
            break;
        }
        let glyphs = glyphs_of(row);
        count += utf16_len(&glyphs);
        let stop = has_space(&glyphs);
        window.extend(glyphs);
        if stop {
            break;
        }
    }
    window
}

fn weblinks(engine: &Engine, line: i32) -> Vec<UrlSpan> {
    let window = weblink_window(engine, line);
    weblink_matches(&window)
        .into_iter()
        .filter_map(|(s, e)| span_of(&window, s, e))
        .filter(|span| span.start.0 <= line && line <= span.end.0)
        .collect()
}

/// Hipervínculos OSC 8 de la fila: tramos seguidos de celdas con el mismo
/// enlace, solo http/https (como `OscLinkProvider` de xterm.js).
fn osc8(engine: &Engine, line: i32) -> Vec<UrlSpan> {
    let Some(row) = cells(engine, line) else {
        return Vec::new();
    };
    let limit = trimmed_len(row);
    let mut out = Vec::new();
    let mut run: Option<(usize, alacritty_terminal::term::cell::Hyperlink)> = None;
    let mut close = |run: Option<(usize, alacritty_terminal::term::cell::Hyperlink)>,
                     end: usize| {
        let Some((from, link)) = run else { return };
        let uri = link.uri();
        let lower = uri.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            out.push(UrlSpan {
                start: (line, col16(from)),
                end: (line, col16(end.saturating_sub(1))),
                url: uri.to_owned(),
            });
        }
    };
    for (col, cell) in row.iter().take(limit).enumerate() {
        let link = cell.hyperlink();
        let same = match (&run, &link) {
            (Some((_, current)), Some(link)) => current == link,
            _ => false,
        };
        if !same {
            close(run.take(), col);
            run = link.map(|link| (col, link));
        }
    }
    close(run, limit);
    out
}

fn overlaps(a: &UrlSpan, b: &UrlSpan) -> bool {
    a.start <= b.end && b.start <= a.end
}

/// Enlaces que tocan la fila absoluta `line`, ordenados por posición.
///
/// Precedencia de xterm.js (gana el primero que cubre una celda): hipervínculos
/// OSC 8, luego `WebLinksAddon` y, el último, el proveedor de URL envueltas
/// por tmux de `term.html`. **Excepción (corrección de un fallo latente de
/// `term.html`):** si el proveedor de tmux encuentra una URL que empieza en
/// la misma celda que una coincidencia de `WebLinksAddon` pero acaba en una
/// fila posterior (el wrap duro de tmux: `WebLinksAddon` solo ve la primera
/// fila y la corta, p. ej. sin el «.» final de la fila llena), gana la
/// del proveedor de tmux, para que una URL con wrap duro no se abra
/// truncada. Con wrap suave ambas acaban en la misma fila y manda
/// `WebLinksAddon`, que no incluye la puntuación final. Al sustituir se
/// retiran todas las coincidencias de `WebLinksAddon` que el tramo de tmux
/// cubre (el resultado nunca tiene solapes); si pisara un enlace OSC 8 no se
/// sustituye nada.
pub fn find_urls(engine: &Engine, line: i32) -> Vec<UrlSpan> {
    /// Quién produjo cada enlace conservado.
    #[derive(PartialEq)]
    enum Source {
        Osc8,
        WebLinks,
        Tmux,
    }
    let mut out: Vec<(Source, UrlSpan)> = Vec::new();
    for span in osc8(engine, line) {
        if !out.iter().any(|(_, kept)| overlaps(kept, &span)) {
            out.push((Source::Osc8, span));
        }
    }
    for span in weblinks(engine, line) {
        if !out.iter().any(|(_, kept)| overlaps(kept, &span)) {
            out.push((Source::WebLinks, span));
        }
    }
    for span in tmux_wrapped(engine, line) {
        let clashes: Vec<usize> = out
            .iter()
            .enumerate()
            .filter(|(_, (_, kept))| overlaps(kept, &span))
            .map(|(index, _)| index)
            .collect();
        if clashes.is_empty() {
            out.push((Source::Tmux, span));
            continue;
        }
        // Solo sustituye si todo lo que pisa es de WebLinks y el primero
        // empieza en su misma celda y acaba antes; así no queda ningún solape.
        let replaces = clashes.iter().all(|index| {
            out.get(*index)
                .is_some_and(|(source, _)| *source == Source::WebLinks)
        }) && clashes
            .first()
            .and_then(|index| out.get(*index))
            .is_some_and(|(_, kept)| kept.start == span.start && span.end.0 > kept.end.0);
        if replaces {
            let mut index = 0;
            out.retain(|_| {
                let drop = clashes.contains(&index);
                index += 1;
                !drop
            });
            out.push((Source::Tmux, span));
        }
    }
    let mut spans: Vec<UrlSpan> = out.into_iter().map(|(_, span)| span).collect();
    spans.sort_unstable_by_key(|span| span.start);
    spans
}

// --- ligaduras ---------------------------------------------------------------

/// Secuencias candidatas de `assets/xterm/addon-ligatures-web.js`, en el
/// orden del addon (dato copiado tal cual; el addon las ordena por longitud
/// y aquí se elige la más larga que case en cada posición).
const CANDIDATES: [&str; 116] = [
    "<===>", "<==>", "<-->", "<-<<", "<<--", "<<==", "==>>", "<==>", "<==<", "===>", "<=>", "<=<",
    "<==", "===", "==>", "!==", "<--", "-->", "-<<", "->>", "<<-", "<<=", ">>=", "||=", "&&&",
    "+++", "---", "***", "...", "::=", "::<", "::>", "//=", "//<", "//>", "///", "###", "##_",
    "?!.", "?::", "?<>", "~~>", "<~~", "|||", "|=>", "||>", "|>=", "<|>", "<|=", "|=<", "}}<",
    "{{-", "!!}", "<=", ">=", "==", "!=", "=>", "->", "<-", "<<", ">>", "::", "&&", "||", "++",
    "--", "**", "//", "/*", "*/", "</", "/>", "<>", "<|", "|>", "##", "..", "::", ".=", ".?", "?=",
    "?.", "?:", "~=", "~@", "~~", "^=", "<~", "~>", "$>", "#!", "#(", "#{", "#[", "#:", "#?", "#=",
    "#_", "\\/", "/\\", "_(", "__", "@_", "}}", "{{", "{|", "[|", "[<", "|-", "|=", "|]", "|}",
    "|{", ";:", "@?",
];

/// Rangos `[inicio, fin)` (en caracteres) de las secuencias candidatas a
/// ligadura de `text`: de izquierda a derecha, sin solaparse, y en cada
/// posición la candidata más larga (la búsqueda voraz del addon).
pub fn ligature_runs(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let rest = chars.get(i..).unwrap_or_default();
        let best = CANDIDATES
            .iter()
            .filter(|cand| {
                let mut it = rest.iter();
                cand.chars().all(|c| it.next() == Some(&c))
            })
            .map(|cand| cand.chars().count())
            .max();
        match best {
            Some(len) => {
                out.push((i, i + len));
                i += len;
            }
            None => i += 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{GridSize, Palette};

    #[test]
    fn ascii_host_guard_preserves_every_original_ascii_prefix() {
        let previous = |url: &str| {
            let Some((_, rest)) = url.split_once("://") else {
                return false;
            };
            let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
            let host = authority.rsplit('@').next().unwrap_or_default();
            host.is_ascii()
                && host
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || matches!(c, '[' | '_' | '-'))
        };
        for byte in 0..=127u8 {
            for prefix in ["", "user@", "user:pass@"] {
                let url = format!("https://{prefix}{}example.mx/p?q=1#f", char::from(byte));
                assert_eq!(valid_web_url(&url), previous(&url), "{url:?}");
            }
        }
        for host in [
            "",
            "é.mx",
            "a雪.mx",
            "K.mx",
            "ſ.mx",
            "Σ.mx",
            "😀.mx",
            "[::1]",
            "_host",
            "-host",
            "EXAMPLE.MX",
        ] {
            let url = format!("https://{host}/p");
            assert_eq!(valid_web_url(&url), previous(&url), "{url:?}");
        }
    }

    #[test]
    fn row_glyphs_reads_tabs_as_spaces() {
        let mut e = Engine::new(
            GridSize { cols: 20, rows: 3 },
            10,
            Palette::xterm_default([255; 3], [0; 3], [255; 3], [0; 3], [80; 3]),
        );
        e.advance(b"a\tb", 0.0);
        let text: String = cells(&e, 0)
            .map(|row| row_glyphs(row, 0))
            .unwrap_or_default()
            .iter()
            .map(|g| g.ch)
            .collect();
        assert_eq!(text, "a       b");
    }
}
