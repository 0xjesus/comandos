//! Chat view over the terminal area: the session's agent transcript as a conversation,
//! and ‹ › / swipe to move between sessions.
//! The session header (‹ name ›) and the bottom bar (keys + composer) live outside the
//! view and serve both modes: in the terminal the page inside the frame hides its own bar
//! (`dock`) and the keys and actions are forwarded to it.
//! One 1 s timer; it fetches only while the view is visible and the page is shown, and
//! the server answers `unchanged` when the transcript did not move.
#[cfg(target_arch = "wasm32")]
pub use web::install;

#[cfg(target_arch = "wasm32")]
mod web {
use super::{PaneRow, choose, cut, pane_label, pane_number, working};
use comandos_web_view::escape::text as esc;
use crate::components::web_support::*;
use comandos_web_dom::port::*;
use serde_json::json;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;

const STORE: &str = "comandos.viewMode";
const BUBBLE: &str = r#"<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12Z"/></svg>"#;
const TERM: &str = r#"<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m5 8 4 4-4 4M12 17h7"/></svg>"#;

#[derive(Default)]
struct Chat {
    on: Cell<bool>,
    session: RefCell<String>,
    token: RefCell<String>,
    busy: Cell<bool>,
    ticks: Cell<u32>,
    swipe: Cell<Option<(f64, f64)>>,
    /// Older messages are folded behind a button until asked for.
    expanded: Cell<bool>,
    /// The transcript as last received (role, server-rendered HTML), and the slice drawn
    /// into `.cv-list` (one child each): a refresh rewrites only the tail that changed.
    messages: RefCell<Vec<(String, String)>>,
    shown: RefCell<Vec<(String, String)>>,
    age: Cell<f64>,
    /// Session whose name the header shows.
    titled: RefCell<String>,
    /// Chips drawn in the dock (unchanged rows skip the DOM).
    chips: RefCell<String>,
    /// `session|pane` last focused in the terminal frame for this phone.
    synced: RefCell<String>,
}
/// Shared by chat and terminal: the session header (‹ name ›) above, keys and composer below.
const BARS: &str = r#"<header id="cv-head" class="cv-head"><button type="button" data-cv="prev" aria-label="Sesión anterior">‹</button><div class="cv-title"><b></b><span></span></div><button type="button" data-cv="next" aria-label="Sesión siguiente">›</button></header><div id="cv-dock"><div class="cv-panes" role="toolbar" aria-label="Panes de la sesión" hidden></div><div class="cv-keys" role="toolbar" aria-label="Teclas y herramientas de la terminal"><button type="button" class="hot" data-key="escape">Esc</button><button type="button" class="hot" data-key="ctrlc" title="Interrumpir (Ctrl+C)" aria-label="Enviar Ctrl+C">Ctrl+C</button><button type="button" data-action="mode">Seleccionar</button><button type="button" data-action="panes">Paneles</button><button type="button" data-key="backspace" title="Borrar un carácter (Backspace)" aria-label="Borrar un carácter en la terminal">⌫</button><button type="button" data-action="history">Historial</button><button type="button" data-action="copy">Copiar</button><button type="button" data-action="paste">Pegar</button><button type="button" data-action="keyboard" title="Escribir directo en la terminal">Teclado directo</button><button type="button" data-action="ctrl">Ctrl</button><button type="button" data-key="tab">Tab</button><button type="button" data-key="left" aria-label="Izquierda" title="Izquierda"><svg aria-hidden="true" viewBox="0 0 24 24"><path d="m15 18-6-6 6-6"/></svg></button><button type="button" data-key="up" aria-label="Arriba" title="Arriba"><svg aria-hidden="true" viewBox="0 0 24 24"><path d="m18 15-6-6-6 6"/></svg></button><button type="button" data-key="down" aria-label="Abajo" title="Abajo"><svg aria-hidden="true" viewBox="0 0 24 24"><path d="m6 9 6 6 6-6"/></svg></button><button type="button" data-key="right" aria-label="Derecha" title="Derecha"><svg aria-hidden="true" viewBox="0 0 24 24"><path d="m9 18 6-6-6-6"/></svg></button><button type="button" data-cv-key="1">1</button><button type="button" data-cv-key="2">2</button></div><form class="cv-compose"><textarea rows="1" aria-label="Mensaje para la sesión" placeholder="Escribe a la sesión…"></textarea><button type="submit" aria-label="Enviar (vacío: Enter)" title="Enviar; vacío envía Enter"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12h14M13 6l6 6-6 6"/></svg></button></form></div>"#;
const SKELETON: &str = "<div class=\"cv-skel\" role=\"status\" aria-label=\"Cargando conversación\"><i></i><i></i><i></i></div>";

fn text(v: &JsValue) -> String {
    v.as_string().unwrap_or_default()
}
fn el() -> JsValue {
    id("chat-view")
}
fn tabs() -> Vec<String> {
    all(&id("tabbar"), ".apptab")
        .iter()
        .map(|t| text(&get(t, "_target")))
        .filter(|t| t.starts_with("term:"))
        .collect()
}
/// The active session's terminal frame (null when none is open).
fn frame() -> JsValue {
    let active = global("activeTerm");
    if !truthy(&active) {
        return JsValue::NULL;
    }
    get(&call(&global("openTerms"), "get", &[active]).unwrap_or(JsValue::NULL), "frame")
}
fn post(frame: &JsValue, message: serde_json::Value) -> bool {
    let window = get(frame, "contentWindow");
    truthy(&window)
        && from_json(&message)
            .and_then(|m| call(&window, "postMessage", &[m, get(&global("location"), "origin")]))
            .is_ok()
}
fn toolbar(term: serde_json::Value) -> bool {
    post(&frame(), json!({"source":"comandos","type":"toolbar","term":term}))
}
const DOCK: &str = r#"{"source":"comandos","type":"dock","on":true}"#;
fn dock_all() {
    let frames = invoke(&get(&global("Array"), "from"), &[call(&global("openTerms"), "values", &[]).unwrap_or(JsValue::NULL)])
        .unwrap_or(JsValue::NULL);
    for i in 0..number(&get(&frames, "length")) as u32 {
        let _ = post(&get(&get(&frames, &i.to_string()), "frame"), serde_json::from_str(DOCK).unwrap_or_default());
    }
}
/// The live panes of a session, in tmux order (`/states` rows, polled by the page).
fn rows(session: &str) -> Vec<PaneRow> {
    let state = invoke(&global("__comandosState"), &[]).unwrap_or_else(|_| global("S"));
    let list = get(&state, "list");
    let mut out: Vec<PaneRow> = Vec::new();
    for i in 0..number(&get(&list, "length")) as u32 {
        let it = get(&list, &i.to_string());
        let pane = text(&get(&it, "pane"));
        if text(&get(&it, "session")) != session
            || pane.is_empty()
            || !truthy(&get(&it, "alive"))
            || out.iter().any(|r| r.pane == pane)
        {
            continue;
        }
        out.push(PaneRow {
            pane,
            agent: text(&get(&it, "agent")),
            model: text(&get(&it, "model")),
            status: text(&get(&it, "status")),
            active: truthy(&get(&it, "paneActive")),
        });
    }
    out.sort_by_key(|r| pane_number(&r.pane));
    out
}
fn saved(session: &str) -> String {
    comandos_web_dom::storage::get(&format!("comandos.pane.{session}")).unwrap_or_default()
}
fn save(session: &str, pane: &str) {
    let _ = comandos_web_dom::storage::set(&format!("comandos.pane.{session}"), pane);
}
fn phone() -> bool {
    global("innerWidth").as_f64().is_some_and(|w| w <= 747.)
}
fn step(delta: i32) {
    let list = tabs();
    if list.is_empty() {
        return;
    }
    let current = format!("term:{}", text(&global("activeTerm")));
    let at = list.iter().position(|t| *t == current).unwrap_or(0) as i32;
    let next = (at + delta).rem_euclid(list.len() as i32) as usize;
    let _ = invoke(&global("showView"), &[list[next].as_str().into(), true.into()]);
}

impl Chat {
    /// The pane this phone writes to and reads as chat (empty: let the server pick).
    fn target(&self, session: &str) -> (String, Vec<PaneRow>) {
        let rows = rows(session);
        let before = saved(session);
        let (pane, closed) = choose(&rows, &before);
        if closed {
            let name = rows.iter().find(|r| r.pane == pane).map(pane_label).unwrap_or_default();
            toast(&format!("El pane que veías se cerró; ahora estás en {name}"));
        }
        if !rows.is_empty() && pane != before {
            save(session, &pane);
        }
        (pane, rows)
    }
    /// Big chips above the keys (phones, splits only) and the composer naming its target.
    fn chips(&self, pane: &str, rows: &[PaneRow]) {
        let split = rows.len() > 1;
        let html: String = if split && phone() {
            rows.iter()
                .enumerate()
                .map(|(i, r)| {
                    let on = r.pane == pane;
                    format!(
                        "<button type=\"button\" class=\"cv-pane\" data-cv-pane=\"{}\" aria-pressed=\"{on}\"><i class=\"{}\"></i><b>{}</b>{}</button>",
                        esc(&r.pane),
                        esc(&r.status),
                        i + 1,
                        esc(&pane_label(r))
                    )
                })
                .collect()
        } else {
            String::new()
        };
        if *self.chips.borrow() == html {
            return;
        }
        let host = query(&id("cv-dock"), ".cv-panes");
        let _ = set(&host, "innerHTML", &html.clone().into());
        let _ = set(&host, "hidden", &html.is_empty().into());
        let target = rows
            .iter()
            .position(|r| r.pane == pane)
            .filter(|_| split)
            .map(|i| format!("Escribe a {} · {}…", i + 1, pane_label(&rows[i])))
            .unwrap_or_else(|| "Escribe a la sesión…".into());
        attr(&query(&id("cv-dock"), "textarea"), "placeholder", &target);
        self.chips.replace(html);
    }
    /// The terminal frame focuses the same pane for this phone only (tmux per-client
    /// focus): toolbar keys and the direct keyboard land there, the desktop keeps its own.
    fn sync(&self, session: &str, pane: &str, split: bool) {
        let key = format!("{session}|{pane}");
        if !split || pane.is_empty() || *self.synced.borrow() == key {
            return;
        }
        if post(&frame(), json!({"source":"comandos","type":"select-pane","pane":pane})) {
            self.synced.replace(key);
        }
    }
    fn head(&self, session: String) {
        let row = all(&id("tabbar"), ".apptab")
            .into_iter()
            .find(|t| text(&get(t, "_target")) == format!("term:{session}"));
        let field = |sel: &str| {
            row.as_ref()
                .map(|r| text(&get(&query(r, sel), "textContent")).trim().to_owned())
                .unwrap_or_default()
        };
        let model = field(".mdl");
        let name = Some(field(".lbl")).filter(|n| !n.is_empty()).unwrap_or(session);
        let _ = set(&query(&id("cv-head"), ".cv-title b"), "textContent", &name.into());
        let _ = set(&query(&id("cv-head"), ".cv-title span"), "textContent", &model.into());
    }
    fn show(self: &Rc<Self>, on: bool) {
        self.on.set(on);
        let _ = call(&global("localStorage"), "setItem", &[STORE.into(), (if on { "chat" } else { "term" }).into()]);
        let _ = set(&el(), "hidden", &(!on).into());
        classes(&get(&doc(), "body"), "chat-on", on);
        let button = id("tab-chat");
        attr(&button, "aria-pressed", if on { "true" } else { "false" });
        attr(&button, "title", if on { "Ver terminal" } else { "Ver como chat" });
        attr(&button, "aria-label", if on { "Ver terminal" } else { "Ver como chat" });
        let _ = set(&button, "innerHTML", &(if on { TERM } else { BUBBLE }).into());
        if on {
            let active = text(&global("activeTerm"));
            if !active.is_empty() && text(&global("activeView")) != format!("term:{active}") {
                let _ = invoke(&global("showView"), &[format!("term:{active}").into(), true.into()]);
            }
            self.session.replace(String::new());
            self.tick(true);
        }
    }
    fn tick(self: &Rc<Self>, force: bool) {
        if text(&get(&doc(), "visibilityState")) == "hidden" {
            return;
        }
        let session = text(&global("activeTerm"));
        if session.is_empty() {
            return;
        }
        self.ticks.set(self.ticks.get().wrapping_add(1));
        let (pane, rows) = self.target(&session);
        self.chips(&pane, &rows);
        self.sync(&session, &pane, rows.len() > 1);
        // The header also shows over the terminal on phones; names and models change
        // rarely, so it is refreshed on a session move and every few seconds.
        if *self.titled.borrow() != session || self.ticks.get() % 5 == 0 {
            self.titled.replace(session.clone());
            self.head(session.clone());
        }
        if !self.on.get() {
            return;
        }
        let key = format!("{session}|{pane}");
        let moved = *self.session.borrow() != key;
        if moved {
            self.session.replace(key.clone());
            self.token.replace(String::new());
            self.expanded.set(false);
            self.messages.borrow_mut().clear();
            self.shown.borrow_mut().clear();
            let _ = set(&query(&el(), ".cv-list"), "innerHTML", &SKELETON.into());
            attr(&query(&el(), ".cv-more"), "hidden", "");
            let _ = set(&query(&el(), ".cv-typing"), "hidden", &true.into());
        }
        if self.busy.get() || !(force || moved || self.ticks.get() % 2 == 0) {
            return;
        }
        self.busy.set(true);
        let me = self.clone();
        spawn_local(async move {
            let since = me.token.borrow().clone();
            let body = from_json(&json!({"session":session,"pane":pane,"since":since})).unwrap_or(JsValue::NULL);
            let reply = request("POST", "/chat/transcript", body).await;
            me.busy.set(false);
            if *me.session.borrow() != key {
                return;
            }
            let Ok(reply) = reply else { return };
            let v = to_json(&get(&reply, "body"));
            me.age.set(v["age"].as_f64().unwrap_or(f64::MAX));
            if v["unchanged"] == true {
                me.typing();
                return;
            }
            if v["agent"].is_null() {
                me.shown.borrow_mut().clear();
                let empty = if rows.len() > 1 {
                    "<p class=\"cv-empty\">Este pane es una terminal sin agente con historial. Elige otro pane arriba del teclado o usa la terminal.</p>"
                } else {
                    "<p class=\"cv-empty\">Esta sesión no tiene un agente con historial (Claude o Codex). Usa la terminal.</p>"
                };
                let _ = set(&query(&el(), ".cv-list"), "innerHTML", &empty.into());
                return;
            }
            me.token.replace(v["token"].as_str().unwrap_or("").into());
            let messages = v["messages"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .map(|m| (m["r"].as_str().unwrap_or("t").to_owned(), m["h"].as_str().unwrap_or("").to_owned()))
                        .collect()
                })
                .unwrap_or_default();
            me.messages.replace(messages);
            me.draw(since.is_empty());
        });
    }
    /// Draws the visible slice into the list, rewriting only from the first message
    /// that changed, and keeps the reader at the bottom when they were there.
    fn draw(&self, first: bool) {
        let feed = query(&el(), ".cv-feed");
        let list = query(&el(), ".cv-list");
        let pinned = first
            || number(&get(&feed, "scrollHeight")) - number(&get(&feed, "scrollTop")) - number(&get(&feed, "clientHeight")) < 120.;
        let messages = self.messages.borrow();
        let folded = cut(&messages);
        let from = if self.expanded.get() { 0 } else { folded };
        for pending in all(&feed, ":scope > .cv-me.pending") {
            let _ = call(&pending, "remove", &[]);
        }
        let slice = &messages[from..];
        let mut shown = self.shown.borrow_mut();
        let children = get(&list, "children");
        let count = number(&get(&children, "length")) as usize;
        let keep = if count == shown.len() {
            shown.iter().zip(slice).take_while(|(a, b)| a == b).count()
        } else {
            0
        };
        if keep == 0 {
            let html = if slice.is_empty() {
                "<p class=\"cv-empty\">Sin conversación todavía. Escribe abajo o vuelve a la terminal.</p>".to_owned()
            } else {
                slice.iter().map(|(_, h)| h.as_str()).collect()
            };
            let _ = set(&list, "innerHTML", &html.into());
        } else {
            for i in (keep..count).rev() {
                let _ = call(&get(&children, &i.to_string()), "remove", &[]);
            }
            let _ = call(&list, "insertAdjacentHTML", &["beforeend".into(), slice[keep..].iter().map(|(_, h)| h.as_str()).collect::<String>().into()]);
        }
        *shown = if slice.is_empty() { Vec::new() } else { slice.to_vec() };
        let more = query(&el(), ".cv-more");
        let _ = set(&more, "hidden", &(folded == 0).into());
        let label = match (self.expanded.get(), folded) {
            (true, _) => "Ocultar mensajes anteriores".to_owned(),
            (false, 1) => "Ver 1 mensaje anterior".to_owned(),
            (false, n) => format!("Ver {n} mensajes anteriores"),
        };
        let _ = set(&more, "textContent", &label.into());
        drop(shown);
        drop(messages);
        self.typing();
        if pinned {
            let _ = set(&feed, "scrollTop", &get(&feed, "scrollHeight"));
        }
    }
    fn typing(&self) {
        let busy = working(&self.messages.borrow(), self.age.get());
        let _ = set(&query(&el(), ".cv-typing"), "hidden", &(!busy).into());
    }
    fn toggle_more(&self) {
        let feed = query(&el(), ".cv-feed");
        let from_bottom = number(&get(&feed, "scrollHeight")) - number(&get(&feed, "scrollTop"));
        self.expanded.set(!self.expanded.get());
        self.draw(false);
        if self.expanded.get() {
            let _ = set(&feed, "scrollTop", &(number(&get(&feed, "scrollHeight")) - from_bottom).into());
        }
    }
    fn send(self: &Rc<Self>) {
        let area = query(&id("cv-dock"), "textarea");
        let value = text(&get(&area, "value"));
        let session = text(&global("activeTerm"));
        if session.is_empty() {
            return;
        }
        // An empty draft makes the arrow a plain Enter: confirms the option highlighted in
        // the agent's menu (a model, a permission, …).
        if value.trim().is_empty() {
            self.press("yes");
            return;
        }
        let _ = set(&area, "value", &"".into());
        if self.on.get() {
            let feed = query(&el(), ".cv-feed");
            let _ = call(&feed, "insertAdjacentHTML", &["beforeend".into(), format!("<div class=\"cv-me pending\"><p>{}</p></div>", esc(&value)).into()]);
            let _ = set(&query(&el(), ".cv-typing"), "hidden", &false.into());
            let _ = call(&feed, "append", &[query(&el(), ".cv-typing")]);
            let _ = set(&feed, "scrollTop", &get(&feed, "scrollHeight"));
        }
        let button = query(&id("cv-dock"), "button[type=submit]");
        classes(&button, "busy", true);
        let pane = self.target(&session).0;
        let me = self.clone();
        spawn_local(async move {
            let body = from_json(&json!({"session":session,"pane":pane,"text":value})).unwrap_or(JsValue::NULL);
            match request("POST", "/send", body).await {
                Ok(r) if number(&get(&r, "status")) < 300. => {}
                _ => {
                    toast("No se pudo enviar el mensaje a la sesión");
                    for pending in all(&query(&el(), ".cv-feed"), ":scope > .cv-me.pending") {
                        let _ = call(&pending, "remove", &[]);
                    }
                }
            }
            classes(&button, "busy", false);
            me.age.set(0.);
            me.tick(true);
        });
    }
    /// A terminal key: sent by the session's frame (it stays connected under the chat).
    fn press(&self, key: &str) {
        if toolbar(json!({"type":"toolbar","key":key})) {
            return;
        }
        match key {
            "yes" => self.key("Enter"),
            "escape" => self.key("Escape"),
            "tab" => self.key("Tab"),
            "up" => self.key("Up"),
            "down" => self.key("Down"),
            _ => toast("Abre una sesión primero"),
        }
    }
    /// A terminal tool (select, panes, history, copy, paste, direct keyboard, Ctrl): it acts
    /// on the terminal, so the chat gives way to it first. Paste in the chat fills the draft.
    fn action(self: &Rc<Self>, action: &str) {
        if self.on.get() && action == "paste" {
            spawn_local(async move {
                let read = match call(&get(&global("navigator"), "clipboard"), "readText", &[]) {
                    Ok(p) => wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&p)).await.ok().map(|v| text(&v)),
                    Err(_) => None,
                };
                match read {
                    Some(clip) if !clip.is_empty() => {
                        let area = query(&id("cv-dock"), "textarea");
                        let _ = set(&area, "value", &format!("{}{clip}", text(&get(&area, "value"))).into());
                        let _ = call(&area, "focus", &[]);
                    }
                    _ => {
                        toast("No se pudo leer el portapapeles; mantén presionado el campo para pegar");
                    }
                }
            });
            return;
        }
        if self.on.get() {
            self.show(false);
        }
        if !toolbar(json!({"type":"action","action":action})) {
            toast("Abre una sesión primero");
        }
    }
    fn key(&self, key: &str) {
        let session = text(&global("activeTerm"));
        if session.is_empty() {
            return;
        }
        let key = key.to_owned();
        let pane = self.target(&session).0;
        spawn_local(async move {
            let body = from_json(&json!({"session":session,"pane":pane,"key":key})).unwrap_or(JsValue::NULL);
            if !matches!(request("POST", "/key", body).await, Ok(r) if number(&get(&r, "status")) < 300.) {
                toast("No se pudo enviar la tecla");
            }
        });
    }
}

