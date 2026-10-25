//! `comandos-term-web`: la terminal de ComandOS en el navegador. Sustituye a
//! xterm.js 5.5.0 + `@xterm/addon-canvas` 0.7.0 + `@xterm/addon-fit` 0.10 en
//! `dash/term.html` (la página la monta A9).
//!
//! - [`metrics`]: aritmética de celdas y de ajuste (pura) y medición.
//! - [`paint`]: qué filas repintar y cuándo (puro, con [`paint::Painter`]).
//! - [`canvas`]: el `<canvas>` que pinta como addon-canvas.
//! - [`theme`]: el tema con los nombres de xterm.js.
//! - [`WebTerm`]: une motor, planificador y canvas para JavaScript.
//!
//! ## Ligero
//!
//! Sin daños no se pide ningún cuadro; como mucho hay uno en vuelo. Un
//! temporizador único despierta para el parpadeo del cursor (solo con foco,
//! como xterm.js) o para vencer una actualización sincronizada. Los
//! búferes, cadenas de color y glifos se reutilizan entre cuadros.
//!
//! ## Entrada, selección y enlaces
//!
//! [`keyboard`] (el `<textarea>` oculto, IME, GBoard, pegado), [`mouse`]
//! (informes de ratón, rueda, barra de desplazamiento, dedo y selección) y
//! [`links`] (enlaces bajo el ratón) traducen los eventos del DOM con las
//! funciones puras de `comandos_term::{input, select}`; [`overlay`] pinta
//! la selección, el subrayado del enlace y las ligaduras en capas encima del
//! texto. Los bytes para el PTY salen por la función de
//! [`WebTerm::set_on_data`] (un `Uint8Array`: el ratón X10 no es UTF-8),
//! siempre después de soltar el estado (la página puede volver a llamar a la
//! terminal desde ella).
//!
//! **OSC 52** (la aplicación pide copiar al portapapeles; Claude Code en
//! pantalla completa copia así su propia selección) se escribe con
//! `navigator.clipboard.writeText`. Si el navegador lo rechaza por no haber
//! gesto, [`WebTerm::get_selection`] lo devuelve durante dos minutos para que
//! el botón «Copiar» lo escriba dentro del toque.
pub mod canvas;
pub mod connection;
pub mod controls;
pub mod keyboard;
mod link_menu;
pub mod links;
pub mod metrics;
pub mod mouse;
mod number_text;
pub mod overlay;
#[cfg(target_arch = "wasm32")]
pub mod page;
pub mod page_theme;
pub mod paint;
pub mod pane_chrome;
pub mod parent;
pub mod theme;
mod word_links;

use canvas::{Canvas2d, CanvasTheme};
use comandos_term::select::{Selection, UrlSpan, selected_text, selected_text_within};
use comandos_term::{
    engine::{ClipboardTarget, Engine, GridSize, MouseMode, Palette},
    render::{CursorShape, RenderOpts},
};
use metrics::CellMetrics;
use paint::{Blink, CursorInput, Painter, Scheduler};
use std::rc::Weak;
use std::{cell::RefCell, rc::Rc};
use theme::{ANSI_NAMES, Theme};
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
use web_sys::{
    AddEventListenerOptions, Document, Event, EventTarget, FontFace, HtmlElement,
    HtmlTextAreaElement, Performance, ResizeObserver, ResizeObserverBoxOptions,
    ResizeObserverOptions, Window,
};

#[wasm_bindgen]
extern "C" {
    /// `promise.then(ok, err)` con funciones ya creadas.
    type PromiseThen;
    #[wasm_bindgen(method, js_name = then)]
    fn then2(this: &PromiseThen, ok: &JsValue, err: &JsValue);

    /// `observer.observe(target, options)` capturando la excepción de los
    /// navegadores sin `device-pixel-content-box`.
    type ObserveCatch;
    #[wasm_bindgen(method, catch, js_name = observe)]
    fn observe_catch(this: &ObserveCatch, target: &JsValue, opts: &JsValue) -> Result<(), JsValue>;
}

/// Opciones de `new Terminal({...})` que usa la terminal (las de xterm.js
/// 5.5 por omisión).
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub font_family: String,
    pub font_size: f64,
    pub line_height: f64,
    pub letter_spacing: f64,
    pub cursor_blink: bool,
    pub cursor_style: Option<CursorShape>,
    pub scrollback: usize,
    pub draw_bold_text_in_bright_colors: bool,
    pub minimum_contrast_ratio: f32,
    pub theme: Theme,
    /// Ligaduras de escritorio; `None` = como `term.html` (sin táctil).
    pub ligatures: Option<bool>,
    /// Fuente de las ligaduras (relativa a la página).
    pub ligature_font_url: String,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            font_family: "courier-new, courier, monospace".to_string(),
            font_size: 15.0,
            line_height: 1.0,
            letter_spacing: 0.0,
            cursor_blink: false,
            cursor_style: None,
            scrollback: 1000,
            draw_bold_text_in_bright_colors: true,
            minimum_contrast_ratio: 1.0,
            theme: Theme::default(),
            ligatures: None,
            ligature_font_url: overlay::LIGA_FONT_URL.to_string(),
        }
    }
}

fn get(obj: &JsValue, key: &str) -> Option<JsValue> {
    if !obj.is_object() {
        return None;
    }
    js_sys::Reflect::get(obj, &JsValue::from_str(key))
        .ok()
        .filter(|v| !v.is_undefined() && !v.is_null())
}

fn get_f64(obj: &JsValue, key: &str) -> Option<f64> {
    get(obj, key)
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
}

/// Lee un objeto `ITheme` de xterm.js.
pub fn read_theme(obj: &JsValue) -> Theme {
    let color = |key: &str| {
        get(obj, key)
            .and_then(|v| v.as_string())
            .and_then(|s| theme::parse_color(&s))
    };
    let mut theme = Theme {
        foreground: color("foreground"),
        background: color("background"),
        cursor: color("cursor"),
        cursor_accent: color("cursorAccent"),
        selection: color("selectionBackground"),
        ..Theme::default()
    };
    for (slot, name) in theme.ansi.iter_mut().zip(ANSI_NAMES) {
        *slot = color(name);
    }
    theme
}

/// Tamaño de letra aceptable: finito y positivo, acotado a 1–512 px (uno
/// absurdo desbordaría las cuentas de los canvases).
pub fn font_size_in_range(size: f64) -> Option<f64> {
    (size.is_finite() && size > 0.0).then(|| size.clamp(1.0, 512.0))
}

/// Acepta el nombre de preferencias y el de xterm.
fn cursor_style(style: &str) -> Option<CursorShape> {
    match style {
        "bar" | "ibeam" => Some(CursorShape::Beam),
        "block" => Some(CursorShape::Block),
        "underline" => Some(CursorShape::Underline),
        _ => None,
    }
}

/// Lee las opciones de `new Terminal({...})`.
pub fn read_options(obj: &JsValue) -> Options {
    let d = Options::default();
    Options {
        font_family: get(obj, "fontFamily")
            .and_then(|v| v.as_string())
            .unwrap_or(d.font_family),
        font_size: get_f64(obj, "fontSize")
            .and_then(font_size_in_range)
            .unwrap_or(d.font_size),
        // xterm.js exige `lineHeight ≥ 1`; el tope evita celdas absurdas.
        line_height: get_f64(obj, "lineHeight")
            .filter(|v| *v >= 1.0)
            .map(|v| v.min(10.0))
            .unwrap_or(d.line_height),
        letter_spacing: get_f64(obj, "letterSpacing")
            .map(|v| v.clamp(-100.0, 100.0))
            .unwrap_or(d.letter_spacing),
        cursor_style: get(obj, "cursorStyle")
            .and_then(|v| v.as_string())
            .and_then(|s| cursor_style(&s)),
        cursor_blink: get(obj, "cursorBlink")
            .and_then(|v| v.as_bool())
            .unwrap_or(d.cursor_blink),
        scrollback: get_f64(obj, "scrollback")
            .filter(|v| *v >= 0.0)
            // Acotado antes de convertir: la conversión satura.
            .map(|v| v.min(1e7) as usize)
            .unwrap_or(d.scrollback),
        draw_bold_text_in_bright_colors: get(obj, "drawBoldTextInBrightColors")
            .and_then(|v| v.as_bool())
            .unwrap_or(d.draw_bold_text_in_bright_colors),
        minimum_contrast_ratio: get_f64(obj, "minimumContrastRatio")
            .map(|v| v.clamp(1.0, 21.0) as f32)
            .unwrap_or(d.minimum_contrast_ratio),
        theme: get(obj, "theme")
            .map(|t| read_theme(&t))
            .unwrap_or_default(),
        ligatures: get(obj, "ligatures").and_then(|v| v.as_bool()),
        ligature_font_url: get(obj, "ligatureFontUrl")
            .and_then(|v| v.as_string())
            .unwrap_or(d.ligature_font_url),
    }
}

/// Color de la capa de selección: `selectionBackground` opaco al 30 %, o
/// blanco al 30 % sin él (`ThemeService`).
pub fn selection_css(theme: &Theme) -> String {
    let [r, g, b] = theme.selection.unwrap_or([255, 255, 255]);
    format!("rgba({r}, {g}, {b}, 0.3)")
}

/// `selectionBackgroundOpaque` de `ThemeService`: el color del tema tal cual,
/// o blanco con alfa `0x4D` mezclado con el fondo. Los glifos seleccionados
/// se rasterizan contra él.
pub fn selection_opaque(theme: &Theme) -> [u8; 3] {
    if let Some(color) = theme.selection {
        return color;
    }
    let bg = theme.background.unwrap_or(theme::DEFAULT_BG);
    let mix = |c: u8| {
        let c = f64::from(c);
        (c + metrics::js_round((255.0 - c) * 77.0 / 255.0)).clamp(0.0, 255.0) as u8
    };
    [mix(bg[0]), mix(bg[1]), mix(bg[2])]
}

fn canvas_theme(p: &Palette) -> CanvasTheme {
    CanvasTheme {
        bg: p.bg,
        cursor: p.cursor,
        cursor_accent: p.cursor_accent,
    }
}

