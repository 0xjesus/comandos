//! Aritmética de celdas de `@xterm/addon-canvas` 0.7.0 (`_updateDimensions`)
//! y del ajuste de `@xterm/addon-fit` 0.10, más la medición en el navegador
//! (estrategia de xterm.js 5.5: `measureText("W")` de un canvas y, si el
//! navegador no da las métricas de la fuente, un `<span>` con 32 «W»).
//!
//! Todo lo de este módulo salvo [`measure`] y [`scrollbar_width`] es
//! aritmética pura y se prueba en host.
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, Document, HtmlCanvasElement, HtmlElement};

/// Medidas de una celda en píxeles CSS y de dispositivo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellMetrics {
    /// Ancho de la celda en píxeles CSS (`dev_w / dpr`).
    pub css_w: f64,
    /// Alto de la celda en píxeles CSS (`dev_h / dpr`).
    pub css_h: f64,
    /// `device.cell.width`: ancho del carácter más el espaciado.
    pub dev_w: u32,
    /// `device.cell.height`: alto del carácter por el interlineado.
    pub dev_h: u32,
    /// `device.char.width`.
    pub dev_char_w: u32,
    /// `device.char.height`.
    pub dev_char_h: u32,
    /// `device.char.top`: hueco del interlineado encima del carácter.
    pub char_top: u32,
    /// `device.char.left`: mitad del espaciado entre letras.
    pub char_left: u32,
    pub dpr: f64,
}

/// `Math.round` de JavaScript: los empates van hacia arriba.
pub fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// Un valor no negativo y finito como `u32` (0 si no lo es).
fn to_u32(v: f64) -> u32 {
    if v.is_finite() {
        // Acotado antes de convertir: la conversión satura y no entra en pánico.
        v.clamp(0.0, f64::from(u32::MAX)) as u32
    } else {
        0
    }
}

/// Celda a partir del carácter medido en píxeles CSS, como
/// `CanvasRenderer._updateDimensions`:
///
/// - `char w = floor(w·dpr)`, `char h = ceil(h·dpr)`;
/// - `cell h = floor(char h · lineHeight)`;
/// - `char top = lineHeight == 1 ? 0 : round((cell h − char h)/2)`;
/// - `cell w = char w + round(letterSpacing)`, `char left = floor(letterSpacing/2)`;
/// - CSS = dispositivo / dpr.
pub fn from_measure(
    char_w_css: f64,
    char_h_css: f64,
    dpr: f64,
    line_height: f64,
    letter_spacing: f64,
) -> CellMetrics {
    let dpr = if dpr.is_finite() && dpr > 0.0 {
        dpr
    } else {
        1.0
    };
    let dev_char_w = to_u32((char_w_css * dpr).floor());
    let dev_char_h = to_u32((char_h_css * dpr).ceil());
    let dev_h = to_u32((f64::from(dev_char_h) * line_height).floor());
    let char_top = if line_height == 1.0 {
        0
    } else {
        to_u32(js_round((f64::from(dev_h) - f64::from(dev_char_h)) / 2.0))
    };
    let dev_w = to_u32(f64::from(dev_char_w) + js_round(letter_spacing));
    let char_left = to_u32((letter_spacing / 2.0).floor());
    CellMetrics {
        css_w: f64::from(dev_w) / dpr,
        css_h: f64::from(dev_h) / dpr,
        dev_w,
        dev_h,
        dev_char_w,
        dev_char_h,
        char_top,
        char_left,
        dpr,
    }
}

impl CellMetrics {
    /// Tamaño del almacén del canvas (`device.canvas`) para la rejilla.
    pub fn device_canvas(&self, cols: u16, rows: u16) -> (u32, u32) {
        (
            self.dev_w.saturating_mul(u32::from(cols)),
            self.dev_h.saturating_mul(u32::from(rows)),
        )
    }

    /// Tamaño CSS del canvas (`css.canvas = round(device / dpr)`).
    pub fn css_canvas(&self, cols: u16, rows: u16) -> (f64, f64) {
        let (w, h) = self.device_canvas(cols, rows);
        (
            js_round(f64::from(w) / self.dpr),
            js_round(f64::from(h) / self.dpr),
        )
    }

    /// `dimensions.css.cell` de addon-canvas para la rejilla actual:
    /// `css.canvas / (cols, rows)`. Es lo que usan FitAddon y el ratón; no
    /// es `dev / dpr` cuando el redondeo del canvas CSS reparte un píxel.
    /// `None` sin tamaño de celda o de rejilla.
    pub fn css_cell(&self, cols: u16, rows: u16) -> Option<(f64, f64)> {
        if !self.is_valid() || cols == 0 || rows == 0 {
            return None;
        }
        let (w, h) = self.css_canvas(cols, rows);
        let cell = (w / f64::from(cols), h / f64::from(rows));
        (cell.0 > 0.0 && cell.1 > 0.0).then_some(cell)
    }

