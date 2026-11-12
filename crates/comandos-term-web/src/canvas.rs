//! `Canvas2d`: pinta filas en un `<canvas>` con las mismas cuentas que el
//! renderizador de `@xterm/addon-canvas` 0.7.0, para que los píxeles salgan
//! iguales que con xterm.js.
//!
//! ## Cómo pinta addon-canvas (y por tanto este módulo)
//!
//! - Cada fila se recorta a su franja (`_clipRow`), se limpia con el fondo
//!   del tema, se pintan los fondos de celda y después los glifos.
//! - Un glifo no se pinta directamente: se rasteriza una vez en un canvas
//!   auxiliar (`TextureAtlas._drawToCache`) sobre su fondo, con subrayado,
//!   tachado y el ajuste del «_», se borran los píxeles del color de fondo
//!   (`clearColor`, con la tolerancia de 1/12 de la distancia entre colores)
//!   y se copia con `drawImage` a la celda. Este módulo hace lo mismo con su
//!   propio atlas: el color y el alisado de los bordes salen idénticos, y en
//!   régimen estable pintar una celda es un `drawImage` sin cadenas.
//! - Los glifos de dibujo (cajas, bloques, tramas, powerline) salen de
//!   [`comandos_term::glyphs::draw_ops`], cacheados por carácter.
//! - El cursor (`CursorRenderLayer`) va en otra capa transparente en
//!   xterm.js; aquí se pinta sobre la fila recién repintada, y el carácter
//!   del cursor de bloque se rasteriza en un canvas transparente pequeño
//!   para conservar el alisado en gris de aquella capa.
//!
//! Las cuentas puras (colores, `clearColor`, cajas, subrayados) se prueban
//! en host; el resto necesita un navegador (pruebas de `tests/web.rs`).
use crate::{
    metrics::CellMetrics,
    paint::{Painter, Selected},
};
use comandos_term::{
    glyphs::{self, DrawOp},
    render::{CursorShape, CursorView, Run, RunKind, Underline},
};
mod box_index;
mod pattern_key;
use box_index::BoxIndex;
use std::{
    borrow::Borrow,
    collections::HashMap,
    hash::{Hash, Hasher},
};
use wasm_bindgen::{Clamped, JsCast, JsValue, prelude::wasm_bindgen};
use web_sys::{
    CanvasPattern, CanvasRenderingContext2d, Document, HtmlCanvasElement, ImageData, Path2d, Window,
};

// ---------------------------------------------------------------------------
// Cuentas puras.
// ---------------------------------------------------------------------------

/// Margen del glifo en el canvas auxiliar del atlas (`E` de `_drawToCache`).
pub const ATLAS_PAD: u32 = 4;

/// `#rrggbb`.
pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// `#rrggbbaa` (lo que da `color.multiplyOpacity` de xterm.js).
pub fn hex_alpha(c: [u8; 3], alpha: u8) -> String {
    format!("#{:02x}{:02x}{:02x}{alpha:02x}", c[0], c[1], c[2])
}

/// Opacidad de un texto atenuado: `round(255 · DIM_OPACITY)`.
pub const DIM_ALPHA: u8 = 128;

/// `clearColor` del atlas de addon-canvas: deja transparentes los píxeles
/// del color de fondo y, con `fuzzy`, los que se le acercan a menos de
/// `floor(Σ|fondo − texto| / 12)`. Devuelve `true` si no quedó ninguno.
pub fn clear_color(data: &mut [u8], bg: [u8; 3], fg: [u8; 3], fuzzy: bool) -> bool {
    let diff = |a: u8, b: u8| i32::from(a.abs_diff(b));
    let threshold = (diff(bg[0], fg[0]) + diff(bg[1], fg[1]) + diff(bg[2], fg[2])) / 12;
    let mut empty = true;
    for px in data.chunks_exact_mut(4) {
        let [r, g, b, a] = px else { continue };
        let exact = [*r, *g, *b] == bg;
        let near = fuzzy && diff(*r, bg[0]) + diff(*g, bg[1]) + diff(*b, bg[2]) < threshold;
        if exact || near {
            *a = 0;
        } else {
            empty = false;
        }
    }
    empty
}

/// `_findGlyphBoundingBox` de xterm.js sobre una imagen RGBA de `width`
/// píxeles de ancho: `rows` filas (alto del auxiliar, o de la celda en los
/// glifos powerline recortados), ancho permitido `allowed` (`l`, o el de la
/// celda) y relleno `pad`. Arriba y abajo se buscan en las columnas
/// `[0, allowed)`; la izquierda en `[0, pad + allowed)` y la derecha en
/// `[pad, pad + allowed)`. Lo que no se encuentra toma el valor por defecto
/// de xterm.js (0, `allowed`, 0, `rows`), recortado al canvas: lo que se
/// añade así es transparente. A diferencia de xterm.js no se leen columnas
/// fuera del canvas (allí el índice saltaría a la fila siguiente).
/// `None` si no hay tinta en la región.
pub fn glyph_bbox(
    data: &[u8],
    width: u32,
    rows: u32,
    allowed: u32,
    pad: u32,
) -> Option<(u32, u32, u32, u32)> {
    let height = u32::try_from(data.len() / 4 / (width.max(1) as usize)).unwrap_or(u32::MAX);
    let rows = rows.min(height);
    let inner = allowed.min(width);
    let outer = pad.saturating_add(allowed).min(width);
    let ink = |x: u32, y: u32| {
        let index = (y as usize)
            .saturating_mul(width as usize)
            .saturating_add(x as usize)
            .saturating_mul(4)
            .saturating_add(3);
        data.get(index).copied().unwrap_or(0) != 0
    };
    let row_has = |y: u32| (0..inner).any(|x| ink(x, y));
    let col_has = |x: u32| (0..rows).any(|y| ink(x, y));
    let top = (0..rows).find(|&y| row_has(y));
    let left = (0..outer).find(|&x| col_has(x));
    let right = (pad.min(outer)..outer).rev().find(|&x| col_has(x));
    let bottom = (0..rows).rev().find(|&y| row_has(y));
    if top.is_none() && left.is_none() {
        return None;
    }
    let last_x = width.saturating_sub(1);
    let last_y = rows.saturating_sub(1);
    let (l, t) = (left.unwrap_or(0), top.unwrap_or(0));
    let r = right.unwrap_or(allowed).min(last_x);
    let b = bottom.unwrap_or(rows).min(last_y);
    (r >= l && b >= t).then_some((l, t, r, b))
}

/// Tamaño del canvas auxiliar del cursor de caja con texto: tres celdas (o
/// cuatro con un carácter ancho) de ancho y una de alto, con el tope del
/// auxiliar.
pub fn cursor_scratch_size(dev_w: u32, dev_h: u32) -> (u32, u32) {
    (
        dev_w.saturating_mul(4).clamp(1, TMP_MAX),
        dev_h.clamp(1, TMP_MAX),
    )
}

/// `isPowerlineGlyph` de xterm.js: U+E0A4–U+E0D6 (sin `clearColor` difuso).
pub fn is_powerline(c: char) -> bool {
    ('\u{E0A4}'..='\u{E0D6}').contains(&c)
}

/// `treatGlyphAsBackgroundColor`: powerline y U+2500–U+259F (cajas y
/// bloques), cuyo color de texto también cambia al seleccionarlos.
pub fn glyph_is_background(c: char) -> bool {
    is_powerline(c) || ('\u{2500}'..='\u{259F}').contains(&c)
}

/// `rgba.blend(color, selección con alfa 128)` de xterm.js.
pub fn blend_half(color: [u8; 3], selection: [u8; 3]) -> [u8; 3] {
    let mix = |c: u8, s: u8| {
        let (c, s) = (f64::from(c), f64::from(s));
        (c + js_round((s - c) * 128.0 / 255.0)).clamp(0.0, 255.0) as u8
    };
    [
        mix(color[0], selection[0]),
        mix(color[1], selection[1]),
        mix(color[2], selection[2]),
    ]
}

/// [`selected_glyph_colors`] más las banderas del glifo: una celda
/// seleccionada deja de estar atenuada (`CellColorResolver` quita
/// `BgFlags.DIM` del fondo, y `TextureAtlas` decide la atenuación con él).
fn selected_glyph(
    style: &comandos_term::render::Style,
    fg: [u8; 3],
    flags: u8,
    c: Option<char>,
    selection: [u8; 3],
) -> ([u8; 3], [u8; 3], u8) {
    let (fg, bg) = selected_glyph_colors(style, fg, c, selection);
    (fg, bg, flags & !DIM)
}

/// `CellColorResolver` de addon-canvas para una celda seleccionada: el
/// fondo contra el que se rasteriza el glifo (la selección opaca, o mezclada
/// a medias con el fondo propio de la celda o con el de vídeo inverso) y el
/// texto (solo cambia en los glifos que hacen de fondo, y entonces parte del
/// color de la paleta sin aclarar por la negrita). `fg` es el color del texto
/// sin atenuar.
pub fn selected_glyph_colors(
    style: &comandos_term::render::Style,
    fg: [u8; 3],
    c: Option<char>,
    selection: [u8; 3],
) -> ([u8; 3], [u8; 3]) {
    let bg = if style.inverse || !style.default_bg {
        blend_half(style.bg, selection)
    } else {
        selection
    };
    let fg = match c {
        Some(c) if glyph_is_background(c) => {
            if style.inverse && style.default_bg {
                selection
            } else {
                blend_half(style.fg_plain.unwrap_or(fg), selection)
            }
        }
        _ => fg,
    };
    (fg, bg)
}

/// `isRestrictedPowerlineGlyph`: U+E0B0–U+E0B7, recortados a la celda.
pub fn is_restricted_powerline(c: char) -> bool {
    ('\u{E0B0}'..='\u{E0B7}').contains(&c)
}

/// Geometría del subrayado de `_drawToCache`, en coordenadas del canvas
/// auxiliar (el carácter empieza en `(pad, pad)`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnderlineGeom {
    /// `lineWidth = max(1, floor(fontSize·dpr/15))`.
    pub width: f64,
    /// Ajuste a medio píxel (0,5 si el grosor es impar).
    pub half: f64,
    /// Primera línea (`s`), medio grosor más abajo (`r`) y segunda (`n`).
    pub s: f64,
    pub r: f64,
    pub n: f64,
}

/// `s = ceil(pad + charH) − t − 2e` (con `restrictToCellHeight`), `r = s + e`,
/// `n = s + 2e`.
pub fn underline_geom(font_size: f64, dpr: f64, dev_char_h: u32, pad: u32) -> UnderlineGeom {
    let e = (font_size * dpr / 15.0).floor().max(1.0);
    let half = if e % 2.0 == 1.0 { 0.5 } else { 0.0 };
    let s = (f64::from(pad) + f64::from(dev_char_h)).ceil() - half - 2.0 * e;
    UnderlineGeom {
        width: e,
        half,
        s,
        r: s + e,
        n: s + 2.0 * e,
    }
}