/// Elementos del DOM, con las clases de xterm.js para que el CSS de
/// `xterm.css` y de `term.html` (barra de desplazamiento) aplique igual.
struct Dom {
    root: HtmlElement,
    viewport: HtmlElement,
    scroll_area: HtmlElement,
    screen: HtmlElement,
    /// `<textarea>` oculto que recibe el teclado (`.xterm-helper-textarea`).
    textarea: HtmlTextAreaElement,
    /// Texto en composición del IME (`.composition-view`).
    composition: HtmlElement,
}

/// Estilo propio de un elemento (la página nueva no carga `xterm.css`).
fn set_styles(el: &HtmlElement, styles: &[(&str, &str)]) {
    let style = el.style();
    for (name, value) in styles {
        let _ = style.set_property(name, value);
    }
}

impl Dom {
    fn new(document: &Document, host: &HtmlElement) -> Result<Dom, JsValue> {
        let div = |class: &str| -> Result<HtmlElement, JsValue> {
            let el: HtmlElement = document.create_element("div")?.dyn_into()?;
            el.set_class_name(class);
            Ok(el)
        };
        let root = div("terminal xterm")?;
        root.set_dir("ltr");
        let viewport = div("xterm-viewport")?;
        let scroll_area = div("xterm-scroll-area")?;
        let screen = div("xterm-screen")?;
        // The page no longer loads xterm.css. Keep history out of document flow:
        // otherwise every scroll-area update shifts the canvas under the pointer.
        set_styles(&root, &[("position", "relative"), ("height", "100%")]);
        set_styles(
            &viewport,
            &[
                ("position", "absolute"),
                ("inset", "0"),
                ("overflow-y", "scroll"),
            ],
        );
        set_styles(&scroll_area, &[("visibility", "hidden")]);
        // Las capas (`overlay`) se colocan encima del canvas del texto.
        set_styles(&screen, &[("position", "relative")]);
        // `.xterm-helpers` con el `<textarea>` y la vista de composición,
        // como xterm.js (mismos atributos y estilos que `xterm.css`).
        let helpers = div("xterm-helpers")?;
        set_styles(
            &helpers,
            &[("position", "absolute"), ("top", "0"), ("z-index", "5")],
        );
        let textarea: HtmlTextAreaElement = document.create_element("textarea")?.dyn_into()?;
        textarea.set_class_name("xterm-helper-textarea");
        for (name, value) in [
            ("aria-label", "Entrada de la terminal"),
            ("aria-multiline", "false"),
            ("autocorrect", "off"),
            ("autocapitalize", "off"),
            ("spellcheck", "false"),
        ] {
            textarea.set_attribute(name, value)?;
        }
        textarea.set_tab_index(0);
        set_styles(
            &textarea,
            &[
                ("padding", "0"),
                ("border", "0"),
                ("margin", "0"),
                ("position", "absolute"),
                ("opacity", "0"),
                ("left", "-9999em"),
                ("top", "0"),
                ("width", "0"),
                ("height", "0"),
                ("z-index", "-5"),
                ("white-space", "nowrap"),
                ("overflow", "hidden"),
                ("resize", "none"),
            ],
        );
        let composition = div("composition-view")?;
        set_styles(
            &composition,
            &[
                ("background", "#000"),
                ("color", "#FFF"),
                ("display", "none"),
                ("position", "absolute"),
                ("white-space", "nowrap"),
                ("z-index", "1"),
            ],
        );
        helpers.append_child(&textarea)?;
        helpers.append_child(&composition)?;
        viewport.append_child(&scroll_area)?;
        root.append_child(&viewport)?;
        root.append_child(&screen)?;
        root.append_child(&helpers)?;
        host.append_child(&root)?;
        Ok(Dom {
            root,
            viewport,
            scroll_area,
            screen,
            textarea,
            composition,
        })
    }
}

/// Lo último que se escribió en la barra (`Viewport._innerRefresh`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct ScrollSync {
    /// `viewport.offsetHeight`, leído una vez por cambio de tamaño (leerlo
    /// fuerza un cálculo de estilo y diseño).
    viewport_h: Option<f64>,
    buffer_len: usize,
    css_canvas_h: f64,
    dev_cell_h: u32,
    scroll_top: f64,
}

struct Inner {
    window: Window,
    document: Document,
    perf: Option<Performance>,
    host: HtmlElement,
    dom: Dom,
    engine: Engine,
    palette: Palette,
    render_opts: RenderOpts,
    scheduler: Scheduler,
    painter: Canvas2d,
    metrics: CellMetrics,
    opts: Options,
    size: GridSize,
    scrollbar_w: f64,
    focused: bool,
    blink: Blink,
    /// Fase visible del parpadeo, fijada por el temporizador.
    blink_on: bool,
    frame_cb: Option<Closure<dyn FnMut(f64)>>,
    frame_id: Option<i32>,
    timer_cb: Option<Closure<dyn FnMut()>>,
    /// `devicePixelContentBox` del canvas (si el navegador lo da).
    observer: Option<ResizeObserver>,
    observer_cb: Option<Closure<dyn FnMut(js_sys::Array)>>,
    /// `contextrestored` del canvas (tras perderse la GPU).
    restored_cb: Option<Closure<dyn FnMut()>>,
    /// Texto reutilizado para la altura del área de desplazamiento.
    css_buf: String,
    /// Temporizador armado y para cuándo (ms de `performance.now()`).
    timer: Option<(i32, f64)>,
    replies: Vec<u8>,
    title: Option<String>,
    bell: bool,
    clipboard: Option<(ClipboardTarget, String)>,
    /// Last OSC 52 text and when it arrived, so "Copiar" can retry it inside a tap
    /// when the browser refused the automatic write (Safari outside a gesture).
    app_copied: Option<(f64, String)>,
    scroll_sync: ScrollSync,
    // --- entrada, selección y enlaces (A7/A8) ---
    /// Lo que sale de un evento; se entrega al soltar el estado.
    outbox: Outbox,
    on_data: Option<js_sys::Function>,
    on_selection: Option<js_sys::Function>,
    /// Escuchas permanentes del DOM (se quitan al soltar la terminal).
    listeners: Vec<Listener>,
    keys: keyboard::Keys,
    /// Composición del IME en curso (el `<textarea>` no se mueve).
    composing: bool,
    /// Celda y tamaño CSS donde quedó el `<textarea>` (`_syncTextArea`).
    textarea_at: Option<(u16, usize, u32, u32)>,
    reporter: mouse::MouseReporter,
    wheel: mouse::Wheel,
    select: mouse::SelectModel,
    /// Selección comunicada a la página por última vez.
    reported_selection: Option<Selection>,
    /// Arrastre en curso (con sus escuchas en `document`).
    drag: Option<Drag>,
    /// El próximo `scroll` de la barra lo provocó `sync_scroll_area`.
    ignore_scroll: bool,
    /// Último punto del dedo (desplazamiento táctil de xterm.js).
    touch_y: Option<f64>,
    touch_link: Option<(UrlSpan, f64, f64)>,
    word_press: Option<word_links::Request>,
    word_generation: u64,
    links: links::LinkCache,
    /// Sube con cada cambio de la rejilla (caché de enlaces).
    link_gen: u64,
    /// Enlace bajo el ratón.
    hover: Option<UrlSpan>,
    /// Última posición del ratón sobre la pantalla (para volver a buscar el
    /// enlace cuando se repintan sus filas, como `Linkifier`).
    hover_xy: Option<(f64, f64)>,
    /// Enlace pulsado (se abre al soltar sobre él).
    link_down: Option<UrlSpan>,
    /// `timeStamp` del último `mousedown` (Alt+clic).
    down_ts: f64,
    /// Protocolo de ratón visto por última vez (`onProtocolChange`).
    mouse_seen: MouseMode,
    overlay: overlay::Overlay,
    /// Color de la selección (`rgba`), según el tema.
    selection_color: String,
    ligatures: Option<overlay::Ligatures>,
    liga_face: Option<FontFace>,
    liga_cb: Option<Closure<dyn FnMut()>>,
    /// Toca repintar las ligaduras en el próximo cuadro.
    liga_due: bool,
    /// `Engine::scrolled_up` ya aplicado a la selección.
    scrolled_seen: u64,
    alt_seen: bool,
    /// `navigator.platform` contiene `Linux` (`isLinux` de xterm.js).
    is_linux: bool,
    /// El propio estado, para las escuchas que se instalan más tarde.
    weak: Weak<RefCell<Inner>>,
    /// Grid imposed by the page instead of fitting the host (the whole tmux window
    /// while a phone shows one pane of it).
    fixed: Option<GridSize>,
    /// Columns `left..right` that a copy keeps (the pane the phone shows).
    clip_cols: Option<(usize, usize)>,
}

/// Qué hace el `<textarea>` cuando el estado ya está libre (dar el foco
/// dispara `focus` en el acto, y ese evento también necesita el estado).
#[derive(Debug)]
enum Focus {
    /// `focus({preventScroll: true})`.
    Plain,
    /// Foco, el texto dentro y seleccionado (menú contextual y selección
    /// primaria de Linux, como xterm.js).
    Select(String),
}

/// Arrastre con el botón pulsado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragKind {
    /// Informes de ratón para la aplicación.
    Report,
    /// Selección.
    Select,
}

struct Drag {
    kind: DragKind,
    /// Líneas por paso de 50 ms con el ratón fuera de la terminal.
    scroll: i32,
    window: Window,
    interval: Option<i32>,
    _interval_cb: Option<Closure<dyn FnMut()>>,
    _listeners: Vec<Listener>,
}

impl Drop for Drag {
    fn drop(&mut self) {
        if let Some(id) = self.interval.take() {
            self.window.clear_interval_with_handle(id);
        }
    }
}

/// Lo que produce un evento y se entrega cuando el estado ya está libre.
#[derive(Debug, Default)]
struct Outbox {
    data: Vec<Vec<u8>>,
    focus: Option<Focus>,
    copy: Option<String>,
    /// Text an agent copied with OSC 52 (Claude Code full screen copies its own
    /// selections this way); written with the async Clipboard API only.
    app_copy: Option<String>,
    open: Option<(String, f64, f64)>,
    word: Option<(Weak<RefCell<Inner>>, word_links::Request)>,
}

