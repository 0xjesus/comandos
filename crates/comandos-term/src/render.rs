//! Render puro: convierte una fila de la rejilla del motor en «tiras» de
//! celdas con un mismo estilo, ya con los colores resueltos como los pinta
//! xterm.js 5.5.0 (renderizador canvas de `@xterm/addon-canvas` 0.7.0, el
//! que carga `dash/term.html`). No toca el DOM ni el canvas: A6 pinta las
//! tiras. Se ejecuta en cada cuadro, así que [`render_row_into`] reutiliza
//! los búferes de la llamada anterior.
//!
//! ## Colores (como xterm.js)
//!
//! - Con nombre o índice → paleta del tema (o el color que la aplicación
//!   cambió por OSC 4/10/11/12); RGB directo tal cual.
//! - Negrita + `bold_is_bright` (`drawBoldTextInBrightColors`): solo el
//!   **texto** con índice 0–7 pasa a 8–15 (nombrado o `38;5;n`).
//! - Inverso: el texto toma el color del fondo (y la negrita lo aclara); el
//!   fondo pintado toma el del texto, sin aclarar (`_drawBackground`).
//! - Atenuado: el texto se pinta a opacidad 0,5 (`DIM_OPACITY`) sobre el
//!   fondo de la celda: mezcla 128/255 hacia el fondo (`fg`); el color sin
//!   mezclar va en `dim_fg` para quien pinta con la opacidad real.
//! - `min_contrast` > 1 (`minimumContrastRatio`): el texto se aclara u
//!   oscurece en pasos del 10 % hasta el contraste pedido (la mitad si está
//!   atenuado, y entonces no se atenúa); los glifos de dibujo y de
//!   powerline quedan fuera, como en xterm.js.
//! - Oculto: el texto toma el color del fondo y no hay subrayado ni tachado
//!   (xterm.js no dibuja nada de la celda).
//! - Subrayado con color propio (SGR 58): ni atenuado ni invertido; la
//!   negrita aclara el índice 0–7. `None` = el color del texto, o no hay
//!   subrayado (así un SGR 58 sin subrayado no parte tiras).
//!
//! ## Tiras
//!
//! Una tira junta celdas seguidas que comparten estilo y enlace, para que
//! quien pinta (A6) cambie de fuente y color una sola vez. **No** implica
//! dibujar su texto de una vez: xterm.js coloca cada glifo en su celda
//! (`deviceCellWidth = floor(charWidth · dpr)`) y el avance real de la
//! fuente es fraccional, así que un `fillText` de la tira entera se iría
//! desplazando. A6 pinta cada **celda** (un carácter con sus marcas
//! combinantes, o un carácter ancho de dos columnas) en `col · cell_w`;
//! [`Run::cell_texts`] da `(columna, texto)` de cada celda: en una tira
//! `Text` de varias celdas cada carácter es una celda (un hueco es `" "`),
//! y una tira de una celda lleva el carácter con todas sus marcas.
//!
//! Se agrupan los caracteres ASCII imprimibles y latinos de ancho 1; los
//! espacios entre ellos se conservan y los del final no generan tira. Un
//! carácter ancho (`Wide`, dos celdas), uno con marcas combinantes o
//! cualquier otro va en su propia tira (el texto lleva el carácter y sus
//! marcas). Los glifos de [`crate::glyphs`] van en una `Box` de una celda,
//! salvo si llevan marcas combinantes: xterm.js busca la cadena entera de la
//! celda en sus tablas y entonces la pinta la fuente. El fondo va aparte en
//! `bg_runs`, solo donde no es el de la terminal.
//!
//! ## Indexación
//!
//! `Grid` (por `Line`) y `Colors` de alacritty solo ofrecen `Index`, sin
//! `get`: los tres accesos de este módulo comprueban el rango antes y
//! están anotados (excepción a la regla de no indexar, como en
//! `engine::color_for_request`).
use crate::{
    engine::{Engine, Palette},
    glyphs::box_ops,
};
use alacritty_terminal::{
    grid::Dimensions,
    index::Line,
    term::{
        TermMode,
        cell::{Cell, Flags},
        color::{COUNT as COLOR_COUNT, Colors},
    },
    vte::ansi::{Color, NamedColor},
};