/// `computeNextVariantOffset`: fase del punteado en la celda siguiente
/// (`%` de JavaScript: el signo es el del dividendo, como el de Rust).
pub fn next_variant_offset(cell_w: f64, e: f64, offset: f64) -> f64 {
    let period = 2.0 * js_round(e);
    (cell_w - (period - offset)) % period
}

/// Fase del punteado de una celda (`CellColorResolver`: `x·cellW % 2·round(e)`).
pub fn dotted_phase(col: u16, cell_w: u32, e: f64) -> f64 {
    (f64::from(col) * f64::from(cell_w)) % (2.0 * js_round(e))
}

fn js_round(v: f64) -> f64 {
    crate::metrics::js_round(v)
}

/// `textureSize` de `TextureAtlas` (xterm.js 5.5): tope del canvas auxiliar
/// para un glifo que pide más (`Math.min(…, textureSize)`).
pub const TEXTURE_SIZE: u32 = 512;

/// Tope absoluto del auxiliar (con el tope de fuente, 512 px a dpr 3, una
/// celda no pasa de ~1100 px de ancho).
pub const TMP_MAX: u32 = 4096;

/// Tamaño del canvas auxiliar para rasterizar un texto de `utf16_len`
/// unidades, como `_drawToCache`: parte de `4·cellW + 4 × cellH + 4` (el de
/// su constructor) y pide `min(cellW·max(len, 2) + 4, textureSize)` de
/// ancho y `min(cellH + 8, textureSize)` de alto. A diferencia de xterm.js
/// se vuelve al tamaño de partida tras un glifo grande (un texto «zalgo» no
/// deja el auxiliar enorme para siempre).
pub fn tmp_size(dev_w: u32, dev_h: u32, utf16_len: usize) -> (u32, u32) {
    let need_w = tmp_size_wanted_w(dev_w, utf16_len);
    let base_w = dev_w.saturating_mul(4).saturating_add(4);
    let base_h = dev_h.saturating_add(4);
    let need_h = dev_h.saturating_add(8).min(TEXTURE_SIZE);
    (
        base_w.max(need_w).clamp(1, TMP_MAX),
        base_h.max(need_h).clamp(1, TMP_MAX),
    )
}

/// `l` de `_drawToCache`: `min(cellW·max(len, 2) + 4, textureSize)`.
pub fn tmp_size_wanted_w(dev_w: u32, utf16_len: usize) -> u32 {
    let len = u32::try_from(utf16_len).unwrap_or(u32::MAX).max(2);
    dev_w
        .saturating_mul(len)
        .saturating_add(4)
        .min(TEXTURE_SIZE)
}

/// Un glifo mayor que esto no se guarda en el atlas: se rasteriza y se
/// copia desde el auxiliar cada vez (no vacía el atlas una y otra vez).
pub const UNCACHED_LIMIT: u32 = PAGE_SIZE / 2;

// Un glifo de tamaño normal (auxiliar ≤ `textureSize`) siempre entra en el
// atlas; solo las celdas enormes van sin guardar.
const _: () = assert!(TEXTURE_SIZE <= UNCACHED_LIMIT);

/// Mapa acotado: al pasar de `cap` entradas se descarta la más antigua
/// (FIFO). Para cachés cuya clave puede venir de la salida (colores de 24
/// bits).
#[derive(Clone)]
pub struct Bounded<K, V> {
    cap: usize,
    map: HashMap<K, V>,
    order: Vec<K>,
    oldest: usize,
}

// Keep the public Debug queue in logical FIFO order, including after wrapping.
impl<K: std::fmt::Debug, V: std::fmt::Debug> std::fmt::Debug for Bounded<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        struct Order<'a, K>(&'a [K], usize);
        impl<K: std::fmt::Debug> std::fmt::Debug for Order<'_, K> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_list()
                    .entries(self.0[self.1..].iter().chain(&self.0[..self.1]))
                    .finish()
            }
        }
        f.debug_struct("Bounded")
            .field("cap", &self.cap)
            .field("map", &self.map)
            .field("order", &Order(&self.order, self.oldest))
            .finish()
    }
}

impl<K: std::hash::Hash + Eq + Clone, V> Bounded<K, V> {
    pub fn new(cap: usize) -> Bounded<K, V> {
        Bounded {
            cap: cap.max(1),
            map: HashMap::new(),
            order: Vec::new(),
            oldest: 0,
        }
    }

    pub fn get(&self, k: &K) -> Option<&V> {
        self.map.get(k)
    }

    pub fn insert(&mut self, k: K, v: V) {
        if self.map.insert(k.clone(), v).is_none() {
            if self.order.len() < self.cap {
                self.order.push(k);
            } else {
                let old = std::mem::replace(&mut self.order[self.oldest], k);
                self.map.remove(&old);
                self.oldest += 1;
                if self.oldest == self.cap {
                    self.oldest = 0;
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.oldest = 0;
    }
}

/// Reparto de los glifos en páginas del atlas por estantes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shelf {
    size: u32,
    x: u32,
    y: u32,
    row_h: u32,
}

impl Shelf {
    pub fn new(size: u32) -> Shelf {
        Shelf {
            size,
            x: 0,
            y: 0,
            row_h: 0,
        }
    }

    /// Hueco para `w × h`, o `None` si la página está llena (o no cabe).
    pub fn place(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > self.size || h > self.size {
            return None;
        }
        if self.x + w > self.size {
            self.y += self.row_h;
            self.x = 0;
            self.row_h = 0;
        }
        if self.y + h > self.size {
            return None;
        }
        let at = (self.x, self.y);
        self.x += w;
        self.row_h = self.row_h.max(h);
        Some(at)
    }
}

// ---------------------------------------------------------------------------
// Enlaces con JavaScript que no dejan basura.
// ---------------------------------------------------------------------------

#[wasm_bindgen]
extern "C" {
    /// Vista de un `CanvasRenderingContext2D` con los definidores que
    /// aceptan un valor ya creado (sin cadena nueva por llamada).
    type Ctx2dSetters;
    #[wasm_bindgen(method, setter = fillStyle)]
    fn set_fill(this: &Ctx2dSetters, v: &JsValue);
    #[wasm_bindgen(method, setter = strokeStyle)]
    fn set_stroke(this: &Ctx2dSetters, v: &JsValue);
    #[wasm_bindgen(method, setter = font)]
    fn set_font(this: &Ctx2dSetters, v: &JsValue);
}

/// `ctx.fillStyle = v`.
fn set_fill_js(ctx: &CanvasRenderingContext2d, v: &JsValue) {
    ctx.unchecked_ref::<Ctx2dSetters>().set_fill(v);
}

/// `ctx.strokeStyle = v`.
fn set_stroke_js(ctx: &CanvasRenderingContext2d, v: &JsValue) {
    ctx.unchecked_ref::<Ctx2dSetters>().set_stroke(v);
}

/// `ctx.font = v`.
fn set_font_js(ctx: &CanvasRenderingContext2d, v: &JsValue) {
    ctx.unchecked_ref::<Ctx2dSetters>().set_font(v);
}

/// Cadenas de color ya convertidas a JavaScript (los colores de un tema
/// son pocos; si crecen mucho se vacía).
#[derive(Default)]
struct ColorCache {
    entries: Vec<([u8; 3], JsValue)>,
}

impl ColorCache {
    const MAX: usize = 128;

    fn get(&mut self, c: [u8; 3]) -> JsValue {
        if let Some((_, v)) = self.entries.iter().find(|(k, _)| *k == c) {
            return v.clone();
        }
        if self.entries.len() >= Self::MAX {
            self.entries.clear();
        }
        let v = JsValue::from_str(&hex(c));
        self.entries.push((c, v.clone()));
        v
    }
}

/// Opciones del contexto 2D (`{alpha, willReadFrequently}`).
fn context(
    canvas: &HtmlCanvasElement,
    alpha: bool,
    read: bool,
) -> Result<CanvasRenderingContext2d, JsValue> {
    let opts = js_sys::Object::new();
    js_sys::Reflect::set(&opts, &"alpha".into(), &JsValue::from_bool(alpha))?;
    if read {
        js_sys::Reflect::set(&opts, &"willReadFrequently".into(), &JsValue::TRUE)?;
    }
    canvas
        .get_context_with_context_options("2d", &opts)?
        .ok_or_else(|| JsValue::from_str("canvas 2d no disponible"))?
        .dyn_into()
        .map_err(JsValue::from)
}

fn new_canvas(document: &Document, w: u32, h: u32) -> Result<HtmlCanvasElement, JsValue> {
    let canvas: HtmlCanvasElement = document.create_element("canvas")?.dyn_into()?;
    canvas.set_width(w);
    canvas.set_height(h);
    Ok(canvas)
}

// ---------------------------------------------------------------------------
// Fuente y colores que pinta el canvas.
// ---------------------------------------------------------------------------

/// Colores del tema que usa quien pinta (el resto llega ya resuelto en las
/// tiras de `render`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasTheme {
    pub bg: [u8; 3],
    pub cursor: [u8; 3],
    pub cursor_accent: [u8; 3],
}

/// Las cuatro fuentes de `BaseRenderLayer._getFont` en píxeles de
/// dispositivo: `"<italic> <peso> <size·dpr>px <family>"`.
struct Fonts {
    /// Regular, negrita, cursiva, negrita cursiva.
    js: [JsValue; 4],
    text: [String; 4],
}

impl Fonts {
    fn new(family: &str, size: f64, dpr: f64) -> Fonts {
        let px = size * dpr;
        let text = [
            format!(" normal {} {family}", crate::number_text::px(px)),
            format!(" bold {} {family}", crate::number_text::px(px)),
            format!("italic normal {} {family}", crate::number_text::px(px)),
            format!("italic bold {} {family}", crate::number_text::px(px)),
        ];
        let js = [
            JsValue::from_str(&text[0]),
            JsValue::from_str(&text[1]),
            JsValue::from_str(&text[2]),
            JsValue::from_str(&text[3]),
        ];
        Fonts { js, text }
    }

    fn index(bold: bool, italic: bool) -> usize {
        usize::from(bold) + 2 * usize::from(italic)
    }
}

/// `TEXT_BASELINE` de addon-canvas: `bottom` en Firefox, `ideographic` en
/// el resto.
pub fn text_baseline(user_agent: &str) -> &'static str {
    if user_agent.contains("Firefox") || user_agent.contains("Edge") {
        "bottom"
    } else {
        "ideographic"
    }
}