impl Outbox {
    fn is_empty(&self) -> bool {
        self.data.is_empty()
            && self.focus.is_none()
            && self.copy.is_none()
            && self.app_copy.is_none()
            && self.open.is_none()
            && self.word.is_none()
    }
}

/// Destino, evento, `passive` y manejador de una escucha.
type Handler<'a> = (
    &'a EventTarget,
    &'static str,
    Option<bool>,
    fn(&mut Inner, &Event),
);

/// Una escucha del DOM que se quita al soltarla.
struct Listener {
    target: EventTarget,
    kind: &'static str,
    capture: bool,
    cb: Closure<dyn FnMut(Event)>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.target.remove_event_listener_with_callback_and_bool(
            self.kind,
            self.cb.as_ref().unchecked_ref(),
            self.capture,
        );
    }
}

/// Instala `handler` para `kind` en `target`; `passive: Some(false)` para
/// poder cancelar la rueda y el dedo.
fn listen(
    weak: &Weak<RefCell<Inner>>,
    target: &EventTarget,
    kind: &'static str,
    passive: Option<bool>,
    handler: fn(&mut Inner, &Event),
) -> Option<Listener> {
    let weak = weak.clone();
    let cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
        dispatch(&weak, |i| handler(i, &e));
    });
    let opts = AddEventListenerOptions::new();
    if let Some(p) = passive {
        opts.set_passive(p);
    }
    target
        .add_event_listener_with_callback_and_add_event_listener_options(
            kind,
            cb.as_ref().unchecked_ref(),
            &opts,
        )
        .ok()?;
    Some(Listener {
        target: target.clone(),
        kind,
        capture: false,
        cb,
    })
}

/// Ejecuta `f` sobre el estado y después, ya suelto, entrega lo que dejó:
/// bytes a `onData`, aviso de selección, copia y enlace que abrir. Así la
/// página puede volver a llamar a la terminal desde sus funciones.
fn dispatch<T>(weak: &Weak<RefCell<Inner>>, f: impl FnOnce(&mut Inner) -> T) -> Option<T> {
    let rc = weak.upgrade()?;
    let (out, delivery) = {
        let mut i = rc.try_borrow_mut().ok()?;
        let out = f(&mut i);
        (out, i.take_delivery())
    };
    if let Some(d) = delivery {
        d.deliver();
    }
    Some(out)
}

/// Lo que [`dispatch`] entrega fuera del préstamo.
struct Delivery {
    outbox: Outbox,
    selection_changed: bool,
    on_data: Option<js_sys::Function>,
    on_selection: Option<js_sys::Function>,
    document: Document,
    textarea: HtmlTextAreaElement,
    window: Window,
}

impl Delivery {
    fn deliver(self) {
        if let Some(f) = &self.on_data {
            for chunk in &self.outbox.data {
                let bytes = js_sys::Uint8Array::from(chunk.as_slice());
                let _ = f.call1(&JsValue::NULL, &bytes);
            }
        }
        if self.selection_changed
            && let Some(f) = &self.on_selection
        {
            let _ = f.call0(&JsValue::NULL);
        }
        match self.outbox.focus {
            Some(Focus::Plain) => focus_quietly(&self.textarea),
            Some(Focus::Select(text)) => {
                let _ = self.textarea.focus();
                self.textarea.set_value(&text);
                self.textarea.select();
            }
            None => {}
        }
        if let Some(text) = self.outbox.copy {
            write_clipboard(&self.window, &self.document, &self.textarea, text);
        }
        if let Some(text) = self.outbox.app_copy {
            // No `execCommand` fallback here: it would need the input textarea and a gesture.
            let navigator = self.window.navigator();
            if js_sys::Reflect::get(&navigator, &"clipboard".into()).is_ok_and(|c| c.is_object()) {
                let promise = navigator.clipboard().write_text(&text);
                let ignore = Closure::<dyn FnMut(JsValue)>::once_into_js(|_: JsValue| {});
                promise
                    .unchecked_ref::<PromiseThen>()
                    .then2(&ignore, &ignore);
            }
        }
        if let Some((weak, request)) = self.outbox.word {
            word_links::resolve(weak, request);
        }
        if let Some((url, x, y)) = self.outbox.open {
            link_menu::show(&self.window, &self.document, &self.textarea, &url, x, y);
        }
    }
}

/// `textarea.focus({preventScroll: true})`.
fn focus_quietly(textarea: &HtmlTextAreaElement) {
    let opts = web_sys::FocusOptions::new();
    opts.set_prevent_scroll(true);
    let _ = textarea.focus_with_options(&opts);
}

/// `navigator.clipboard.writeText`; sin él (contexto no seguro) o si lo
/// rechaza, `execCommand('copy')` desde el `<textarea>`, como `term.html`.
fn write_clipboard(
    window: &Window,
    document: &Document,
    textarea: &HtmlTextAreaElement,
    text: String,
) {
    let fallback = {
        let (document, textarea, text) = (document.clone(), textarea.clone(), text.clone());
        move || {
            textarea.set_value(&text);
            textarea.select();
            if let Ok(doc) = document.dyn_into::<web_sys::HtmlDocument>() {
                let _ = doc.exec_command("copy");
            }
        }
    };
    let navigator = window.navigator();
    let has_api =
        js_sys::Reflect::get(&navigator, &"clipboard".into()).is_ok_and(|c| c.is_object());
    if !has_api {
        fallback();
        return;
    }
    let promise = navigator.clipboard().write_text(&text);
    // Un solo cierre para los dos casos (la promesa llama a uno y
    // `once_into_js` se libera al llamarlo): `writeText` se cumple con
    // `undefined` y se rechaza con un error.
    let settle = Closure::<dyn FnMut(JsValue)>::once_into_js(move |v: JsValue| {
        if !v.is_undefined() {
            fallback();
        }
    });
    promise
        .unchecked_ref::<PromiseThen>()
        .then2(&settle, &settle);
}

impl Inner {
    fn now(&self) -> f64 {
        self.perf.as_ref().map_or(0.0, Performance::now)
    }

    fn dpr(&self) -> f64 {
        let dpr = self.window.device_pixel_ratio();
        if dpr.is_finite() && dpr > 0.0 {
            dpr
        } else {
            1.0
        }
    }

    fn measure(&self) -> Result<CellMetrics, JsValue> {
        let (w, h) = metrics::measure(
            &self.document,
            &self.dom.root,
            &self.opts.font_family,
            self.opts.font_size,
        )?;
        Ok(metrics::from_measure(
            w,
            h,
            self.dpr(),
            self.opts.line_height,
            self.opts.letter_spacing,
        ))
    }

    /// Estado del cursor para pintar. La fase del parpadeo se fija en el
    /// temporizador (como el `setInterval` de xterm.js), no al pintar: un
    /// cuadro retrasado no se salta una fase.
    fn cursor_input(&mut self) -> CursorInput {
        let enabled = paint::blink_enabled(&self.engine, self.focused);
        if self.blink.set_enabled(enabled, self.now()) {
            // Vuelve a parpadear (cursor de nuevo en la vista, foco…): como
            // `restartBlinkAnimation`, empieza visible.
            self.blink_on = true;
        }
        CursorInput {
            focused: self.focused,
            blink_on: !self.blink.enabled || self.blink_on,
            preferred_shape: self.opts.cursor_style,
        }
    }

    /// `restartBlinkAnimation`: el cursor vuelve a verse y el ciclo empieza.
    fn restart_blink(&mut self, now: f64) {
        self.blink.restart(now);
        self.blink_on = true;
    }

    /// Lo que [`dispatch`] entrega fuera del préstamo; `None` si no hay nada.
    fn take_delivery(&mut self) -> Option<Delivery> {
        // Durante un arrastre de selección la página se entera al soltar
        // (`_fireEventIfSelectionChanged` de xterm.js).
        let dragging = self
            .drag
            .as_ref()
            .is_some_and(|d| d.kind == DragKind::Select);
        let current = self.selection();
        let selection_changed = !dragging && current != self.reported_selection;
        if selection_changed {
            self.reported_selection = current;
        }
        if !selection_changed && self.outbox.is_empty() {
            return None;
        }
        Some(Delivery {
            outbox: std::mem::take(&mut self.outbox),
            selection_changed,
            on_data: self.on_data.clone(),
            on_selection: self.on_selection.clone(),
            document: self.document.clone(),
            textarea: self.dom.textarea.clone(),
            window: self.window.clone(),
        })
    }

    /// Selección actual para `comandos_term::select`.
    fn selection(&self) -> Option<Selection> {
        let engine = &self.engine;
        self.select.selection_with(self.size.cols, |p| {
            comandos_term::select::word_at(engine, p)
        })
    }

    fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    /// Texto seleccionado como lo copia xterm.js.
    fn selection_text(&self) -> String {
        self.selection()
            .map(|s| match self.clip_cols {
                Some(cols) => selected_text_within(&self.engine, &s, cols),
                None => selected_text(&self.engine, &s),
            })
            .unwrap_or_default()
    }

    fn clear_selection(&mut self) {
        if self.select.is_started() {
            self.select.clear();
            self.overlay.dirty = true;
            self.request_frame();
        }
    }

    /// Bytes para el PTY. Lo que escribe el usuario (`wasUserInput` de
    /// xterm.js) baja la vista al final y quita la selección.
    fn send(&mut self, bytes: Vec<u8>, user: bool) {
        if bytes.is_empty() {
            return;
        }
        if user {
            self.scroll_to_bottom();
            self.clear_selection();
        }
        self.outbox.data.push(bytes);
    }

    /// `scrollOnUserInput`: la vista vuelve al final.
    fn scroll_to_bottom(&mut self) {
        let offset = self.engine.display_offset();
        if offset != 0 {
            self.scroll_lines(-i32::try_from(offset).unwrap_or(i32::MAX));
        }
    }

    /// Desplaza la vista (positivo = hacia atrás en la historia).
    fn scroll_lines(&mut self, lines: i32) {
        if lines == 0 {
            return;
        }
        self.engine.scroll_display(lines);
        // `handleGridChanged` al repintar las filas.
        let now = self.now();
        self.restart_blink(now);
        if self.select.is_started() {
            self.overlay.dirty = true;
        }
        // El enlace bajo el ratón cae al repintarse sus filas (todas).
        self.schedule_ligatures();
        self.touch();
    }