/// Mounts the view and its toggle once; remote dashboards only.
pub fn install() -> Result<(), JsValue> {
    let area = id("term-area");
    if !truthy(&area) || truthy(&el()) {
        return Ok(());
    }
    call(&area, "insertAdjacentHTML", &["beforeend".into(), r#"<section id="chat-view" hidden aria-label="Sesión como chat"><div class="cv-feed"><button type="button" class="cv-more" data-cv="more" hidden></button><div class="cv-list" aria-live="polite"></div><div class="cv-typing" role="status" aria-label="El agente está trabajando" hidden><i></i><i></i><i></i></div></div></section>"#.into()])?;
    call(&area, "insertAdjacentHTML", &["afterend".into(), BARS.into()])?;
    let open = id("tab-open");
    if truthy(&open) {
        call(&open, "insertAdjacentHTML", &["beforebegin".into(), format!(r#"<button type="button" id="tab-chat" class="tab-nav-btn" aria-pressed="false" aria-label="Ver como chat" title="Ver como chat">{BUBBLE}</button>"#).into()])?;
    }
    let chat = Rc::new(Chat::default());
    dock_all();
    let me_pane = chat.clone();
    // Frames that (re)load after this ask for nothing; they announce `ready`.
    listen(&js_sys::global(), "message", function(move |args| {
        let e = args.get(0);
        let data = get(&e, "data");
        let data = if data.is_string() { invoke(&get(&global("JSON"), "parse"), &[data]).unwrap_or(JsValue::NULL) } else { data };
        let from_term = text(&get(&data, "source")) == "comandos-term";
        let pane = text(&get(&data, "pane"));
        // A pane touched inside the terminal becomes this phone's target too.
        if from_term && text(&get(&data, "type")) == "pane-selected" && pane.starts_with('%')
            && get(&e, "source") == get(&frame(), "contentWindow")
        {
            let session = text(&global("activeTerm"));
            if !session.is_empty() {
                save(&session, &pane);
                me_pane.synced.replace(format!("{session}|{pane}"));
                me_pane.tick(true);
            }
        }
        if from_term && text(&get(&data, "type")) == "ready" {
            // A reloaded frame lost its per-client focus: select again.
            me_pane.synced.replace(String::new());
            let source = get(&e, "source");
            if let Ok(m) = invoke(&get(&global("JSON"), "parse"), &[DOCK.into()]) {
                let _ = call(&source, "postMessage", &[m, get(&global("location"), "origin")]);
            }
        }
        Ok(JsValue::UNDEFINED)
    }));
    let me = chat.clone();
    listen(&id("tab-chat"), "click", function(move |_| {
        if !me.on.get() && !has_term() {
            toast("Abre una sesión primero");
            return Ok(JsValue::UNDEFINED);
        }
        me.show(!me.on.get());
        Ok(JsValue::UNDEFINED)
    }));
    let me = chat.clone();
    let clicks = function(move |args| {
        let target = get(&args.get(0), "target");
        let button = call(&target, "closest", &["button".into()]).unwrap_or(JsValue::NULL);
        if !truthy(&button) {
            return Ok(JsValue::UNDEFINED);
        }
        let data = get(&button, "dataset");
        match text(&get(&data, "cv")).as_str() {
            "prev" => {
                step(-1);
                me.tick(true);
            }
            "next" => {
                step(1);
                me.tick(true);
            }
            "more" => me.toggle_more(),
            _ => {}
        }
        let pane = text(&get(&data, "cvPane"));
        if !pane.is_empty() {
            let session = text(&global("activeTerm"));
            if !session.is_empty() {
                save(&session, &pane);
                me.tick(true);
            }
        }
        let key = text(&get(&data, "cvKey"));
        if !key.is_empty() {
            me.key(&key);
        }
        let key = text(&get(&data, "key"));
        if !key.is_empty() {
            me.press(&key);
        }
        let action = text(&get(&data, "action"));
        if !action.is_empty() {
            me.action(&action);
        }
        Ok(JsValue::UNDEFINED)
    });
    for host in [el(), id("cv-head"), id("cv-dock")] {
        listen(&host, "click", clicks.clone());
    }
    // Tapping a key or a pane chip leaves the keyboard and the draft's focus where they were.
    for row in [".cv-keys", ".cv-panes"] {
        listen(&query(&id("cv-dock"), row), "mousedown", function(|args| {
            let _ = call(&args.get(0), "preventDefault", &[]);
            Ok(JsValue::UNDEFINED)
        }));
    }
    let me = chat.clone();
    listen(&query(&id("cv-dock"), ".cv-compose"), "submit", function(move |args| {
        stop(&args.get(0));
        me.send();
        Ok(JsValue::UNDEFINED)
    }));
    let me = chat.clone();
    listen(&query(&id("cv-dock"), "textarea"), "keydown", function(move |args| {
        let e = args.get(0);
        let coarse = truthy(&call(&js_sys::global(), "matchMedia", &["(pointer:coarse)".into()]).map(|m| get(&m, "matches")).unwrap_or(JsValue::FALSE));
        if text(&get(&e, "key")) == "Enter" && !truthy(&get(&e, "shiftKey")) && !coarse {
            stop(&e);
            me.send();
        }
        Ok(JsValue::UNDEFINED)
    }));
    let feed = query(&el(), ".cv-feed");
    let me = chat.clone();
    listen(&feed, "touchstart", function(move |args| {
        let t = get(&get(&args.get(0), "touches"), "0");
        me.swipe.set(Some((number(&get(&t, "clientX")), number(&get(&t, "clientY")))));
        Ok(JsValue::UNDEFINED)
    }));
    let me = chat.clone();
    listen(&feed, "touchend", function(move |args| {
        let Some((x, y)) = me.swipe.take() else { return Ok(JsValue::UNDEFINED) };
        let t = get(&get(&args.get(0), "changedTouches"), "0");
        let (dx, dy) = (number(&get(&t, "clientX")) - x, number(&get(&t, "clientY")) - y);
        if dx.abs() > 70. && dx.abs() > dy.abs() * 2. {
            step(if dx < 0. { 1 } else { -1 });
        }
        Ok(JsValue::UNDEFINED)
    }));
    let me = chat.clone();
    let _ = invoke(&global("setInterval"), &[function(move |_| {
        me.tick(false);
        Ok(JsValue::UNDEFINED)
    }), 1000.into()]);
    let me = chat.clone();
    listen(&doc(), "visibilitychange", function(move |_| {
        me.tick(true);
        Ok(JsValue::UNDEFINED)
    }));
    let saved = call(&global("localStorage"), "getItem", &[STORE.into()]).ok().map(|v| text(&v));
    // Phones open in chat unless the person chose the terminal before.
    let phone = global("innerWidth").as_f64().is_some_and(|w| w <= 747.);
    if saved.as_deref() == Some("chat") || phone && saved.as_deref().unwrap_or("").is_empty() {
        chat.show(true);
    }
    Ok(())
}
fn has_term() -> bool {
    !text(&global("activeTerm")).is_empty()
}
}

/// One pane of the active session, from the `/states` rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaneRow {
    pub pane: String,
    pub agent: String,
    pub model: String,
    pub status: String,
    pub active: bool,
}
pub fn pane_number(pane: &str) -> u64 {
    pane.trim_start_matches('%').parse().unwrap_or(u64::MAX)
}
/// The pane the phone writes to and reads as chat: the one saved for this tab while it
/// lives, else tmux's active one, else the first. A saved pane that closed gives way to
/// the next one (`true`: tell the person). No rows yet: keep what was saved.
pub fn choose(rows: &[PaneRow], saved: &str) -> (String, bool) {
    if rows.is_empty() || rows.iter().any(|r| r.pane == saved) {
        return (saved.to_owned(), false);
    }
    if !saved.is_empty() {
        let after = pane_number(saved);
        let next = rows.iter().find(|r| pane_number(&r.pane) > after).unwrap_or(&rows[0]);
        return (next.pane.clone(), true);
    }
    (rows.iter().find(|r| r.active).unwrap_or(&rows[0]).pane.clone(), false)
}
/// «Claude · opus-5.5», «Codex · gpt-6-astra», «Terminal».
pub fn pane_label(row: &PaneRow) -> String {
    let agent = match row.agent.as_str() {
        "" => return "Terminal".into(),
        "claude" => "Claude".to_owned(),
        "codex" => "Codex".to_owned(),
        other => {
            let mut c = other.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    };
    let model = row.model.strip_prefix("claude-").unwrap_or(&row.model);
    let parts: Vec<&str> = model.split('-').collect();
    let model = match parts.as_slice() {
        [head @ .., a, b] if !head.is_empty() && a.bytes().all(|c| c.is_ascii_digit()) && b.bytes().all(|c| c.is_ascii_digit()) => {
            format!("{}-{a}.{b}", head.join("-"))
        }
        _ => model.to_owned(),
    };
    if model.is_empty() { agent } else { format!("{agent} · {model}") }
}

/// Where the collapsed view starts: the last agent answer (and what followed it), or
/// the last prompt while no answer exists yet.
pub fn cut(messages: &[(String, String)]) -> usize {
    messages
        .iter()
        .rposition(|(r, _)| r == "a")
        .or_else(|| messages.iter().rposition(|(r, _)| r == "u"))
        .unwrap_or(0)
}
/// The agent is still on it: the transcript moved recently and does not end in an answer.
pub fn working(messages: &[(String, String)], age: f64) -> bool {
    age < 90. && messages.last().is_some_and(|(r, _)| r != "a")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn m(r: &str, t: &str) -> (String, String) {
        (r.into(), t.into())
    }
    fn row(pane: &str, agent: &str, model: &str, active: bool) -> PaneRow {
        PaneRow { pane: pane.into(), agent: agent.into(), model: model.into(), status: String::new(), active }
    }
    #[test]
    fn the_phone_keeps_its_pane_and_moves_on_when_it_closes() {
        let rows = [row("%7", "claude", "", false), row("%39", "claude", "", true), row("%44", "", "", false)];
        assert_eq!(choose(&rows, ""), ("%39".into(), false), "first time: tmux's active pane");
        assert_eq!(choose(&rows, "%7"), ("%7".into(), false), "a saved pane wins over tmux focus");
        assert_eq!(choose(&rows, "%40"), ("%44".into(), true), "closed: the next one, with a notice");
        assert_eq!(choose(&rows, "%99"), ("%7".into(), true), "closed last: wrap to the first");
        assert_eq!(choose(&[], "%7"), ("%7".into(), false), "rows not loaded yet: keep it");
        assert_eq!(choose(&rows[..1], ""), ("%7".into(), false));
    }
    #[test]
    fn pane_chips_name_agent_and_model() {
        assert_eq!(pane_label(&row("%1", "claude", "claude-opus-5-5", false)), "Claude · opus-5.5");
        assert_eq!(pane_label(&row("%1", "claude", "claude-fable-5-1", false)), "Claude · fable-5.1");
        assert_eq!(pane_label(&row("%1", "codex", "gpt-6-astra", false)), "Codex · gpt-6-astra");
        assert_eq!(pane_label(&row("%1", "codex", "gpt-6.1-sol", false)), "Codex · gpt-6.1-sol");
        assert_eq!(pane_label(&row("%1", "claude", "", false)), "Claude");
        assert_eq!(pane_label(&row("%1", "", "", false)), "Terminal");
        assert_eq!(pane_label(&row("%1", "grok", "", false)), "Grok");
    }
    #[test]
    fn collapsed_view_starts_at_the_last_answer() {
        let list = [m("u", "a"), m("a", "b"), m("u", "c"), m("t", "Bash"), m("a", "d"), m("t", "Read")];
        assert_eq!(cut(&list), 4);
        assert_eq!(cut(&list[..4]), 1);
        assert_eq!(cut(&[m("t", "x"), m("u", "y")]), 1);
        assert!(working(&list, 5.));
        assert!(!working(&list, 600.));
        assert!(!working(&list[..5], 5.));
    }
}