// ---------------------------------------------------------------------------
// Atlas de glifos.
// ---------------------------------------------------------------------------

/// Todo lo que cambia cómo se rasteriza un glifo, salvo el texto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphStyle {
    fg: [u8; 3],
    bg: [u8; 3],
    /// Bits: 1 negrita, 2 cursiva, 4 atenuado, 8 tachado, 16 ancho,
    /// 32 glifo de dibujo.
    flags: u8,
    underline: u8,
    underline_color: Option<[u8; 3]>,
    /// Fase del punteado (solo en `Dotted`), en píxeles de dispositivo.
    phase: u16,
}

const BOLD: u8 = 1;
const ITALIC: u8 = 2;
const DIM: u8 = 4;
const STRIKE: u8 = 8;
const WIDE: u8 = 16;
const CUSTOM: u8 = 32;

/// Un glifo del atlas: dónde está y dónde va respecto a la celda.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    page: u16,
    /// Generación del índice del atlas en que se colocó.
    generation: u32,
    sx: u32,
    sy: u32,
    w: u32,
    h: u32,
    dx: i32,
    dy: i32,
}

impl Slot {
    /// Página de un glifo que no se guardó: se copia desde el auxiliar.
    const UNCACHED: u16 = u16::MAX;

    const EMPTY: Slot = Slot {
        page: 0,
        generation: 0,
        sx: 0,
        sy: 0,
        w: 0,
        h: 0,
        dx: 0,
        dy: 0,
    };
}

struct Page {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
}

/// Lado de una página del atlas.
const PAGE_SIZE: u32 = 1024;
/// Páginas como mucho; al llenarse se vacía el atlas entero.
const MAX_PAGES: usize = 4;

/// Relleno actual del canvas auxiliar (`fillStyle` puede quedar en trama).
enum Fill {
    Css(String),
    /// Un color ya convertido (de `ColorCache`).
    Js(JsValue),
    Pattern(CanvasPattern),
}

/// Entradas del atlas como mucho; al pasar se vacía entero (las vacías no
/// llenan páginas y con colores de 24 bits crecerían sin tope).
pub const MAX_ENTRIES: usize = 8192;

/// Bytes de texto de las claves como mucho (los textos de varios puntos de
/// código no tienen tope de longitud).
pub const MAX_KEY_BYTES: usize = 256 * 1024;

// A combined text/style key avoids one hash-table implementation and one
// hash-table lookup for multiscalar glyphs. The single-character table and
// allocation-free fast path remain unchanged.
#[derive(PartialEq, Eq)]
struct GlyphKey<K> {
    text: String,
    style: K,
}
trait GlyphKeyView<K> {
    fn text(&self) -> &str;
    fn style(&self) -> &K;
}
impl<K> GlyphKeyView<K> for GlyphKey<K> {
    fn text(&self) -> &str {
        &self.text
    }
    fn style(&self) -> &K {
        &self.style
    }
}
struct GlyphQuery<'a, K> {
    text: &'a str,
    style: &'a K,
}
impl<K> GlyphKeyView<K> for GlyphQuery<'_, K> {
    fn text(&self) -> &str {
        self.text
    }
    fn style(&self) -> &K {
        self.style
    }
}
impl<K: Hash> Hash for GlyphKey<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.style.hash(state);
    }
}
impl<K: Hash> Hash for dyn GlyphKeyView<K> + '_ {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text().hash(state);
        self.style().hash(state);
    }
}
impl<K: Eq> PartialEq for dyn GlyphKeyView<K> + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.text() == other.text() && self.style() == other.style()
    }
}
impl<K: Eq> Eq for dyn GlyphKeyView<K> + '_ {}
impl<'a, K: 'a> Borrow<dyn GlyphKeyView<K> + 'a> for GlyphKey<K> {
    fn borrow(&self) -> &(dyn GlyphKeyView<K> + 'a) {
        self
    }
}

/// Índice texto + estilo → glifo del atlas, con sus topes. La generación
/// cambia en cada vaciado: un glifo guardado siempre es de la generación
/// actual (`cached_glyph` vacía antes de rasterizar, nunca después).
pub struct GlyphIndex<K, S> {
    single: HashMap<(char, K), S>,
    multi: HashMap<GlyphKey<K>, S>,
    /// Entradas guardadas (incluidas las vacías, que no ocupan página).
    entries: usize,
    key_bytes: usize,
    generation: u32,
}

impl<K, S> Default for GlyphIndex<K, S> {
    fn default() -> Self {
        GlyphIndex {
            single: HashMap::new(),
            multi: HashMap::new(),
            entries: 0,
            key_bytes: 0,
            generation: 0,
        }
    }
}

impl<K: Copy + Eq + std::hash::Hash, S: Copy> GlyphIndex<K, S> {
    pub fn lookup(&self, text: &str, key: &K) -> Option<S> {
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => self.single.get(&(c, *key)).copied(),
            _ => {
                let query = GlyphQuery { text, style: key };
                self.multi.get(&query as &dyn GlyphKeyView<K>).copied()
            }
        }
    }

    /// Guardar `text` pasaría de algún tope.
    pub fn needs_room(&self, text: &str) -> bool {
        self.entries >= MAX_ENTRIES || self.key_bytes.saturating_add(text.len()) > MAX_KEY_BYTES
    }

    pub fn insert(&mut self, text: &str, key: K, slot: S) {
        self.entries += 1;
        self.key_bytes = self.key_bytes.saturating_add(text.len());
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => {
                self.single.insert((c, key), slot);
            }
            _ => {
                self.multi.insert(
                    GlyphKey {
                        text: text.to_owned(),
                        style: key,
                    },
                    slot,
                );
            }
        }
    }

    pub fn clear(&mut self) {
        self.single.clear();
        self.multi.clear();
        self.entries = 0;
        self.key_bytes = 0;
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn len(&self) -> usize {
        self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries == 0
    }

    pub fn key_bytes(&self) -> usize {
        self.key_bytes
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Recorre las entradas guardadas.
    pub fn for_each(&self, mut f: impl FnMut(&str, &K, &S)) {
        let mut buf = [0u8; 4];
        for ((c, k), slot) in &self.single {
            f(c.encode_utf8(&mut buf), k, slot);
        }
        for (key, slot) in &self.multi {
            f(&key.text, &key.style, slot);
        }
    }
}

/// Un atlas: su índice y cómo vaciarlo entero (páginas incluidas).
pub trait GlyphStore {
    type Key: Copy + Eq + std::hash::Hash;
    type Slot: Copy;
    fn index(&mut self) -> &mut GlyphIndex<Self::Key, Self::Slot>;
    fn clear_all(&mut self);
}

/// El glifo de `text` con `key`, del índice o rasterizado con `raster`
/// (recibe si puede guardarse en el atlas y da el hueco y si se guarda).
/// Si guardarlo pasaría de los topes, el atlas se vacía *antes* de
/// rasterizar: así el hueco nuevo nunca apunta a una página que se vacía
/// justo después. Una clave que por sí sola pasa del tope de bytes se
/// dibuja sin caché y no vacía nada.
pub fn cached_glyph<A: GlyphStore, E>(
    atlas: &mut A,
    text: &str,
    key: A::Key,
    raster: impl FnOnce(&mut A, bool) -> Result<(A::Slot, bool), E>,
) -> Result<A::Slot, E> {
    if text.len() > MAX_KEY_BYTES {
        return raster(atlas, false).map(|(slot, _)| slot);
    }
    if let Some(slot) = atlas.index().lookup(text, &key) {
        return Ok(slot);
    }
    if atlas.index().needs_room(text) {
        atlas.clear_all();
    }
    let (slot, cacheable) = raster(atlas, true)?;
    if cacheable {
        atlas.index().insert(text, key, slot);
    }
    Ok(slot)
}

struct Atlas {
    tmp: HtmlCanvasElement,
    tmp_ctx: CanvasRenderingContext2d,
    pages: Vec<Page>,
    shelf: Shelf,
    index: GlyphIndex<GlyphStyle, Slot>,
}

impl GlyphStore for Atlas {
    type Key = GlyphStyle;
    type Slot = Slot;
    fn index(&mut self) -> &mut GlyphIndex<GlyphStyle, Slot> {
        &mut self.index
    }
    fn clear_all(&mut self) {
        self.clear();
    }
}

/// Lo que el atlas necesita para rasterizar.
struct RasterCtx<'a> {
    document: &'a Document,
    m: &'a CellMetrics,
    font_size: f64,
    fonts: &'a Fonts,
    baseline: &'static str,
    boxes: &'a mut BoxCache,
    patterns: &'a mut PatternCache,
}

impl Atlas {
    fn new(document: &Document, m: &CellMetrics) -> Result<Atlas, JsValue> {
        let (w, h) = tmp_size(m.dev_w, m.dev_h, 0);
        let tmp = new_canvas(document, w, h)?;
        let tmp_ctx = context(&tmp, false, true)?;
        Ok(Atlas {
            tmp,
            tmp_ctx,
            pages: Vec::new(),
            shelf: Shelf::new(PAGE_SIZE),
            index: GlyphIndex::default(),
        })
    }

    fn clear(&mut self) {
        self.pages.clear();
        self.shelf = Shelf::new(PAGE_SIZE);
        self.index.clear();
    }

    /// Canvas del que se copia un glifo.
    fn source(&self, slot: &Slot) -> Option<&HtmlCanvasElement> {
        if slot.page == Slot::UNCACHED {
            Some(&self.tmp)
        } else if slot.generation != self.index.generation() {
            // Nunca debería pasar: un hueco de páginas ya vaciadas.
            None
        } else {
            self.pages.get(usize::from(slot.page)).map(|p| &p.canvas)
        }
    }

    /// El glifo, rasterizado si hace falta.
    fn glyph(
        &mut self,
        text: &str,
        key: GlyphStyle,
        rc: &mut RasterCtx<'_>,
    ) -> Result<Slot, JsValue> {
        cached_glyph(self, text, key, |atlas, cache| {
            let started = crate::metrics::perf_now();
            let slot = atlas.rasterize(text, &key, rc, cache)?;
            crate::metrics::note_raster(crate::metrics::perf_now() - started);
            Ok((slot, slot.page != Slot::UNCACHED))
        })
    }