    /// Fila superior de la vista contando desde lo más antiguo (`ydisp`) y
    /// la mayor posible (`ybase`).
    fn ydisp(&self) -> (usize, usize) {
        let history = self.engine.history_len();
        (
            history.saturating_sub(self.engine.display_offset()),
            history,
        )
    }

    /// Alto de fila del área de desplazamiento (`_currentRowHeight`).
    fn row_h(&self) -> f64 {
        f64::from(self.metrics.dev_h) / self.metrics.dpr
    }

    /// Foco del `<textarea>`: parpadeo, contorno del cursor, clase `focus` e
    /// informe de foco si la aplicación lo pidió.
    fn set_focused(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        let now = self.now();
        self.restart_blink(now);
        self.touch_cursor();
        let _ = self
            .dom
            .root
            .class_list()
            .toggle_with_force("focus", focused);
        if let Some(report) = comandos_term::input::focus(focused, &self.engine.modes()) {
            self.send(report.to_vec(), false);
        }
    }

    /// Tras escribir en el motor: la selección sigue a su texto, en las dos
    /// pantallas (`onTrim`), se recorta si la historia se borró (ED 3), y se
    /// va al cambiar de pantalla (`_handleBufferActivate`) o al activar el
    /// ratón de la aplicación (`onProtocolChange` → `disable()`). Los
    /// enlaces calculados ya no valen; el que está bajo el ratón solo cae
    /// si se repintan sus filas ([`Inner::on_frame`]).
    fn after_output(&mut self) {
        self.link_gen = self.link_gen.wrapping_add(1);
        let scrolled = self.engine.scrolled_up();
        let lines = scrolled.wrapping_sub(self.scrolled_seen);
        self.scrolled_seen = scrolled;
        // Refresh hover after scrolling. Keep the pressed link until release:
        // tmux may clear/repaint an identical viewport and advance history too.
        // mouseup checks its destination and span against the current cells.
        if lines > 0 {
            if self.hover.is_some() {
                self.forget_hover();
            }
        }
        let modes = self.engine.modes();
        let mouse_changed = modes.mouse != self.mouse_seen;
        self.mouse_seen = modes.mouse;
        if modes.alt_screen != self.alt_seen {
            self.alt_seen = modes.alt_screen;
            self.clear_selection();
        } else if mouse_changed && modes.mouse != MouseMode::Off {
            self.clear_selection();
        } else if self.select.is_started() {
            // La misma selección en otras filas no es un cambio para la
            // página (xterm.js no avisa al recortar la historia).
            let before = self.select.clone();
            let reported = self.reported_selection == self.selection();
            let first = -i32::try_from(self.engine.history_len()).unwrap_or(i32::MAX);
            self.select.shift_up(lines, first);
            if self.select != before {
                if reported && self.select.is_started() {
                    self.reported_selection = self.selection();
                }
                self.overlay.dirty = true;
            }
        }
        // `onRender` del addon de ligaduras.
        self.schedule_ligatures();
    }

    /// `onRenderedViewportChange` de `Linkifier`: si el cuadro repintó las
    /// filas del enlace bajo el ratón, ese enlace (y su pulsación) ya no
    /// vale; se vuelve a buscar en la última posición del ratón.
    fn forget_repainted_link(&mut self, range: (usize, usize)) {
        let offset = self.engine.display_offset();
        let stale = |span: &Option<UrlSpan>| {
            span.as_ref()
                .is_some_and(|s| links::repainted(s, offset, range))
        };
        // A repaint (including tmux focus feedback) does not cancel a click.
        // mouseup revalidates both the URL and its exact span against fresh cells.
        if stale(&self.hover) {
            self.forget_hover();
        }
    }

    /// Suelta el enlace bajo el ratón y lo vuelve a buscar en la última
    /// posición del ratón (fuera de un arrastre).
    fn forget_hover(&mut self) {
        self.hover = None;
        self.set_pointer(false);
        self.overlay.dirty = true;
        if self.drag.is_none()
            && let Some((x, y)) = self.hover_xy
        {
            self.update_hover(x, y);
        }
    }

    /// Quita el enlace bajo el ratón (su subrayado y el puntero).
    fn drop_hover(&mut self) {
        if self.hover.take().is_some() {
            self.overlay.dirty = true;
            self.set_pointer(false);
        }
        self.link_down = None;
    }

    /// Puntero de enlace (`xterm-cursor-pointer`).
    fn set_pointer(&self, on: bool) {
        let _ = self
            .dom
            .root
            .class_list()
            .toggle_with_force("xterm-cursor-pointer", on);
        let _ = self
            .dom
            .screen
            .style()
            .set_property("cursor", if on { "pointer" } else { "" });
    }

    /// Rectángulos de la selección en la vista.
    fn selection_rects(&self) -> Vec<overlay::CellRect> {
        let offset = self.engine.display_offset();
        let (rows, cols) = (self.size.rows, self.size.cols);
        self.selection()
            .and_then(|s| {
                let (lo, hi) = comandos_term::select::selection_bounds(&self.engine, &s)?;
                let end = (hi.0, hi.1.saturating_add(1));
                Some(if s.mode == comandos_term::select::SelectMode::Block {
                    overlay::block_rects(lo, end, offset, rows, cols)
                } else {
                    overlay::selection_rects(lo, end, offset, rows, cols)
                })
            })
            .unwrap_or_default()
    }

    /// Repinta la capa de selección y enlace.
    fn paint_overlay(&mut self) {
        let offset = self.engine.display_offset();
        let (rows, cols) = (self.size.rows, self.size.cols);
        let rects = self.selection_rects();
        let lines = self
            .hover
            .as_ref()
            .map(|span| links::underline_rects(span, offset, rows, cols))
            .unwrap_or_default();
        let link_color = canvas::hex(self.palette.fg);
        self.overlay.paint(
            self.painter.element(),
            &self.metrics,
            (&rects, &self.selection_color),
            (&lines, &link_color),
        );
    }

    /// Repinta las ligaduras de toda la vista.
    fn paint_ligatures(&mut self) {
        if self.ligatures.is_none() {
            return;
        }
        let (rows, cols) = (self.size.rows, self.size.cols);
        let offset = i32::try_from(self.engine.display_offset()).unwrap_or(i32::MAX);
        let spans: Vec<Vec<(usize, &'static str)>> = (0..i32::from(rows))
            .map(|y| {
                let row = y - offset;
                let whole = Selection {
                    anchor: (row, 0),
                    head: (row, cols.saturating_sub(1)),
                    mode: comandos_term::select::SelectMode::Simple,
                };
                overlay::liga_spans(&selected_text(&self.engine, &whole))
            })
            .collect();
        let css = self.metrics.css_canvas(cols, rows);
        let (bg, fg) = (canvas::hex(self.palette.bg), canvas::hex(self.palette.fg));
        if let Some(liga) = self.ligatures.as_mut() {
            liga.paint(
                self.painter.element(),
                css,
                (cols, rows),
                self.opts.font_size,
                (&bg, &fg),
                &spans,
            );
        }
    }

    /// Repinta las ligaduras 90 ms después del último cambio (escritura,
    /// desplazamiento), como el addon con `onRender`; el parpadeo del cursor
    /// no cuenta.
    fn schedule_ligatures(&mut self) {
        // Sin la cara cargada no hay nada que pintar (se pinta al cargar).
        let Some(liga) = self.ligatures.as_mut().filter(|l| l.loaded) else {
            return;
        };
        if let Some(id) = liga.timer.take() {
            self.window.clear_timeout_with_handle(id);
        }
        let Some(cb) = self.liga_cb.as_ref() else {
            return;
        };
        if let Ok(id) = self
            .window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                overlay::LIGA_DEBOUNCE_MS,
            )
        {
            liga.timer = Some(id);
        }
    }

    fn on_liga_timer(&mut self) {
        if let Some(liga) = self.ligatures.as_mut() {
            liga.timer = None;
        }
        self.liga_due = true;
        self.request_frame();
    }

    /// `_syncTextArea`: el `<textarea>` sobre la celda del cursor, para que
    /// el IME ponga sus candidatos junto a él.
    fn sync_textarea(&mut self) {
        if self.composing {
            return;
        }
        let c = comandos_term::render::cursor(&self.engine);
        if c.line >= usize::from(self.size.rows) {
            return;
        }
        let Some((cw, ch)) = self.metrics.css_cell(self.size.cols, self.size.rows) else {
            return;
        };
        let col = c.col.min(self.size.cols.saturating_sub(1));
        let key = (col, c.line, (cw * 1000.0) as u32, (ch * 1000.0) as u32);
        if self.textarea_at == Some(key) {
            return;
        }
        self.textarea_at = Some(key);
        let width = if c.wide { 2.0 } else { 1.0 };
        let px = |v: f64| crate::number_text::px(v);
        set_styles(
            &self.dom.textarea,
            &[
                ("left", &px(f64::from(col) * cw)),
                ("top", &px(c.line as f64 * ch)),
                ("width", &px(cw * width)),
                ("height", &px(ch)),
                ("line-height", &px(ch)),
                ("z-index", "-5"),
            ],
        );
    }

    /// Junta lo que dejó el motor (respuestas, título, campana…).
    fn drain_engine(&mut self) {
        let d = self.engine.drain();
        self.replies.extend_from_slice(&d.replies);
        if d.title.is_some() {
            self.title = d.title;
        }
        self.bell |= d.bell;
        if let (Some(target), Some(text)) = (d.clipboard_target, d.clipboard) {
            if !text.is_empty() {
                self.outbox.app_copy = Some(text.clone());
                self.app_copied = Some((self.now(), text.clone()));
            }
            self.clipboard = Some((target, text));
        }
    }

    /// Recoge daños y pide cuadro si hace falta.
    fn touch(&mut self) {
        let input = self.cursor_input();
        self.scheduler.absorb(&mut self.engine, input);
        self.after_touch();
    }