pub use alacritty_terminal::vte::ansi::CursorShape;

/// Estilo ya resuelto de una celda.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub bold: bool,
    pub italic: bool,
    pub dim: bool,
    /// Color del texto antes de atenuarlo (`Some` solo si se atenuó): quien
    /// pinta lo usa con opacidad 0,5 como addon-canvas, porque la mezcla ya
    /// hecha en `fg` no da los mismos bordes alisados.
    pub dim_fg: Option<[u8; 3]>,
    pub underline: Underline,
    /// Color del subrayado (SGR 58); `None` = el del texto.
    pub underline_color: Option<[u8; 3]>,
    pub strike: bool,
    pub hidden: bool,
    /// Vídeo inverso (SGR 7): `bg` sale del color del texto.
    pub inverse: bool,
    /// La celda no tiene fondo propio (el del tema). Con `inverse` lo usa
    /// `CellColorResolver` de addon-canvas para el color de un glifo
    /// seleccionado.
    pub default_bg: bool,
    /// Color del texto de la paleta sin aclarar por la negrita (`Some` solo
    /// si la negrita lo aclaró): el que mezcla `CellColorResolver` en un
    /// glifo de fondo seleccionado (`ansi[fg & 255]` en crudo).
    pub fg_plain: Option<[u8; 3]>,
}

/// Subrayado (SGR 4, 4:2–4:5, 21).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// Qué contiene una tira.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// Texto de una celda por carácter.
    Text,
    /// Un carácter de dos celdas.
    Wide,
    /// Un glifo que se dibuja con [`crate::glyphs::box_ops`].
    Box(char),
}

/// Tira de celdas seguidas que se pintan juntas.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// Primera columna.
    pub col: u16,
    /// Celdas que ocupa.
    pub cells: u16,
    pub text: String,
    pub style: Style,
    pub kind: RunKind,
    /// Índice en [`RowRender::links`] del hipervínculo OSC 8.
    pub link: Option<u32>,
}

impl Run {
    /// `(columna, texto)` de cada celda de la tira, para pintarla en
    /// `columna · cell_w`. Una celda es un carácter con sus marcas
    /// combinantes (o un carácter ancho de dos columnas). Solo las tiras
    /// `Text` de varias celdas tienen más de una entrada, y en ellas cada
    /// carácter es una celda: nunca se agrupan celdas con marcas.
    pub fn cell_texts(&self) -> CellTexts<'_> {
        CellTexts {
            rest: self.text.as_str(),
            col: self.col,
            per_char: self.kind == RunKind::Text && self.cells > 1,
        }
    }
}

/// Iterador de [`Run::cell_texts`].
#[derive(Debug, Clone)]
pub struct CellTexts<'a> {
    rest: &'a str,
    col: u16,
    per_char: bool,
}

impl<'a> Iterator for CellTexts<'a> {
    type Item = (u16, &'a str);

    fn next(&mut self) -> Option<(u16, &'a str)> {
        if self.rest.is_empty() {
            return None;
        }
        let len = if self.per_char {
            self.rest.chars().next().map_or(0, char::len_utf8)
        } else {
            self.rest.len()
        };
        let (cell, rest) = self.rest.split_at_checked(len)?;
        let col = self.col;
        self.rest = rest;
        self.col = self.col.saturating_add(1);
        Some((col, cell))
    }
}

/// Una fila lista para pintar.
#[derive(Debug, Clone, Default)]
pub struct RowRender {
    /// Fila de la vista (0 = arriba), la misma que se pidió.
    pub line: usize,
    /// `(columna, celdas, color)` de los fondos que no son el de la terminal.
    pub bg_runs: Vec<(u16, u16, [u8; 3])>,
    pub runs: Vec<Run>,
    /// URI de los hipervínculos de la fila, sin repetir.
    pub links: Vec<String>,
    /// Cadenas vacías de tiras y enlaces que sobraron en un cuadro, para no
    /// volver a asignarlas cuando la fila vuelva a tener más.
    spare: Vec<String>,
}

/// Igualdad de lo que se pinta (sin las cadenas de reserva).
impl PartialEq for RowRender {
    fn eq(&self, other: &RowRender) -> bool {
        self.line == other.line
            && self.bg_runs == other.bg_runs
            && self.runs == other.runs
            && self.links == other.links
    }
}

/// Tope de cadenas de reserva por fila (una fila tiene como mucho tantas
/// tiras como columnas; más allá no merece la pena guardarlas).
const MAX_SPARE: usize = 512;

/// Opciones de color, con los nombres de xterm.js.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderOpts {
    /// `drawBoldTextInBrightColors` (xterm.js 5.5: `true`).
    pub bold_is_bright: bool,
    /// `minimumContrastRatio` (xterm.js 5.5: `1`, sin ajuste).
    pub min_contrast: f32,
}