    /// `_drawToCache` paso a paso. Con `cache = false` el glifo no ocupa
    /// página: se copia desde el auxiliar.
    fn rasterize(
        &mut self,
        text: &str,
        key: &GlyphStyle,
        rc: &mut RasterCtx<'_>,
        cache: bool,
    ) -> Result<Slot, JsValue> {
        let m = rc.m;
        let (cell_w, cell_h) = (f64::from(m.dev_w), f64::from(m.dev_h));
        let char_h = f64::from(m.dev_char_h);
        let cells: u32 = if key.flags & WIDE != 0 { 2 } else { 1 };
        let (tw, th) = tmp_size(m.dev_w, m.dev_h, text.encode_utf16().count());
        if self.tmp.width() != tw {
            self.tmp.set_width(tw);
        }
        if self.tmp.height() != th {
            self.tmp.set_height(th);
        }
        let ctx = &self.tmp_ctx;
        let single = {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            }
        };
        let powerline = single.is_some_and(is_powerline);
        let restricted = single.is_some_and(is_restricted_powerline);
        let custom = key.flags & CUSTOM != 0;
        // Texto atenuado: el color sin mezclar con opacidad 0,5.
        let (fg, dim) = (key.fg, key.flags & DIM != 0);
        let bg_css = hex(key.bg);
        let fg_css = if dim {
            hex_alpha(fg, DIM_ALPHA)
        } else {
            hex(fg)
        };

        ctx.save();
        ctx.set_global_composite_operation("copy")?;
        ctx.set_fill_style_str(&bg_css);
        ctx.fill_rect(0.0, 0.0, f64::from(tw), f64::from(th));
        ctx.set_global_composite_operation("source-over")?;
        let font = Fonts::index(key.flags & BOLD != 0, key.flags & ITALIC != 0);
        if let Some(f) = rc.fonts.js.get(font) {
            set_font_js(ctx, f);
        }
        ctx.set_text_baseline(rc.baseline);
        ctx.set_fill_style_str(&fg_css);
        let mut fill = Fill::Css(fg_css);
        let pad = if restricted { 0 } else { ATLAS_PAD };
        let e = f64::from(pad);
        if custom && let Some(c) = single {
            let ops = rc.boxes.ops(c, m, rc.font_size);
            if let Some(p) = apply_ops(ctx, rc.document, ops, (e, e), m, &fill, fg, rc.patterns)? {
                fill = Fill::Pattern(p);
            }
        }
        let mut fuzzy = !powerline;
        let ul = underline_geom(rc.font_size, m.dpr, m.dev_char_h, pad);
        if key.underline != 0 {
            ctx.save();
            ctx.set_line_width(ul.width);
            match key.underline_color {
                None => set_stroke(ctx, &fill),
                Some(c) => {
                    fuzzy = false;
                    ctx.set_stroke_style_str(&hex(c));
                }
            }
            ctx.begin_path();
            let mut phase = f64::from(key.phase);
            for a in 0..cells {
                ctx.save();
                let h = e + f64::from(a) * cell_w;
                let c = e + f64::from(a + 1) * cell_w;
                let d = h + cell_w / 2.0;
                let (s, r, n, w) = (ul.s, ul.r, ul.n, ul.width);
                match key.underline {
                    2 => {
                        ctx.move_to(h, s);
                        ctx.line_to(c, s);
                        ctx.move_to(h, n);
                        ctx.line_to(c, n);
                    }
                    3 => {
                        let top = if w <= 1.0 {
                            n
                        } else {
                            (e + char_h - w / 2.0).ceil() - ul.half
                        };
                        let low = if w <= 1.0 {
                            s
                        } else {
                            (e + char_h + w / 2.0).ceil() - ul.half
                        };
                        // Recorte con `Path2D`: el trazo en curso sigue
                        // acumulando las celdas anteriores, como en xterm.js.
                        let clip = Path2d::new()?;
                        clip.rect(h, s, cell_w, n - s);
                        ctx.clip_with_path_2d(&clip);
                        let half_cell = cell_w / 2.0;
                        ctx.move_to(h - half_cell, r);
                        ctx.bezier_curve_to(h - half_cell, low, h, low, h, r);
                        ctx.bezier_curve_to(h, top, d, top, d, r);
                        ctx.bezier_curve_to(d, low, c, low, c, r);
                        ctx.bezier_curve_to(c, top, c + half_cell, top, c + half_cell, r);
                    }
                    4 => {
                        let dash = js_round(w);
                        let u = if phase == 0.0 {
                            0.0
                        } else if phase >= w {
                            2.0 * w - phase
                        } else {
                            w - phase
                        };
                        set_dash(ctx, &[dash, dash])?;
                        if phase >= w || u == 0.0 {
                            ctx.move_to(h + u, s);
                            ctx.line_to(c, s);
                        } else {
                            ctx.move_to(h, s);
                            ctx.line_to(h + u, s);
                            ctx.move_to(h + u + w, s);
                            ctx.line_to(c, s);
                        }
                        phase = next_variant_offset(c - h, w, phase);
                    }
                    5 => {
                        let v = c - h;
                        let long = (0.6 * v).floor();
                        let gap = (0.3 * v).floor();
                        set_dash(ctx, &[long, gap, v - long - gap])?;
                        ctx.move_to(h, s);
                        ctx.line_to(c, s);
                    }
                    _ => {
                        ctx.move_to(h, s);
                        ctx.line_to(c, s);
                    }
                }
                ctx.stroke();
                ctx.restore();
            }
            ctx.restore();
            // Los trazos bajo la línea de base se apartan del subrayado.
            if !custom && rc.font_size >= 12.0 && text != " " {
                ctx.save();
                ctx.set_text_baseline("alphabetic");
                let measured = ctx.measure_text(text)?;
                ctx.restore();
                if measured.actual_bounding_box_descent() > 0.0 {
                    ctx.save();
                    let lift = (ul.width / 2.0).ceil();
                    ctx.begin_path();
                    ctx.rect(
                        e,
                        ul.s - lift,
                        cell_w * f64::from(cells),
                        ul.n - ul.s + lift,
                    );
                    ctx.clip();
                    ctx.set_line_width(3.0 * m.dpr);
                    ctx.set_stroke_style_str(&bg_css);
                    ctx.stroke_text(text, e, e + char_h)?;
                    ctx.restore();
                }
            }
        }
        if !custom {
            ctx.fill_text(text, e, e + char_h)?;
        }
        if text == "_" {
            // El «_» puede caer bajo la celda: se sube hasta 5 px.
            let probe = |ctx: &CanvasRenderingContext2d| -> Result<bool, JsValue> {
                let data = ctx.get_image_data(e, e, cell_w, cell_h)?;
                let mut bytes = data.data().0;
                Ok(clear_color(&mut bytes, key.bg, fg, fuzzy))
            };
            let mut empty = probe(ctx)?;
            let mut lift = 1.0;
            while empty && lift <= 5.0 {
                ctx.save();
                ctx.set_fill_style_str(&bg_css);
                ctx.fill_rect(0.0, 0.0, f64::from(tw), f64::from(th));
                ctx.restore();
                ctx.fill_text(text, e, e + char_h - lift)?;
                empty = probe(ctx)?;
                lift += 1.0;
            }
        }
        if key.flags & STRIKE != 0 {
            let w = (rc.font_size * m.dpr / 10.0).floor().max(1.0);
            let half = if ctx.line_width() % 2.0 == 1.0 {
                0.5
            } else {
                0.0
            };
            ctx.set_line_width(w);
            set_stroke(ctx, &fill);
            ctx.begin_path();
            let y = e + (char_h / 2.0).floor() - half;
            ctx.move_to(e, y);
            ctx.line_to(e + f64::from(m.dev_char_w) * f64::from(cells), y);
            ctx.stroke();
        }
        ctx.restore();

        let image = ctx.get_image_data(0.0, 0.0, f64::from(tw), f64::from(th))?;
        let mut bytes = image.data().0;
        if clear_color(&mut bytes, key.bg, fg, fuzzy) {
            return Ok(Slot::EMPTY);
        }
        // `_findGlyphBoundingBox(O, box, l, restricted, custom, pad)`.
        let (rows, allowed) = if restricted {
            (m.dev_h, m.dev_w)
        } else {
            (th, tmp_size_wanted_w(m.dev_w, text.encode_utf16().count()))
        };
        let Some((left, top, right, bottom)) = glyph_bbox(&bytes, tw, rows, allowed, pad) else {
            return Ok(Slot::EMPTY);
        };
        let (w, h) = (right - left + 1, bottom - top + 1);
        let (page, sx, sy) = if !cache || w > UNCACHED_LIMIT || h > UNCACHED_LIMIT {
            // Demasiado grande para el atlas (o clave sin caché): la imagen
            // ya limpia vuelve al auxiliar y se copia desde allí.
            let data = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&bytes), tw, th)?;
            self.tmp_ctx.put_image_data(&data, 0.0, 0.0)?;
            (Slot::UNCACHED, left, top)
        } else {
            let (page, sx, sy) = self.place(rc.document, w, h)?;
            let crop = crop(&bytes, tw, (left, top, w, h));
            let data = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&crop), w, h)?;
            if let Some(p) = self.pages.get(usize::from(page)) {
                p.ctx.put_image_data(&data, f64::from(sx), f64::from(sy))?;
            }
            (page, sx, sy)
        };
        // Destino respecto a la esquina de la celda (`offset` de xterm.js).
        let to_i32 = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
        let (dx0, dy0) = if custom || restricted {
            let center = (m.dev_w.saturating_sub(m.dev_char_w)) / 2;
            (to_i32(m.char_left) - to_i32(center), 0)
        } else {
            (to_i32(m.char_left), to_i32(m.char_top))
        };
        Ok(Slot {
            page,
            // Tras `place`, que puede haber vaciado el atlas.
            generation: self.index.generation(),
            sx,
            sy,
            w,
            h,
            dx: dx0 + to_i32(left) - to_i32(pad),
            dy: dy0 + to_i32(top) - to_i32(pad),
        })
    }

    /// Hueco en una página; si no hay sitio, página nueva o atlas vacío.
    fn place(&mut self, document: &Document, w: u32, h: u32) -> Result<(u16, u32, u32), JsValue> {
        for _ in 0..2 {
            if !self.pages.is_empty()
                && let Some((x, y)) = self.shelf.place(w, h)
            {
                let page = u16::try_from(self.pages.len() - 1).unwrap_or(0);
                return Ok((page, x, y));
            }
            if self.pages.len() >= MAX_PAGES {
                self.clear();
            }
            let canvas = new_canvas(document, PAGE_SIZE, PAGE_SIZE)?;
            let ctx = context(&canvas, true, false)?;
            self.pages.push(Page { canvas, ctx });
            self.shelf = Shelf::new(PAGE_SIZE);
        }
        Err(JsValue::from_str("glifo mayor que una página del atlas"))
    }
}

/// Copia el rectángulo `(x, y, w, h)` de una imagen RGBA de ancho `width`.
fn crop(data: &[u8], width: u32, rect: (u32, u32, u32, u32)) -> Vec<u8> {
    let (x, y, w, h) = rect;
    let mut out = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for row in y..y + h {
        let start = ((row as usize) * (width as usize) + x as usize) * 4;
        let end = start + (w as usize) * 4;
        if let Some(line) = data.get(start..end) {
            out.extend_from_slice(line);
        }
    }
    out
}