    /// Como [`Inner::touch`] cuando el motor no cambió (foco, parpadeo):
    /// solo cuenta el cursor, no el daño que alacritty pone siempre en su
    /// fila.
    fn touch_cursor(&mut self) {
        let input = self.cursor_input();
        self.scheduler.absorb_cursor(&self.engine, input);
        self.after_touch();
    }

    fn after_touch(&mut self) {
        let now = self.now();
        self.request_frame();
        self.arm_timer(now);
    }

    fn request_frame(&mut self) {
        if self.frame_id.is_some() {
            // Ya viene un cuadro: pinta todo lo pendiente.
            return;
        }
        let rows = self.scheduler.wants_frame();
        if !rows && !self.overlay.dirty && !self.liga_due {
            return;
        }
        let Some(cb) = self.frame_cb.as_ref() else {
            self.scheduler.frame_cancelled();
            return;
        };
        match self
            .window
            .request_animation_frame(cb.as_ref().unchecked_ref())
        {
            Ok(id) => self.frame_id = Some(id),
            Err(_) => self.scheduler.frame_cancelled(),
        }
    }

    fn on_frame(&mut self) {
        self.frame_id = None;
        let now = self.now();
        let input = self.cursor_input();
        if self.overlay.dirty {
            // Las filas cuyas celdas seleccionadas cambiaron se repintan:
            // sus glifos van contra la selección opaca (`CellColorResolver`).
            let ranges = overlay::row_ranges(&self.selection_rects(), self.size.rows);
            self.scheduler
                .set_selection(&ranges, selection_opaque(&self.opts.theme));
        }
        let stats = self.scheduler.frame(
            &mut self.engine,
            &self.palette,
            &self.render_opts,
            input,
            &self.metrics,
            &mut self.painter,
        );
        self.sync_scroll_area();
        if let Some(range) = stats.content {
            self.forget_repainted_link(range);
        }
        if self.overlay.dirty {
            self.paint_overlay();
        }
        if std::mem::take(&mut self.liga_due) {
            self.paint_ligatures();
        }
        self.sync_textarea();
        self.arm_timer(now);
    }

    /// Un solo temporizador: el próximo cambio del parpadeo o el
    /// vencimiento de una actualización sincronizada, lo que llegue antes.
    fn arm_timer(&mut self, now: f64) {
        let blink = self.blink.next_toggle(now);
        let sync = self.engine.next_deadline_ms();
        let wake = match (blink, sync) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let Some(wake) = wake else {
            if let Some((id, _)) = self.timer.take() {
                self.window.clear_timeout_with_handle(id);
            }
            return;
        };
        if let Some((id, at)) = self.timer {
            if at <= wake {
                return;
            }
            self.window.clear_timeout_with_handle(id);
            self.timer = None;
        }
        let Some(cb) = self.timer_cb.as_ref() else {
            return;
        };
        // Un temporizador nunca es negativo; un retardo absurdo se acota.
        let delay = (wake - now).clamp(0.0, 1e9).ceil() as i32;
        if let Ok(id) = self
            .window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                delay,
            )
        {
            self.timer = Some((id, wake));
        }
    }

    fn on_timer(&mut self) {
        self.timer = None;
        let now = self.now();
        let flushed = self.engine.tick(now);
        if flushed {
            self.drain_engine();
        }
        self.blink_on = self.blink.visible(now);
        if flushed {
            self.touch();
        } else {
            self.touch_cursor();
        }
    }

    /// `Viewport._innerRefresh`: alto del área y posición de la barra.
    fn sync_scroll_area(&mut self) {
        let rows = usize::from(self.size.rows);
        let history = self.engine.history_len();
        let buffer_len = history + rows;
        let row_h = f64::from(self.metrics.dev_h) / self.metrics.dpr;
        let (_, css_canvas_h) = self.metrics.css_canvas(self.size.cols, self.size.rows);
        let ydisp = history.saturating_sub(self.engine.display_offset());
        let scroll_top = ydisp as f64 * row_h;
        let viewport_h = self
            .scroll_sync
            .viewport_h
            .unwrap_or_else(|| f64::from(self.dom.viewport.offset_height()));
        let next = ScrollSync {
            viewport_h: Some(viewport_h),
            buffer_len,
            css_canvas_h,
            dev_cell_h: self.metrics.dev_h,
            scroll_top,
        };
        if next == self.scroll_sync {
            return;
        }
        let layout_changed = (
            next.viewport_h,
            next.buffer_len,
            next.css_canvas_h,
            next.dev_cell_h,
        ) != (
            self.scroll_sync.viewport_h,
            self.scroll_sync.buffer_len,
            self.scroll_sync.css_canvas_h,
            self.scroll_sync.dev_cell_h,
        );
        if layout_changed {
            let height = metrics::js_round(row_h * buffer_len as f64) + (viewport_h - css_canvas_h);
            self.css_buf.clear();
            self.css_buf.push_str(&crate::number_text::px(height));
            let _ = self
                .dom
                .scroll_area
                .style()
                .set_property("height", &self.css_buf);
        }
        let top = scroll_top.round() as i32;
        if self.dom.viewport.scroll_top() != top {
            // Como `_ignoreNextScrollEvent`: ese `scroll` no es del usuario.
            self.ignore_scroll = true;
            self.dom.viewport.set_scroll_top(top);
        }
        self.scroll_sync = next;
    }

    /// Aplica medidas y tamaño al canvas y al DOM y repinta todo.
    fn relayout(&mut self, size: GridSize) {
        let changed = size != self.size;
        self.size = size;
        self.overlay.dirty = true;
        self.liga_due = true;
        self.textarea_at = None;
        if changed {
            // xterm.js no toca la selección al cambiar de tamaño: se queda en
            // las mismas filas de búfer (índice desde lo más antiguo de la
            // historia). Aquí las filas cuentan desde la pantalla, así que se
            // corrigen con lo que cambió la historia.
            let history_before = self.engine.history_len();
            self.drop_hover();
            let (cw, ch) = self
                .metrics
                .css_cell(size.cols, size.rows)
                .unwrap_or((self.metrics.css_w, self.metrics.css_h));
            let cell = (
                metrics::js_round(cw).clamp(0.0, 65535.0) as u16,
                metrics::js_round(ch).clamp(0.0, 65535.0) as u16,
            );
            self.engine.resize(size, cell);
            self.scrolled_seen = self.engine.scrolled_up();
            let history_after = self.engine.history_len();
            if self.select.is_started() {
                let to_i32 = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
                let down = to_i32(history_before).saturating_sub(to_i32(history_after));
                self.select.shift(down, -to_i32(history_after));
            }
        }
        self.painter.resize(size.cols, size.rows, &self.metrics);
        self.scheduler.resize(size.rows);
        let (w, h) = self.metrics.css_canvas(size.cols, size.rows);
        let style = self.dom.screen.style();
        let _ = style.set_property("width", &crate::number_text::px(w));
        let _ = style.set_property("height", &crate::number_text::px(h));
        self.scroll_sync = ScrollSync::default();
        self.touch();
    }

    fn on_device_pixels(&mut self, entries: &js_sys::Array) {
        let canvas: &JsValue = self.painter.element();
        let entry = entries
            .iter()
            .find(|e| js_sys::Reflect::get(e, &"target".into()).is_ok_and(|t| t == *canvas));
        let Some(entry) = entry else {
            return;
        };
        let size = js_sys::Reflect::get(&entry, &"devicePixelContentBoxSize".into())
            .ok()
            .filter(|v| !v.is_undefined())
            .and_then(|list| js_sys::Reflect::get_u32(&list, 0).ok());
        let Some(size) = size.filter(|v| !v.is_undefined()) else {
            // Sin la medida en píxeles de dispositivo no hay nada que seguir.
            if let Some(observer) = self.observer.take() {
                observer.disconnect();
            }
            return;
        };
        let dim = |key: &str| {
            js_sys::Reflect::get(&size, &key.into())
                .ok()
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite() && *v > 0.0)
                .map(|v| v.min(f64::from(u32::MAX)) as u32)
        };
        let (Some(w), Some(h)) = (dim("inlineSize"), dim("blockSize")) else {
            return;
        };
        let (w, h) = self.trusted_device_size(&entry, w, h);
        // Otro monitor con otro `devicePixelRatio` sin cambio de tamaño CSS:
        // hay que volver a medir, si no se pinta con la celda vieja.
        if self.dpr() != self.metrics.dpr && matches!(self.remeasure(), Ok(true)) {
            let grid = self.size;
            self.painter.set_device_size(w, h);
            self.relayout(grid);
            return;
        }
        if self.painter.set_device_size(w, h) {
            self.scheduler.resize(self.size.rows);
            self.touch();
        }
    }

    /// The device size to paint at. A browser can report the CSS size as the device
    /// size while `devicePixelRatio` is not 1 (Chrome's device emulation does); the
    /// fonts are drawn at `dpr`, so trusting it would draw the text `dpr` times too big
    /// into a third of the screen. Then the size comes from the CSS box times `dpr`.
    fn trusted_device_size(&self, entry: &JsValue, w: u32, h: u32) -> (u32, u32) {
        let css = js_sys::Reflect::get(entry, &"contentBoxSize".into())
            .ok()
            .and_then(|list| js_sys::Reflect::get_u32(&list, 0).ok())
            .filter(|v| v.is_object());
        let Some(css) = css else { return (w, h) };
        let get = |key: &str| {
            js_sys::Reflect::get(&css, &key.into())
                .ok()
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite() && *v > 0.0)
        };
        let (Some(cw), Some(ch)) = (get("inlineSize"), get("blockSize")) else {
            return (w, h);
        };
        comandos_term::render::device_size(w, h, (cw, ch), self.dpr())
    }

    fn proposed_size(&mut self) -> Option<GridSize> {
        if let Some(fixed) = self.fixed {
            return Some(fixed);
        }
        if self.dpr() != self.metrics.dpr {
            let _ = self.remeasure();
        }
        let style = self.window.get_computed_style(&self.host).ok().flatten();
        let prop = |name: &str| {
            style
                .as_ref()
                .and_then(|s| s.get_property_value(name).ok())
                .map(|v| metrics::parse_css_int(&v))
                .unwrap_or(f64::NAN)
        };
        // `Math.max(0, NaN)` es NaN: `f64::max` daría 0.
        let width = prop("width");
        let width = if width.is_nan() {
            width
        } else {
            width.max(0.0)
        };
        let height = prop("height");
        // El elemento `.xterm` no tiene relleno propio (xterm.css).
        let scrollbar = if self.opts.scrollback == 0 {
            0.0
        } else {
            self.scrollbar_w
        };
        let current = (self.size.cols, self.size.rows);
        let Some((cols, rows)) = metrics::fit(width, height, scrollbar, &self.metrics, current)
        else {
            // Sin celda medida o sin tamaño numérico FitAddon no toca
            // la terminal.
            return None;
        };
        Some(GridSize { cols, rows })
    }

    fn remeasure(&mut self) -> Result<bool, JsValue> {
        let m = self.measure()?;
        if !m.is_valid() || m == self.metrics {
            return Ok(false);
        }
        self.metrics = m;
        Ok(true)
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        link_menu::close();
        if let Some(cb) = self.restored_cb.take() {
            for canvas in [self.painter.element(), self.painter.cursor_element()] {
                let _ = canvas.remove_event_listener_with_callback(
                    "contextrestored",
                    cb.as_ref().unchecked_ref(),
                );
            }
        }
        if let Some(observer) = self.observer.take() {
            observer.disconnect();
        }
        if let Some(id) = self.frame_id.take() {
            let _ = self.window.cancel_animation_frame(id);
        }
        if let Some((id, _)) = self.timer.take() {
            self.window.clear_timeout_with_handle(id);
        }
        if let Some(id) = self.ligatures.as_mut().and_then(|l| l.timer.take()) {
            self.window.clear_timeout_with_handle(id);
        }
        // La cara de ligaduras no se queda en `document.fonts`.
        if let Some(face) = self.liga_face.take() {
            let _ = self.document.fonts().delete(&face);
        }
        // Las escuchas y el arrastre se quitan al soltarse los campos.
        self.drag = None;
        self.listeners.clear();
        self.dom.root.remove();
    }
}