impl Default for RenderOpts {
    /// Los valores por omisión de xterm.js 5.5.0, los que usa `term.html`.
    fn default() -> RenderOpts {
        RenderOpts {
            bold_is_bright: true,
            min_contrast: 1.0,
        }
    }
}

/// Dónde y cómo pintar el cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorView {
    /// Fila de la vista (con la historia desplazada puede quedar fuera).
    pub line: usize,
    pub col: u16,
    /// Forma pedida por la aplicación (DECSCUSR) o la de por omisión.
    pub shape: CursorShape,
    /// La aplicación lo muestra (DECTCEM) y cae dentro de la vista.
    pub visible: bool,
    /// Está sobre un carácter de dos celdas.
    pub wide: bool,
}

/// Fila `line` de la vista (0 = arriba) como tiras. Las filas fuera de la
/// pantalla (por ejemplo índices de daño con la vista desplazada) salen
/// vacías.
pub fn render_row(engine: &Engine, line: usize, palette: &Palette, opts: &RenderOpts) -> RowRender {
    let mut out = RowRender::default();
    render_row_into(engine, line, palette, opts, &mut out);
    out
}

/// Como [`render_row`], pero reutiliza los vectores y cadenas de `out`.
pub fn render_row_into(
    engine: &Engine,
    line: usize,
    palette: &Palette,
    opts: &RenderOpts,
    out: &mut RowRender,
) {
    out.line = line;
    out.bg_runs.clear();
    let spare = &mut out.spare;
    let mut runs = Runs::new(&mut out.runs);
    let mut links = Links::new(&mut out.links);
    if let Some(cells) = row_cells(engine, line) {
        let mut colors = Resolver::new(palette, engine.term().colors(), opts);
        for (col, cell) in cells.iter().enumerate() {
            let col = u16::try_from(col).unwrap_or(u16::MAX);
            let style = colors.style(cell);
            push_bg(&mut out.bg_runs, col, style.bg, palette.bg);
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let link = links.index_of(cell, spare);
            runs.push_cell(cell, col, style, link, spare);
        }
    }
    runs.finish(spare);
    links.finish(spare);
}

/// Celdas de la fila de la vista `line`, si está en pantalla.
fn row_cells(engine: &Engine, line: usize) -> Option<&[Cell]> {
    let grid = engine.term().grid();
    if line >= grid.screen_lines() {
        return None;
    }
    let view = i32::try_from(line).ok()?;
    let offset = i32::try_from(grid.display_offset()).ok()?;
    let index = Line(view - offset);
    if index < grid.topmost_line() || index > grid.bottommost_line() {
        return None;
    }
    // `Grid` solo ofrece `Index`; la línea se comprobó arriba.
    Some(&grid[index][..])
}

fn push_bg(bg_runs: &mut Vec<(u16, u16, [u8; 3])>, col: u16, bg: [u8; 3], default: [u8; 3]) {
    if bg == default {
        return;
    }
    if let Some(last) = bg_runs.last_mut()
        && last.2 == bg
        && last.0.saturating_add(last.1) == col
    {
        last.1 = last.1.saturating_add(1);
        return;
    }
    bg_runs.push((col, 1, bg));
}