fn set_stroke(ctx: &CanvasRenderingContext2d, fill: &Fill) {
    match fill {
        Fill::Css(css) => ctx.set_stroke_style_str(css),
        Fill::Js(v) => set_stroke_js(ctx, v),
        Fill::Pattern(p) => ctx.set_stroke_style_canvas_pattern(p),
    }
}

fn set_dash(ctx: &CanvasRenderingContext2d, segments: &[f64]) -> Result<(), JsValue> {
    let array = js_sys::Array::new();
    for s in segments {
        array.push(&JsValue::from_f64(*s));
    }
    ctx.set_line_dash(&array)
}

/// Órdenes de los glifos de dibujo por carácter (`draw_ops` analiza las
/// definiciones en cada llamada).
#[derive(Default)]
struct BoxCache {
    ops: BoxIndex<Vec<DrawOp>>,
}

impl BoxCache {
    fn ops(&mut self, c: char, m: &CellMetrics, font_size: f64) -> &[DrawOp] {
        self.ops.entry(c, || {
            let mut out = Vec::new();
            let metrics = glyphs::CellMetrics {
                cell_w: f64::from(m.dev_w),
                cell_h: f64::from(m.dev_h),
                dpr: m.dpr,
                font_size,
            };
            glyphs::draw_ops(c, &metrics, &mut out);
            out
        })
    }
}

/// Tramas ░▒▓ por color (xterm.js las guarda por `fillStyle`), acotadas.
struct PatternCache {
    patterns: Bounded<pattern_key::Key, CanvasPattern>,
}

/// Tramas guardadas como mucho (3 máscaras × colores en pantalla).
const MAX_PATTERNS: usize = 64;

impl Default for PatternCache {
    fn default() -> PatternCache {
        PatternCache {
            patterns: Bounded::new(MAX_PATTERNS),
        }
    }
}

impl PatternCache {
    /// La trama de `mask` en `color`. Su alfa es `máscara · 255 · g`, con
    /// `g` el alfa del `fillStyle`; con `#rrggbb80` satura a 255, así que
    /// también una trama atenuada sale opaca, como en xterm.js.
    fn get(
        &mut self,
        document: &Document,
        ctx: &CanvasRenderingContext2d,
        mask: &'static [&'static [u8]],
        color: [u8; 3],
    ) -> Result<CanvasPattern, JsValue> {
        let key = pattern_key::key(mask.as_ptr() as usize, color);
        if let Some(p) = self.patterns.get(&key) {
            return Ok(p.clone());
        }
        let rows = u32::try_from(mask.len()).unwrap_or(1).max(1);
        let cols = u32::try_from(mask.first().map_or(1, |r| r.len()))
            .unwrap_or(1)
            .max(1);
        let canvas = new_canvas(document, cols, rows)?;
        let pctx: CanvasRenderingContext2d = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("canvas 2d no disponible"))?
            .dyn_into()?;
        let mut bytes = Vec::with_capacity((rows * cols * 4) as usize);
        for row in mask {
            for &bit in *row {
                bytes.extend_from_slice(&[color[0], color[1], color[2], bit.saturating_mul(255)]);
            }
        }
        let data = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&bytes), cols, rows)?;
        pctx.put_image_data(&data, 0.0, 0.0)?;
        let pattern = ctx
            .create_pattern_with_html_canvas_element(&canvas, "repeat")?
            .ok_or_else(|| JsValue::from_str("trama no disponible"))?;
        self.patterns.insert(key, pattern.clone());
        Ok(pattern)
    }
}

/// Repite las órdenes de un glifo de dibujo en `ctx` con la esquina de la
/// celda en `origin`. Devuelve la trama si el relleno quedó en ella.
#[allow(clippy::too_many_arguments)]
fn apply_ops(
    ctx: &CanvasRenderingContext2d,
    document: &Document,
    ops: &[DrawOp],
    origin: (f64, f64),
    m: &CellMetrics,
    fill: &Fill,
    color: [u8; 3],
    patterns: &mut PatternCache,
) -> Result<Option<CanvasPattern>, JsValue> {
    let (ox, oy) = origin;
    let (cell_w, cell_h) = (f64::from(m.dev_w), f64::from(m.dev_h));
    let mut pattern = None;
    for op in ops {
        match *op {
            DrawOp::FillRect { x, y, w, h } => ctx.fill_rect(ox + x, oy + y, w, h),
            DrawOp::FillPattern { mask } => {
                let p = patterns.get(document, ctx, mask, color)?;
                ctx.set_fill_style_canvas_pattern(&p);
                ctx.fill_rect(ox, oy, cell_w, cell_h);
                pattern = Some(p);
            }
            DrawOp::ClipCell => {
                ctx.begin_path();
                ctx.rect(ox, oy, cell_w, cell_h);
                ctx.clip();
            }
            DrawOp::BeginPath => ctx.begin_path(),
            DrawOp::MoveTo { x, y } => ctx.move_to(ox + x, oy + y),
            DrawOp::LineTo { x, y } => ctx.line_to(ox + x, oy + y),
            DrawOp::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => ctx.bezier_curve_to(ox + x1, oy + y1, ox + x2, oy + y2, ox + x, oy + y),
            DrawOp::Stroke { line_width } => {
                ctx.set_line_width(line_width);
                set_stroke(ctx, fill);
                ctx.stroke();
                ctx.close_path();
            }
            DrawOp::Fill => {
                ctx.fill();
                ctx.close_path();
            }
        }
    }
    Ok(pattern)
}

// ---------------------------------------------------------------------------
// El canvas de la terminal.
// ---------------------------------------------------------------------------

/// Un `<canvas>` por terminal, con el almacén en píxeles de dispositivo.
pub struct Canvas2d {
    document: Document,
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    /// Capa del cursor encima de la selección y del enlace
    /// (`CursorRenderLayer`, z-index 3 de addon-canvas); transparente.
    cursor: HtmlCanvasElement,
    cursor_ctx: CanvasRenderingContext2d,
    /// Celdas donde quedó dibujado el cursor (columna, fila, celdas).
    cursor_at: Option<(u16, usize, u16)>,
    /// Canvas transparente para el carácter del cursor de bloque.
    scratch: HtmlCanvasElement,
    scratch_ctx: CanvasRenderingContext2d,
    theme: CanvasTheme,
    family: String,
    font_size: f64,
    fonts: Fonts,
    baseline: &'static str,
    m: CellMetrics,
    cols: u16,
    colors: ColorCache,
    /// Último color de relleno puesto en `ctx` (para no repetirlo).
    current_fill: Option<[u8; 3]>,
    atlas: Atlas,
    boxes: BoxCache,
    patterns: PatternCache,
    /// Primer error de pintado (no se lanza por cuadro: se consulta).
    error: Option<JsValue>,
    /// Último `devicePixelContentBox` observado y el tamaño CSS al que
    /// corresponde.
    observed: Option<((f64, f64), (u32, u32))>,
}

impl Canvas2d {
    /// Canvas nuevo (el llamador lo inserta en el DOM con [`Canvas2d::element`]).
    pub fn new(
        window: &Window,
        document: &Document,
        theme: CanvasTheme,
        family: &str,
        font_size: f64,
        m: CellMetrics,
    ) -> Result<Canvas2d, JsValue> {
        let canvas = new_canvas(document, 0, 0)?;
        let ctx = context(&canvas, false, false)?;
        let cursor = new_canvas(document, 0, 0)?;
        let cursor_ctx = context(&cursor, true, false)?;
        let style = cursor.style();
        for (name, value) in [
            ("position", "absolute"),
            ("left", "0"),
            ("top", "0"),
            ("pointer-events", "none"),
            ("z-index", "3"),
        ] {
            style.set_property(name, value)?;
        }
        let (scratch_w, scratch_h) = cursor_scratch_size(m.dev_w, m.dev_h);
        let scratch = new_canvas(document, scratch_w, scratch_h)?;
        let scratch_ctx = context(&scratch, true, false)?;
        let agent = window.navigator().user_agent().unwrap_or_default();
        Ok(Canvas2d {
            atlas: Atlas::new(document, &m)?,
            document: document.clone(),
            canvas,
            ctx,
            cursor,
            cursor_ctx,
            cursor_at: None,
            scratch,
            scratch_ctx,
            theme,
            family: family.to_string(),
            font_size,
            fonts: Fonts::new(family, font_size, m.dpr),
            baseline: text_baseline(&agent),
            m,
            cols: 0,
            colors: ColorCache::default(),
            current_fill: None,
            boxes: BoxCache::default(),
            patterns: PatternCache::default(),
            error: None,
            observed: None,
        })
    }

    /// El elemento `<canvas>`.
    pub fn element(&self) -> &HtmlCanvasElement {
        &self.canvas
    }

    /// La capa del cursor: va en el DOM encima de la selección.
    pub fn cursor_element(&self) -> &HtmlCanvasElement {
        &self.cursor
    }

    /// Mismo almacén y tamaño CSS que el canvas del texto (vacía la capa).
    fn size_cursor_layer(&mut self, w: u32, h: u32) {
        self.cursor.set_width(w);
        self.cursor.set_height(h);
        self.cursor_at = None;
        let (main, style) = (self.canvas.style(), self.cursor.style());
        for name in ["width", "height"] {
            let value = main.get_property_value(name).unwrap_or_default();
            let r = style.set_property(name, &value);
            self.note(r);
        }
    }

    /// `_clearCursor`: borra el cursor si está en la fila `line` (que se va
    /// a repintar; si sigue ahí, el planificador lo vuelve a dibujar).
    fn clear_cursor_on(&mut self, line: usize) {
        let Some((col, at, cells)) = self.cursor_at else {
            return;
        };
        if at != line {
            return;
        }
        let (cw, ch) = (f64::from(self.m.dev_w), f64::from(self.m.dev_h));
        self.cursor_ctx.clear_rect(
            f64::from(col) * cw,
            self.row_y(at),
            f64::from(cells) * cw,
            ch,
        );
        self.cursor_at = None;
    }

    /// Nuevo tema: los glifos rasterizados con los colores viejos sobran.
    pub fn set_theme(&mut self, theme: CanvasTheme) {
        self.theme = theme;
        self.reset_caches();
    }

    /// Vacía glifos y tramas (tema nuevo, fuente nueva, contexto del canvas
    /// restaurado tras perderse la GPU).
    pub fn reset_caches(&mut self) {
        self.atlas.clear();
        self.patterns = PatternCache::default();
        self.current_fill = None;
    }