/// Terminal en el navegador: motor, planificador y canvas.
#[wasm_bindgen]
pub struct WebTerm {
    inner: Rc<RefCell<Inner>>,
}

/// Ejecuta `f` sobre el estado si sigue vivo y nadie lo tiene prestado.
fn with_weak(weak: &std::rc::Weak<RefCell<Inner>>, f: impl FnOnce(&mut Inner)) {
    if let Some(rc) = weak.upgrade()
        && let Ok(mut inner) = rc.try_borrow_mut()
    {
        f(&mut inner);
    }
}

#[wasm_bindgen]
impl WebTerm {
    /// Monta la terminal dentro de `host` con las opciones de
    /// `new Terminal({...})` de xterm.js (`fontFamily`, `fontSize`,
    /// `lineHeight`, `letterSpacing`, `cursorBlink`, `scrollback`, `theme`).
    /// Empieza con 80×24; el tamaño real llega con [`WebTerm::resize_to_fit`].
    #[wasm_bindgen(constructor)]
    pub fn new(host: HtmlElement, opts: JsValue) -> Result<WebTerm, JsValue> {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("sin window"))?;
        let document = window
            .document()
            .ok_or_else(|| JsValue::from_str("sin document"))?;
        let opts = read_options(&opts);
        let palette = opts.theme.palette();
        let dom = Dom::new(&document, &host)?;
        let _ = dom
            .viewport
            .style()
            .set_property("background-color", &canvas::hex(palette.bg));
        let size = GridSize { cols: 80, rows: 24 };
        let dpr = {
            let d = window.device_pixel_ratio();
            if d.is_finite() && d > 0.0 { d } else { 1.0 }
        };
        let (cw, ch) = metrics::measure(&document, &dom.root, &opts.font_family, opts.font_size)?;
        let m = metrics::from_measure(cw, ch, dpr, opts.line_height, opts.letter_spacing);
        let painter = Canvas2d::new(
            &window,
            &document,
            canvas_theme(&palette),
            &opts.font_family,
            opts.font_size,
            m,
        )?;
        dom.screen.append_child(painter.element())?;
        let overlay = overlay::Overlay::new(&document)?;
        dom.screen.append_child(overlay.element())?;
        // Capas de addon-canvas: texto, selección (1), enlace, cursor (3).
        dom.screen.append_child(painter.cursor_element())?;
        // Ligaduras solo sin táctil, como `term.html` (`IS_TOUCH`).
        let ligatures = if opts.ligatures.unwrap_or(!is_touch(&window)) {
            let liga = overlay::Ligatures::new(&document)?;
            dom.screen.append_child(liga.element())?;
            Some(liga)
        } else {
            None
        };
        // Bytes en vez de `str::contains`, que arrastra el buscador de
        // subcadenas al wasm.
        let is_linux = window
            .navigator()
            .platform()
            .is_ok_and(|p| p.as_bytes().windows(5).any(|w| w == b"Linux"));
        let selection_color = selection_css(&opts.theme);
        let scrollbar_w = metrics::scrollbar_width(&dom.viewport, &dom.scroll_area);
        let engine =
            Engine::with_cursor_blink(size, opts.scrollback, palette.clone(), opts.cursor_blink);
        let render_opts = RenderOpts {
            bold_is_bright: opts.draw_bold_text_in_bright_colors,
            min_contrast: opts.minimum_contrast_ratio,
        };
        let perf = window.performance();
        let now = perf.as_ref().map_or(0.0, Performance::now);
        let inner = Rc::new(RefCell::new(Inner {
            window,
            document,
            perf,
            host,
            dom,
            engine,
            palette,
            render_opts,
            scheduler: Scheduler::new(size.rows),
            painter,
            metrics: m,
            opts,
            size,
            scrollbar_w,
            focused: false,
            blink: Blink::new(false, now),
            blink_on: true,
            frame_cb: None,
            frame_id: None,
            timer_cb: None,
            observer: None,
            observer_cb: None,
            restored_cb: None,
            css_buf: String::new(),
            timer: None,
            replies: Vec::new(),
            title: None,
            bell: false,
            clipboard: None,
            app_copied: None,
            scroll_sync: ScrollSync::default(),
            outbox: Outbox::default(),
            on_data: None,
            on_selection: None,
            listeners: Vec::new(),
            keys: keyboard::Keys::default(),
            composing: false,
            textarea_at: None,
            reporter: mouse::MouseReporter::default(),
            wheel: mouse::Wheel::default(),
            select: mouse::SelectModel::default(),
            reported_selection: None,
            drag: None,
            ignore_scroll: false,
            touch_y: None,
            touch_link: None,
            word_press: None,
            word_generation: 0,
            links: links::LinkCache::default(),
            link_gen: 0,
            hover: None,
            hover_xy: None,
            link_down: None,
            down_ts: 0.0,
            mouse_seen: MouseMode::Off,
            overlay,
            selection_color,
            ligatures,
            liga_face: None,
            liga_cb: None,
            liga_due: false,
            scrolled_seen: 0,
            alt_seen: false,
            is_linux,
            weak: Weak::new(),
            fixed: None,
            clip_cols: None,
        }));
        let weak = Rc::downgrade(&inner);
        let frame_cb = Closure::<dyn FnMut(f64)>::new(move |_t: f64| {
            with_weak(&weak, Inner::on_frame);
        });
        let weak = Rc::downgrade(&inner);
        let timer_cb = Closure::<dyn FnMut()>::new(move || {
            with_weak(&weak, Inner::on_timer);
        });
        let weak = Rc::downgrade(&inner);
        let liga_cb = Closure::<dyn FnMut()>::new(move || {
            dispatch(&weak, Inner::on_liga_timer);
        });
        {
            let mut i = inner
                .try_borrow_mut()
                .map_err(|_| JsValue::from_str("estado ocupado"))?;
            i.weak = Rc::downgrade(&inner);
            i.frame_cb = Some(frame_cb);
            i.timer_cb = Some(timer_cb);
            i.liga_cb = Some(liga_cb);
            i.relayout(size);
            i.install_listeners();
        }
        let term = WebTerm { inner };
        term.watch_font();
        term.observe_device_pixels();
        term.watch_context();
        term.load_ligature_font();
        Ok(term)
    }

    fn with<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> Option<T> {
        self.inner.try_borrow_mut().ok().map(|mut i| f(&mut i))
    }

    /// Como [`WebTerm::with`], entregando después lo que haya salido
    /// (bytes, aviso de selección…).
    fn dispatch<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> Option<T> {
        dispatch(&Rc::downgrade(&self.inner), f)
    }

    /// Carga la cara `comandos-liga` (como `opentype.load` del addon) y
    /// pinta las ligaduras cuando esté.
    fn load_ligature_font(&self) {
        let Some(Some(face)) = self.with(|i| {
            i.ligatures.as_ref()?;
            let face = FontFace::new_with_str(
                overlay::LIGA_FAMILY,
                &format!("url({})", i.opts.ligature_font_url),
            )
            .ok()?;
            let _ = i.document.fonts().add(&face);
            i.liga_face = Some(face.clone());
            Some(face)
        }) else {
            return;
        };
        let Ok(promise) = face.load() else {
            return;
        };
        let weak = Rc::downgrade(&self.inner);
        // Un solo cierre para cumplir y rechazar (ver `watch_font`).
        let settle = Closure::<dyn FnMut(JsValue)>::once_into_js(move |_v: JsValue| {
            dispatch(&weak, |i| {
                let loaded = i
                    .liga_face
                    .as_ref()
                    .is_some_and(|f| f.status() == web_sys::FontFaceLoadStatus::Loaded);
                if let Some(liga) = i.ligatures.as_mut() {
                    liga.loaded = loaded;
                }
                i.liga_due = true;
                i.request_frame();
            });
        });
        promise
            .unchecked_ref::<PromiseThen>()
            .then2(&settle, &settle);
    }

    /// Cuando la fuente termine de cargar (`document.fonts.load`), vuelve a
    /// medir y repinta si la celda cambió.
    fn watch_font(&self) {
        let Some((promise, weak)) = self.with(|i| {
            let px = i.opts.font_size;
            let spec = format!("{} {}", crate::number_text::px(px), i.opts.font_family);
            (i.document.fonts().load(&spec), Rc::downgrade(&self.inner))
        }) else {
            return;
        };
        // Un solo cierre para los dos casos: la promesa llama exactamente a
        // uno, y `once_into_js` lo libera tras esa llamada (dos cierres
        // dejarían siempre uno sin liberar).
        let settle = Closure::<dyn FnMut(JsValue)>::once_into_js(move |_v: JsValue| {
            with_weak(&weak, |i| {
                // Aunque la celda mida igual, lo rasterizado con la fuente de
                // respaldo sobra.
                let (family, size) = (i.opts.font_family.clone(), i.opts.font_size);
                i.painter.set_font(&family, size);
                let _ = i.remeasure();
                let grid = i.size;
                i.relayout(grid);
            });
        });
        promise
            .unchecked_ref::<PromiseThen>()
            .then2(&settle, &settle);
    }

    /// Si el navegador pierde y restaura el contexto del canvas (reinicio de
    /// la GPU), las páginas del atlas pueden haber quedado vacías: se vacía
    /// todo y se repinta. La capa del cursor se vigila igual (perderla deja
    /// el cursor sin pintar hasta que se repinte su fila). xterm.js 0.7 no lo
    /// trata.
    fn watch_context(&self) {
        let weak = Rc::downgrade(&self.inner);
        let cb = Closure::<dyn FnMut()>::new(move || {
            with_weak(&weak, |i| {
                i.painter.reset_caches();
                let grid = i.size;
                i.relayout(grid);
            });
        });
        self.with(|i| {
            let mut added = false;
            for canvas in [i.painter.element(), i.painter.cursor_element()] {
                added |= canvas
                    .add_event_listener_with_callback(
                        "contextrestored",
                        cb.as_ref().unchecked_ref(),
                    )
                    .is_ok();
            }
            if added {
                i.restored_cb = Some(cb);
            }
        });
    }

    /// `observeDevicePixelDimensions` de xterm.js: el almacén del canvas
    /// sigue a los píxeles de dispositivo que el navegador le da.
    fn observe_device_pixels(&self) {
        let weak = Rc::downgrade(&self.inner);
        let cb = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
            with_weak(&weak, |i| i.on_device_pixels(&entries));
        });
        let Ok(observer) = ResizeObserver::new(cb.as_ref().unchecked_ref()) else {
            return;
        };
        let opts = ResizeObserverOptions::new();
        opts.set_box(ResizeObserverBoxOptions::DevicePixelContentBox);
        self.with(|i| {
            // Sin `device-pixel-content-box` (navegadores viejos) se lanza:
            // se queda el tamaño calculado, como en xterm.js.
            let target: &JsValue = i.painter.element();
            if observer
                .unchecked_ref::<ObserveCatch>()
                .observe_catch(target, &opts)
                .is_ok()
            {
                i.observer = Some(observer);
                i.observer_cb = Some(cb);
            } else {
                observer.disconnect();
            }
        });
    }

    /// Clear PTY state, history, selection and overlays before a fresh attachment.
    pub fn reset(&mut self) {
        self.dispatch(|i| {
            i.engine = Engine::with_cursor_blink(
                i.size,
                i.opts.scrollback,
                i.palette.clone(),
                i.opts.cursor_blink,
            );
            let (cw, ch) = i
                .metrics
                .css_cell(i.size.cols, i.size.rows)
                .unwrap_or((i.metrics.css_w, i.metrics.css_h));
            i.engine.resize(
                i.size,
                (
                    metrics::js_round(cw).clamp(0.0, 65535.0) as u16,
                    metrics::js_round(ch).clamp(0.0, 65535.0) as u16,
                ),
            );
            i.restart_blink(i.now());
            i.select = mouse::SelectModel::default();
            i.drag.take();
            i.keys = keyboard::Keys::default();
            i.reporter = mouse::MouseReporter::default();
            i.wheel = mouse::Wheel::default();
            i.composing = false;
            i.dom.textarea.set_value("");
            i.replies.clear();
            i.title = None;
            i.bell = false;
            i.clipboard = None;
            i.overlay.dirty = true;
            i.drop_hover();
            i.scrolled_seen = 0;
            i.scroll_sync = ScrollSync::default();
            i.scheduler.resize(i.size.rows);
            i.after_output();
            i.touch();
        });
    }

    /// Bytes del PTY.
    pub fn write(&mut self, bytes: &[u8]) {
        self.dispatch(|i| {
            let now = i.now();
            i.engine.advance(bytes, now);
            i.drain_engine();
            i.after_output();
            // `handleGridChanged`: cada cambio deja el cursor visible.
            i.restart_blink(now);
            i.touch();
        });
    }

    /// Ajusta filas y columnas al contenedor como `FitAddon.fit()` y
    /// devuelve `{cols, rows}`.
    pub fn resize_to_fit(&mut self) -> JsValue {
        let size = self.dispatch(|i| {
            let Some(size) = i.proposed_size() else {
                return i.size;
            };
            if size != i.size || i.painter_needs_layout() {
                i.relayout(size);
            } else {
                // El contenedor cambió sin cambiar la rejilla: la barra
                // depende del alto del viewport.
                i.scroll_sync.viewport_h = None;
                i.sync_scroll_area();
            }
            size
        });
        let size = size.unwrap_or(GridSize { cols: 0, rows: 0 });
        size_object(size.cols, size.rows)
    }

    /// Cambia el tema (objeto `ITheme` de xterm.js) y repinta.
    pub fn set_theme(&mut self, theme: JsValue) {
        self.with(|i| {
            let parsed = read_theme(&theme);
            i.selection_color = selection_css(&parsed);
            i.opts.theme = parsed;
            i.palette = i.opts.theme.palette();
            i.engine.set_palette(i.palette.clone());
            i.painter.set_theme(canvas_theme(&i.palette));
            let _ = i
                .dom
                .viewport
                .style()
                .set_property("background-color", &canvas::hex(i.palette.bg));
            let size = i.size;
            i.relayout(size);
        });
    }

    /// Preferencia de cursor; repinta solo la fila afectada.
    pub fn set_cursor_style(&mut self, style: &str) {
        let Some(shape) = cursor_style(style) else {
            return;
        };
        self.with(|i| {
            if i.opts.cursor_style != Some(shape) {
                i.opts.cursor_style = Some(shape);
                i.touch_cursor();
            }
        });
    }

    /// Cambia la fuente; las columnas y filas se ajustan con
    /// [`WebTerm::resize_to_fit`].
    pub fn set_font(&mut self, family: &str, size: f64) {
        let changed = self.with(|i| {
            let Some(size) = font_size_in_range(size) else {
                return false;
            };
            i.opts.font_family = family.to_string();
            i.opts.font_size = size;
            i.painter.set_font(family, size);
            let _ = i.remeasure();
            let grid = i.size;
            i.relayout(grid);
            true
        });
        if changed == Some(true) {
            self.watch_font();
        }
    }

    /// Fixes the grid at `cols`×`rows` whatever the host measures (`0` = fit the
    /// host again); [`WebTerm::resize_to_fit`] applies it.
    pub fn set_fixed_size(&mut self, cols: u16, rows: u16) {
        self.with(|i| {
            i.fixed = (cols >= 2 && rows >= 1).then_some(GridSize { cols, rows });
        });
    }

    /// Copies keep only the columns `left..right` (`right == 0` copies whole rows).
    pub fn set_clip_columns(&mut self, left: u16, right: u16) {
        self.with(|i| {
            i.clip_cols = (right > left).then_some((usize::from(left), usize::from(right)));
        });
    }

    /// Respuestas para el PTY (DA, DSR, colores…) desde la última llamada.
    pub fn take_replies(&mut self) -> Vec<u8> {
        self.with(|i| std::mem::take(&mut i.replies))
            .unwrap_or_default()
    }

    /// Último título pedido por la aplicación (`""` = el de por omisión).
    pub fn take_title(&mut self) -> Option<String> {
        self.with(|i| i.title.take()).flatten()
    }

    /// Sonó la campana desde la última llamada.
    pub fn take_bell(&mut self) -> bool {
        self.with(|i| std::mem::take(&mut i.bell)).unwrap_or(false)
    }

    /// Texto que la aplicación quiso copiar (OSC 52); la página decide si
    /// lo lleva al portapapeles.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.with(|i| i.clipboard.take().map(|(_, text)| text))
            .flatten()
    }

    /// Estado de foco de la terminal. Normalmente lo marcan los eventos
    /// `focus`/`blur` del `<textarea>`; con foco el cursor parpadea y sin él
    /// se dibuja su contorno.
    pub fn set_focus(&mut self, focused: bool) {
        self.dispatch(|i| i.set_focused(focused));
    }

    /// Da el foco al `<textarea>` de la terminal (`term.focus()`).
    pub fn focus(&self) {
        if let Ok(i) = self.inner.try_borrow() {
            let textarea = i.dom.textarea.clone();
            drop(i);
            focus_quietly(&textarea);
        }
    }

    /// Quita el foco (`term.blur()`).
    pub fn blur(&self) {
        if let Ok(i) = self.inner.try_borrow() {
            let textarea = i.dom.textarea.clone();
            drop(i);
            let _ = textarea.blur();
        }
    }

    /// El `<textarea>` que recibe el teclado (`term.textarea`).
    pub fn textarea(&self) -> Option<HtmlTextAreaElement> {
        self.inner.try_borrow().ok().map(|i| i.dom.textarea.clone())
    }

    /// Función que recibe los bytes para el PTY (`Uint8Array`), como
    /// `term.onData` + `term.onBinary`; `null` la quita.
    pub fn set_on_data(&mut self, f: Option<js_sys::Function>) {
        self.with(|i| i.on_data = f);
    }

    /// Función que se llama cuando cambia la selección
    /// (`term.onSelectionChange`).
    pub fn set_on_selection_change(&mut self, f: Option<js_sys::Function>) {
        self.with(|i| i.on_selection = f);
    }

    /// Pega texto como si viniera del portapapeles (`term.paste`).
    pub fn paste(&mut self, text: &str) {
        self.dispatch(|i| i.paste(text));
    }

    /// Hay texto seleccionado (`term.hasSelection()`).
    pub fn has_selection(&self) -> bool {
        self.inner.try_borrow().is_ok_and(|i| i.has_selection())
    }

    /// Texto seleccionado (`term.getSelection()`).
    /// Without a local selection, the agent's own recent copy (OSC 52, last two
    /// minutes) is returned, so the "Copiar" button works for full-screen agents.
    pub fn get_selection(&self) -> String {
        self.inner
            .try_borrow()
            .map(|i| {
                let local = i.selection_text();
                if !local.is_empty() {
                    return local;
                }
                match &i.app_copied {
                    Some((at, text)) if i.now() - at < 120_000.0 => text.clone(),
                    _ => String::new(),
                }
            })
            .unwrap_or_default()
    }

    /// Quita la selección (`term.clearSelection()`).
    pub fn clear_selection(&mut self) {
        self.dispatch(Inner::clear_selection);
    }

    /// Copia la selección al portapapeles (el botón «Copiar» de la barra).
    /// `false` si no hay selección.
    pub fn copy_selection(&mut self) -> bool {
        self.dispatch(|i| {
            let has = i.has_selection();
            if has {
                i.outbox.copy = Some(i.selection_text());
            }
            has
        })
        .unwrap_or(false)
    }

    /// Modos que la página necesita, con los nombres de `term.modes` de
    /// xterm.js (`bracketedPasteMode`, `mouseTrackingMode`…).
    pub fn modes(&self) -> JsValue {
        let Ok(i) = self.inner.try_borrow() else {
            return JsValue::NULL;
        };
        let m = i.engine.modes();
        let obj = js_sys::Object::new();
        let tracking = match m.mouse {
            comandos_term::engine::MouseMode::Off => "none",
            comandos_term::engine::MouseMode::Click => "vt200",
            comandos_term::engine::MouseMode::Drag => "drag",
            comandos_term::engine::MouseMode::Motion => "any",
        };
        for (key, value) in [
            (
                "applicationCursorKeysMode",
                JsValue::from_bool(m.app_cursor),
            ),
            ("applicationKeypadMode", JsValue::from_bool(m.app_keypad)),
            ("bracketedPasteMode", JsValue::from_bool(m.bracketed_paste)),
            ("mouseTrackingMode", JsValue::from_str(tracking)),
            ("sendFocusMode", JsValue::from_bool(m.focus_events)),
        ] {
            let _ = js_sys::Reflect::set(&obj, &key.into(), &value);
        }
        obj.into()
    }

    /// Desplaza la vista por la historia (positivo = hacia atrás).
    pub fn scroll_lines(&mut self, lines: i32) {
        self.dispatch(|i| i.scroll_lines(lines));
    }

    /// Elemento `.xterm-screen` (para la entrada y los enlaces).
    pub fn screen(&self) -> Option<HtmlElement> {
        self.inner.try_borrow().ok().map(|i| i.dom.screen.clone())
    }

    /// Medidas actuales: `{cols, rows, cssCellWidth, cssCellHeight,
    /// deviceCellWidth, deviceCellHeight, dpr, cssCanvasWidth, cssCanvasHeight}`.
    pub fn dimensions(&self) -> JsValue {
        let Ok(i) = self.inner.try_borrow() else {
            return JsValue::NULL;
        };
        let obj = size_object(i.size.cols, i.size.rows);
        let m = &i.metrics;
        let (cw, ch) = m.css_canvas(i.size.cols, i.size.rows);
        // La celda CSS de addon-canvas (la que usan FitAddon y el ratón).
        let (cell_w, cell_h) = m.css_cell(i.size.cols, i.size.rows).unwrap_or((0.0, 0.0));
        for (key, value) in [
            ("cssCellWidth", cell_w),
            ("cssCellHeight", cell_h),
            ("deviceCellWidth", f64::from(m.dev_w)),
            ("deviceCellHeight", f64::from(m.dev_h)),
            ("dpr", m.dpr),
            ("cssCanvasWidth", cw),
            ("cssCanvasHeight", ch),
        ] {
            let _ = js_sys::Reflect::set(&obj, &key.into(), &JsValue::from_f64(value));
        }
        obj
    }

    /// No hay cuadro pedido ni nada pendiente de pintar (para las pruebas
    /// del presupuesto de CPU en reposo).
    pub fn is_idle(&self) -> bool {
        self.inner
            .try_borrow()
            .is_ok_and(|i| i.frame_id.is_none() && !i.scheduler.has_work())
    }

    /// Primer error de pintado desde la última consulta (`null` si no hubo).
    pub fn take_paint_error(&mut self) -> JsValue {
        self.with(|i| i.painter.take_error())
            .flatten()
            .unwrap_or(JsValue::NULL)
    }
}

