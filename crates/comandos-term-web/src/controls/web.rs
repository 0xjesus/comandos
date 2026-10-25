use comandos_web_dom::port::*;
use js_sys::{Array, JsString};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsValue, prelude::wasm_bindgen};
use wasm_bindgen_futures::spawn_local;
#[path = "gestures.rs"]
mod gestures;
type Ui = Rc<RefCell<State>>;
struct State {
    gesture: Option<Rc<RefCell<gestures::Gesture>>>,
    term: JsValue,
    session: String,
    auth: String,
    controls: bool,
    touch: bool,
    disposed: bool,
    mobile: bool,
    composing: bool,
    pending: bool,
    refocus: bool,
    history: bool,
    ready: bool,
    dirty: bool,
    revision: u64,
    request: Value,
    anchor: Option<(u32, u32)>,
    pane_busy: bool,
    pane_revision: u64,
    pending_close: Value,
    draft: JsValue,
    reading: JsValue,
    reading_checked: bool,
    listeners: Vec<(JsValue, String, JsValue, bool)>,
    observer: JsValue,
}
fn global(k: &str) -> JsValue {
    get(&js_sys::global(), k)
}
fn doc() -> JsValue {
    global("document")
}
fn id(k: &str) -> JsValue {
    call(&doc(), "getElementById", &[k.into()]).unwrap_or(JsValue::NULL)
}
fn query(el: &JsValue, s: &str) -> JsValue {
    call(el, "querySelector", &[s.into()]).unwrap_or(JsValue::NULL)
}
fn all(el: &JsValue, s: &str) -> Array {
    Array::from(&call(el, "querySelectorAll", &[s.into()]).unwrap_or(JsValue::NULL))
}
fn text(k: &str, s: &str) {
    let _ = set(&id(k), "textContent", &s.into());
}
fn hidden(k: &str, on: bool) {
    let _ = set(&id(k), "hidden", &on.into());
}
fn attr(el: &JsValue, k: &str, v: &str) {
    let _ = call(el, "setAttribute", &[k.into(), v.into()]);
}
fn classes(el: &JsValue, k: &str, on: bool) {
    let _ = call(&get(el, "classList"), "toggle", &[k.into(), on.into()]);
}
fn bind(
    ui: &Ui,
    el: JsValue,
    event: &str,
    f: impl Fn(Array) -> Result<JsValue, JsValue> + 'static,
) -> Result<(), JsValue> {
    let callback = function(f);
    call(&el, "addEventListener", &[event.into(), callback.clone()])?;
    ui.borrow_mut()
        .listeners
        .push((el, event.into(), callback, false));
    Ok(())
}
fn focus(el: &JsValue) {
    let _ = call(
        el,
        "focus",
        &[from_json(&json!({"preventScroll":true})).unwrap_or(JsValue::NULL)],
    );
}
fn blur(el: &JsValue) {
    let _ = call(el, "blur", &[]);
}
fn error(e: &JsValue) {
    text(
        "err",
        &get(e, "message")
            .as_string()
            .or_else(|| e.as_string())
            .unwrap_or_default(),
    );
    let _ = set(&get(&id("err"), "style"), "display", &"block".into());
}
fn length(v: &JsValue) -> u32 {
    JsString::from(v.clone()).length()
}
fn slice(v: &JsValue, a: u32, b: u32) -> JsValue {
    JsString::from(v.clone()).slice(a, b).into()
}
fn index(v: &JsValue, k: &str, from: u32, last: bool) -> i32 {
    if last {
        JsString::from(v.clone()).last_index_of(k, from as i32)
    } else {
        JsString::from(v.clone()).index_of(k, from as i32)
    }
}
fn line(v: &JsValue, caret: u32) -> (u32, u32) {
    let caret = caret.min(length(v));
    let a = if caret > 0 {
        (index(v, "\n", caret - 1, true) + 1) as u32
    } else {
        0
    };
    let b = index(v, "\n", caret, false);
    (a, if b < 0 { length(v) } else { b as u32 })
}
fn select(a: u32, b: u32) {
    let _ = call(
        &id("history-text"),
        "setSelectionRange",
        &[a.into(), b.into(), "forward".into()],
    );
}
fn range() -> (u32, u32) {
    (
        number(&get(&id("history-text"), "selectionStart")) as u32,
        number(&get(&id("history-text"), "selectionEnd")) as u32,
    )
}
fn terminal(ui: &Ui) -> JsValue {
    ui.borrow().term.clone()
}
fn is_live(ui: &Ui) -> bool {
    !ui.borrow().disposed
}
fn alive(ui: &Ui, revision: u64) -> bool {
    let u = ui.borrow();
    !u.disposed && u.history && u.revision == revision
}
fn controls(ui: &Ui) {
    let u = ui.borrow();
    let button = query(&doc(), "[data-action='mode']");
    if u.controls {
        let _ = set(&button, "disabled", &false.into());
        classes(&button, "on", u.history);
        attr(
            &button,
            "aria-pressed",
            if u.history { "true" } else { "false" },
        );
        let _ = set(
            &button,
            "textContent",
            &if u.history {
                "Volver a terminal"
            } else {
                "Seleccionar"
            }
            .into(),
        );
    }
}
fn typography(ui: &Ui) {
    let term = terminal(ui);
    let opts = get(&term, "options");
    let style = get(&id("history-text"), "style");
    let touch = ui.borrow().touch;
    let size = number(&get(&opts, "fontSize")).max(if touch { 16. } else { 12. });
    let ch = number(&get(&term, "cellHeight"));
    let _ = set(&style, "fontFamily", &get(&opts, "fontFamily"));
    let _ = set(&style, "fontSize", &crate::number_text::px(size).into());
    let _ = set(
        &style,
        "lineHeight",
        &if ch.is_finite() && ch > 0. {
            crate::number_text::px(ch.max(if touch { 22. } else { 16. }))
        } else {
            "normal".into()
        }
        .into(),
    );
    let _ = set(
        &style,
        "letterSpacing",
        &crate::number_text::px(number(&get(&opts, "letterSpacing"))).into(),
    );
}
fn inset() {
    let mut height = 0.;
    for k in ["term-toolbar", "mobile-compose", "selection-toolbar"] {
        height += call(&id(k), "getBoundingClientRect", &[])
            .ok()
            .map_or(0., |r| number(&get(&r, "height")));
    }
    let _ = set(
        &get(&id("terminal-history"), "style"),
        "bottom",
        &crate::number_text::px(height).into(),
    );
}
fn set_mobile(ui: &Ui, on: bool) {
    ui.borrow_mut().mobile = on;
    hidden("mobile-compose", !on || ui.borrow().history);
    let textarea = get(&terminal(ui), "textarea");
    let _ = set(&textarea, "readOnly", &on.into());
    attr(&textarea, "inputmode", if on { "none" } else { "text" });
    let button = query(&doc(), "[data-action='keyboard']");
    let _ = set(
        &button,
        "textContent",
        &if on {
            "Teclado directo"
        } else {
            "Escribir borrador"
        }
        .into(),
    );
    attr(&button, "aria-pressed", if on { "false" } else { "true" });
    inset();
    let _ = call(&terminal(ui), "scheduleFit", &[]);
}
fn focus_terminal(ui: &Ui) {
    if ui.borrow().mobile {
        focus(&id("mobile-draft"));
    } else {
        focus(&get(&terminal(ui), "textarea"));
    }
}
fn changed(ui: &Ui) {
    let u = ui.borrow();
    let value = get(&id("mobile-draft"), "value");
    let storage = global("sessionStorage");
    let key = format!("comandos:terminal-draft:{}", u.session);
    let _ = if truthy(&value) {
        call(&storage, "setItem", &[key.into(), value.clone()])
    } else {
        call(&storage, "removeItem", &[key.into()])
    };
    if truthy(&u.draft) {
        let _ = call(
            &u.draft,
            "changed",
            &[
                value,
                get(&id("mobile-draft"), "selectionStart"),
                get(&id("mobile-draft"), "selectionEnd"),
            ],
        );
    }
}
fn paste_payload(text: &JsValue, bracketed: bool, enter: bool) -> JsValue {
    let normalized = JsString::from(text.clone())
        .replace_by_pattern(&js_sys::RegExp::new("\\r?\\n", "g"), "\r")
        .replace_by_pattern(&js_sys::RegExp::new("\\x1b", "g"), "");
    let value = if normalized.length() > 0 && bracketed {
        JsString::from("\x1b[200~")
            .concat(&normalized.into())
            .concat(&"\x1b[201~".into())
    } else {
        normalized
    };
    if enter {
        value.concat(&"\r".into()).into()
    } else {
        value.into()
    }
}
fn submit(ui: &Ui, enter: bool) {
    let blocked = {
        let u = ui.borrow();
        u.disposed || u.composing || u.pending
    };
    if blocked {
        return;
    }
    let value = get(&id("mobile-draft"), "value");
    if !truthy(&value) && !enter {
        return;
    }
    let term = terminal(ui);
    let payload = paste_payload(
        &value,
        truthy(&get(&get(&term, "modes"), "bracketedPasteMode")),
        enter,
    );
    let Ok(sent) = call(&term, "sendWithAck", &[payload, 4000.into()]) else {
        return;
    };
    if !truthy(&sent) {
        return;
    }
    ui.borrow_mut().pending = true;
    let ui = ui.clone();
    spawn_local(async move {
        let ok = wait(Ok(sent)).await.is_ok_and(|v| truthy(&v));
        if !is_live(&ui) {
            return;
        }
        ui.borrow_mut().pending = false;
        if !ok {
            error(&"La terminal no confirmó el envío. El borrador se conserva; comprueba antes de reenviar.".into());
            return;
        }
        if js_sys::Object::is(&value, &get(&id("mobile-draft"), "value")) {
            let _ = set(&id("mobile-draft"), "value", &"".into());
            changed(&ui);
        }
    });
}
fn insert_draft(ui: &Ui, value: JsValue) {
    let d = id("mobile-draft");
    let _ = call(
        &d,
        "setRangeText",
        &[
            value,
            get(&d, "selectionStart"),
            get(&d, "selectionEnd"),
            "end".into(),
        ],
    );
    changed(ui);
}
fn update_selection(ui: &Ui) {
    let value = get(&id("history-text"), "value");
    for el in all(
        &id("selection-toolbar"),
        "[data-selection-line],#selection-all",
    ) {
        let _ = set(
            &el,
            "disabled",
            &(!ui.borrow().ready || !truthy(&value)).into(),
        );
    }
    let (a, b) = range();
    let _ = set(
        &id("selection-copy"),
        "disabled",
        &(!ui.borrow().ready || a == b).into(),
    );
}
fn describe_selection() {
    let (a, b) = range();
    let v = slice(&get(&id("history-text"), "value"), a, b);
    let count = JsString::from(v).split("\n").length();
    text(
        "history-status",
        &format!(
            "{count} {} · toca otra línea para extender o pulsa Copiar selección",
            if count == 1 {
                "línea seleccionada"
            } else {
                "líneas seleccionadas"
            }
        ),
    );
}
async fn fetch(ui: &Ui, path: &str, body: Option<Value>, timeout: i32) -> Result<JsValue, JsValue> {
    let opts = object();
    set(&opts, "cache", &"no-store".into())?;
    let headers = object();
    set(
        &headers,
        "X-Comandos-Token",
        &ui.borrow().auth.clone().into(),
    )?;
    if let Some(body) = body {
        set(&opts, "method", &"POST".into())?;
        set(&headers, "Content-Type", &"application/json".into())?;
        set(&opts, "body", &body.to_string().into())?;
    }
    set(&opts, "headers", &headers)?;
    if let Ok(signal) = call(&global("AbortSignal"), "timeout", &[timeout.into()]) {
        set(&opts, "signal", &signal)?;
    }
    let response = wait(call(&js_sys::global(), "fetch", &[path.into(), opts])).await?;
    let data = wait(call(&response, "json", &[])).await?;
    if !truthy(&get(&response, "ok")) {
        let msg = get(&data, "error");
        return Err(js_sys::Error::new(&if truthy(&msg) {
            string(&msg)
        } else {
            format!("Error {}", string(&get(&response, "status")))
        })
        .into());
    }
    Ok(data)
}
fn pane_label(pane: &JsValue, index: u32) -> (String, String) {
    let title = get(pane, "title").as_string().unwrap_or_default();
    let kind =
        if title.is_empty() || matches!(title.as_str(), "zsh" | "bash" | "sh" | "fish" | "dash") {
            "terminal"
        } else {
            &title
        };
    let path = get(pane, "path").as_string().unwrap_or_default();
    let folder = if path == "~" || path == "/" {
        path.as_str()
    } else {
        path.trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default()
    };
    let label = if folder.is_empty() {
        format!(
            "Panel {} · {}",
            index + 1,
            if title.is_empty() { "Terminal" } else { &title }
        )
    } else {
        format!("{}. {folder} · {kind}", index + 1)
    };
    (path, label)
}
/// History of the pane a phone shows alone (the whole window has two or more).
fn shown_pane() -> Value {
    crate::page::crop_pane().map_or_else(|| json!({}), |pane| json!({ "pane": pane }))
}
async fn history_open(ui: Ui, request: Value) -> Result<JsValue, JsValue> {
    gestures::prepare_history(&ui);
    let (revision, session, auth) = {
        let mut u = ui.borrow_mut();
        u.history = true;
        u.ready = false;
        u.dirty = false;
        u.revision = u.revision.wrapping_add(1);
        u.request = request.clone();
        (u.revision, u.session.clone(), u.auth.clone())
    };
    blur(&get(&terminal(&ui), "textarea"));
    blur(&id("mobile-draft"));
    hidden("terminal-history", false);
    hidden("term-toolbar", true);
    hidden("mobile-compose", true);
    hidden("selection-toolbar", false);
    attr(
        &query(&doc(), "[data-action='history']"),
        "aria-pressed",
        "true",
    );
    typography(&ui);
    inset();
    controls(&ui);
    let _ = set(&id("history-text"), "value", &"".into());
    let _ = set(&id("history-text"), "disabled", &true.into());
    update_selection(&ui);
    text("history-status", "Cargando historial…");
    let result = if session.is_empty() || auth.is_empty() {
        let text = call(&terminal(&ui), "localHistory", &[]).unwrap_or("".into());
        let v = object();
        set(&v, "text", &text)?;
        set(&v, "local", &true.into())?;
        set(&v, "panes", &Array::new())?;
        Ok(v)
    } else {
        let mut body = json!({"session":session,"lines":2000});
        if let Some(m) = request.as_object() {
            for (k, v) in m {
                body[k] = v.clone();
            }
        }
        fetch(&ui, "/terminal-history", Some(body), 10000).await
    };
    if !alive(&ui, revision) {
        return Ok(false.into());
    }
    let snapshot = match result {
        Ok(v)
            if get(&v, "text").is_string()
                && get(&v, "ok") != JsValue::FALSE
                && !truthy(&get(&v, "error")) =>
        {
            v
        }
        Ok(v) => {
            let error = get(&v, "error")
                .as_string()
                .unwrap_or_else(|| "Respuesta sin texto".into());
            text(
                "history-status",
                &format!("No pude cargar el historial: {error}"),
            );
            return Ok(false.into());
        }
        Err(e) => {
            text(
                "history-status",
                &format!(
                    "No pude cargar el historial: {}",
                    get(&e, "message")
                        .as_string()
                        .or_else(|| e.as_string())
                        .unwrap_or_default()
                ),
            );
            return Ok(false.into());
        }
    };
    if let Some(pane) = get(&snapshot, "pane").as_string() {
        ui.borrow_mut().request = json!({"pane":pane});
    }
    let value = get(&snapshot, "text");
    set(&id("history-text"), "value", &value)?;
    set(&id("history-text"), "disabled", &false.into())?;
    ui.borrow_mut().ready = true;
    call(&id("history-pane"), "replaceChildren", &[])?;
    let panes = Array::from(&get(&snapshot, "panes"));
    for (index, pane) in panes.iter().enumerate() {
        let option = call(&doc(), "createElement", &["option".into()])?;
        set(&option, "value", &get(&pane, "id"))?;
        let (path, label) = pane_label(&pane, index as u32);
        set(
            &option,
            "textContent",
            &format!(
                "{label}{}",
                if path.is_empty() {
                    String::new()
                } else {
                    format!(" — {path}")
                }
            )
            .into(),
        )?;
        set(
            &option,
            "selected",
            &js_sys::Object::is(&get(&pane, "id"), &get(&snapshot, "pane")).into(),
        )?;
        call(&id("history-pane"), "appendChild", &[option])?;
    }
    hidden("history-pane", panes.length() == 0);
    let trimmed: JsValue = JsString::from(value.clone()).trim_end().into();
    let end = length(&trimmed);
    let start = if end > 0 {
        (index(&value, "\n", end - 1, true) + 1) as u32
    } else {
        0
    };
    ui.borrow_mut().anchor = Some((start, end));
    focus(&id("history-text"));
    select(start, end);
    set(
        &id("history-text"),
        "scrollTop",
        &get(&id("history-text"), "scrollHeight"),
    )?;
    update_selection(&ui);
    text(
        "history-status",
        if truthy(&get(&snapshot, "local")) {
            "Buffer local. El historial de tmux no está disponible."
        } else {
            "Toca una línea para empezar y otra para marcar hasta dónde. Luego, Copiar selección."
        },
    );
    let reading = {
        let mut u = ui.borrow_mut();
        if truthy(&u.reading) && !u.reading_checked {
            u.reading_checked = true;
            Some(u.reading.clone())
        } else {
            None
        }
    };
    if let Some(reading) = reading {
        let u = ui.clone();
        let found = call(&reading, "find", &[value]);
        spawn_local(async move {
            if let Ok(found) = wait(found).await
                && alive(&u, revision)
            {
                match get(&found, "state").as_string().as_deref() {
                    Some("found") => {
                        let index = number(&get(&found, "index")) as u32;
                        let value = get(&id("history-text"), "value");
                        let before = JsString::from(slice(&value, 0, index))
                            .split("\n")
                            .length()
                            .saturating_sub(1);
                        let style =
                            call(&js_sys::global(), "getComputedStyle", &[id("history-text")])
                                .unwrap_or(JsValue::NULL);
                        let height = invoke(&global("parseFloat"), &[get(&style, "lineHeight")])
                            .ok()
                            .map_or(16., |v| number(&v));
                        let _ = set(
                            &id("history-text"),
                            "scrollTop",
                            &(f64::from(before)
                                * if height.is_finite() && height > 0. {
                                    height
                                } else {
                                    16.
                                })
                            .into(),
                        );
                    }
                    Some("missing") => text(
                        "history-status",
                        "Tu punto de lectura anterior ya no está en el historial disponible. Se muestra lo más reciente.",
                    ),
                    _ => {}
                }
            }
        });
    }
    Ok(true.into())
}
fn history_close(ui: &Ui) {
    let (controls, mobile) = {
        let mut u = ui.borrow_mut();
        u.history = false;
        u.revision = u.revision.wrapping_add(1);
        (u.controls, u.mobile)
    };
    hidden("terminal-history", true);
    hidden("selection-toolbar", true);
    hidden("term-toolbar", !controls);
    hidden("mobile-compose", !mobile);
    inset();
    controls_fn(ui);
    attr(
        &query(&doc(), "[data-action='history']"),
        "aria-pressed",
        "false",
    );
    let selection = call(&js_sys::global(), "getSelection", &[]).unwrap_or(JsValue::NULL);
    if history_selection(&selection) {
        let _ = call(&selection, "removeAllRanges", &[]);
    }
    let _ = call(&terminal(ui), "scheduleFit", &[]);
}
fn controls_fn(ui: &Ui) {
    controls(ui)
}
fn history_selection(selection: &JsValue) -> bool {
    truthy(selection)
        && call(
            &id("history-text"),
            "contains",
            &[get(selection, "anchorNode")],
        )
        .is_ok_and(|v| truthy(&v))
}
async fn copy(ui: &Ui) -> bool {
    let text = if ui.borrow().history {
        let (a, b) = range();
        slice(&get(&id("history-text"), "value"), a, b)
    } else {
        let selection = call(&js_sys::global(), "getSelection", &[]).unwrap_or(JsValue::NULL);
        if history_selection(&selection) {
            call(&selection, "toString", &[]).unwrap_or("".into())
        } else {
            call(&terminal(ui), "getSelection", &[]).unwrap_or("".into())
        }
    };
    if !truthy(&text) {
        return false;
    }
    if wait(call(
        &get(&global("navigator"), "clipboard"),
        "writeText",
        &[text],
    ))
    .await
    .is_ok()
    {
        return true;
    }
    if ui.borrow().history {
        focus(&id("history-text"));
    }
    call(&doc(), "execCommand", &["copy".into()]).is_ok_and(|v| truthy(&v))
}
async fn panes_request(
    ui: &Ui,
    action: &str,
    pane: &Value,
    direction: Option<&str>,
) -> Result<JsValue, JsValue> {
    let (session, auth) = {
        let u = ui.borrow();
        (u.session.clone(), u.auth.clone())
    };
    if session.is_empty() || auth.is_empty() {
        return Err(js_sys::Error::new(
            "Abre la terminal desde ComandOS para administrar sus paneles.",
        )
        .into());
    }
    let mut body = json!({"session":session,"action":action});
    if pane.is_object() {
        body["pane"] = pane["id"].clone();
        body["identity"] = pane["identity"].clone();
    }
    if let Some(d) = direction {
        body["direction"] = d.into();
    }
    if action == "select" {
        body["scope"] = "client".into();
    }
    let result = fetch(ui, "/terminal-panes", Some(body), 10000).await?;
    if get(&result, "ok") != JsValue::TRUE || !Array::is_array(&get(&result, "panes")) {
        return Err(js_sys::Error::new(
            &get(&result, "error")
                .as_string()
                .unwrap_or_else(|| "No se pudo consultar el estado de los paneles".into()),
        )
        .into());
    }
    Ok(result)
}
fn pane_busy(ui: &Ui, on: bool) {
    ui.borrow_mut().pane_busy = on;
    for button in all(&id("pane-dialog"), "button") {
        let _ = set(
            &button,
            "disabled",
            &(on || get(&get(&button, "dataset"), "unavailable")
                .as_string()
                .as_deref()
                == Some("true"))
            .into(),
        );
    }
}
fn cancel_close(ui: &Ui) {
    ui.borrow_mut().pending_close = Value::Null;
    hidden("pane-confirm", true);
    hidden("pane-list", false);
}
fn render_panes(ui: &Ui, panes: JsValue) -> Result<(), JsValue> {
    call(&id("pane-list"), "replaceChildren", &[])?;
    let panes = Array::from(&panes);
    for (index, pane) in panes.iter().enumerate() {
        let row = call(&doc(), "createElement", &["div".into()])?;
        set(&row, "className", &"pane-option".into())?;
        let (path, label) = pane_label(&pane, index as u32);
        let select = call(&doc(), "createElement", &["button".into()])?;
        set(&select, "type", &"button".into())?;
        let strong = call(&doc(), "createElement", &["strong".into()])?;
        set(
            &strong,
            "textContent",
            &format!(
                "{label}{}",
                if truthy(&get(&pane, "active")) {
                    " · Activo"
                } else {
                    ""
                }
            )
            .into(),
        )?;
        call(&select, "appendChild", &[strong])?;
        if !path.is_empty() {
            let small = call(&doc(), "createElement", &["small".into()])?;
            set(&small, "className", &"pane-path".into())?;
            set(&small, "textContent", &path.into())?;
            call(&select, "appendChild", &[small])?;
        }
        attr(
            &select,
            "aria-pressed",
            if truthy(&get(&pane, "active")) {
                "true"
            } else {
                "false"
            },
        );
        let u = ui.clone();
        let p = to_json(&pane);
        bind(ui, select.clone(), "click", move |_| {
            Ok(promise(pane_action(u.clone(), "select", p.clone(), None)))
        })?;
        call(&row, "appendChild", &[select])?;
        for (direction, label, aria) in [
            ("right", "Dividir →", "Abrir split a la derecha de"),
            ("down", "Dividir ↓", "Abrir split debajo de"),
        ] {
            let button = call(&doc(), "createElement", &["button".into()])?;
            set(&button, "type", &"button".into())?;
            set(&button, "textContent", &label.into())?;
            attr(
                &button,
                "aria-label",
                &format!("{aria} {}", pane_label(&pane, index as u32).1),
            );
            let u = ui.clone();
            let p = to_json(&pane);
            bind(ui, button.clone(), "click", move |_| {
                Ok(promise(pane_action(
                    u.clone(),
                    "split",
                    p.clone(),
                    Some(direction),
                )))
            })?;
            call(&row, "appendChild", &[button])?;
        }
        let close = call(&doc(), "createElement", &["button".into()])?;
        set(&close, "type", &"button".into())?;
        set(&close, "className", &"danger".into())?;
        set(&close, "textContent", &"Cerrar".into())?;
        set(&close, "disabled", &(panes.length() <= 1).into())?;
        set(
            &get(&close, "dataset"),
            "unavailable",
            &if panes.length() <= 1 { "true" } else { "false" }.into(),
        )?;
        attr(&close, "aria-label", &format!("Cerrar {label}"));
        let u = ui.clone();
        let p = to_json(&pane);
        bind(ui, close.clone(), "click", move |_| {
            u.borrow_mut().pending_close = p.clone();
            text("pane-confirm-label", &format!("¿Cerrar {label}?"));
            hidden("pane-list", true);
            hidden("pane-confirm", false);
            Ok(JsValue::UNDEFINED)
        })?;
        call(&row, "appendChild", &[close])?;
        call(&id("pane-list"), "appendChild", &[row])?;
    }
    text(
        "pane-status",
        if panes.length() == 1 {
            "Un solo panel. Divídelo para abrir un split."
        } else {
            "Elige un panel para activarlo, dividirlo o cerrar su split."
        },
    );
    Ok(())
}
async fn panes_open(ui: Ui) -> Result<JsValue, JsValue> {
    if ui.borrow().pane_busy {
        return Ok(JsValue::UNDEFINED);
    }
    let revision = {
        let mut u = ui.borrow_mut();
        u.pane_revision = u.pane_revision.wrapping_add(1);
        u.pane_revision
    };
    cancel_close(&ui);
    call(&id("pane-list"), "replaceChildren", &[])?;
    text("pane-status", "Cargando paneles…");
    blur(&get(&terminal(&ui), "textarea"));
    blur(&id("mobile-draft"));
    if !truthy(&get(&id("pane-dialog"), "open")) {
        call(&id("pane-dialog"), "showModal", &[])?;
    }
    let result = panes_request(&ui, "list", &Value::Null, None).await;
    if is_live(&ui)
        && ui.borrow().pane_revision == revision
        && truthy(&get(&id("pane-dialog"), "open"))
    {
        match result {
            Ok(v) => render_panes(&ui, get(&v, "panes"))?,
            Err(e) => text(
                "pane-status",
                &get(&e, "message")
                    .as_string()
                    .or_else(|| e.as_string())
                    .unwrap_or_default(),
            ),
        }
    }
    Ok(JsValue::UNDEFINED)
}
async fn pane_action(
    ui: Ui,
    action: &str,
    pane: Value,
    direction: Option<&str>,
) -> Result<JsValue, JsValue> {
    if ui.borrow().pane_busy || !pane.is_object() {
        return Ok(JsValue::UNDEFINED);
    }
    let revision = {
        let mut u = ui.borrow_mut();
        u.pane_revision = u.pane_revision.wrapping_add(1);
        u.pane_revision
    };
    pane_busy(&ui, true);
    text(
        "pane-status",
        match action {
            "close" => "Guardando copia y cerrando el split…",
            "split" => "Abriendo split…",
            _ => "Activando panel…",
        },
    );
    let result = async {
        let result = panes_request(&ui, action, &pane, direction).await?;
        if !is_live(&ui) || ui.borrow().pane_revision != revision {
            return Ok(JsValue::UNDEFINED);
        }
        if action == "select" || action == "split" {
            let selected = if action == "split" {
                get(&result, "opened")
            } else {
                from_json(&pane["id"])?
            };
            let focus = if action == "split" {
                let opened = Array::from(&get(&result, "panes"))
                    .iter()
                    .find(|p| js_sys::Object::is(&get(p, "id"), &selected));
                if let Some(p) = opened {
                    panes_request(&ui, "select", &to_json(&p), None).await?
                } else {
                    object()
                }
            } else {
                result
            };
            if !is_live(&ui) || ui.borrow().pane_revision != revision {
                return Ok(JsValue::UNDEFINED);
            }
            let keys = get(&focus, "clientKeys");
            if keys.is_string()
                && !truthy(&call(&terminal(&ui), "sendInput", &[keys, false.into()])?)
            {
                return Ok(JsValue::UNDEFINED);
            }
            if truthy(&selected) {
                call(
                    &global("parent"),
                    "postMessage",
                    &[
                        {
                            let mut message =
                                json!({"source":"comandos-term","type":"pane-selected"});
                            message["pane"] = to_json(&selected);
                            from_json(&message)?
                        },
                        get(&global("location"), "origin"),
                    ],
                )?;
            }
            call(&id("pane-dialog"), "close", &[])?;
        } else {
            cancel_close(&ui);
            render_panes(&ui, get(&result, "panes"))?;
            text(
                "pane-status",
                "Split cerrado. Los demás paneles siguen abiertos.",
            );
        }
        Ok(JsValue::UNDEFINED)
    }
    .await;
    if let Err(e) = result
        && is_live(&ui)
        && ui.borrow().pane_revision == revision
    {
        cancel_close(&ui);
        call(&id("pane-list"), "replaceChildren", &[])?;
        text(
            "pane-status",
            &format!(
                "{} Vuelve a abrir Paneles para comprobar el estado.",
                get(&e, "message")
                    .as_string()
                    .or_else(|| e.as_string())
                    .unwrap_or_default()
            ),
        );
    }
    pane_busy(&ui, false);
    Ok(JsValue::UNDEFINED)
}
#[wasm_bindgen]
#[derive(Clone)]
pub struct TerminalControls {
    ui: Ui,
}
#[wasm_bindgen]
impl TerminalControls {
    pub fn disconnect(&self) {
        gestures::finish_active(&self.ui);
    }
    #[wasm_bindgen(constructor)]
    pub fn new(term: JsValue) -> Result<TerminalControls, JsValue> {
        let params = web_sys::UrlSearchParams::new_with_str(
            &get(&global("location"), "search")
                .as_string()
                .unwrap_or_default(),
        )?;
        let session = params.get("arg").unwrap_or_default();
        let auth = params.get("auth").unwrap_or_default();
        let touch = js_sys::Reflect::has(&js_sys::global(), &"ontouchstart".into())
            .unwrap_or(false)
            || number(&get(&global("navigator"), "maxTouchPoints")) > 0.;
        let controls = touch
            || (get(&global("location"), "protocol").as_string().as_deref() != Some("file:")
                && !session.is_empty());
        let ui = Rc::new(RefCell::new(State {
            gesture: None,
            term,
            session,
            auth,
            controls,
            touch,
            disposed: false,
            mobile: false,
            composing: false,
            pending: false,
            refocus: true,
            history: false,
            ready: false,
            dirty: false,
            revision: 0,
            request: json!({}),
            anchor: None,
            pane_busy: false,
            pane_revision: 0,
            pending_close: Value::Null,
            draft: JsValue::NULL,
            reading: JsValue::NULL,
            reading_checked: false,
            listeners: Vec::new(),
            observer: JsValue::NULL,
        }));
        let key = format!("comandos:terminal-draft:{}", ui.borrow().session);
        if let Ok(v) = call(&global("sessionStorage"), "getItem", &[key.into()]) {
            set(
                &id("mobile-draft"),
                "value",
                &if truthy(&v) { v } else { "".into() },
            )?;
        }
        let mounted = mount(&ui)
            .and_then(|()| device_state(&ui))
            .and_then(|()| gestures::mount(&ui));
        if let Err(e) = mounted {
            let controls = Self { ui };
            controls.dispose();
            return Err(e);
        }
        if let Err(e) = layout_observer(&ui) {
            let controls = Self { ui };
            controls.dispose();
            return Err(e);
        }
        if touch {
            set_mobile(&ui, true);
        }
        controls_fn(&ui);
        Ok(Self { ui })
    }
    pub fn dispose(&self) {
        gestures::dispose(&self.ui);
        let listeners = {
            let mut u = self.ui.borrow_mut();
            if u.disposed {
                return;
            }
            u.disposed = true;
            u.history = false;
            u.revision = u.revision.wrapping_add(1);
            std::mem::take(&mut u.listeners)
        };
        for (el, event, f, capture) in listeners {
            let _ = call(
                &el,
                "removeEventListener",
                &[event.into(), f, capture.into()],
            );
        }
        let _ = call(&self.ui.borrow().observer, "disconnect", &[]);
    }
    pub fn set_mobile(&self, on: bool) {
        set_mobile(&self.ui, on);
    }
    pub fn refresh_interaction(&self) {
        controls_fn(&self.ui);
    }
    pub fn focus(&self) {
        focus_terminal(&self.ui);
    }
}
fn layout_observer(ui: &Ui) -> Result<(), JsValue> {
    use wasm_bindgen::JsCast;
    if let Ok(constructor) = global("ResizeObserver").dyn_into::<js_sys::Function>() {
        let callback = function(|_| {
            inset();
            Ok(JsValue::UNDEFINED)
        });
        let args = Array::new();
        args.push(&callback);
        let observer = js_sys::Reflect::construct(&constructor, &args)?;
        ui.borrow_mut().observer = observer.clone();
        for k in ["term-toolbar", "mobile-compose", "selection-toolbar"] {
            call(&observer, "observe", &[id(k)])?;
        }
    }
    Ok(())
}
fn device_state(ui: &Ui) -> Result<(), JsValue> {
    comandos_web_dom::drafts::web::export()?;
    let device = call(
        &global("localStorage"),
        "getItem",
        &["comandos.deviceId".into()],
    )
    .ok()
    .and_then(|v| v.as_string())
    .unwrap_or_default();
    if device.is_empty() || ui.borrow().session.is_empty() || ui.borrow().auth.is_empty() {
        return Ok(());
    }
    let options = object();
    set(
        &options,
        "key",
        &format!("tab:{}", ui.borrow().session).into(),
    )?;
    let u = ui.clone();
    let d = device.clone();
    set(
        &options,
        "load",
        &function(move |_| {
            let u = u.clone();
            let d = d.clone();
            Ok(promise(async move {
                let encoded = invoke(&global("encodeURIComponent"), &[d.into()])?;
                fetch(
                    &u,
                    &format!("/workspace/client?deviceId={}", string(&encoded)),
                    None,
                    8000,
                )
                .await
            }))
        }),
    )?;
    let u = ui.clone();
    let d = device;
    set(
        &options,
        "save",
        &function(move |a| {
            let u = u.clone();
            let d = d.clone();
            let mut body = json!({"deviceId":d});
            if let Some(patch) = to_json(&a.get(0)).as_object() {
                for (k, v) in patch {
                    body[k] = v.clone();
                }
            }
            Ok(promise(async move {
                fetch(&u, "/workspace/client", Some(body), 8000).await?;
                Ok(JsValue::UNDEFINED)
            }))
        }),
    )?;
    set(
        &options,
        "read",
        &function(move |_| Ok(get(&id("mobile-draft"), "value"))),
    )?;
    set(
        &options,
        "write",
        &function(move |a| {
            set(&id("mobile-draft"), "value", &a.get(0))?;
            Ok(JsValue::UNDEFINED)
        }),
    )?;
    set(
        &options,
        "select",
        &function(move |a| {
            call(
                &id("mobile-draft"),
                "setSelectionRange",
                &[a.get(0), a.get(1)],
            )
        }),
    )?;
    let draft = call(
        &global("ComandosDeviceDrafts"),
        "createDrafts",
        std::slice::from_ref(&options),
    )?;
    let reading = call(
        &global("ComandosDeviceDrafts"),
        "createAnchor",
        std::slice::from_ref(&options),
    )?;
    ui.borrow_mut().draft = draft.clone();
    ui.borrow_mut().reading = reading;
    let restore = call(&draft, "restore", &[]);
    let u = ui.clone();
    spawn_local(async move {
        if let Ok(state) = wait(restore).await
            && state.as_string().as_deref() == Some("restored")
            && is_live(&u)
        {
            let _ = call(
                &global("sessionStorage"),
                "setItem",
                &[
                    format!("comandos:terminal-draft:{}", u.borrow().session).into(),
                    get(&id("mobile-draft"), "value"),
                ],
            );
        }
    });
    let u = ui.clone();
    bind(ui, id("history-text"), "scroll", move |_| {
        let reading = u.borrow().reading.clone();
        if !u.borrow().ready {
            return Ok(JsValue::UNDEFINED);
        }
        let computed = call(&js_sys::global(), "getComputedStyle", &[id("history-text")])?;
        let height = invoke(&global("parseFloat"), &[get(&computed, "lineHeight")])
            .ok()
            .map_or(16., |v| number(&v));
        let height = if height.is_finite() && height > 0. {
            height
        } else {
            16.
        };
        let top = (number(&get(&id("history-text"), "scrollTop")) / height)
            .floor()
            .max(0.) as u32;
        let value = get(&id("history-text"), "value");
        let lines = JsString::from(value.clone()).split("\n");
        let mut offset = 0;
        for i in 0..top.min(lines.length()) {
            offset += length(&lines.get(i)) + 1;
        }
        let scroll = number(&get(&id("history-text"), "scrollHeight"));
        let ratio = if scroll > 0. {
            number(&get(&id("history-text"), "scrollTop")) / scroll
        } else {
            0.
        };
        call(
            &reading,
            "remember",
            &[
                value.clone(),
                offset.min(length(&value)).into(),
                ratio.min(1.).into(),
            ],
        )?;
        Ok(JsValue::UNDEFINED)
    })?;
    Ok(())
}
fn capture(
    ui: &Ui,
    el: JsValue,
    event: &str,
    f: impl Fn(Array) -> Result<JsValue, JsValue> + 'static,
) -> Result<(), JsValue> {
    let callback = function(f);
    call(
        &el,
        "addEventListener",
        &[event.into(), callback.clone(), true.into()],
    )?;
    ui.borrow_mut()
        .listeners
        .push((el, event.into(), callback, true));
    Ok(())
}
fn mount(ui: &Ui) -> Result<(), JsValue> {
    for event in [
        "keydown",
        "keypress",
        "beforeinput",
        "input",
        "compositionstart",
        "compositionupdate",
        "compositionend",
        "paste",
    ] {
        let u = ui.clone();
        capture(ui, get(&terminal(ui), "textarea"), event, move |a| {
            if u.borrow().mobile {
                let e = a.get(0);
                if truthy(&get(&e, "cancelable")) {
                    call(&e, "preventDefault", &[])?;
                }
                call(&e, "stopImmediatePropagation", &[])?;
            }
            Ok(JsValue::UNDEFINED)
        })?;
    }

    let u = ui.clone();
    bind(ui, id("mobile-draft"), "compositionstart", move |_| {
        u.borrow_mut().composing = true;
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("mobile-draft"), "compositionend", move |_| {
        u.borrow_mut().composing = false;
        changed(&u);
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("mobile-draft"), "input", move |_| {
        changed(&u);
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("mobile-compose"), "submit", move |a| {
        call(&a.get(0), "preventDefault", &[])?;
        submit(&u, true);
        focus(&id("mobile-draft"));
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("draft-insert"), "click", move |_| {
        submit(&u, false);
        focus(&id("mobile-draft"));
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, get(&terminal(ui), "textarea"), "focus", move |_| {
        if u.borrow().mobile {
            focus(&id("mobile-draft"));
        }
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("term-toolbar"), "pointerdown", move |a| {
        let e = a.get(0);
        let touch = get(&e, "pointerType").as_string().as_deref() == Some("touch");
        if !touch {
            call(&e, "preventDefault", &[])?;
        }
        let focused = get(&doc(), "activeElement");
        u.borrow_mut().refocus = !touch
            || js_sys::Object::is(&focused, &get(&terminal(&u), "textarea"))
            || get(&focused, "id").as_string().as_deref() == Some("mobile-draft");
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("term-toolbar"), "click", move |a| {
        let button = call(&get(&a.get(0), "target"), "closest", &["button".into()])?;
        if !truthy(&button) {
            return Ok(JsValue::UNDEFINED);
        }
        let data = get(&button, "dataset");
        let key = get(&data, "key");
        if let Some(key) = key.as_string() {
            if let Some(bytes) = crate::parent::toolbar_key(&key) {
                call(
                    &terminal(&u),
                    "sendInput",
                    &[String::from_utf8_lossy(bytes).as_ref().into(), true.into()],
                )?;
            }
            if u.borrow().refocus {
                focus_terminal(&u);
            }
            return Ok(JsValue::UNDEFINED);
        }
        match get(&data, "action").as_string().as_deref() {
            Some("history") => return Ok(promise(history_open(u.clone(), shown_pane()))),
            Some("mode") if u.borrow().controls => {
                if u.borrow().history {
                    history_close(&u)
                } else {
                    return Ok(promise(history_open(u.clone(), shown_pane())));
                }
            }
            Some("panes") => return Ok(promise(panes_open(u.clone()))),
            Some("keyboard") => {
                let on = !u.borrow().mobile;
                set_mobile(&u, on)
            }
            Some("ctrl") => {
                call(
                    &terminal(&u),
                    "setCtrlArmed",
                    &[(!truthy(&get(&terminal(&u), "ctrl"))).into()],
                )?;
                set_mobile(&u, false);
                focus_terminal(&u)
            }
            Some("paste") => {
                let u = u.clone();
                return Ok(promise(async move {
                    match wait(call(
                        &get(&global("navigator"), "clipboard"),
                        "readText",
                        &[],
                    ))
                    .await
                    {
                        Ok(v) => {
                            if u.borrow().mobile {
                                insert_draft(&u, v)
                            } else {
                                call(&terminal(&u), "paste", &[v])?;
                            }
                            focus_terminal(&u)
                        }
                        Err(_) => {
                            call(&id("paste-dialog"), "showModal", &[])?;
                        }
                    }
                    Ok(JsValue::UNDEFINED)
                }));
            }
            Some("copy") => {
                let u = u.clone();
                return Ok(promise(async move {
                    if !copy(&u).await {
                        if !u.borrow().history {
                            history_open(u.clone(), shown_pane()).await?;
                        }
                        text(
                            "history-status",
                            "Selecciona texto para copiar, o pulsa Todo.",
                        );
                    } else {
                        let b = query(&doc(), "[data-action='copy']");
                        set(&b, "textContent", &"Copiado".into())?;
                        let _ = call(
                            &js_sys::global(),
                            "setTimeout",
                            &[
                                function(move |_| {
                                    set(&b, "textContent", &"Copiar".into())?;
                                    Ok(JsValue::UNDEFINED)
                                }),
                                1200.into(),
                            ],
                        );
                    }
                    Ok(JsValue::UNDEFINED)
                }));
            }
            Some("mode") => {
                call(&terminal(&u), "requestInteractionMode", &[])?;
                focus_terminal(&u)
            }
            _ => {}
        }
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, query(&id("paste-dialog"), "form"), "submit", move |a| {
        let e = a.get(0);
        if get(&get(&e, "submitter"), "id").as_string().as_deref() != Some("paste-submit") {
            return Ok(JsValue::UNDEFINED);
        }
        let value = get(&id("paste-text"), "value");
        if u.borrow().mobile {
            insert_draft(&u, value)
        } else if !truthy(&call(&terminal(&u), "paste", &[value])?) {
            call(&e, "preventDefault", &[])?;
            return Ok(JsValue::UNDEFINED);
        }
        set(&id("paste-text"), "value", &"".into())?;
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("paste-dialog"), "close", move |_| {
        focus_terminal(&u);
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("selection-close"), "click", move |_| {
        history_close(&u);
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("history-refresh"), "click", move |_| {
        let r = u.borrow().request.clone();
        Ok(promise(history_open(u.clone(), r)))
    })?;
    let u = ui.clone();
    bind(ui, id("history-pane"), "change", move |_| {
        Ok(promise(history_open(
            u.clone(),
            json!({"pane":string(&get(&id("history-pane"),"value"))}),
        )))
    })?;
    bind(ui, id("history-bottom"), "click", move |_| {
        set(
            &id("history-text"),
            "scrollTop",
            &get(&id("history-text"), "scrollHeight"),
        )?;
        Ok(JsValue::UNDEFINED)
    })?;
    for (k, all) in [("selection-copy", false), ("selection-all", true)] {
        let u = ui.clone();
        bind(ui, id(k), "click", move |_| {
            if all {
                focus(&id("history-text"));
                select(0, length(&get(&id("history-text"), "value")));
                u.borrow_mut().anchor = None;
                update_selection(&u);
            }
            let u = u.clone();
            Ok(promise(async move {
                let ok = copy(&u).await;
                text(
                    "history-status",
                    if ok {
                        if all { "Todo copiado." } else { "Copiado." }
                    } else {
                        "No se pudo copiar. Usa Copiar en el menú del navegador."
                    },
                );
                Ok(JsValue::UNDEFINED)
            }))
        })?;
    }
    bind(ui, id("selection-toolbar"), "pointerdown", move |a| {
        if truthy(&call(
            &get(&a.get(0), "target"),
            "closest",
            &["button".into()],
        )?) {
            call(&a.get(0), "preventDefault", &[])?;
        }
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("history-text"), "click", move |_| {
        if !u.borrow().ready {
            return Ok(JsValue::UNDEFINED);
        }
        let (a, b) = range();
        if a != b {
            u.borrow_mut().anchor = None;
            return Ok(JsValue::UNDEFINED);
        }
        let v = get(&id("history-text"), "value");
        let current = line(&v, a);
        let anchor = u.borrow().anchor.filter(|(a0, b0)| a < *a0 || a > *b0);
        let (a0, b0) = anchor.unwrap_or(current);
        u.borrow_mut().anchor = Some((a0, b0));
        select(a0.min(current.0), b0.max(current.1));
        update_selection(&u);
        describe_selection();
        Ok(JsValue::UNDEFINED)
    })?;
    for event in ["select", "selectionchange"] {
        let u = ui.clone();
        bind(ui, id("history-text"), event, move |_| {
            update_selection(&u);
            Ok(JsValue::UNDEFINED)
        })?;
    }
    let u = ui.clone();
    bind(ui, id("selection-toolbar"), "click", move |a| {
        let button = call(
            &get(&a.get(0), "target"),
            "closest",
            &["[data-selection-line]".into()],
        )?;
        let direction = get(&get(&button, "dataset"), "selectionLine");
        if !truthy(&direction) || !u.borrow().ready {
            return Ok(JsValue::UNDEFINED);
        }
        let v = get(&id("history-text"), "value");
        let (mut start, mut end) = range();
        if direction.as_string().as_deref() == Some("up") {
            let a = if start > 0 {
                (index(&v, "\n", start - 1, true) + 1) as u32
            } else {
                0
            };
            start = if a == start && start > 0 {
                (index(&v, "\n", start.saturating_sub(2), true) + 1) as u32
            } else {
                a
            };
        } else {
            let n = index(&v, "\n", end, false);
            let next = if n == end as i32 {
                index(&v, "\n", end + 1, false)
            } else {
                n
            };
            end = if next < 0 { length(&v) } else { next as u32 };
        }
        u.borrow_mut().anchor = Some((start, end));
        focus(&id("history-text"));
        select(start, end);
        let offset = if direction.as_string().as_deref() == Some("up") {
            start
        } else {
            end
        };
        let line = JsString::from(slice(&v, 0, offset))
            .split("\n")
            .length()
            .saturating_sub(1);
        let style = call(&js_sys::global(), "getComputedStyle", &[id("history-text")])?;
        let height = invoke(&global("parseFloat"), &[get(&style, "lineHeight")])
            .ok()
            .map_or(22., |v| number(&v));
        let height = if height.is_finite() && height > 0. {
            height
        } else {
            22.
        };
        let top = f64::from(line) * height;
        let scroll = number(&get(&id("history-text"), "scrollTop"));
        let visible = number(&get(&id("history-text"), "clientHeight"));
        if top < scroll || top + height > scroll + visible {
            set(
                &id("history-text"),
                "scrollTop",
                &(top - visible / 2.).max(0.).into(),
            )?;
        }
        update_selection(&u);
        describe_selection();
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("terminal-history"), "keydown", move |a| {
        if get(&a.get(0), "key").as_string().as_deref() == Some("Escape") {
            history_close(&u)
        }
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    call(
        &terminal(ui),
        "onWriteParsed",
        &[function(move |_| {
            let mut state = u.borrow_mut();
            if state.history && !state.dirty {
                state.dirty = true;
                text(
                    "history-status",
                    "Hay salida nueva. Actualiza cuando termines de seleccionar.",
                );
            }
            Ok(JsValue::UNDEFINED)
        })],
    )?;
    let u = ui.clone();
    bind(ui, id("pane-close-confirm"), "click", move |_| {
        let p = u.borrow().pending_close.clone();
        Ok(promise(pane_action(u.clone(), "close", p, None)))
    })?;
    let u = ui.clone();
    bind(ui, id("pane-cancel"), "click", move |_| {
        if !u.borrow().pane_busy {
            cancel_close(&u)
        }
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("pane-dismiss"), "click", move |_| {
        if !u.borrow().pane_busy {
            call(&id("pane-dialog"), "close", &[])?;
        }
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("pane-dialog"), "cancel", move |a| {
        if u.borrow().pane_busy {
            call(&a.get(0), "preventDefault", &[])?;
        }
        Ok(JsValue::UNDEFINED)
    })?;
    let u = ui.clone();
    bind(ui, id("pane-dialog"), "close", move |_| {
        let mut u = u.borrow_mut();
        u.pane_revision = u.pane_revision.wrapping_add(1);
        u.pending_close = Value::Null;
        Ok(JsValue::UNDEFINED)
    })?;
    Ok(())
}