    /// La celda tiene tamaño (la medición dio algo).
    pub fn is_valid(&self) -> bool {
        self.dev_w > 0 && self.dev_h > 0
    }
}

/// `FitAddon.proposeDimensions` sobre las dimensiones de addon-canvas:
/// `cols = max(2, floor((w − barra) / css.cell.width))`,
/// `rows = max(1, floor(h / css.cell.height))`, con la celda CSS de la
/// rejilla **actual** (`round(cols·dev_w/dpr) / cols`, ver
/// [`CellMetrics::css_cell`]). Por eso el resultado depende del tamaño de
/// partida, como en xterm.js (a dpr fraccionario o 3 un mismo contenedor
/// puede alternar entre dos anchos). Recibe el ancho y alto del padre ya
/// truncados (`parseInt`). `None` si la celda no tiene tamaño: FitAddon no
/// toca la terminal entonces.
pub fn fit(
    parent_w: f64,
    parent_h: f64,
    scrollbar_w: f64,
    m: &CellMetrics,
    current: (u16, u16),
) -> Option<(u16, u16)> {
    // `fit()` no hace nada si alguna cuenta da NaN (`parseInt("auto")`).
    if parent_w.is_nan() || parent_h.is_nan() {
        return None;
    }
    let (cell_w, cell_h) = m.css_cell(current.0, current.1)?;
    let count = |avail: f64, cell: f64, min: u16| -> u16 {
        let n = (avail / cell).floor();
        if n.is_finite() {
            // `clamp` deja el valor en el rango de u16 antes de convertir.
            (crate::number_text::clamp(n, f64::from(min), f64::from(u16::MAX)) as u16).max(min)
        } else {
            min
        }
    };
    Some((
        count(parent_w.max(0.0) - scrollbar_w, cell_w, 2),
        count(parent_h, cell_h, 1),
    ))
}