impl Inner {
    /// Escuchas del teclado, el ratón, la rueda y el dedo.
    fn install_listeners(&mut self) {
        let weak = self.weak.clone();
        let textarea: EventTarget = self.dom.textarea.clone().into();
        let root: EventTarget = self.dom.root.clone().into();
        let screen: EventTarget = self.dom.screen.clone().into();
        let viewport: EventTarget = self.dom.viewport.clone().into();
        let table: [Handler<'_>; 23] = [
            (&textarea, "keydown", None, Inner::on_keydown),
            (
                &textarea,
                "compositionstart",
                None,
                Inner::on_composition_start,
            ),
            (
                &textarea,
                "compositionupdate",
                None,
                Inner::on_composition_update,
            ),
            (&textarea, "compositionend", None, Inner::on_composition_end),
            (&textarea, "beforeinput", None, Inner::on_before_input),
            (&textarea, "input", None, Inner::on_input),
            (&textarea, "paste", None, Inner::on_paste),
            (&textarea, "focus", None, Inner::on_focus),
            (&textarea, "blur", None, Inner::on_blur),
            (&root, "paste", None, Inner::on_paste),
            (&root, "copy", None, Inner::on_copy),
            (&root, "contextmenu", None, Inner::on_context_menu),
            (&root, "auxclick", None, Inner::on_aux_click),
            (&root, "wheel", Some(false), Inner::on_wheel),
            (&root, "touchstart", Some(true), Inner::on_touch_start),
            (&root, "touchmove", Some(false), Inner::on_touch_move),
            (&root, "touchend", Some(false), Inner::on_touch_end),
            (&root, "touchcancel", Some(true), Inner::on_touch_cancel),
            // `bindMouse` escucha en la raíz: la franja que deja `fit` también
            // da el foco e informa (acotado a la última celda).
            (&root, "mousedown", None, Inner::on_mouse_down),
            (&root, "mousemove", None, Inner::on_mouse_move),
            (&screen, "mouseup", None, Inner::on_mouse_up),
            (&screen, "mouseleave", None, Inner::on_mouse_leave),
            (&viewport, "scroll", None, Inner::on_viewport_scroll),
        ];
        for (target, kind, passive, handler) in table {
            if let Some(l) = listen(&weak, target, kind, passive, handler) {
                self.listeners.push(l);
            }
        }
    }