/// Celda que no pinta nada encima del fondo.
fn is_blank(cell: &Cell, style: &Style) -> bool {
    matches!(cell.c, ' ' | '\t' | '\0')
        && style.underline == Underline::None
        && !style.strike
        && !cell.flags.contains(Flags::WIDE_CHAR)
        && cell.zerowidth().is_none_or(<[char]>::is_empty)
}

/// ASCII imprimible o latino de ancho 1: la fuente principal los dibuja con
/// el avance exacto de la celda, así que pueden ir juntos en una tira.
fn joins_runs(c: char) -> bool {
    matches!(c, ' '..='~' | '\u{A0}'..='\u{24F}' | '\u{1E00}'..='\u{1EFF}')
}

/// Constructor de tiras que reutiliza las de la llamada anterior.
struct Runs<'a> {
    out: &'a mut Vec<Run>,
    len: usize,
    /// La última tira admite más caracteres.
    open: bool,
    /// Espacios vistos tras la última tira abierta, aún sin añadir.
    blanks: u16,
}

impl<'a> Runs<'a> {
    fn new(out: &'a mut Vec<Run>) -> Runs<'a> {
        Runs {
            out,
            len: 0,
            open: false,
            blanks: 0,
        }
    }

    /// La tira abierta, si sigue con este estilo y enlace.
    fn open_run(&mut self, style: &Style, link: Option<u32>) -> Option<&mut Run> {
        if !self.open {
            return None;
        }
        let index = self.len.checked_sub(1)?;
        self.out
            .get_mut(index)
            .filter(|run| run.style == *style && run.link == link)
    }

    fn push_cell(
        &mut self,
        cell: &Cell,
        col: u16,
        style: Style,
        link: Option<u32>,
        spare: &mut Vec<String>,
    ) {
        if is_blank(cell, &style) {
            if self.open_run(&style, link).is_some() {
                self.blanks = self.blanks.saturating_add(1);
            } else {
                self.close();
            }
            return;
        }
        let c = cell.c;
        // Un tabulador o NUL con subrayado se pinta como espacio.
        let glyph = if matches!(c, '\t' | '\0') { ' ' } else { c };
        let zerowidth = cell.zerowidth().filter(|z| !z.is_empty());
        let wide = cell.flags.contains(Flags::WIDE_CHAR);
        // xterm.js busca la cadena entera de la celda en sus tablas: con
        // marcas combinantes, el glifo de dibujo lo pinta la fuente.
        let kind = if wide {
            RunKind::Wide
        } else if zerowidth.is_none() && box_ops(c).is_some() {
            RunKind::Box(c)
        } else {
            RunKind::Text
        };
        let joins = kind == RunKind::Text && zerowidth.is_none() && joins_runs(c);
        let blanks = self.blanks;
        if joins && let Some(run) = self.open_run(&style, link) {
            for _ in 0..blanks {
                run.text.push(' ');
            }
            run.text.push(glyph);
            run.cells = run.cells.saturating_add(blanks).saturating_add(1);
            self.blanks = 0;
            return;
        }
        self.close();
        if let Some(run) = self.start(col, style, kind, link, spare) {
            run.text.push(glyph);
            if let Some(marks) = zerowidth {
                run.text.extend(marks.iter());
            }
            run.cells = if wide { 2 } else { 1 };
            self.open = joins;
        }
    }

    fn close(&mut self) {
        self.open = false;
        self.blanks = 0;
    }

    /// Nueva tira vacía, reutilizando la cadena de una tira vieja si la hay.
    fn start(
        &mut self,
        col: u16,
        style: Style,
        kind: RunKind,
        link: Option<u32>,
        spare: &mut Vec<String>,
    ) -> Option<&mut Run> {
        let index = self.len;
        if index >= self.out.len() {
            self.out.push(Run {
                col,
                cells: 0,
                text: spare.pop().unwrap_or_default(),
                style,
                kind,
                link,
            });
        }
        let run = self.out.get_mut(index)?;
        self.len += 1;
        run.col = col;
        run.cells = 0;
        run.text.clear();
        run.style = style;
        run.kind = kind;
        run.link = link;
        Some(run)
    }

    /// Quita las tiras sobrantes y guarda sus cadenas para otro cuadro.
    fn finish(self, spare: &mut Vec<String>) {
        keep_spare(self.out.drain(self.len..).map(|run| run.text), spare);
    }
}

/// Guarda cadenas vaciadas (sin perder su capacidad) hasta [`MAX_SPARE`].
fn keep_spare(texts: impl Iterator<Item = String>, spare: &mut Vec<String>) {
    for mut text in texts {
        if spare.len() >= MAX_SPARE {
            break;
        }
        text.clear();
        spare.push(text);
    }
}

/// Tabla de hipervínculos de la fila, reutilizando las cadenas anteriores.
struct Links<'a> {
    out: &'a mut Vec<String>,
    len: usize,
    /// Índice del enlace de la celda anterior (las celdas de un enlace van
    /// seguidas: se evita buscar en la tabla en cada una).
    last: Option<u32>,
}

