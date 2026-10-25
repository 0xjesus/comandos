//! Terminal page controller: rendering, reconnection and parent messages.
//! Composer input shares the socket and its response acknowledgements.
use crate::{
    WebTerm,
    connection::{AckTracker, InputQueue, Reconnect},
    parent::{self, Command},
};
use comandos_web_dom::{events, timers};
use js_sys::{Array, Function, Promise, Reflect, Uint8Array};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{CloseEvent, HtmlElement, MessageEvent, ResizeObserver, WebSocket};

const FONT: &str = "'Ubuntu Sans Mono', 'JetBrainsMono Nerd Font Mono', 'JetBrainsMono Nerd Font', 'JetBrains Mono', 'DejaVu Sans Mono', monospace";
thread_local! { static PAGE: RefCell<Option<Rc<RefCell<Page>>>> = const { RefCell::new(None) }; }

#[path = "page_adapter.rs"]
mod adapter;

type Term = Rc<RefCell<WebTerm>>;
pub struct Page {
    ui: Option<crate::controls::TerminalControls>,
    data_hooks: Vec<Function>,
    resize_hooks: Vec<Function>,
    adapter_size: Option<(u16, u16)>,
    write_hooks: Vec<Function>,
    term: Option<Term>,
    socket: Option<WebSocket>,
    socket_id: u64,
    session: String,
    auth: String,
    base: String,
    theme: String,
    cursor_preference_changed: bool,
    queue: InputQueue,
    reconnect: Reconnect,
    acks: AckTracker,
    ack_callbacks: BTreeMap<u64, Function>,
    reconnect_pending: bool,
    down: bool,
    resizing: bool,
    disposed: bool,
    fit_timer: Option<timers::Handle>,
    last_size: Option<(u16, u16)>,
    last_interaction: f64,
    ctrl: bool,
    pasting: bool,
    docked: bool,
    known: bool,
    busy: bool,
    selecting: bool,
    debug: bool,
    listeners: Vec<events::Listener>,
    socket_listeners: Vec<events::Listener>,
    callbacks: Vec<Closure<dyn FnMut(JsValue)>>,
    observer: Option<ResizeObserver>,
    observer_cb: Option<Closure<dyn FnMut(Array, ResizeObserver)>>,
    ack_timers: BTreeMap<u64, timers::Handle>,
    /// Phone showing one pane of a split ([`crate::crop`]): the client attaches with
    /// `ignore-size` and the grid is the whole window.
    crop: Option<comandos_term::crop::Plan>,
    ignore_size: bool,
    /// Pane the parent asked for (`select-pane`), and the last pane listing.
    wanted_pane: Option<String>,
    panes: Vec<Value>,
    /// The person's font size (server preferences), restored when the crop ends.
    base_font: f64,
}
fn window() -> Result<web_sys::Window, JsValue> {
    web_sys::window().ok_or_else(|| "sin window".into())
}
fn element(id: &str) -> Option<HtmlElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id(id)?
        .dyn_into()
        .ok()
}
fn value(text: &str) -> JsValue {
    js_sys::JSON::parse(text).unwrap_or(JsValue::NULL)
}
fn string_field(obj: &JsValue, key: &str) -> Option<String> {
    Reflect::get(obj, &key.into()).ok()?.as_string()
}
fn now() -> f64 {
    js_sys::Date::now()
}
fn term(page: &Rc<RefCell<Page>>) -> Option<Term> {
    page.borrow().term.clone()
}
fn error(text: &str) {
    if let Some(e) = element("err") {
        e.set_text_content(Some(text));
        let _ = e
            .style()
            .set_property("display", if text.is_empty() { "none" } else { "block" });
    }
}
fn debug(page: &Rc<RefCell<Page>>, text: &str) {
    if page.borrow().debug
        && let Some(e) = element("dbg")
    {
        e.set_text_content(Some(text));
    }
}
fn post(kind: &str, extra: Option<(&str, &str)>) {
    let Ok(w) = window() else { return };
    let Ok(Some(p)) = w.parent() else { return };
    if p == w {
        return;
    }
    let mut data = json!({"source":"comandos-term","type":kind});
    if let Some((key, val)) = extra {
        data[key] = val.into();
    }
    if let Ok(origin) = w.location().origin() {
        let _ = p.post_message(&value(&data.to_string()), &origin);
    }
}
fn apply_buttons(style: &str) {
    if parent::button_style(style)
        && let Ok(w) = window()
        && let Some(root) = w.document().and_then(|d| d.document_element())
    {
        let _ = root.set_attribute("data-btn-style", style);
    }
}
fn apply_theme(page: &Rc<RefCell<Page>>, name: &str) {
    let Some(data) = crate::page_theme::theme(name) else {
        return;
    };
    let cfg = value(data);
    page.borrow_mut().theme = name.into();
    if let Some(t) = term(page) {
        t.borrow_mut().set_theme(cfg.clone());
    }
    let Ok(w) = window() else { return };
    let Some(doc) = w.document() else { return };
    if let Some(root) = doc
        .document_element()
        .and_then(|e| e.dyn_into::<HtmlElement>().ok())
    {
        for (css, key) in [
            ("bg", "background"),
            ("panel", "panel"),
            ("panel2", "panel2"),
            ("line", "line"),
            ("line2", "line2"),
            ("text", "foreground"),
            ("dim", "dim"),
            ("faint", "faint"),
            ("brand", "brand"),
        ] {
            if let Some(color) = string_field(&cfg, key) {
                let _ = root.style().set_property(&format!("--term-{css}"), &color);
            }
        }
        if let Some(bg) = string_field(&cfg, "background") {
            let _ = root.style().set_property("background", &bg);
            for el in [doc.body(), element("term-shell")].into_iter().flatten() {
                let _ = el.style().set_property("background", &bg);
            }
        }
    }
}
fn socket_open(p: &Page) -> bool {
    p.socket
        .as_ref()
        .is_some_and(|s| s.ready_state() == WebSocket::OPEN)
}
fn send_raw(p: &Page, data: &[u8]) -> bool {
    let Some(s) = p.socket.as_ref().filter(|_| socket_open(p)) else {
        return false;
    };
    let mut frame = Vec::with_capacity(data.len() + 1);
    frame.push(b'0');
    frame.extend_from_slice(data);
    s.send_with_u8_array(&frame).is_ok()
}
fn send_input(page: &Rc<RefCell<Page>>, data: &[u8], queue: bool) -> bool {
    let mut p = page.borrow_mut();
    if p.disposed {
        return false;
    }
    if !socket_open(&p) {
        if queue {
            if p.queue.push(data, now()) {
                return true;
            }
            error("Terminal desconectada y demasiado texto en espera. No se ha enviado.");
        } else {
            error("Terminal desconectada. El borrador se conserva; vuelve a enviar al conectar.");
        }
        return false;
    }
    if !send_raw(&p, data) {
        error("No se pudo enviar. El borrador se conserva.");
        return false;
    }
    true
}
fn settle(p: &mut Page, ids: Vec<u64>, ok: bool) {
    for id in ids {
        p.ack_timers.remove(&id);
        if let Some(cb) = p.ack_callbacks.remove(&id) {
            let _ = cb.call1(&JsValue::NULL, &JsValue::from_bool(ok));
        }
    }
}
/// Composer hook: no queued drafts, and success only after a response from this socket.
#[wasm_bindgen]
pub fn send_with_ack(data: &[u8], timeout_ms: u32) -> Promise {
    let bytes = data.to_vec();
    Promise::new(&mut |resolve, _reject| {
        PAGE.with(|slot| {
            let Some(page) = slot.borrow().clone() else {
                let _ = resolve.call1(&JsValue::NULL, &false.into());
                return;
            };
            if !send_input(&page, &bytes, false) {
                let _ = resolve.call1(&JsValue::NULL, &false.into());
                return;
            }
            let mut p = page.borrow_mut();
            let socket = p.socket_id;
            let id = p.acks.wait(socket, now(), f64::from(timeout_ms));
            p.ack_callbacks.insert(id, resolve.clone());
            let weak = Rc::downgrade(&page);
            let timer = timers::timeout(timeout_ms.min(i32::MAX as u32) as i32, move || {
                if let Some(page) = weak.upgrade() {
                    let mut p = page.borrow_mut();
                    let ids = p.acks.cancel(id);
                    settle(&mut p, ids, false);
                }
            });
            p.ack_timers.insert(id, timer);
        });
    })
}
fn schedule_fit(page: &Rc<RefCell<Page>>) {
    if page.borrow().crop.is_some() {
        crop_view(page);
    }
    let mut p = page.borrow_mut();
    p.fit_timer.take();
    if p.disposed {
        return;
    }
    let Some(t) = p.term.clone() else { return };
    let Ok(t) = t.try_borrow() else {
        let weak = Rc::downgrade(page);
        p.fit_timer = Some(timers::timeout(0, move || {
            if let Some(page) = weak.upgrade() {
                schedule_fit(&page);
            }
        }));
        return;
    };
    if t.has_selection() {
        return;
    }
    let Some((next, current)) = t.with(|i| (i.proposed_size(), i.size)) else {
        return;
    };
    let Some(next) = next else { return };
    if next == current {
        return;
    }
    let columns = next.cols != current.cols;
    drop(t);
    let weak = Rc::downgrade(page);
    p.fit_timer = Some(timers::timeout(
        if columns { 180 } else { 120 },
        move || {
            if let Some(page) = weak.upgrade() {
                page.borrow_mut().fit_timer.take();
                // Recheck dimensions: a delayed height fit must not reload columns.
                let changed_columns = term(&page)
                    .and_then(|t| {
                        t.borrow()
                            .with(|i| i.proposed_size().map(|next| next.cols != i.size.cols))
                    })
                    .flatten();
                if changed_columns.is_some_and(|value| value != columns) {
                    schedule_fit(&page);
                } else {
                    fit(&page);
                }
            }
        },
    ));
}
fn fit(page: &Rc<RefCell<Page>>) {
    if page.borrow().disposed {
        return;
    }
    if let Ok(Some(frame)) = window().and_then(|w| w.frame_element()) {
        let rect = frame.get_bounding_client_rect();
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
    }
    let Some(el) = element("term") else { return };
    let rect = el.get_bounding_client_rect();
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let Some(t) = term(page) else { return };
    if t.borrow().has_selection() {
        return;
    }
    let size = t.borrow_mut().resize_to_fit();
    let number = |key| {
        Reflect::get(&size, &JsValue::from_str(key))
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as u16
    };
    adapter::resized(page, size.clone());
    let size = (number("cols"), number("rows"));
    let mut p = page.borrow_mut();
    if size.0 < 2 || size.1 == 0 || p.last_size == Some(size) || !socket_open(&p) {
        return;
    }
    if p.last_size.is_some_and(|old| old.0 != size.0) {
        p.resizing = true;
        if let Some(socket) = &p.socket {
            let _ = socket.close_with_code_and_reason(1000, "resize");
        }
        return;
    }
    let frame = format!("1{}", json!({"columns":size.0,"rows":size.1}));
    if p.socket
        .as_ref()
        .is_some_and(|s| s.send_with_str(&frame).is_ok())
    {
        p.last_size = Some(size);
    }
}
fn schedule_connect(page: &Rc<RefCell<Page>>, immediate: bool) {
    let (delay, generation) = {
        let mut p = page.borrow_mut();
        if p.disposed || p.reconnect_pending {
            return;
        }
        p.reconnect_pending = true;
        (
            if immediate {
                0
            } else {
                p.reconnect.next_delay_ms()
            },
            p.socket_id,
        )
    };
    let weak = Rc::downgrade(page);
    timers::timeout_detached(delay as i32, move || {
        if let Some(page) = weak.upgrade() {
            let valid = {
                let mut p = page.borrow_mut();
                if p.disposed || p.socket_id != generation {
                    false
                } else {
                    p.reconnect_pending = false;
                    true
                }
            };
            if valid {
                connect(&page);
            }
        }
    });
}
fn connect(page: &Rc<RefCell<Page>>) {
    if page.borrow().disposed {
        return;
    }
    let url = {
        let p = page.borrow();
        let Ok(url) = web_sys::Url::new(&p.base) else {
            error("No pude conectar con la terminal. Reconectando…");
            return;
        };
        // Repeated args let a tty-only server select the session during upgrade.
        // V1 ignores them and reads the explicit session from init.
        if !p.auth.is_empty() {
            url.search_params().append("arg", &p.auth);
        }
        if !p.session.is_empty() {
            url.search_params().append("arg", &p.session);
        }
        // V1 authenticates during the HTTP upgrade, before its session init.
        // Browsers cannot attach an Authorization header to WebSocket.
        if !p.auth.is_empty() {
            url.search_params().append("token", &p.auth);
        }
        url.href()
    };
    let protocols = Array::new();
    for protocol in comandos_term::proto::PROTOCOLS {
        protocols.push(&JsValue::from_str(protocol));
    }
    let socket = match WebSocket::new_with_str_sequence(&url, &protocols) {
        Ok(s) => s,
        Err(_) => {
            error("No pude conectar con la terminal. Reconectando…");
            schedule_connect(page, false);
            return;
        }
    };
    socket.set_binary_type(web_sys::BinaryType::Arraybuffer);
    let id = {
        let mut p = page.borrow_mut();
        p.socket_listeners.clear();
        p.socket_id = p.socket_id.wrapping_add(1);
        p.socket = Some(socket.clone());
        p.last_size = None;
        p.socket_id
    };
    let weak = Rc::downgrade(page);
    let s = socket.clone();
    let opened = events::on(socket.as_ref(), "open", move |_| {
        let Some(page) = weak.upgrade() else { return };
        if !current(&page, id) {
            return;
        }
        let Some(t) = term(&page) else { return };
        let down = page.borrow().down;
        if down {
            t.borrow_mut().reset();
            t.borrow_mut()
                .write("\r\n\x1b[90m[conexión restablecida]\x1b[0m\r\n".as_bytes());
        }
        let size = t.borrow_mut().resize_to_fit();
        let n = |key| {
            Reflect::get(&size, &JsValue::from_str(key))
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as u16
        };
        let (cols, rows) = (n("cols"), n("rows"));
        adapter::resized(&page, size);
        let mut p = page.borrow_mut();
        p.down = false;
        p.reconnect.connected();
        p.last_size = Some((cols, rows));
        let init = if s.protocol() == "tty" {
            json!({"AuthToken":"","columns":cols,"rows":rows})
        } else if p.ignore_size {
            json!({"v":1,"cols":cols,"rows":rows,"session":p.session,"ignoreSize":true})
        } else {
            json!({"v":1,"cols":cols,"rows":rows,"session":p.session})
        };
        if s.send_with_str(&init.to_string()).is_err() {
            error("No se pudo enviar. El borrador se conserva.");
            return;
        }
        let drained = p.queue.drain(now());
        if drained.stale {
            error("Se descartó lo tecleado durante la desconexión: era demasiado antiguo.");
        } else {
            error("");
            for data in drained.items {
                if !send_raw(&p, &data) {
                    error("No se pudo enviar lo tecleado durante la reconexión.");
                    break;
                }
            }
        }
        drop(p);
        debug(&page, &format!("conectado · {}", s.protocol()));
        schedule_fit(&page);
    });
    let weak = Rc::downgrade(page);
    let message = events::on(socket.as_ref(), "message", move |event| {
        let Some(page) = weak.upgrade() else { return };
        if !current(&page, id) {
            return;
        }
        let Ok(event) = event.dyn_into::<MessageEvent>() else {
            return;
        };
        {
            let mut p = page.borrow_mut();
            let ids = p.acks.output(id);
            settle(&mut p, ids, true);
        }
        let data = event.data();
        let bytes = if let Some(text) = data.as_string() {
            text.into_bytes()
        } else if data.is_instance_of::<js_sys::ArrayBuffer>() {
            Uint8Array::new(&data).to_vec()
        } else {
            return;
        };
        let Some((&kind, payload)) = bytes.split_first() else {
            return;
        };
        let Some(t) = term(&page) else { return };
        match kind {
            b'0' => {
                t.borrow_mut().write(payload);
                adapter::output(&page);
                let replies = t.borrow_mut().take_replies();
                if !replies.is_empty() {
                    send_input(&page, &replies, false);
                }
            }
            b'1' => {
                if let Ok(w) = window()
                    && let Some(d) = w.document()
                {
                    d.set_title(&String::from_utf8_lossy(payload));
                }
            }
            b'2' => {
                let prefs = value(&String::from_utf8_lossy(payload));
                let (current_family, current_size) = t
                    .borrow()
                    .with(|i| (i.opts.font_family.clone(), i.opts.font_size))
                    .unwrap_or_else(|| (FONT.into(), 14.0));
                let family = string_field(&prefs, "fontFamily")
                    .filter(|s| !s.is_empty())
                    .unwrap_or(current_family);
                let size = Reflect::get(&prefs, &"fontSize".into())
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(current_size);
                // A cropped pane keeps its fitted size; the preference waits for the end.
                let cropped = {
                    let mut p = page.borrow_mut();
                    p.base_font = size;
                    p.crop.is_some()
                };
                t.borrow_mut().set_font(&family, if cropped { current_size } else { size });
                schedule_fit(&page);
            }
            _ => {}
        }
    });
    let weak = Rc::downgrade(page);
    let closed = events::on(socket.as_ref(), "close", move |event| {
        let Some(page) = weak.upgrade() else { return };
        if !current(&page, id) {
            return;
        }
        let code = event.dyn_ref::<CloseEvent>().map_or(0, CloseEvent::code);
        let ui = page.borrow().ui.clone();
        if let Some(ui) = ui {
            ui.disconnect();
        }
        if page.borrow().resizing {
            {
                let mut p = page.borrow_mut();
                p.resizing = false;
                let ids = p.acks.closed(id);
                settle(&mut p, ids, false);
            }
            if let Some(t) = term(&page) {
                t.borrow_mut().reset();
            }
            schedule_connect(&page, true);
            return;
        }
        let was_down = {
            let mut p = page.borrow_mut();
            let ids = p.acks.closed(id);
            settle(&mut p, ids, false);
            let down = p.down;
            p.down = true;
            down
        };
        if !was_down && let Some(t) = term(&page) {
            t.borrow_mut().write(
                format!("\r\n\x1b[90m[conexión cerrada · {code}; reconectando]\x1b[0m\r\n")
                    .as_bytes(),
            );
        }
        error("Terminal desconectada. Reconectando…");
        schedule_connect(&page, false);
    });
    let weak = Rc::downgrade(page);
    let failed = events::on(socket.as_ref(), "error", move |_| {
        if let Some(page) = weak.upgrade()
            && current(&page, id)
        {
            error("No pude conectar con la terminal. Reconectando…");
        }
    });
    page.borrow_mut().socket_listeners = vec![opened, message, closed, failed];
}
fn current(page: &Rc<RefCell<Page>>, id: u64) -> bool {
    let p = page.borrow();
    !p.disposed && p.socket_id == id
}
fn interaction(page: &Rc<RefCell<Page>>, known: bool, busy: bool, selecting: bool) {
    {
        let mut p = page.borrow_mut();
        p.known = known;
        p.busy = busy;
        p.selecting = selecting;
    }
    if let Ok(w) = window()
        && let Some(button) = w
            .document()
            .and_then(|d| d.query_selector("[data-action='mode']").ok().flatten())
    {
        let _ = button.set_attribute(
            "aria-pressed",
            if known {
                if selecting { "true" } else { "false" }
            } else {
                "mixed"
            },
        );
        if !known || busy {
            let _ = button.set_attribute("disabled", "");
        } else {
            let _ = button.remove_attribute("disabled");
        }
        let _ = button
            .class_list()
            .toggle_with_force("on", known && selecting);
        button.set_text_content(Some(if !known {
            "Comprobando…"
        } else if selecting {
            "Interactuar"
        } else {
            "Seleccionar"
        }));
    }
    if let Some(ui) = page.borrow().ui.as_ref() {
        ui.refresh_interaction();
    }
}
fn set_ctrl(page: &Rc<RefCell<Page>>, armed: bool) {
    page.borrow_mut().ctrl = armed;
    if let Ok(w) = window()
        && let Some(button) = w
            .document()
            .and_then(|d| d.query_selector("[data-action='ctrl']").ok().flatten())
    {
        let _ = button.class_list().toggle_with_force("on", armed);
        let _ = button.set_attribute("aria-pressed", if armed { "true" } else { "false" });
    }
}
fn handle(page: &Rc<RefCell<Page>>, command: Command) {
    match command {
        Command::Theme(name) => apply_theme(page, &name),
        Command::CursorStyle(style) => {
            page.borrow_mut().cursor_preference_changed = true;
            if let Some(t) = term(page) {
                t.borrow_mut().set_cursor_style(&style);
            }
        }
        Command::ButtonStyle(style) => apply_buttons(&style),
        Command::Session(session) => switch_session(page, &session),
        Command::SelectPane(pane) => {
            page.borrow_mut().wanted_pane = Some(pane.clone());
            crop_update(page);
            select_pane(page, pane)
        }
        Command::Interaction {
            known,
            busy,
            selecting,
        } => interaction(page, known, busy, selecting),
        Command::Key(key) => {
            if let Some(bytes) = parent::toolbar_key(&key) {
                send_input(page, bytes, true);
            }
            if !page.borrow().docked
                && let Some(t) = term(page)
            {
                t.borrow().focus();
            }
        }
        Command::Paste(text) => {
            if text.is_empty() {
                return;
            }
            if !socket_open(&page.borrow()) {
                error("Terminal desconectada. El texto no se ha enviado.");
                return;
            }
            page.borrow_mut().pasting = true;
            let docked = page.borrow().docked;
            if let Some(t) = term(page) {
                t.borrow_mut().paste(&text);
                if !docked {
                    t.borrow().focus();
                }
            }
            page.borrow_mut().pasting = false;
        }
        Command::Ctrl => {
            let ui = page.borrow().ui.clone();
            if let Some(ui) = ui {
                ui.set_mobile(false);
            }
            set_ctrl(page, true);
            if let Some(t) = term(page) {
                t.borrow().focus();
            }
        }
        Command::Focus => {
            let ui = page.borrow().ui.clone();
            if let Some(ui) = ui {
                ui.focus();
                return;
            }
            if let Some(t) = term(page) {
                t.borrow().focus();
            }
        }
        Command::Dock(on) => {
            page.borrow_mut().docked = on;
            if let Some(root) = window().ok().and_then(|w| w.document()).and_then(|d| d.document_element()) {
                let _ = root.class_list().toggle_with_force("docked", on);
            }
        }
        Command::Action(action) => {
            if let Some(button) = window()
                .ok()
                .and_then(|w| w.document())
                .and_then(|d| d.query_selector(&format!("[data-action='{action}']")).ok().flatten())
                .and_then(|e| e.dyn_into::<HtmlElement>().ok())
            {
                button.click();
            }
        }
        Command::Mode(selecting) => {
            let p = page.borrow();
            if p.known && !p.busy && p.selecting != selecting {
                let Ok(w) = window() else { return };
                if let Ok(Some(parent)) = w.parent()
                    && let Ok(origin) = w.location().origin()
                {
                    let _=parent.post_message(&value(&json!({"source":"comandos-term","type":"interaction-request","selecting":!p.selecting}).to_string()),&origin);
                }
            }
        }
    }
}
fn switch_session(page: &Rc<RefCell<Page>>, session: &str) {
    let old = {
        let mut p = page.borrow_mut();
        if p.session == session || p.disposed {
            return;
        }
        p.session = session.into();
        p.socket_id = p.socket_id.wrapping_add(1);
        p.down = false;
        p.resizing = false;
        p.reconnect_pending = false;
        p.reconnect.connected();
        p.queue.drain(now());
        let ids = p.acks.expire(f64::INFINITY);
        settle(&mut p, ids, false);
        p.socket.take()
    };
    if let Some(s) = old {
        let _ = s.close_with_code_and_reason(1000, "session");
    }
    if let Some(t) = term(page) {
        t.borrow_mut().reset();
    }
    schedule_connect(page, true);
}
fn dispose(page: &Rc<RefCell<Page>>) {
    let ui = {
        let mut p = page.borrow_mut();
        if p.disposed {
            return;
        }
        p.ui.take()
    };
    // Subscription disposal and gesture release can re-enter the live adapter.
    crate::pane_chrome::dispose();
    if let Some(ui) = ui {
        ui.dispose();
    }
    let mut p = page.borrow_mut();
    p.disposed = true;
    p.data_hooks.clear();
    p.resize_hooks.clear();
    p.write_hooks.clear();
    if let Some(s) = p.socket.take() {
        let _ = s.close_with_code_and_reason(1000, "pagehide");
    }
    if let Some(observer) = p.observer.take() {
        observer.disconnect();
    }
    p.ack_timers.clear();
    p.fit_timer.take();
    let ids = p.acks.expire(f64::INFINITY);
    settle(&mut p, ids, false);
    if let Some(t) = p.term.take() {
        t.borrow_mut().set_on_data(None);
        t.borrow_mut().set_on_selection_change(None);
    }
    // Retain the callback currently on the stack; all listeners disappear when
    // the next boot drops this disposed controller. Sockets/timers/rendering stop now.
}
async fn delay(ms: i32) {
    let promise = Promise::new(&mut |resolve, _| {
        timers::timeout_detached(ms, move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
    });
    let _ = JsFuture::from(promise).await;
}
async fn pane_request(
    session: &str,
    auth: &str,
    action: &str,
    pane: Option<&Value>,
) -> Result<Value, JsValue> {
    if session.is_empty() || auth.is_empty() {
        return Err("Abre la terminal desde ComandOS para administrar sus paneles.".into());
    }
    let mut body = json!({"session":session,"action":action});
    if let Some(pane) = pane {
        body["pane"] = pane["id"].clone();
        body["identity"] = pane["identity"].clone();
    }
    if action == "select" {
        body["scope"] = "client".into();
    }
    let result = page_request(auth, "/terminal-panes", Some(body)).await?;
    if result["ok"] != true || !result["panes"].is_array() {
        return Err(result["error"]
            .as_str()
            .unwrap_or("No se pudo consultar el estado de los paneles")
            .into());
    }
    Ok(result)
}
async fn page_request(auth: &str, path: &str, body: Option<Value>) -> Result<Value, JsValue> {
    let abort = web_sys::AbortController::new()?;
    let signal = abort.signal();
    let timeout = timers::timeout(10000, move || abort.abort());
    let result = crate::pane_chrome::fetch_api(
        auth.to_owned(),
        path,
        body,
        Some("No se pudo consultar el estado de los paneles"),
        Some(signal),
    )
    .await
    .map_err(JsValue::from);
    drop(timeout);
    result
}
fn select_pane(page: &Rc<RefCell<Page>>, pane: String) {
    let weak = Rc::downgrade(page);
    let (session, auth, generation) = {
        let p = page.borrow();
        (p.session.clone(), p.auth.clone(), p.socket_id)
    };
    spawn_local(async move {
        for _ in 0..20 {
            let Some(page) = weak.upgrade() else { return };
            if !current(&page, generation) {
                return;
            }
            if socket_open(&page.borrow()) {
                break;
            }
            delay(250).await;
        }
        let result = async {
            let listing = pane_request(&session, &auth, "list", None).await?;
            let chosen = listing["panes"]
                .as_array()
                .and_then(|items| items.iter().find(|p| p["id"].as_str() == Some(&pane)))
                .ok_or_else(|| {
                    JsValue::from_str("Ese panel ya no existe. El aviso sigue en la franja.")
                })?;
            pane_request(&session, &auth, "select", Some(chosen)).await
        }
        .await;
        let Some(page) = weak.upgrade() else { return };
        if !current(&page, generation) {
            return;
        }
        match result {
            Ok(result) => {
                if let Some(keys) = result["clientKeys"].as_str()
                    && !send_input(&page, keys.as_bytes(), false)
                {
                    return;
                }
                post("pane-selected", Some(("pane", &pane)));
            }
            Err(err) => error(&err.as_string().unwrap_or_else(|| {
                string_field(&err, "message")
                    .unwrap_or_else(|| "No se pudo consultar el estado de los paneles".into())
            })),
        }
    });
}
/// Phones get the pane chips in the parent; the frame follows the same breakpoint
/// (a cross-origin parent cannot be read: then the frame's own width decides).
fn phone() -> bool {
    let Ok(w) = window() else { return false };
    let own = w.inner_width().ok().and_then(|v| v.as_f64());
    let top = w
        .parent()
        .ok()
        .flatten()
        .filter(|p| *p != w)
        .and_then(|p| Reflect::get(&p, &"innerWidth".into()).ok())
        .and_then(|v| v.as_f64());
    top.or(own).is_some_and(|width| width <= comandos_term::crop::PHONE_WIDTH)
}
/// The pane chrome's latest `/terminal-panes` listing (no extra request to tmux).
pub fn panes_seen(panes: &[Value]) {
    let Some(page) = PAGE.with(|slot| slot.borrow().clone()) else { return };
    if page.borrow().disposed {
        return;
    }
    page.borrow_mut().panes = panes.to_vec();
    crop_update(&page);
}
/// The pane a phone shows alone, if any (history and copies use it).
pub fn crop_pane() -> Option<String> {
    PAGE.with(|slot| slot.borrow().as_ref().and_then(|p| p.try_borrow().ok()?.crop.as_ref().map(|c| c.pane.clone())))
}
fn content_size(el: &HtmlElement) -> (f64, f64) {
    let style = window().ok().and_then(|w| w.get_computed_style(el).ok().flatten());
    let px = |name: &str| {
        style
            .as_ref()
            .and_then(|s| s.get_property_value(name).ok())
            .and_then(|v| v.trim_end_matches("px").trim().parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    (px("width"), px("height"))
}
fn screen_style(t: &Term, view: Option<(String, String)>) {
    let Some(screen) = t.try_borrow().ok().and_then(|t| t.screen()) else { return };
    let style = screen.style();
    match view {
        Some((transform, clip)) => {
            let _ = style.set_property("transform-origin", "0 0");
            let _ = style.set_property("transform", &transform);
            let _ = style.set_property("clip-path", &clip);
        }
        None => {
            for name in ["transform-origin", "transform", "clip-path"] {
                let _ = style.remove_property(name);
            }
        }
    }
}
/// Starts, moves or ends the one-pane view after a listing or a `select-pane`.
fn crop_update(page: &Rc<RefCell<Page>>) {
    let (plan, was) = {
        let p = page.borrow();
        if p.disposed {
            return;
        }
        (comandos_term::crop::plan(&p.panes, p.wanted_pane.as_deref(), phone()), p.crop.clone())
    };
    if plan == was {
        return;
    }
    let Some(t) = term(page) else { return };
    {
        // Busy terminal: the next listing (2 s) tries again.
        let Ok(mut tb) = t.try_borrow_mut() else { return };
        match &plan {
            Some(c) => {
                tb.set_fixed_size(c.cols, c.rows);
                tb.set_clip_columns(c.left, c.left.saturating_add(c.width));
            }
            None => {
                tb.set_fixed_size(0, 0);
                tb.set_clip_columns(0, 0);
            }
        }
    }
    let reconnect = plan.is_some() != was.is_some();
    let base = {
        let mut p = page.borrow_mut();
        p.crop = plan.clone();
        p.ignore_size = plan.is_some();
        p.base_font
    };
    if let Some(root) = window().ok().and_then(|w| w.document()).and_then(|d| d.document_element()) {
        let _ = root.class_list().toggle_with_force("cropped", plan.is_some());
    }
    if plan.is_none() {
        screen_style(&t, None);
        if let Ok(mut tb) = t.try_borrow_mut() {
            let family = tb.with(|i| i.opts.font_family.clone()).unwrap_or_else(|| FONT.into());
            tb.set_font(&family, base);
        }
    }
    if reconnect {
        // The attach flags change: a new client, exactly like a column change.
        let mut p = page.borrow_mut();
        if socket_open(&p) {
            p.resizing = true;
            if let Some(socket) = &p.socket {
                let _ = socket.close_with_code_and_reason(1000, "resize");
            }
        }
        drop(p);
        crop_view(page);
    } else {
        schedule_fit(page);
    }
}
/// Fits the chosen pane to the frame: font from its width and height, then the
/// window's cells shifted and clipped to it.
fn crop_view(page: &Rc<RefCell<Page>>) {
    let (plan, base) = {
        let p = page.borrow();
        (p.crop.clone(), p.base_font)
    };
    let (Some(plan), Some(t), Some(host)) = (plan, term(page), element("term")) else {
        return;
    };
    let host_size = content_size(&host);
    if host_size.0 <= 0.0 || host_size.1 <= 0.0 {
        return;
    }
    let measure = |t: &Term| {
        t.try_borrow().ok()?.with(|i| {
            let (cw, ch) = i.metrics.css_cell(i.size.cols, i.size.rows).unwrap_or((0.0, 0.0));
            (i.opts.font_size, i.opts.font_family.clone(), cw, ch)
        })
    };
    let Some((font, family, cw, ch)) = measure(&t) else { return };
    if cw <= 0.0 || ch <= 0.0 {
        return;
    }
    let target = comandos_term::crop::font_size(base, (cw / font, ch / font), host_size, (plan.width, plan.height));
    let mut cell = (cw, ch);
    if (target - font).abs() >= 0.25 {
        if let Ok(mut tb) = t.try_borrow_mut() {
            tb.set_font(&family, target);
        }
        // Cells round to device pixels: step down until the pane really fits.
        for _ in 0..8 {
            let Some((now, _, w, h)) = measure(&t) else { break };
            cell = (w, h);
            if f64::from(plan.width) * w <= host_size.0 + 0.5 || now <= comandos_term::crop::MIN_FONT {
                break;
            }
            if let Ok(mut tb) = t.try_borrow_mut() {
                tb.set_font(&family, (now - 0.25).max(comandos_term::crop::MIN_FONT));
            }
        }
    }
    screen_style(&t, Some(comandos_term::crop::view_css(&plan, cell)));
    if (target - font).abs() >= 0.25 {
        // The new size finishes loading asynchronously and may re-measure the cell.
        let weak = Rc::downgrade(page);
        timers::timeout_detached(400, move || {
            if let Some(page) = weak.upgrade()
                && page.borrow().crop.is_some()
                && let (Some(plan), Some(t)) = (page.borrow().crop.clone(), term(&page))
                && let Some(cell) = t.try_borrow().ok().and_then(|t| {
                    t.with(|i| i.metrics.css_cell(i.size.cols, i.size.rows)).flatten()
                })
            {
                screen_style(&t, Some(comandos_term::crop::view_css(&plan, cell)));
            }
        });
    }
}
/// Whether a pointer at `(x, y)` (client px) falls outside the pane shown alone:
/// those cells belong to hidden panes and a tap there would focus one of them.
fn outside_crop(page: &Rc<RefCell<Page>>, x: f64, y: f64) -> bool {
    let Some(plan) = page.borrow().crop.clone() else { return false };
    let Some(t) = term(page) else { return false };
    let Some((screen, cell)) = t.try_borrow().ok().and_then(|t| {
        Some((t.screen()?, t.with(|i| i.metrics.css_cell(i.size.cols, i.size.rows)).flatten()?))
    }) else {
        return false;
    };
    let r = screen.get_bounding_client_rect();
    let (x0, y0) = (r.left() + f64::from(plan.left) * cell.0, r.top() + f64::from(plan.top) * cell.1);
    let (x1, y1) = (x0 + f64::from(plan.width) * cell.0, y0 + f64::from(plan.height) * cell.1);
    !(x >= x0 && x < x1 && y >= y0 && y < y1)
}
/// Generic web-build loader entrypoint.
#[wasm_bindgen]
pub fn boot(k: &str) -> Result<(), JsValue> {
    boot_term(k)
}
#[wasm_bindgen]
pub fn boot_term(k: &str) -> Result<(), JsValue> {
    if window()?
        .document()
        .and_then(|d| {
            d.query_selector("meta[name='comandos-web-mode']")
                .ok()
                .flatten()
        })
        .and_then(|m| m.get_attribute("content"))
        .as_deref()
        == Some("native-term")
    {
        native_boot(k);
        return Ok(());
    }
    if element("term").is_none()
        && window()?
            .document()
            .is_some_and(|d| d.ready_state() == "loading")
    {
        // The web gate pauses parsing in head. Acknowledge the registered
        // controller first, then attach its DOM once the document is ready.
        comandos_web_dom::dom::on_ready(|| {
            if let Err(err) = attach() {
                error(&err.as_string().unwrap_or_else(|| "sin #term".into()));
            }
        });
        ready(k)?;
        return Ok(());
    }
    attach()?;
    ready(k)
}
thread_local! {static NATIVE_BOOTED:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
fn native_report(k: &str, mounted: &[&str], failed: Option<(&str, &JsValue)>, phase: &str) {
    // These entries are already Values. Re-serializing them through json!
    // retains serde's RawValue emitter and its full Value deserializer.
    let mut report = json!({"k":k,"mounted":mounted,"failed":[]});
    report["failed"] = Value::Array(failed.map(|(id,e)|vec![json!({"id":id,"error":string_field(e,"message").or_else(||e.as_string()).unwrap_or_else(||"terminal boot failed".into())})]).unwrap_or_default());
    let _ = Reflect::set(
        &js_sys::global(),
        &"__comandosBootReport".into(),
        &value(&report.to_string()),
    );
    let _ = Reflect::set(
        &js_sys::global(),
        &"__comandosBootState".into(),
        &phase.into(),
    );
}
fn native_boot(k: &str) {
    if NATIVE_BOOTED.with(|s| s.replace(true)) {
        return;
    }
    let k = k.to_owned();
    native_report(&k, &[], None, "waiting-dom");
    comandos_web_dom::dom::on_ready(move || {
        native_report(&k, &[], None, "mounting");
        if let Err(e) = attach_main() {
            native_report(&k, &[], Some(("term-main", &e)), "failed");
            PAGE.with(|slot| {
                if let Some(p) = slot.borrow_mut().take() {
                    dispose(&p);
                }
            });
            return;
        }
        if let Err(e) = crate::pane_chrome::attach() {
            native_report(&k, &["term-main"], Some(("term-tail", &e)), "failed");
            return;
        }
        native_report(&k, &["term-main", "term-tail"], None, "reporting");
        let k = k.clone();
        spawn_local(async move {
            let result = async {
                let init = web_sys::RequestInit::new();
                init.set_method("POST");
                let headers = web_sys::Headers::new()?;
                headers.set("Content-Type", "application/json")?;
                init.set_headers(&headers);
                init.set_body(&JsValue::from_str(
                    &json!({"k":k,"mounted":["term-main","term-tail"],"failed":[]}).to_string(),
                ));
                let response =
                    JsFuture::from(window()?.fetch_with_str_and_init("/web/ready", &init)).await?;
                if Reflect::get(&response, &"ok".into())? != JsValue::TRUE {
                    return Err(
                        js_sys::Error::new("native terminal readiness rejected by server").into(),
                    );
                }
                Ok::<(), JsValue>(())
            }
            .await;
            if let Err(e) = result {
                native_report(
                    &k,
                    &["term-main", "term-tail"],
                    Some(("@ready", &e)),
                    "failed",
                );
                return;
            }
            let _ = Reflect::set(
                &js_sys::global(),
                &"__comandosAttached".into(),
                &JsValue::TRUE,
            );
            let _ = Reflect::set(&js_sys::global(), &"__comandosReady".into(), &JsValue::TRUE);
            native_report(&k, &["term-main", "term-tail"], None, "ready");
        });
    });
}
fn ready(k: &str) -> Result<(), JsValue> {
    let w = window()?;
    Reflect::set(&w, &"__comandosReady".into(), &JsValue::TRUE)?;
    if k.is_empty() {
        return Ok(());
    }
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    let headers = web_sys::Headers::new()?;
    headers.set("Content-Type", "application/json")?;
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(
        &json!({"k":k,"mounted":["term"],"failed":[]}).to_string(),
    ));
    let promise = w.fetch_with_str_and_init("/web/ready", &init);
    spawn_local(async move {
        let _ = JsFuture::from(promise).await;
    });
    Ok(())
}
fn load_cursor_preference(page: &Rc<RefCell<Page>>) {
    let weak = Rc::downgrade(page);
    let auth = page.borrow().auth.clone();
    spawn_local(async move {
        if let Ok(prefs) = page_request(&auth, "/prefs", None).await
            && let Some(page) = weak.upgrade()
            && !page.borrow().disposed
            && !page.borrow().cursor_preference_changed
            && let Some(style) = prefs["cursor_shape"].as_str()
            && let Some(t) = term(&page)
        {
            t.borrow_mut().set_cursor_style(style);
        }
    });
}
fn attach() -> Result<(), JsValue> {
    attach_main()?;
    crate::pane_chrome::attach()
}
fn attach_main() -> Result<(), JsValue> {
    PAGE.with(|slot| {
        if let Some(old) = slot.borrow_mut().take() {
            dispose(&old);
        }
    });
    let w = window()?;
    let params = web_sys::UrlSearchParams::new_with_str(&w.location().search()?)?;
    let session = params.get("arg").unwrap_or_default();
    let auth = params.get("auth").unwrap_or_default();
    // Match the original String.toLowerCase at the browser boundary instead
    // of retaining Rust's Unicode case tables solely for this query value.
    let theme = String::from(
        js_sys::JsString::from(params.get("theme").unwrap_or_else(|| "noche".into()))
            .to_lower_case(),
    );
    let base = params
        .get("ws")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let host = w.location().host().unwrap_or_default();
            if host.is_empty() {
                return "ws://127.0.0.1:4779/ws".into();
            }
            let path = w.location().pathname().unwrap_or_default();
            let path = path.rsplit_once('/').map_or("", |(p, _)| p);
            format!(
                "{}://{host}{path}/ws",
                if w.location().protocol().ok().as_deref() == Some("https:") {
                    "wss"
                } else {
                    "ws"
                }
            )
        });
    let inherited = w
        .parent()
        .ok()
        .flatten()
        .filter(|p| *p != w)
        .and_then(|p| Reflect::get(&p, &"document".into()).ok())
        .and_then(|d| d.dyn_into::<web_sys::Document>().ok())
        .and_then(|d| d.document_element())
        .and_then(|e| e.get_attribute("data-btn-style"));
    apply_buttons(
        &params
            .get("btn")
            .filter(|s| !s.is_empty())
            .or(inherited)
            .unwrap_or_else(|| "sutil".into()),
    );
    let host = element("term").ok_or_else(|| JsValue::from_str("sin #term"))?;
    let mut options = json!({"fontFamily":FONT,"fontSize":14,"lineHeight":1.2,"letterSpacing":0,"cursorBlink":true,"cursorStyle":"bar","scrollback":10000});
    options["theme"] = comandos_web_dom::port::to_json(&value(
        crate::page_theme::theme(&theme)
            .unwrap_or(crate::page_theme::THEMES.first().map_or("{}", |(_, v)| v)),
    ));
    let opts = value(&options.to_string());
    let term = Rc::new(RefCell::new(WebTerm::new(host.clone(), opts)?));
    let page = Rc::new(RefCell::new(Page {
        term: Some(term.clone()),
        ui: None,
        data_hooks: Vec::new(),
        resize_hooks: Vec::new(),
        adapter_size: None,
        write_hooks: Vec::new(),
        socket: None,
        socket_id: 0,
        session,
        auth,
        base,
        theme: theme.clone(),
        cursor_preference_changed: false,
        queue: InputQueue::new(4096, 15000.0),
        reconnect: Reconnect::default(),
        acks: AckTracker::default(),
        ack_callbacks: BTreeMap::new(),
        reconnect_pending: false,
        down: false,
        resizing: false,
        disposed: false,
        fit_timer: None,
        last_size: None,
        last_interaction: f64::NEG_INFINITY,
        ctrl: false,
        pasting: false,
        docked: false,
        known: false,
        busy: false,
        selecting: false,
        debug: params.get("debug").as_deref() == Some("1"),
        listeners: Vec::new(),
        socket_listeners: Vec::new(),
        callbacks: Vec::new(),
        observer: None,
        observer_cb: None,
        ack_timers: BTreeMap::new(),
        crop: None,
        ignore_size: false,
        wanted_pane: None,
        panes: Vec::new(),
        base_font: 14.0,
    }));
    if page.borrow().debug
        && let Some(dbg) = element("dbg")
    {
        let _ = dbg.class_list().add_1("on");
    }
    apply_theme(
        &page,
        if crate::page_theme::theme(&theme).is_some() {
            &theme
        } else {
            "noche"
        },
    );
    if let Some(toolbar) = element("term-toolbar") {
        let touch = Reflect::has(&w, &"ontouchstart".into()).unwrap_or(false)
            || w.navigator().max_touch_points() > 0;
        toolbar.set_hidden(
            !touch
                && (w.location().protocol()?.as_str() == "file:"
                    || page.borrow().session.is_empty()),
        );
    }
    interaction(&page, false, false, false);
    let weak = Rc::downgrade(&page);
    let data = Closure::<dyn FnMut(JsValue)>::new(move |data: JsValue| {
        if let Some(page) = weak.upgrade() {
            let bytes = Uint8Array::new(&data).to_vec();
            adapter::data(&page, &bytes);
            let converted = {
                let p = page.borrow();
                if p.ctrl && !p.pasting {
                    parent::control_byte(&bytes)
                } else {
                    None
                }
            };
            if let Some(byte) = converted {
                if send_input(&page, &[byte], true) {
                    set_ctrl(&page, false);
                }
            } else {
                send_input(&page, &bytes, true);
            }
        }
    });
    term.borrow_mut()
        .set_on_data(Some(data.as_ref().unchecked_ref::<Function>().clone()));
    let weak = Rc::downgrade(&page);
    let selection = Closure::<dyn FnMut(JsValue)>::new(move |_| {
        if let Some(page) = weak.upgrade() {
            schedule_fit(&page);
        }
    });
    term.borrow_mut()
        .set_on_selection_change(Some(selection.as_ref().unchecked_ref::<Function>().clone()));
    page.borrow_mut().callbacks = vec![data, selection];
    let _ = Reflect::set(&w, &"__comandosOwnsTouchGestures".into(), &JsValue::TRUE);
    let mut listeners = Vec::new();
    for (kind, prefix) in [("error", "JS error: "), ("unhandledrejection", "rej: ")] {
        let weak = Rc::downgrade(&page);
        listeners.push(events::on(w.as_ref(), kind, move |event| {
            if let Some(page) = weak.upgrade()
                && !page.borrow().disposed
            {
                let message = string_field(event.as_ref(), "message")
                    .or_else(|| {
                        Reflect::get(event.as_ref(), &"reason".into())
                            .ok()
                            .and_then(|v| string_field(&v, "message").or_else(|| v.as_string()))
                    })
                    .unwrap_or_default();
                debug(&page, &format!("{prefix}{message}"));
            }
        }));
    }
    let weak = Rc::downgrade(&page);
    listeners.push(events::on(w.as_ref(), "message", move |event| {
        let Some(page) = weak.upgrade() else { return };
        if page.borrow().disposed {
            return;
        }
        let Ok(event) = event.dyn_into::<MessageEvent>() else {
            return;
        };
        let Ok(w) = window() else { return };
        let Ok(Some(parent)) = w.parent() else { return };
        if w.location().origin().ok().as_deref() != Some(event.origin().as_str())
            || event
                .source()
                .is_none_or(|source| JsValue::from(source) != JsValue::from(parent.clone()))
        {
            return;
        }
        if let Ok(data) = js_sys::JSON::stringify(&event.data())
            && let Some(data) = data
                .as_string()
                .and_then(|s| comandos_web_dom::api::serialized_value(&s, false))
            && let Some(command) = parent::command(&data)
        {
            handle(&page, command);
        }
    }));
    for kind in ["keydown", "pointerdown"] {
        let weak = Rc::downgrade(&page);
        listeners.push(events::on_with(
            w.as_ref(),
            kind,
            events::Options {
                capture: true,
                ..Default::default()
            },
            move |event| {
                let Some(page) = weak.upgrade() else { return };
                let mut p = page.borrow_mut();
                if !event.is_trusted() || p.disposed || now() - p.last_interaction < 5000.0 {
                    return;
                }
                p.last_interaction = now();
                post("user-interaction", None);
            },
        ));
    }
    // Before every other handler (window, capture): pointers on the hidden cells
    // around a cropped pane go nowhere.
    for kind in ["pointerdown", "mousedown", "mouseup", "click", "dblclick", "contextmenu", "wheel", "touchstart"] {
        let weak = Rc::downgrade(&page);
        listeners.push(events::on_with(
            w.as_ref(),
            kind,
            events::Options {
                capture: true,
                ..Default::default()
            },
            move |event| {
                let Some(page) = weak.upgrade() else { return };
                if page.borrow().crop.is_none() {
                    return;
                }
                let inside_term = event
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                    .and_then(|e| e.closest("#term").ok().flatten())
                    .is_some();
                if !inside_term {
                    return;
                }
                let point = if let Some(touch) = event.dyn_ref::<web_sys::TouchEvent>() {
                    touch.touches().get(0).map(|t| (f64::from(t.client_x()), f64::from(t.client_y())))
                } else {
                    event
                        .dyn_ref::<web_sys::MouseEvent>()
                        .map(|m| (f64::from(m.client_x()), f64::from(m.client_y())))
                };
                if let Some((x, y)) = point
                    && outside_crop(&page, x, y)
                {
                    event.stop_immediate_propagation();
                    if event.cancelable() && event.type_() != "touchstart" {
                        event.prevent_default();
                    }
                }
            },
        ));
    }
    let weak = Rc::downgrade(&page);
    listeners.push(events::on(w.as_ref(), "resize", move |_| {
        if let Some(page) = weak.upgrade() {
            schedule_fit(&page);
        }
    }));
    if let Some(viewport) = w.visual_viewport() {
        let weak = Rc::downgrade(&page);
        listeners.push(events::on(viewport.as_ref(), "resize", move |_| {
            if let Some(page) = weak.upgrade() {
                schedule_fit(&page);
            }
        }));
    }
    let weak = Rc::downgrade(&page);
    listeners.push(events::on(w.as_ref(), "pagehide", move |_| {
        if let Some(page) = weak.upgrade() {
            dispose(&page);
            let weak = Rc::downgrade(&page);
            timers::timeout_detached(0, move || {
                if let Some(page) = weak.upgrade() {
                    let mut p = page.borrow_mut();
                    p.listeners.clear();
                    p.socket_listeners.clear();
                    p.callbacks.clear();
                    p.observer_cb.take();
                }
            });
        }
    }));
    let weak = Rc::downgrade(&page);
    let observer_cb = Closure::<dyn FnMut(Array, ResizeObserver)>::new(move |_, _| {
        if let Some(page) = weak.upgrade() {
            schedule_fit(&page);
        }
    });
    let observer = ResizeObserver::new(observer_cb.as_ref().unchecked_ref())?;
    observer.observe(&host);
    {
        let mut p = page.borrow_mut();
        p.listeners = listeners;
        p.observer = Some(observer);
        p.observer_cb = Some(observer_cb);
    }
    PAGE.with(|slot| *slot.borrow_mut() = Some(page.clone()));
    let adapter = adapter::publish(&page)?;
    if element("mobile-compose").is_some() {
        let ui = crate::controls::TerminalControls::new(adapter)?;
        page.borrow_mut().ui = Some(ui);
    }
    connect(&page);
    load_cursor_preference(&page);
    post("ready", None);
    Ok(())
}

#[cfg(test)]
#[path = "page_tests.rs"]
mod tests;