    /// El canvas aún no tiene tamaño (primer ajuste tras medir).
    fn painter_needs_layout(&self) -> bool {
        let (w, h) = self.metrics.device_canvas(self.size.cols, self.size.rows);
        let canvas = self.painter.element();
        canvas.width() != w || canvas.height() != h
    }
}

/// `('ontouchstart' in window) || navigator.maxTouchPoints > 0`, el
/// `IS_TOUCH` de `term.html`.
fn is_touch(window: &Window) -> bool {
    js_sys::Reflect::has(window, &"ontouchstart".into()).unwrap_or(false)
        || window.navigator().max_touch_points() > 0
}

fn size_object(cols: u16, rows: u16) -> JsValue {
    let obj = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&obj, &"cols".into(), &JsValue::from(cols));
    let _ = js_sys::Reflect::set(&obj, &"rows".into(), &JsValue::from(rows));
    obj.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_xterm_defaults() {
        let o = Options::default();
        assert_eq!(o.font_size, 15.0);
        assert_eq!(o.line_height, 1.0);
        assert!(!o.cursor_blink);
        assert_eq!(o.scrollback, 1000);
        assert!(o.draw_bold_text_in_bright_colors);
    }

    #[test]
    fn canvas_theme_takes_palette_colors() {
        let p = Palette::xterm_default([1; 3], [2; 3], [3; 3], [4; 3], [5; 3]);
        let t = canvas_theme(&p);
        assert_eq!((t.bg, t.cursor, t.cursor_accent), ([2; 3], [3; 3], [4; 3]));
    }

    #[test]
    fn selection_layer_is_the_theme_color_at_30_percent() {
        let mut t = Theme::default();
        assert_eq!(selection_css(&t), "rgba(255, 255, 255, 0.3)");
        t.selection = Some([0x2E, 0x38, 0x52]);
        assert_eq!(selection_css(&t), "rgba(46, 56, 82, 0.3)");
    }

    #[test]
    fn selected_glyphs_rasterize_against_the_opaque_selection() {
        let mut t = Theme::default();
        // Blanco con alfa 0x4D sobre negro (`DEFAULT_SELECTION`).
        assert_eq!(selection_opaque(&t), [0x4D, 0x4D, 0x4D]);
        t.background = Some([0x0A, 0x0D, 0x13]);
        assert_eq!(selection_opaque(&t), [84, 86, 90]);
        t.selection = Some([0x2E, 0x38, 0x52]);
        assert_eq!(selection_opaque(&t), [0x2E, 0x38, 0x52]);
    }

    #[test]
    fn ligatures_default_to_auto_with_the_addon_font() {
        let o = Options::default();
        assert_eq!(o.ligatures, None);
        assert_eq!(o.ligature_font_url, overlay::LIGA_FONT_URL);
    }

    #[test]
    fn absurd_font_sizes_are_rejected_or_clamped() {
        assert_eq!(font_size_in_range(14.0), Some(14.0));
        assert_eq!(font_size_in_range(0.25), Some(1.0));
        assert_eq!(font_size_in_range(1e9), Some(512.0));
        for bad in [0.0, -3.0, f64::NAN, f64::INFINITY] {
            assert_eq!(font_size_in_range(bad), None);
        }
    }
}