/// `parseInt` de una longitud CSS como `"783.5px"` (el que usa FitAddon);
/// `NaN` si no empieza por un número (`"auto"`), como en JS.
pub fn parse_css_int(value: &str) -> f64 {
    let trimmed = value.trim_start();
    let (sign, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    match digits.get(..end).and_then(crate::number_text::parse) {
        Some(n) => sign * n,
        None => f64::NAN,
    }
}

/// Tamaño en píxeles CSS del carácter «W» con `family` a `size` px, como
/// `CharSizeService` de xterm.js 5.5: `measureText` de un canvas
/// (`width`, `fontBoundingBoxAscent + fontBoundingBoxDescent`) y, si el
/// navegador no da esas métricas, un `<span>` oculto con 32 «W» dentro de
/// `container` (un `.xterm`, para que apliquen los mismos estilos).
pub fn measure(
    document: &Document,
    container: &HtmlElement,
    family: &str,
    size: f64,
) -> Result<(f64, f64), JsValue> {
    if let Some(found) = measure_canvas(document, family, size)? {
        return Ok(found);
    }
    let span: HtmlElement = document.create_element("span")?.dyn_into()?;
    span.set_class_name("xterm-char-measure-element");
    span.set_text_content(Some(&"W".repeat(32)));
    span.set_attribute("aria-hidden", "true")?;
    let style = span.style();
    style.set_property("white-space", "pre")?;
    style.set_property("font-kerning", "none")?;
    style.set_property("font-family", family)?;
    style.set_property("font-size", &crate::number_text::px(size))?;
    container.append_child(&span)?;
    let w = f64::from(span.offset_width()) / 32.0;
    let h = f64::from(span.offset_height());
    span.remove();
    Ok((w, h))
}

fn measure_canvas(
    document: &Document,
    family: &str,
    size: f64,
) -> Result<Option<(f64, f64)>, JsValue> {
    let canvas: HtmlCanvasElement = document.create_element("canvas")?.dyn_into()?;
    canvas.set_width(100);
    canvas.set_height(100);
    let Some(ctx) = canvas.get_context("2d")? else {
        return Ok(None);
    };
    let ctx: CanvasRenderingContext2d = ctx.dyn_into()?;
    ctx.set_font(&format!("{} {family}", crate::number_text::px(size)));
    let metrics = ctx.measure_text("W")?;
    // Sin `fontBoundingBox*` (navegadores viejos) xterm.js usa el DOM.
    if !js_sys::Reflect::has(&metrics, &JsValue::from_str("fontBoundingBoxAscent"))? {
        return Ok(None);
    }
    let w = metrics.width();
    let h = metrics.font_bounding_box_ascent() + metrics.font_bounding_box_descent();
    Ok((w > 0.0 && h > 0.0).then_some((w, h)))
}

/// Ancho de la barra de desplazamiento como `Viewport` de xterm.js 5.5:
/// `viewport.offsetWidth − scrollArea.offsetWidth`, o 15 si da 0.
pub fn scrollbar_width(viewport: &HtmlElement, scroll_area: &HtmlElement) -> f64 {
    let w = viewport.offset_width() - scroll_area.offset_width();
    if w == 0 { 15.0 } else { f64::from(w) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_addon_canvas_arithmetic_at_dpr_1_and_2() {
        // Ubuntu Sans Mono 11 px: char 6.6 × 13 css (medidas de ejemplo).
        let m1 = from_measure(6.6, 13.0, 1.0, 1.2, 0.0);
        assert_eq!(
            (m1.dev_w, m1.dev_char_h, m1.dev_h, m1.char_top),
            (6, 13, 15, 1)
        );
        let m2 = from_measure(6.6, 13.0, 2.0, 1.2, 0.0);
        assert_eq!((m2.dev_w, m2.dev_char_h, m2.dev_h), (13, 26, 31));
        assert!((m2.css_h - 15.5).abs() < 1e-9);
    }
    #[test]
    fn fit_mirrors_fitaddon() {
        let m = from_measure(6.6, 13.0, 1.0, 1.2, 0.0);
        assert_eq!(fit(800.0, 450.0, 10.0, &m, (80, 24)), Some((131, 30)));
        assert_eq!(fit(5.0, 5.0, 0.0, &m, (80, 24)), Some((2, 1)));
        // `parseInt("auto")` es NaN y FitAddon no propone nada.
        assert_eq!(fit(f64::NAN, 450.0, 10.0, &m, (80, 24)), None);
        assert_eq!(fit(800.0, f64::NAN, 10.0, &m, (80, 24)), None);
    }

    #[test]
    fn line_height_one_and_letter_spacing() {
        let m = from_measure(8.4, 17.0, 1.25, 1.0, 3.0);
        // floor(10.5) = 10; ceil(21.25) = 22; cell h = 22; top = 0.
        assert_eq!(
            (m.dev_char_w, m.dev_char_h, m.dev_h, m.char_top),
            (10, 22, 22, 0)
        );
        // cell w = 10 + round(3) = 13; left = floor(1.5) = 1.
        assert_eq!((m.dev_w, m.char_left), (13, 1));
        assert!((m.css_w - 10.4).abs() < 1e-9);
        // char top con empate .5 redondea hacia arriba como Math.round.
        let tie = from_measure(6.0, 13.0, 1.0, 1.1, 0.0);
        assert_eq!((tie.dev_h, tie.char_top), (14, 1));
    }

    #[test]
    fn css_canvas_rounds_like_xterm() {
        let m = from_measure(6.6, 13.0, 2.0, 1.2, 0.0);
        assert_eq!(m.device_canvas(81, 3), (1053, 93));
        // 1053 / 2 = 526.5 → 527; 93 / 2 = 46.5 → 47.
        assert_eq!(m.css_canvas(81, 3), (527.0, 47.0));
        let (cw, ch) = m.css_cell(81, 3).unwrap_or_default();
        assert!((cw - 527.0 / 81.0).abs() < 1e-12 && (ch - 47.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn degenerate_measures_do_not_panic() {
        let m = from_measure(f64::NAN, -3.0, 0.0, 1.2, f64::INFINITY);
        assert!(!m.is_valid());
        // Sin celda FitAddon no propone nada (no se encoge a 2×1).
        assert_eq!(fit(800.0, 450.0, 0.0, &m, (80, 24)), None);
        assert_eq!(
            fit(
                800.0,
                450.0,
                0.0,
                &from_measure(6.0, 13.0, 1.0, 1.0, 0.0),
                (0, 24)
            ),
            None
        );
        let tiny = from_measure(1.0, 1.0, 1.0, 1.0, 0.0);
        // Un padre enorme satura en el tope de u16.
        assert_eq!(
            fit(1e12, 1e12, 0.0, &tiny, (80, 24)),
            Some((u16::MAX, u16::MAX))
        );
        assert_eq!(fit(-50.0, -50.0, 10.0, &tiny, (80, 24)), Some((2, 1)));
    }

    #[test]
    fn parses_css_lengths_like_parse_int() {
        assert_eq!(parse_css_int("783.5px"), 783.0);
        assert_eq!(parse_css_int(" 12px"), 12.0);
        assert_eq!(parse_css_int("-4px"), -4.0);
        assert!(parse_css_int("auto").is_nan());
        assert!(parse_css_int("").is_nan());
        assert!(parse_css_int("-px").is_nan());
    }
}

thread_local! {
    /// Glyphs rasterized and their time, read by `stats()` (diagnostics).
    static RASTER: std::cell::Cell<(u32, f64)> = const { std::cell::Cell::new((0, 0.0)) };
}

/// `performance.now()`, or 0 where there is none.
pub fn perf_now() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

pub fn note_raster(ms: f64) {
    RASTER.with(|r| {
        let (n, t) = r.get();
        r.set((n.saturating_add(1), t + ms));
    });
}

/// Glyphs rasterized so far and the milliseconds they took.
pub fn raster_stats() -> (u32, f64) {
    RASTER.with(std::cell::Cell::get)
}