impl<'a> Links<'a> {
    fn new(out: &'a mut Vec<String>) -> Links<'a> {
        Links {
            out,
            len: 0,
            last: None,
        }
    }

    fn index_of(&mut self, cell: &Cell, spare: &mut Vec<String>) -> Option<u32> {
        let link = cell.hyperlink()?;
        let uri = link.uri();
        let known = self.out.get(..self.len).unwrap_or_default();
        let index = match self.last {
            Some(last) if known.get(last as usize).is_some_and(|s| s == uri) => last,
            _ => match known.iter().position(|s| s == uri) {
                Some(found) => u32::try_from(found).ok()?,
                None => self.insert(uri, spare)?,
            },
        };
        self.last = Some(index);
        Some(index)
    }

    fn insert(&mut self, uri: &str, spare: &mut Vec<String>) -> Option<u32> {
        let index = self.len;
        match self.out.get_mut(index) {
            Some(slot) => {
                slot.clear();
                slot.push_str(uri);
            }
            None => {
                let mut slot = spare.pop().unwrap_or_default();
                slot.push_str(uri);
                self.out.push(slot);
            }
        }
        self.len += 1;
        u32::try_from(index).ok()
    }

    fn finish(self, spare: &mut Vec<String>) {
        keep_spare(self.out.drain(self.len..), spare);
    }
}

/// Último ajuste de contraste: `(fondo, texto, razón) → resultado`. Las
/// celdas seguidas suelen repetir colores.
type ContrastMemo = ([u8; 3], [u8; 3], f32, Option<[u8; 3]>);

/// Resuelve colores y estilos de celda con la paleta del tema.
struct Resolver<'a> {
    palette: &'a Palette,
    changed: &'a Colors,
    opts: &'a RenderOpts,
    /// Último ajuste de contraste calculado.
    contrast_memo: Option<ContrastMemo>,
}

