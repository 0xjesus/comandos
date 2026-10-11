//! Chat view over the terminal area: the session's agent transcript as a conversation,
//! a composer that types into its pane, and ‹ › / swipe to move between sessions.
//! One 1 s timer; it fetches only while the view is visible and the page is shown, and
//! the server answers `unchanged` when the transcript did not move.
#[cfg(target_arch = "wasm32")]
pub use web::install;

#[cfg(target_arch = "wasm32")]
mod web {
use super::{cut, working};
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
}
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
    fn head(&self) {
        let session = self.session.borrow().clone();
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
        let _ = set(&query(&el(), ".cv-title b"), "textContent", &name.into());
        let _ = set(&query(&el(), ".cv-title span"), "textContent", &model.into());
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
        if !self.on.get() || text(&get(&doc(), "visibilityState")) == "hidden" {
            return;
        }
        let session = text(&global("activeTerm"));
        if session.is_empty() {
            return;
        }
        let moved = *self.session.borrow() != session;
        if moved {
            self.session.replace(session.clone());
            self.token.replace(String::new());
            self.expanded.set(false);
            self.messages.borrow_mut().clear();
            self.shown.borrow_mut().clear();
            let _ = set(&query(&el(), ".cv-list"), "innerHTML", &SKELETON.into());
            attr(&query(&el(), ".cv-more"), "hidden", "");
            let _ = set(&query(&el(), ".cv-typing"), "hidden", &true.into());
            self.head();
        }
        self.ticks.set(self.ticks.get().wrapping_add(1));
        if self.busy.get() || !(force || moved || self.ticks.get() % 2 == 0) {
            return;
        }
        self.busy.set(true);
        let me = self.clone();
        spawn_local(async move {
            let since = me.token.borrow().clone();
            let body = from_json(&json!({"session":session,"since":since})).unwrap_or(JsValue::NULL);
            let reply = request("POST", "/chat/transcript", body).await;
            me.busy.set(false);
            if *me.session.borrow() != session {
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
                let _ = set(&query(&el(), ".cv-list"), "innerHTML", &"<p class=\"cv-empty\">Esta sesión no tiene un agente con historial (Claude o Codex). Usa la terminal.</p>".into());
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
        let area = query(&el(), ".cv-compose textarea");
        let value = text(&get(&area, "value"));
        let session = text(&global("activeTerm"));
        if value.trim().is_empty() || session.is_empty() {
            return;
        }
        let _ = set(&area, "value", &"".into());
        let feed = query(&el(), ".cv-feed");
        let _ = call(&feed, "insertAdjacentHTML", &["beforeend".into(), format!("<div class=\"cv-me pending\"><p>{}</p></div>", esc(&value)).into()]);
        let _ = set(&query(&el(), ".cv-typing"), "hidden", &false.into());
        let _ = call(&feed, "append", &[query(&el(), ".cv-typing")]);
        let _ = set(&feed, "scrollTop", &get(&feed, "scrollHeight"));
        let button = query(&el(), ".cv-compose button");
        classes(&button, "busy", true);
        let me = self.clone();
        spawn_local(async move {
            let body = from_json(&json!({"session":session,"text":value})).unwrap_or(JsValue::NULL);
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
    fn key(&self, key: &str) {
        let session = text(&global("activeTerm"));
        if session.is_empty() {
            return;
        }
        let key = key.to_owned();
        spawn_local(async move {
            let body = from_json(&json!({"session":session,"key":key})).unwrap_or(JsValue::NULL);
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
    call(&area, "insertAdjacentHTML", &["beforeend".into(), r#"<section id="chat-view" hidden aria-label="Sesión como chat"><header class="cv-head"><button type="button" data-cv="prev" aria-label="Sesión anterior">‹</button><div class="cv-title"><b></b><span></span></div><button type="button" data-cv="next" aria-label="Sesión siguiente">›</button></header><div class="cv-feed"><button type="button" class="cv-more" data-cv="more" hidden></button><div class="cv-list" aria-live="polite"></div><div class="cv-typing" role="status" aria-label="El agente está trabajando" hidden><i></i><i></i><i></i></div></div><div class="cv-keys" role="toolbar" aria-label="Teclas rápidas"><button type="button" class="y" data-cv-key="Enter">Sí</button><button type="button" class="n" data-cv-key="Escape">No</button><button type="button" data-cv-key="Escape">Esc</button><button type="button" data-cv-key="Up">↑</button><button type="button" data-cv-key="Down">↓</button><button type="button" data-cv-key="1">1</button><button type="button" data-cv-key="2">2</button><button type="button" data-cv-key="Tab">Tab</button></div><form class="cv-compose"><textarea rows="1" aria-label="Mensaje para la sesión" placeholder="Escribe a la sesión…"></textarea><button type="submit" aria-label="Enviar"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12h14M13 6l6 6-6 6"/></svg></button></form></section>"#.into()])?;
    let open = id("tab-open");
    if truthy(&open) {
        call(&open, "insertAdjacentHTML", &["beforebegin".into(), format!(r#"<button type="button" id="tab-chat" class="tab-nav-btn" aria-pressed="false" aria-label="Ver como chat" title="Ver como chat">{BUBBLE}</button>"#).into()])?;
    }
    let chat = Rc::new(Chat::default());
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
    listen(&el(), "click", function(move |args| {
        let target = get(&args.get(0), "target");
        let button = call(&target, "closest", &["button".into()]).unwrap_or(JsValue::NULL);
        if !truthy(&button) {
            return Ok(JsValue::UNDEFINED);
        }
        let data = get(&button, "dataset");
        match text(&get(&data, "cv")).as_str() {
            "prev" => step(-1),
            "next" => step(1),
            "more" => me.toggle_more(),
            _ => {}
        }
        let key = text(&get(&data, "cvKey"));
        if !key.is_empty() {
            me.key(&key);
        }
        Ok(JsValue::UNDEFINED)
    }));
    let me = chat.clone();
    listen(&query(&el(), ".cv-compose"), "submit", function(move |args| {
        stop(&args.get(0));
        me.send();
        Ok(JsValue::UNDEFINED)
    }));
    let me = chat.clone();
    listen(&query(&el(), ".cv-compose textarea"), "keydown", function(move |args| {
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