    /// Nueva fuente o tamaño (las medidas llegan después con `resize`).
    pub fn set_font(&mut self, family: &str, font_size: f64) {
        self.family = family.to_string();
        self.font_size = font_size;
        self.fonts = Fonts::new(family, font_size, self.m.dpr);
        self.reset_caches();
        self.boxes = BoxCache::default();
    }

    /// El navegador dice que el canvas ocupa `w × h` píxeles de dispositivo
    /// (`ResizeObserver` con `device-pixel-content-box`, como
    /// `observeDevicePixelDimensions` de xterm.js): con un `dpr` fraccionario
    /// o un ancho CSS redondeado no siempre es `cols·cell`, y un almacén de
    /// otro tamaño se vería escalado. `true` si cambió (hay que repintar).
    pub fn set_device_size(&mut self, w: u32, h: u32) -> bool {
        let style = self.canvas.style();
        let css = |name: &str| {
            style
                .get_property_value(name)
                .ok()
                .and_then(|v| crate::number_text::parse(v.trim_end_matches("px")))
                .unwrap_or(0.0)
        };
        self.observed = Some(((css("width"), css("height")), (w, h)));
        if w == 0 || h == 0 || (self.canvas.width(), self.canvas.height()) == (w, h) {
            return false;
        }
        self.canvas.set_width(w);
        self.canvas.set_height(h);
        self.size_cursor_layer(w, h);
        self.current_fill = None;
        self.fill(self.theme.bg);
        self.ctx.fill_rect(0.0, 0.0, f64::from(w), f64::from(h));
        true
    }

    /// Primer error de pintado desde la última consulta.
    pub fn take_error(&mut self) -> Option<JsValue> {
        self.error.take()
    }

    fn note(&mut self, r: Result<(), JsValue>) {
        if let Err(e) = r
            && self.error.is_none()
        {
            self.error = Some(e);
        }
    }

    fn fill(&mut self, c: [u8; 3]) {
        if self.current_fill != Some(c) {
            let v = self.colors.get(c);
            set_fill_js(&self.ctx, &v);
            self.current_fill = Some(c);
        }
    }

    /// `_clipRow`: recorta a la franja de la fila (dentro de un `save`).
    fn clip_row(&self, y: f64) {
        clip_row_on(&self.ctx, self.cols, &self.m, y);
    }

    fn row_y(&self, line: usize) -> f64 {
        // Una fila cabe en f64 de sobra; u32 satura en filas absurdas.
        f64::from(u32::try_from(line).unwrap_or(u32::MAX)) * f64::from(self.m.dev_h)
    }

    /// Glifos de una tira. Un error en una celda se anota y se sigue con la
    /// siguiente (no se pierde el resto de la fila). Con `draw = false` solo
    /// deja los glifos en el atlas (ver `prepare_row`).
    fn paint_glyphs(&mut self, run: &Run, y: f64, selected: Option<Selected>, draw: bool) {
        let style = &run.style;
        if style.hidden {
            return;
        }
        let mut flags = 0;
        if style.bold {
            flags |= BOLD;
        }
        if style.italic {
            flags |= ITALIC;
        }
        // `fg` ya viene mezclado con el fondo; addon-canvas pinta el color
        // original a opacidad 0,5 (`dim_fg`), y eso cambia los bordes.
        let fg = match style.dim_fg {
            Some(original) => {
                flags |= DIM;
                original
            }
            None => style.fg,
        };
        if style.strike {
            flags |= STRIKE;
        }
        match run.kind {
            RunKind::Wide => flags |= WIDE,
            RunKind::Box(_) => flags |= CUSTOM,
            RunKind::Text => {}
        }
        let underline = underline_code(style.underline);
        let decorated = underline != 0 || style.strike;
        let ul_w = underline_geom(self.font_size, self.m.dpr, self.m.dev_char_h, 0).width;
        for (col, text) in run.cell_texts() {
            if text == " " && !decorated {
                continue;
            }
            let (fg, bg, flags) = match selected.filter(|s| s.contains(col)) {
                Some(s) => {
                    let c = match run.kind {
                        RunKind::Box(c) => Some(c),
                        _ => text.chars().next(),
                    };
                    selected_glyph(style, fg, flags, c, s.color)
                }
                None => (fg, style.bg, flags),
            };
            let phase = if style.underline == Underline::Dotted {
                dotted_phase(col, self.m.dev_w, ul_w)
            } else {
                0.0
            };
            let key = GlyphStyle {
                fg,
                bg,
                flags,
                underline,
                underline_color: style.underline_color,
                phase: phase.clamp(0.0, f64::from(u16::MAX)) as u16,
            };
            let mut rc = RasterCtx {
                document: &self.document,
                m: &self.m,
                font_size: self.font_size,
                fonts: &self.fonts,
                baseline: self.baseline,
                boxes: &mut self.boxes,
                patterns: &mut self.patterns,
            };
            let slot = match self.atlas.glyph(text, key, &mut rc) {
                Ok(slot) => slot,
                Err(e) => {
                    self.note(Err(e));
                    continue;
                }
            };
            if !draw || slot.w == 0 || slot.h == 0 {
                continue;
            }
            let Some(source) = self.atlas.source(&slot) else {
                continue;
            };
            let x = f64::from(col) * f64::from(self.m.dev_w) + f64::from(slot.dx);
            let (w, h) = (f64::from(slot.w), f64::from(slot.h));
            let drawn = self
                .ctx
                .draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                    source,
                    f64::from(slot.sx),
                    f64::from(slot.sy),
                    w,
                    h,
                    x,
                    y + f64::from(slot.dy),
                    w,
                    h,
                );
            self.note(drawn);
        }
    }

    fn try_paint_row(
        &mut self,
        row: &comandos_term::render::RowRender,
        selected: Option<Selected>,
    ) -> Result<(), JsValue> {
        let y = self.row_y(row.line);
        let (cw, ch) = (f64::from(self.m.dev_w), f64::from(self.m.dev_h));
        self.ctx.save();
        self.clip_row(y);
        self.fill(self.theme.bg);
        self.ctx.fill_rect(0.0, y, f64::from(self.cols) * cw, ch);
        for &(col, cells, color) in &row.bg_runs {
            self.fill(color);
            self.ctx
                .fill_rect(f64::from(col) * cw, y, f64::from(cells) * cw, ch);
        }
        for run in &row.runs {
            self.paint_glyphs(run, y, selected, true);
        }
        self.ctx.restore();
        // `restore` devuelve el relleno de antes del `save`.
        self.current_fill = None;
        Ok(())
    }

    /// El cursor en su capa (encima de la selección, como addon-canvas):
    /// primero se borra el anterior.
    fn try_paint_cursor(&mut self, c: &CursorView, under: Option<&Run>) -> Result<(), JsValue> {
        if let Some((_, line, _)) = self.cursor_at {
            self.clear_cursor_on(line);
        }
        let m = self.m;
        let (cw, ch, dpr) = (f64::from(m.dev_w), f64::from(m.dev_h), m.dpr);
        let x = f64::from(c.col) * cw;
        let y = self.row_y(c.line);
        let cells = if c.wide { 2.0 } else { 1.0 };
        let cursor = self.colors.get(self.theme.cursor);
        let ctx = self.cursor_ctx.clone();
        ctx.save();
        match c.shape {
            CursorShape::Beam => {
                set_fill_js(&ctx, &cursor);
                ctx.fill_rect(x, y, dpr, ch);
            }
            CursorShape::Underline => {
                set_fill_js(&ctx, &cursor);
                ctx.fill_rect(x, y + ch - dpr - 1.0, cw, dpr);
            }
            CursorShape::HollowBlock => {
                set_stroke_js(&ctx, &cursor);
                ctx.set_line_width(dpr);
                ctx.stroke_rect(x + dpr / 2.0, y + dpr / 2.0, cells * cw - dpr, ch - dpr);
            }
            CursorShape::Block => {
                set_fill_js(&ctx, &cursor);
                ctx.fill_rect(x, y, cells * cw, ch);
                self.cursor_glyph(&ctx, c, under, x, y)?;
            }
            CursorShape::Hidden => {}
        }
        ctx.restore();
        if c.shape != CursorShape::Hidden {
            self.cursor_at = Some((c.col, c.line, if c.wide { 2 } else { 1 }));
        }
        Ok(())
    }

    /// `_fillCharTrueColor`: el carácter bajo el cursor de bloque, en el
    /// color de acento, con la fuente regular.
    fn cursor_glyph(
        &mut self,
        ctx: &CanvasRenderingContext2d,
        c: &CursorView,
        under: Option<&Run>,
        x: f64,
        y: f64,
    ) -> Result<(), JsValue> {
        let Some(run) = under else {
            return Ok(());
        };
        let Some((_, text)) = run.cell_texts().find(|(col, _)| *col == c.col) else {
            return Ok(());
        };
        let m = self.m;
        let accent = self.theme.cursor_accent;
        clip_row_on(ctx, self.cols, &m, y);
        if let RunKind::Box(ch) = run.kind {
            let accent_js = self.colors.get(accent);
            set_fill_js(ctx, &accent_js);
            let ops = self.boxes.ops(ch, &m, self.font_size);
            apply_ops(
                ctx,
                &self.document,
                ops,
                (x, y),
                &m,
                &Fill::Js(accent_js),
                accent,
                &mut self.patterns,
            )?;
            return Ok(());
        }
        // En un canvas transparente, como la capa del cursor de xterm.js.
        let cw = f64::from(m.dev_w);
        let cells = if c.wide { 2.0 } else { 1.0 };
        let (sw, sh) = (
            (cw * (cells + 2.0)).min(f64::from(self.scratch.width())),
            f64::from(m.dev_h).min(f64::from(self.scratch.height())),
        );
        let s = &self.scratch_ctx;
        s.clear_rect(0.0, 0.0, sw, sh);
        if let Some(f) = self.fonts.js.first() {
            set_font_js(s, f);
        }
        s.set_text_baseline(self.baseline);
        let accent_js = self.colors.get(accent);
        set_fill_js(s, &accent_js);
        s.fill_text(
            text,
            cw + f64::from(m.char_left),
            f64::from(m.char_top) + f64::from(m.dev_char_h),
        )?;
        ctx.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
            &self.scratch,
            0.0,
            0.0,
            sw,
            sh,
            x - cw,
            y,
            sw,
            sh,
        )
    }
}

/// `_clipRow` sobre `ctx` (dentro de un `save`).
fn clip_row_on(ctx: &CanvasRenderingContext2d, cols: u16, m: &CellMetrics, y: f64) {
    let w = f64::from(cols) * f64::from(m.dev_w);
    ctx.begin_path();
    ctx.rect(0.0, y, w, f64::from(m.dev_h));
    ctx.clip();
}