impl<'a> Resolver<'a> {
    fn new(palette: &'a Palette, changed: &'a Colors, opts: &'a RenderOpts) -> Resolver<'a> {
        Resolver {
            palette,
            changed,
            opts,
            contrast_memo: None,
        }
    }

    fn style(&mut self, cell: &Cell) -> Style {
        let flags = cell.flags;
        let bold = flags.contains(Flags::BOLD);
        let dim = flags.contains(Flags::DIM);
        let hidden = flags.contains(Flags::HIDDEN);
        let (fg_src, bg_src) = if flags.contains(Flags::INVERSE) {
            (cell.bg, cell.fg)
        } else {
            (cell.fg, cell.bg)
        };
        let bg = self.color(bg_src, false);
        let mut fg = self.color(fg_src, bold && self.opts.bold_is_bright);
        let fg_plain = (bold && self.opts.bold_is_bright)
            .then(|| self.color(fg_src, false))
            .filter(|plain| *plain != fg);
        let mut adjusted = false;
        if self.opts.min_contrast > 1.0 && !excluded_from_contrast(cell.c) {
            let ratio = if dim {
                self.opts.min_contrast / 2.0
            } else {
                self.opts.min_contrast
            };
            if let Some(better) = self.contrast(bg, fg, ratio) {
                fg = better;
                adjusted = true;
            }
        }
        let mut dim_fg = None;
        if dim && !adjusted {
            dim_fg = Some(fg);
            fg = dim_toward(fg, bg);
        }
        // `_drawToCache`: un subrayado con color propio no se atenúa ni se
        // invierte; la negrita aclara el índice 0–7.
        // Sin subrayado no hay nada que colorear: no debe partir tiras.
        let underline = underline(flags);
        let underline_color = cell
            .underline_color()
            .filter(|_| underline != Underline::None)
            .map(|color| self.color(color, bold && self.opts.bold_is_bright));
        let mut style = Style {
            fg,
            bg,
            bold,
            italic: flags.contains(Flags::ITALIC),
            dim,
            dim_fg,
            underline,
            underline_color,
            strike: flags.contains(Flags::STRIKEOUT),
            hidden,
            inverse: flags.contains(Flags::INVERSE),
            default_bg: cell.bg == Color::Named(NamedColor::Background),
            fg_plain,
        };
        if hidden {
            // xterm.js no dibuja nada de una celda oculta (`isInvisible`).
            style.fg = bg;
            style.dim_fg = None;
            style.underline = Underline::None;
            style.underline_color = None;
            style.strike = false;
            style.fg_plain = None;
        }
        style
    }

    fn contrast(&mut self, bg: [u8; 3], fg: [u8; 3], ratio: f32) -> Option<[u8; 3]> {
        if let Some((mb, mf, mr, result)) = self.contrast_memo
            && (mb, mf) == (bg, fg)
            && mr == ratio
        {
            return result;
        }
        let result = ensure_contrast(bg, fg, f64::from(ratio));
        self.contrast_memo = Some((bg, fg, ratio, result));
        result
    }

    fn color(&self, color: Color, bright: bool) -> [u8; 3] {
        match color {
            Color::Spec(rgb) => [rgb.r, rgb.g, rgb.b],
            Color::Indexed(index) => self.indexed(brighten(usize::from(index), bright)),
            Color::Named(name) => self.named(name, bright),
        }
    }

    fn named(&self, name: NamedColor, bright: bool) -> [u8; 3] {
        let index = name as usize;
        if index < 16 {
            return self.indexed(brighten(index, bright));
        }
        match name {
            NamedColor::Background => self.special(index, self.palette.bg),
            NamedColor::Cursor => self.special(index, self.palette.cursor),
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite => {
                self.indexed(index.saturating_sub(NamedColor::DimBlack as usize))
            }
            // Foreground, BrightForeground y DimForeground: el texto del tema.
            _ => self.special(NamedColor::Foreground as usize, self.palette.fg),
        }
    }

    fn indexed(&self, index: usize) -> [u8; 3] {
        self.changed_color(index).unwrap_or_else(|| {
            self.palette
                .ansi
                .get(index)
                .copied()
                .unwrap_or(self.palette.fg)
        })
    }

    fn special(&self, index: usize, theme: [u8; 3]) -> [u8; 3] {
        self.changed_color(index).unwrap_or(theme)
    }

    /// Color que la aplicación cambió por OSC 4/10/11/12.
    fn changed_color(&self, index: usize) -> Option<[u8; 3]> {
        if index >= COLOR_COUNT {
            return None;
        }
        // `Colors` solo ofrece `Index`; el índice se comprobó arriba.
        self.changed[index].map(|rgb| [rgb.r, rgb.g, rgb.b])
    }
}

/// `drawBoldTextInBrightColors`: índices 0–7 → 8–15.
fn brighten(index: usize, bright: bool) -> usize {
    if bright && index < 8 {
        index + 8
    } else {
        index
    }
}

fn underline(flags: Flags) -> Underline {
    if flags.contains(Flags::DOUBLE_UNDERLINE) {
        Underline::Double
    } else if flags.contains(Flags::UNDERCURL) {
        Underline::Curly
    } else if flags.contains(Flags::DOTTED_UNDERLINE) {
        Underline::Dotted
    } else if flags.contains(Flags::DASHED_UNDERLINE) {
        Underline::Dashed
    } else if flags.contains(Flags::UNDERLINE) {
        Underline::Single
    } else {
        Underline::None
    }
}

/// `treatGlyphAsBackgroundColor` de xterm.js: powerline (U+E0A4–U+E0D6) y
/// dibujo (U+2500–U+259F) no se ajustan por contraste.
fn excluded_from_contrast(c: char) -> bool {
    matches!(c, '\u{E0A4}'..='\u{E0D6}' | '\u{2500}'..='\u{259F}')
}

/// Texto a opacidad 128/255 (`DIM_OPACITY` 0,5 redondeado a 8 bits) sobre
/// el fondo: lo que deja el canvas en cada canal.
fn dim_toward(fg: [u8; 3], bg: [u8; 3]) -> [u8; 3] {
    let mix = |f: u8, b: u8| {
        let v = (u32::from(f) * 128 + u32::from(b) * 127 + 127) / 255;
        u8::try_from(v).unwrap_or(u8::MAX)
    };
    [mix(fg[0], bg[0]), mix(fg[1], bg[1]), mix(fg[2], bg[2])]
}

/// `rgb.relativeLuminance2` de xterm.js (WCAG).
fn luminance(c: [u8; 3]) -> f64 {
    let channel = |v: u8| {
        let s = f64::from(v) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c[0]) + 0.7152 * channel(c[1]) + 0.0722 * channel(c[2])
}

/// `contrastRatio` de xterm.js.
fn contrast_ratio(a: f64, b: f64) -> f64 {
    if a < b {
        (b + 0.05) / (a + 0.05)
    } else {
        (a + 0.05) / (b + 0.05)
    }
}

/// `rgba.ensureContrastRatio` de xterm.js: `None` si ya hay contraste; si
/// no, el texto más oscuro u más claro (lo que más se acerque) en pasos
/// del 10 %.
fn ensure_contrast(bg: [u8; 3], fg: [u8; 3], ratio: f64) -> Option<[u8; 3]> {
    let bg_l = luminance(bg);
    let fg_l = luminance(fg);
    if contrast_ratio(bg_l, fg_l) >= ratio {
        return None;
    }
    let ratio_of = |c: [u8; 3]| contrast_ratio(bg_l, luminance(c));
    let (first, second): (Step, Step) = if fg_l < bg_l {
        (darker, lighter)
    } else {
        (lighter, darker)
    };
    let a = shift_until(bg, fg, ratio, first);
    let a_ratio = ratio_of(a);
    if a_ratio >= ratio {
        return Some(a);
    }
    let b = shift_until(bg, fg, ratio, second);
    Some(if a_ratio > ratio_of(b) { a } else { b })
}

type Step = fn(u8) -> u8;

/// `reduceLuminance`: resta `ceil(10 %)` a cada canal.
fn darker(v: u8) -> u8 {
    v.saturating_sub(v.div_ceil(10))
}

/// `increaseLuminance`: suma `ceil(10 %)` de lo que falta hasta 255.
fn lighter(v: u8) -> u8 {
    v.saturating_add((255 - v).div_ceil(10))
}

fn shift_until(bg: [u8; 3], fg: [u8; 3], ratio: f64, step: Step) -> [u8; 3] {
    let bg_l = luminance(bg);
    let mut c = fg;
    while contrast_ratio(luminance(c), bg_l) < ratio {
        let next = [step(c[0]), step(c[1]), step(c[2])];
        if next == c {
            break;
        }
        c = next;
    }
    c
}

/// Forma visible del cursor compartida por GTK y canvas.
pub fn preferred_cursor_shape(
    application: CursorShape,
    preferred: Option<CursorShape>,
    focused: bool,
) -> CursorShape {
    if application == CursorShape::Hidden {
        return CursorShape::Hidden;
    }
    if !focused && !matches!(preferred, Some(CursorShape::Beam | CursorShape::Underline)) {
        return CursorShape::HollowBlock;
    }
    preferred.unwrap_or(application)
}

/// Cursor de la terminal, en coordenadas de la vista.
pub fn cursor(engine: &Engine) -> CursorView {
    let term = engine.term();
    let grid = term.grid();
    let point = grid.cursor.point;
    // `Grid` solo ofrece `Index`; la línea se comprueba antes.
    let row = (point.line >= grid.topmost_line() && point.line <= grid.bottommost_line())
        .then(|| &grid[point.line][..]);
    let cell_at = |col: usize| row.and_then(|cells| cells.get(col));
    let mut col = point.column.0;
    if col > 0 && cell_at(col).is_some_and(|cell| cell.flags.contains(Flags::WIDE_CHAR_SPACER)) {
        col -= 1;
    }
    let wide = cell_at(col).is_some_and(|cell| cell.flags.contains(Flags::WIDE_CHAR));
    let view = i64::from(point.line.0) + i64::try_from(grid.display_offset()).unwrap_or(0);
    let line = usize::try_from(view).unwrap_or(usize::MAX);
    let on_screen = line < grid.screen_lines();
    CursorView {
        line,
        col: u16::try_from(col).unwrap_or(u16::MAX),
        shape: term.cursor_style().shape,
        visible: on_screen && term.mode().contains(TermMode::SHOW_CURSOR),
        wide,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_bar_survives_application_style_and_focus() {
        for focused in [true, false] {
            for application in [
                CursorShape::Block,
                CursorShape::Underline,
                CursorShape::Beam,
            ] {
                assert_eq!(
                    preferred_cursor_shape(application, Some(CursorShape::Beam), focused),
                    CursorShape::Beam
                );
            }
            assert_eq!(
                preferred_cursor_shape(CursorShape::Hidden, Some(CursorShape::Beam), focused),
                CursorShape::Hidden
            );
        }
        assert_eq!(
            preferred_cursor_shape(CursorShape::Underline, None, true),
            CursorShape::Underline
        );
        assert_eq!(
            preferred_cursor_shape(CursorShape::Block, None, false),
            CursorShape::HollowBlock
        );
    }

    #[test]
    fn contrast_steps_match_xterm_js() {
        // reduceLuminance / increaseLuminance con números de xterm.js.
        assert_eq!(darker(255), 229);
        assert_eq!(darker(1), 0);
        assert_eq!(lighter(0), 26);
        assert_eq!(lighter(254), 255);
        assert_eq!(ensure_contrast([0, 0, 0], [255, 255, 255], 4.5), None);
        let lifted = ensure_contrast([0, 0, 0], [40, 40, 40], 7.0).unwrap_or([0; 3]);
        assert!(contrast_ratio(luminance(lifted), luminance([0, 0, 0])) >= 7.0);
    }

    #[test]
    fn dim_matches_canvas_compositing() {
        assert_eq!(dim_toward([255, 0, 10], [0, 255, 10]), [128, 127, 10]);
    }
}

/// The canvas size in device pixels: the observed `(w, h)` when it matches the CSS box
/// times `dpr` within rounding, else the CSS box times `dpr`. Chrome's device emulation
/// reports the CSS size as the device size at `dpr` 3; the glyphs are drawn at `dpr`, so
/// trusting it would draw the text three times too big into a third of the screen.
pub fn device_size(w: u32, h: u32, css: (f64, f64), dpr: f64) -> (u32, u32) {
    let want = |c: f64| (c * dpr).round().clamp(1.0, f64::from(u32::MAX)) as u32;
    let near = |got: u32, c: f64| (f64::from(got) - c * dpr).abs() <= dpr.ceil() + 1.0;
    if near(w, css.0) && near(h, css.1) {
        (w, h)
    } else {
        (want(css.0), want(css.1))
    }
}