/// Código de `underlineStyle` de xterm.js.
fn underline_code(u: Underline) -> u8 {
    match u {
        Underline::None => 0,
        Underline::Single => 1,
        Underline::Double => 2,
        Underline::Curly => 3,
        Underline::Dotted => 4,
        Underline::Dashed => 5,
    }
}

impl Painter for Canvas2d {
    fn clear_rows(&mut self, rows: &[usize]) {
        let (cw, ch) = (f64::from(self.m.dev_w), f64::from(self.m.dev_h));
        let width = f64::from(self.cols) * cw;
        self.fill(self.theme.bg);
        for &line in rows {
            let y = self.row_y(line);
            self.ctx.fill_rect(0.0, y, width, ch);
            self.clear_cursor_on(line);
        }
    }

    fn prepare_row(
        &mut self,
        row: &comandos_term::render::RowRender,
        selected: Option<Selected>,
        _m: &CellMetrics,
    ) {
        for run in &row.runs {
            self.paint_glyphs(run, 0.0, selected, false);
        }
    }

    fn paint_row(
        &mut self,
        row: &comandos_term::render::RowRender,
        selected: Option<Selected>,
        _m: &CellMetrics,
    ) {
        self.clear_cursor_on(row.line);
        let r = self.try_paint_row(row, selected);
        self.note(r);
    }

    fn paint_cursor(&mut self, c: &CursorView, under: Option<&Run>, _m: &CellMetrics) {
        let r = self.try_paint_cursor(c, under);
        self.note(r);
    }

    fn resize(&mut self, cols: u16, rows: u16, m: &CellMetrics) {
        let metrics_changed = *m != self.m;
        self.m = *m;
        self.cols = cols;
        let (css_w, css_h) = m.css_canvas(cols, rows);
        // El almacén es `cols·cell × rows·cell`, salvo que el navegador ya
        // haya dicho cuántos píxeles de dispositivo ocupa este tamaño CSS.
        let (w, h) = match self.observed {
            Some((css, dev)) if css == (css_w, css_h) => dev,
            _ => m.device_canvas(cols, rows),
        };
        // Cambiar el tamaño vacía el canvas y reinicia el estado del contexto.
        self.canvas.set_width(w);
        self.canvas.set_height(h);
        let style = self.canvas.style();
        let r = style
            .set_property("width", &crate::number_text::px(css_w))
            .and_then(|()| style.set_property("height", &crate::number_text::px(css_h)));
        self.note(r);
        self.size_cursor_layer(w, h);
        self.current_fill = None;
        if metrics_changed {
            self.fonts = Fonts::new(&self.family, self.font_size, m.dpr);
            // Atlas nuevo, como xterm.js al cambiar la celda: el auxiliar
            // vuelve a su tamaño inicial.
            match Atlas::new(&self.document, m) {
                Ok(atlas) => self.atlas = atlas,
                Err(e) => {
                    self.atlas.clear();
                    self.note(Err(e));
                }
            }
            self.patterns = PatternCache::default();
            self.boxes = BoxCache::default();
            let (scratch_w, scratch_h) = cursor_scratch_size(m.dev_w, m.dev_h);
            self.scratch.set_width(scratch_w);
            self.scratch.set_height(scratch_h);
        }
        // `_clearAll` de una capa opaca.
        self.fill(self.theme.bg);
        self.ctx.fill_rect(0.0, 0.0, f64::from(w), f64::from(h));
    }
}

/// Texto de diagnóstico de las fuentes (para pruebas y depuración).
impl Canvas2d {
    pub fn font_strings(&self) -> &[String; 4] {
        &self.fonts.text
    }
}

#[cfg(test)]
mod tests {

    use comandos_term::render::Style;

    fn style(fg: [u8; 3], bg: [u8; 3], inverse: bool, default_bg: bool) -> Style {
        Style {
            fg,
            bg,
            bold: false,
            italic: false,
            dim: false,
            dim_fg: None,
            underline: Underline::None,
            underline_color: None,
            strike: false,
            hidden: false,
            inverse,
            default_bg,
            fg_plain: None,
        }
    }

    /// `CellColorResolver` con una celda seleccionada.
    #[test]
    fn selected_cells_use_the_opaque_selection_or_a_half_blend() {
        let sel = [0x2E, 0x38, 0x52];
        let (fg, bg) = ([0xEA, 0xF0, 0xFB], [0x0A, 0x0D, 0x13]);
        // Fondo del tema: la selección opaca; el texto no cambia.
        assert_eq!(
            selected_glyph_colors(&style(fg, bg, false, true), fg, Some('a'), sel),
            (fg, sel)
        );
        // Fondo propio (rojo #cc0000): mezcla a medias (alfa 128/255).
        let red = [0xCC, 0x00, 0x00];
        assert_eq!(blend_half(red, sel), [0x7D, 0x1C, 0x29]);
        assert_eq!(
            selected_glyph_colors(&style(fg, red, false, false), fg, Some('a'), sel).1,
            [0x7D, 0x1C, 0x29]
        );
        // Vídeo inverso: el fondo (el color del texto) también se mezcla.
        assert_eq!(
            selected_glyph_colors(&style(bg, fg, true, true), bg, Some('a'), sel).1,
            blend_half(fg, sel)
        );
        // Bloques y powerline: el texto también cambia.
        assert_eq!(
            selected_glyph_colors(&style(fg, bg, false, true), fg, Some('█'), sel).0,
            blend_half(fg, sel)
        );
        assert_eq!(
            selected_glyph_colors(&style(bg, fg, true, true), bg, Some('\u{E0B0}'), sel).0,
            sel
        );
        assert_eq!(blend_half(sel, sel), sel);
    }

    /// Ronda 4, I1: una celda atenuada (SGR 2) seleccionada pierde la
    /// atenuación (`~BgFlags.DIM`); el resto de banderas se queda.
    #[test]
    fn selected_cells_are_not_dimmed() {
        let sel = [0x2E, 0x38, 0x52];
        let (fg, bg) = ([0xEA, 0xF0, 0xFB], [0x0A, 0x0D, 0x13]);
        let dimmed = Style {
            dim: true,
            dim_fg: Some(fg),
            ..style(fg, bg, false, true)
        };
        let (out_fg, out_bg, flags) =
            selected_glyph(&dimmed, fg, DIM | BOLD | ITALIC, Some('a'), sel);
        assert_eq!((out_fg, out_bg), (fg, sel));
        assert_eq!(flags, BOLD | ITALIC);
        assert_eq!(selected_glyph(&dimmed, fg, BOLD, Some('a'), sel).2, BOLD);
    }

    /// Ronda 4, m1: un glifo de fondo seleccionado con negrita mezcla el
    /// color de la paleta sin aclarar.
    #[test]
    fn selected_bold_box_glyphs_blend_the_plain_palette_color() {
        let sel = [0x2E, 0x38, 0x52];
        let (bright, plain) = ([0xFF, 0x55, 0x55], [0xCD, 0x00, 0x00]);
        let bold = Style {
            bold: true,
            fg_plain: Some(plain),
            ..style(bright, [0, 0, 0], false, true)
        };
        assert_eq!(
            selected_glyph_colors(&bold, bright, Some('█'), sel).0,
            blend_half(plain, sel)
        );
        // Un glifo normal conserva el color aclarado (lo aclara el atlas).
        assert_eq!(
            selected_glyph_colors(&bold, bright, Some('a'), sel).0,
            bright
        );
    }
    use super::*;

    /// Atlas de mentira: cada página guarda el texto de sus glifos, con el
    /// mismo tope de páginas que se vacía al llenarse.
    struct FakeAtlas {
        index: GlyphIndex<u8, (usize, usize, u32)>,
        pages: Vec<Vec<String>>,
        per_page: usize,
        max_pages: usize,
    }

    impl GlyphStore for FakeAtlas {
        type Key = u8;
        type Slot = (usize, usize, u32);
        fn index(&mut self) -> &mut GlyphIndex<u8, (usize, usize, u32)> {
            &mut self.index
        }
        fn clear_all(&mut self) {
            self.pages.clear();
            self.index.clear();
        }
    }

    impl FakeAtlas {
        fn raster(&mut self, text: &str, cache: bool) -> Result<((usize, usize, u32), bool), ()> {
            if !cache {
                // Sin caché: no ocupa página.
                return Ok(((usize::MAX, 0, self.index.generation()), false));
            }
            if self.pages.last().is_none_or(|p| p.len() >= self.per_page) {
                if self.pages.len() >= self.max_pages {
                    self.clear_all();
                }
                self.pages.push(Vec::new());
            }
            let page = self.pages.len() - 1;
            let list = self.pages.get_mut(page).ok_or(())?;
            list.push(text.to_string());
            Ok(((page, list.len() - 1, self.index.generation()), true))
        }
    }

    #[test]
    fn every_cached_slot_resolves_to_its_own_glyph_past_the_caps() {
        let mut atlas = FakeAtlas {
            index: GlyphIndex::default(),
            pages: Vec::new(),
            per_page: 3000,
            max_pages: 4,
        };
        let mut clears = 0;
        let mut last_gen = atlas.index.generation();
        for n in 0..40_000u32 {
            // Glifos de uno y de muchos puntos de código (colores distintos).
            let text = if n % 7 == 0 {
                format!("e{}", "\u{301}".repeat((n % 50) as usize))
            } else {
                char::from_u32(0x4E00 + n % 20_000)
                    .map(String::from)
                    .unwrap_or_default()
            };
            let key = (n % 251) as u8;
            let slot = cached_glyph(&mut atlas, &text, key, |a, cache| a.raster(&text, cache))
                .unwrap_or_else(|()| panic!("raster"));
            let (page, pos, generation) = slot;
            assert_eq!(generation, atlas.index.generation());
            assert_eq!(atlas.pages.get(page).and_then(|p| p.get(pos)), Some(&text));
            if atlas.index.generation() != last_gen {
                clears += 1;
                last_gen = atlas.index.generation();
            }
        }
        assert!(clears > 3, "los topes deben saltar: {clears}");
        assert!(atlas.index.len() <= MAX_ENTRIES);
        assert!(atlas.index.key_bytes() <= MAX_KEY_BYTES);
        let current = atlas.index.generation();
        let mut seen = 0;
        atlas
            .index
            .for_each(|text, _key, &(page, pos, generation)| {
                seen += 1;
                assert_eq!(generation, current, "slot de una generación vaciada");
                assert_eq!(
                    atlas
                        .pages
                        .get(page)
                        .and_then(|p| p.get(pos))
                        .map(String::as_str),
                    Some(text)
                );
            });
        assert_eq!(seen, atlas.index.len());
    }

    #[test]
    fn a_key_longer_than_the_byte_cap_is_drawn_uncached_without_clearing() {
        let mut atlas = FakeAtlas {
            index: GlyphIndex::default(),
            pages: Vec::new(),
            per_page: 3000,
            max_pages: 4,
        };
        for text in ["a", "b", "e\u{301}"] {
            let _ = cached_glyph(&mut atlas, text, 0, |a, cache| a.raster(text, cache));
        }
        let (generation, len, pages) = (
            atlas.index.generation(),
            atlas.index.len(),
            atlas.pages.len(),
        );
        // Una celda con más de 256 KiB de marcas combinantes.
        let huge = format!("e{}", "\u{301}".repeat(MAX_KEY_BYTES / 2 + 1));
        assert!(huge.len() > MAX_KEY_BYTES);
        for _ in 0..3 {
            let slot = cached_glyph(&mut atlas, &huge, 0, |a, cache| a.raster(&huge, cache))
                .unwrap_or_else(|()| panic!("raster"));
            assert_eq!(slot.0, usize::MAX, "debe dibujarse sin caché");
        }
        assert_eq!(atlas.index.generation(), generation, "no vacía el atlas");
        assert_eq!((atlas.index.len(), atlas.pages.len()), (len, pages));
        assert_eq!(atlas.index.lookup("a", &0).map(|s| s.1), Some(0));
    }

    #[test]
    fn long_keys_count_against_the_byte_cap() {
        let mut index: GlyphIndex<u8, u8> = GlyphIndex::default();
        // 3 bytes por repetición: 60 % del tope.
        let long = "a\u{301}".repeat(MAX_KEY_BYTES / 5);
        assert!(!index.needs_room(&long));
        index.insert(&long, 0, 1);
        assert!(index.needs_room(&long));
        assert!(!index.needs_room("b"));
        let generation = index.generation();
        index.clear();
        assert_eq!((index.len(), index.key_bytes()), (0, 0));
        assert_ne!(index.generation(), generation);
    }

    #[test]
    fn glyph_bbox_scans_the_region_xterm_scans() {
        // Imagen de 12×4; alfa en (1,1), en (9,2) y en (11,3).
        let mut data = vec![0u8; 12 * 4 * 4];
        for (x, y) in [(1usize, 1usize), (9, 2), (11, 3)] {
            if let Some(a) = data.get_mut((y * 12 + x) * 4 + 3) {
                *a = 255;
            }
        }
        // `l = 6`, `pad = 2`: izquierda y derecha en columnas [0, 8),
        // arriba y abajo en [0, 6). La tinta de (9,2) y (11,3) se recorta y
        // la derecha, que solo se busca en [pad, pad + l), queda en `l`.
        assert_eq!(glyph_bbox(&data, 12, 4, 6, 2), Some((1, 1, 6, 1)));
        // Con más ancho permitido entra (9,2); la fila de abajo sigue
        // buscándose solo en las primeras columnas.
        assert_eq!(glyph_bbox(&data, 12, 4, 8, 2), Some((1, 1, 9, 1)));
        // Tinta solo en el relleno derecho: arriba 0 y abajo `n` por
        // defecto (aquí recortado a la última fila: lo que sobra es
        // transparente).
        let mut pad_only = vec![0u8; 12 * 4 * 4];
        if let Some(a) = pad_only.get_mut((2 * 12 + 7) * 4 + 3) {
            *a = 255;
        }
        assert_eq!(glyph_bbox(&pad_only, 12, 4, 6, 2), Some((7, 0, 7, 3)));
        // Nada en la región: vacío.
        let mut far = vec![0u8; 12 * 4 * 4];
        if let Some(a) = far.get_mut((3 * 12 + 11) * 4 + 3) {
            *a = 255;
        }
        assert_eq!(glyph_bbox(&far, 12, 4, 6, 2), None);
        // Columnas fuera del canvas no se leen.
        assert_eq!(glyph_bbox(&data, 12, 4, 100, 100), Some((1, 1, 11, 3)));
    }

    #[test]
    fn cursor_scratch_is_bounded() {
        assert_eq!(cursor_scratch_size(25, 61), (100, 61));
        assert_eq!(cursor_scratch_size(0, 0), (1, 1));
        assert_eq!(cursor_scratch_size(5000, 18_000), (TMP_MAX, TMP_MAX));
    }

    #[test]
    fn css_colors() {
        assert_eq!(hex([0x0A, 0xD, 0xFF]), "#0a0dff");
        assert_eq!(hex_alpha([1, 2, 3], DIM_ALPHA), "#01020380");
    }

    #[test]
    fn clear_color_matches_xterm_tolerance() {
        let bg = [0, 0, 0];
        let fg = [120, 120, 120]; // umbral floor(360/12) = 30
        let mut px = vec![
            0, 0, 0, 255, // fondo exacto
            10, 10, 9, 255, // a 29 del fondo: difuso
            10, 10, 10, 255, // a 30: queda
            120, 120, 120, 255,
        ];
        assert!(!clear_color(&mut px, bg, fg, true));
        let alphas: Vec<u8> = px.chunks(4).map(|p| p[3]).collect();
        assert_eq!(alphas, vec![0, 0, 255, 255]);
        let mut strict = vec![10, 10, 9, 255, 0, 0, 0, 255];
        assert!(!clear_color(&mut strict, bg, fg, false));
        assert_eq!(strict[3], 255);
        let mut only_bg = vec![0, 0, 0, 255, 5, 5, 5, 255];
        assert!(clear_color(&mut only_bg, bg, fg, true));
    }

    #[test]
    fn bbox_of_alpha_pixels() {
        // Imagen 4×3 con dos píxeles visibles.
        let mut data = vec![0_u8; 4 * 3 * 4];
        let mut set = |x: usize, y: usize| {
            if let Some(a) = data.get_mut((y * 4 + x) * 4 + 3) {
                *a = 255;
            }
        };
        set(1, 0);
        set(2, 2);
        // Sin relleno y todo el ancho permitido: la caja de la tinta.
        assert_eq!(glyph_bbox(&data, 4, 3, 4, 0), Some((1, 0, 2, 2)));
        assert_eq!(crop(&data, 4, (1, 0, 2, 1)).len(), 8);
    }

    #[test]
    fn underline_geometry_like_draw_to_cache() {
        // 14 px a dpr 1: e = max(1, floor(14/15)) = 1 (impar → 0,5).
        let g = underline_geom(14.0, 1.0, 17, ATLAS_PAD);
        assert_eq!((g.width, g.half), (1.0, 0.5));
        assert_eq!((g.s, g.r, g.n), (18.5, 19.5, 20.5));
        // 14 px a dpr 2: e = floor(28/15) = 1; 30 px a dpr 1: e = 2 (par).
        let g2 = underline_geom(30.0, 1.0, 36, ATLAS_PAD);
        assert_eq!((g2.width, g2.half), (2.0, 0.0));
        assert_eq!(g2.s, 36.0);
    }

    #[test]
    fn dotted_phase_and_next_offset_follow_js_remainder() {
        assert_eq!(dotted_phase(3, 7, 1.0), 1.0);
        assert_eq!(dotted_phase(4, 7, 1.0), 0.0);
        // (7 − (2 − 1)) % 2 = 0; (7 − (2 − 0)) % 2 = 1.
        assert_eq!(next_variant_offset(7.0, 1.0, 1.0), 0.0);
        assert_eq!(next_variant_offset(7.0, 1.0, 0.0), 1.0);
        // El signo sigue al dividendo, como en JavaScript.
        assert_eq!(next_variant_offset(1.0, 2.0, 0.0), -3.0);
    }

    #[test]
    fn powerline_ranges() {
        assert!(is_powerline('\u{E0B0}') && is_restricted_powerline('\u{E0B0}'));
        assert!(is_powerline('\u{E0BC}') && !is_restricted_powerline('\u{E0BC}'));
        assert!(!is_powerline('\u{2500}'));
    }

    #[test]
    fn shelf_packs_rows_and_reports_full_pages() {
        let mut s = Shelf::new(10);
        assert_eq!(s.place(6, 3), Some((0, 0)));
        assert_eq!(s.place(4, 2), Some((6, 0)));
        assert_eq!(s.place(5, 4), Some((0, 3)));
        assert_eq!(s.place(5, 4), Some((5, 3)));
        assert_eq!(s.place(10, 4), None);
        assert_eq!(s.place(11, 1), None);
    }

    #[test]
    fn zalgo_and_long_combining_text_keep_the_scratch_canvas_bounded() {
        // Celda de 14 px a dpr 3 (25 × 61): un carácter con 1000 marcas.
        let zalgo: String = std::iter::once('Z')
            .chain(std::iter::repeat_n('\u{0336}', 1000))
            .collect();
        let (w, h) = tmp_size(25, 61, zalgo.encode_utf16().count());
        assert_eq!((w, h), (TEXTURE_SIZE, 69));
        // Un glifo normal vuelve al tamaño de partida (4·cellW + 4).
        assert_eq!(tmp_size(25, 61, 1), (104, 69));
        // Celdas enormes (fuente de 512 px a dpr 3): con tope absoluto.
        let (w, h) = tmp_size(u32::MAX / 2, u32::MAX / 2, 3);
        assert_eq!((w, h), (TMP_MAX, TMP_MAX));
    }

    #[test]
    fn bounded_cache_keeps_at_most_cap_entries_fifo() {
        let mut cache: Bounded<[u8; 3], u32> = Bounded::new(MAX_PATTERNS);
        // Muchos colores de 24 bits distintos.
        for i in 0..10_000_u32 {
            let [_, r, g, b] = i.to_be_bytes();
            cache.insert([r, g, b], i);
            assert!(cache.len() <= MAX_PATTERNS);
        }
        assert_eq!(cache.len(), MAX_PATTERNS);
        // Quedan los últimos; los primeros se fueron.
        let [_, r, g, b] = 9_999_u32.to_be_bytes();
        assert_eq!(cache.get(&[r, g, b]), Some(&9_999));
        assert_eq!(cache.get(&[0, 0, 0]), None);
        // Reinsertar una clave no duplica su orden.
        cache.insert([r, g, b], 1);
        assert_eq!(cache.len(), MAX_PATTERNS);
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn baseline_per_browser() {
        assert_eq!(text_baseline("Mozilla/5.0 (X11) Firefox/131.0"), "bottom");
        assert_eq!(
            text_baseline("Mozilla/5.0 (Macintosh) Chrome/129.0 Safari/537.36"),
            "ideographic"
        );
    }

    #[test]
    fn underline_codes_match_xterm() {
        assert_eq!(underline_code(Underline::Single), 1);
        assert_eq!(underline_code(Underline::Dashed), 5);
    }
}
