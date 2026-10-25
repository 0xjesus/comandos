use super::resize::ResizeRequest;
use super::*;
use crate::number_text::decimal;
use comandos_web_dom::{events, timers};
use js_sys::{Array, Function, Promise, Reflect};
use serde_json::json;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Element, Event, MouseEvent, ResizeObserver};
type Shared = Rc<RefCell<State>>;
thread_local! {static CURRENT:RefCell<Option<Shared>>=const{RefCell::new(None)};}
struct State {
    session: String,
    token: String,
    term: JsValue,
    layer: Element,
    frames: Element,
    grip: Element,
    menu: Element,
    panes: Vec<Value>,
    models: Vec<Value>,
    busy: bool,
    gutters: Vec<Gutter>,
    hot: Option<usize>,
    drag: Option<(Gutter, Option<Value>)>,
    local: Option<String>,
    local_at: f64,
    remote: Option<String>,
    prefix: f64,
    want: Option<ResizeRequest>,
    sent: Option<ResizeRequest>,
    inflight: bool,
    acct: Option<String>,
    disposed: bool,
    listeners: Vec<events::Listener>,
    timers: Vec<timers::Handle>,
    soon: Option<timers::Handle>,
    callbacks: Vec<Closure<dyn FnMut(JsValue)>>,
    subscriptions: Vec<JsValue>,
    observer: Option<ResizeObserver>,
    observer_cb: Option<Closure<dyn FnMut(Array, ResizeObserver)>>,
}
fn w() -> web_sys::Window {
    web_sys::window().expect("window")
}
fn doc() -> web_sys::Document {
    w().document().expect("document")
}
fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &k.into()).unwrap_or(JsValue::UNDEFINED)
}
fn call(o: &JsValue, k: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let f: Function = get(o, k).dyn_into()?;
    let a = Array::new();
    for x in args {
        a.push(x);
    }
    f.apply(o, &a)
}
fn js(v: &Value) -> JsValue {
    js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL)
}
fn val(v: &JsValue) -> Value {
    comandos_web_dom::port::to_json(v)
}
fn error(e: JsValue) -> String {
    get(&e, "message")
        .as_string()
        .or_else(|| e.as_string())
        .unwrap_or_else(|| "La acción no se completó".into())
}
fn n(o: &JsValue, k: &str) -> f64 {
    get(o, k).as_f64().unwrap_or(0.)
}
fn now() -> f64 {
    js_sys::Date::now()
}
fn query(e: &Element, s: &str) -> Option<Element> {
    e.query_selector(s).ok().flatten()
}
fn all(e: &Element, s: &str) -> Vec<Element> {
    let list = call(e.as_ref(), "querySelectorAll", &[s.into()]).unwrap_or(JsValue::NULL);
    (0..n(&list, "length") as u32)
        .filter_map(|i| get(&list, &i.to_string()).dyn_into().ok())
        .collect()
}
fn html(e: &Element, s: &str) {
    if get(e.as_ref(), "_html").as_string().as_deref() != Some(s) {
        let _ = Reflect::set(e.as_ref(), &"_html".into(), &s.into());
        e.set_inner_html(s);
    }
}
fn dataset(e: &Element, k: &str) -> String {
    e.get_attribute(&format!("data-{k}")).unwrap_or_default()
}
fn rect(e: &Element, k: &str) -> f64 {
    n(
        call(e.as_ref(), "getBoundingClientRect", &[])
            .unwrap_or(JsValue::NULL)
            .as_ref(),
        k,
    )
}
fn screen(s: &State) -> Option<Element> {
    get(&s.term, "screen")
        .dyn_into()
        .ok()
        .or_else(|| doc().query_selector("#term .xterm-screen").ok().flatten())
}
fn cell(s: &State) -> (f64, f64) {
    let (cw, ch) = (n(&s.term, "cellWidth"), n(&s.term, "cellHeight"));
    if cw > 0. && ch > 0. {
        return (cw, ch);
    }
    if let Some(sc) = screen(s) {
        let (cols, rows) = (n(&s.term, "cols"), n(&s.term, "rows"));
        if cols > 0. && rows > 0. {
            return (
                f64::from(sc.client_width()) / cols,
                f64::from(sc.client_height()) / rows,
            );
        }
    }
    (0., 0.)
}
fn geometry(s: &State) -> Option<Geometry> {
    let sc = screen(s)?;
    let (cw, ch) = cell(s);
    if cw <= 0. || ch <= 0. {
        return None;
    }
    Some(Geometry {
        ox: rect(&sc, "left") - rect(&s.layer, "left"),
        oy: rect(&sc, "top") - rect(&s.layer, "top"),
        cw,
        ch,
        cols: n(&s.term, "cols"),
        rows: n(&s.term, "rows"),
    })
}
fn ai(s: &State, pane: &str) -> String {
    let Ok(Some(parent)) = w().parent() else {
        return String::new();
    };
    let wm = get(parent.as_ref(), "ComandosWorkMarks");
    let list = get(&get(parent.as_ref(), "S"), "list");
    let status = (0..n(&list, "length") as u32)
        .find_map(|i| {
            let it = get(&list, &i.to_string());
            (get(&it, "session").as_string().as_deref() == Some(&s.session)
                && get(&it, "pane").as_string().as_deref() == Some(pane))
            .then(|| {
                let st = get(&it, "status");
                if st.is_truthy() {
                    st
                } else {
                    get(&it, "state")
                }
            })
        })
        .unwrap_or(JsValue::UNDEFINED);
    call(&wm, "channels", &["none".into(), status])
        .ok()
        .and_then(|c| call(&wm, "aiIconSvg", &[get(&c, "ai"), 16.into()]).ok())
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}
fn label(s: &State) -> String {
    w().parent()
        .ok()
        .flatten()
        .and_then(|p| {
            call(
                &get(p.as_ref(), "openTerms"),
                "get",
                &[s.session.clone().into()],
            )
            .ok()
        })
        .and_then(|t| get(&t, "label").as_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| s.session.clone())
}
const USER: &str = "<svg viewBox=\"0 0 24 24\" width=\"12\" height=\"12\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\"><circle cx=\"12\" cy=\"8\" r=\"4\"/><path d=\"M4 21c1.5-4 4.5-6 8-6s6.5 2 8 6\"/></svg>";
const SPARK: &str = "<svg viewBox=\"0 0 24 24\" width=\"12\" height=\"12\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\"><path d=\"M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z\"/></svg>";
fn logo(m: &str) -> Option<(&'static str, &'static str)> {
    Some(match m {
        "claude" => ("A", "#d97757"),
        "codex" => ("O", "#10a37f"),
        "grok" => ("G", "#fb923c"),
        "opencode" => ("OC", "#f0abfc"),
        "gemini" => ("G", "#4285f4"),
        "agy" => ("AG", "#93c5fd"),
        "acp" => ("AC", "#67e8f9"),
        _ => return None,
    })
}
fn draw(rc: &Shared) {
    let mut s = rc.borrow_mut();
    if s.disposed {
        return;
    }
    // A phone showing one pane alone: no frames over it and no gutters to drag
    // (they would resize the desktop's split).
    let cropped = crate::page::crop_pane().is_some();
    let Some(a) = geometry(&s).filter(|_| !cropped) else {
        s.frames.set_inner_html("");
        let _ = Reflect::set(s.frames.as_ref(), &"_html".into(), &"".into());
        s.hot = None;
        s.gutters.clear();
        drop(s);
        draw_grip(rc);
        return;
    };
    let on = active(&s.panes, &s.local, &s.remote);
    let mut output = String::new();
    let mut gutters = Vec::new();
    for p in &s.panes {
        let Some((r, gs)) = frame(p, a) else { continue };
        gutters.extend(gs);
        let [x0, y0, x1, y1, rail_ch, _] = r;
        let id = text(p, "id");
        output += &format!(
            "<div class=\"pc-frame{}\" style=\"left:{x0}px;top:{y0}px;width:{}px;height:{}px\"></div>",
            if on.as_deref() == Some(&id) {
                " on"
            } else {
                ""
            },
            decimal(x1 - x0),
            decimal(y1 - y0),
            x0 = decimal(x0),
            y0 = decimal(y0)
        );
        let Some(m) = s.models.iter().find(|m| text(m, "pane") == id) else {
            continue;
        };
        let motor = text(m, "motor");
        let harness = text(m, "harness");
        let (letter, color) = logo(&motor)
            .or_else(|| logo(&harness))
            .unwrap_or(("·", "#56627a"));
        let state = text(m, "state");
        let model = if state == "changing" {
            format!("→ {}", text(m, "target"))
        } else if state == "verified" {
            text(m, "model")
        } else {
            "…".into()
        };
        let model = if model == "→ " {
            "→ …".into()
        } else {
            model
        };
        let hy = y0 + 1.;
        let hh = (rail_ch - hy).max(18.);
        output += &format!(
            "<div class=\"pc-head\" style=\"left:{}px;top:{hy}px;width:{}px;height:{hh}px\">{}<b>{}</b><span class=\"pc-sep\"></span><span class=\"pc-logo\" style=\"background:{color}\">{}</span><span class=\"pc-model\">{}</span>",
            decimal(x0 + 1.),
            decimal(x1 - x0 - 2.),
            ai(&s, &id),
            escape(&label(&s)),
            escape(letter),
            escape(&model),
            hy = decimal(hy),
            hh = decimal(hh)
        );
        let effort = text(m, "effort");
        if !effort.is_empty() {
            output += &format!("<span class=\"pc-eff\">{}</span>", escape(&effort));
        }
        if state == "verified" {
            output += "<span class=\"pc-ok\">✓</span>";
        }
        output += "<span class=\"pc-keys\">";
        if matches!(harness.as_str(), "claude" | "codex" | "grok") {
            let acct = text(m, "hAcct");
            let acct = if acct.is_empty() || acct == "unknown" {
                "…"
            } else {
                &acct
            };
            output += &format!(
                "<button type=\"button\" data-act=\"acct\" data-pane=\"{}\" data-harness=\"{}\" title=\"Pasar este pane a otra cuenta sin perder la conversación\">{USER}<span>Cuenta:</span> <b>{}</b></button>",
                escape(&id),
                escape(&harness),
                escape(acct)
            );
        }
        output += &format!(
            "<button type=\"button\" data-act=\"ext\" data-pane=\"{}\" data-harness=\"{}\" title=\"Extensiones de este pane\">{SPARK}<span>MCPs · Skills</span></button></span></div>",
            escape(&id),
            escape(&harness)
        );
    }
    s.gutters = gutters;
    if s.hot.is_some_and(|i| i >= s.gutters.len()) {
        s.hot = None
    }
    html(&s.frames, &output);
    drop(s);
    draw_grip(rc);
}
fn draw_grip(rc: &Shared) {
    let s = rc.borrow();
    let mut output = String::new();
    if let Some(g) = s.hot.and_then(|i| s.gutters.get(i)) {
        let o = if g.vertical { "v" } else { "h" };
        output = format!(
            "<div class=\"pc-grip {o}{}\" style=\"left:{}px;top:{}px;width:{}px;height:{}px\"><span class=\"ln\"></span><span class=\"pill\"></span>",
            if s.drag.is_some() { " drag" } else { "" },
            decimal(g.x),
            decimal(g.y),
            decimal(g.w),
            decimal(g.h)
        );
        if s.drag.is_some() {
            let measure = s
                .panes
                .iter()
                .find(|p| text(p, "id") == g.pane)
                .map(|p| {
                    let key = if g.vertical { "width" } else { "height" };
                    let v = number(p, key);
                    let nb = neighbor(&s.panes, g)
                        .map(|p| {
                            format!(
                                " {} {}",
                                if g.vertical { "|" } else { "/" },
                                decimal(number(p, key))
                            )
                        })
                        .unwrap_or_default();
                    format!(
                        "{}{nb} {}",
                        decimal(v),
                        if g.vertical { "col" } else { "filas" }
                    )
                })
                .unwrap_or_default();
            output += &format!("<span class=\"tip\">{}</span>", escape(&measure));
        }
        output += "</div>";
    }
    html(&s.grip, &output);
    if let Some(term) = doc().get_element_by_id("term") {
        let g = s.hot.and_then(|i| s.gutters.get(i));
        for (k, on) in [
            ("resize-col", g.is_some_and(|g| g.vertical)),
            ("resize-row", g.is_some_and(|g| !g.vertical)),
            ("grabbing", g.is_some() && s.drag.is_some()),
        ] {
            let _ = term.class_list().toggle_with_force(k, on);
        }
    }
}
fn token(rc: &Shared) -> String {
    rc.borrow().token.clone()
}
pub(crate) async fn fetch(
    token: String,
    path: &str,
    body: Option<Value>,
    check: Option<&str>,
    signal: Option<web_sys::AbortSignal>,
) -> Result<Value, String> {
    let opt = js(&json!({"cache":"no-store","headers":{}}));
    if !token.is_empty() {
        let _ = Reflect::set(
            &get(&opt, "headers"),
            &"X-Comandos-Token".into(),
            &token.into(),
        );
    }
    if let Some(signal) = signal {
        let _ = Reflect::set(&opt, &"signal".into(), signal.as_ref());
    }
    if path.starts_with("/tab-models?") {
        let _ = Reflect::set(
            &get(&opt, "headers"),
            &"Content-Type".into(),
            &"application/json".into(),
        );
    }
    if let Some(body) = body {
        let _ = Reflect::set(&opt, &"method".into(), &"POST".into());
        let _ = Reflect::set(
            &get(&opt, "headers"),
            &"Content-Type".into(),
            &"application/json".into(),
        );
        let _ = Reflect::set(&opt, &"body".into(), &body.to_string().into());
    }
    let p = call(w().as_ref(), "fetch", &[path.into(), opt]).map_err(error)?;
    let response = JsFuture::from(Promise::resolve(&p)).await.map_err(error)?;
    let p = call(&response, "json", &[]).map_err(error)?;
    let data = val(&JsFuture::from(Promise::resolve(&p)).await.map_err(error)?);
    if let Some(fallback) = check
        && (!get(&response, "ok").is_truthy() || data["ok"] == false)
    {
        let e = text(&data, "error");
        return Err(if e.is_empty() { fallback.into() } else { e });
    }
    Ok(data)
}
async fn select_touched_pane(rc: Shared, pane: Value, selected_at: f64) {
    let (session, auth) = {
        let s = rc.borrow();
        (s.session.clone(), s.token.clone())
    };
    let result = fetch(auth, "/terminal-panes", Some(json!({"session":session,"action":"select","scope":"client","pane":pane["id"],"identity":pane["identity"]})), None, None).await;
    let s = rc.borrow();
    if s.disposed || s.local_at != selected_at || s.local.as_deref() != pane["id"].as_str() {
        return;
    }
    let term = s.term.clone();
    drop(s);
    let mut delivered = false;
    if let Ok(result) = result {
        if let Some(keys) = result["clientKeys"].as_str() {
            if call(&term, "sendInput", &[keys.into(), false.into()])
                .ok()
                .and_then(|v| v.as_bool())
                == Some(true)
            {
                delivered = true;
                if let Ok(Some(parent)) = w().parent() {
                    let _ = parent.post_message(&js(&json!({"source":"comandos-term","type":"pane-selected","pane":pane["id"]})), &w().location().origin().unwrap_or_default());
                }
            }
        }
    }
    if !delivered {
        // A disconnected client must be able to retry the same pane.
        rc.borrow_mut().local = None;
        draw(&rc);
    }
    delayed_refresh(&rc, 80);
}

fn refresh(rc: &Shared) {
    if rc.borrow().busy || rc.borrow().disposed || doc().hidden() {
        return;
    }
    rc.borrow_mut().busy = true;
    let rc = rc.clone();
    spawn_local(async move {
        let t0 = now();
        let session = rc.borrow().session.clone();
        let list = fetch(
            token(&rc),
            "/terminal-panes",
            Some(json!({"session":session,"action":"list"})),
            None,
            None,
        );
        let model_path = format!(
            "/tab-models?session={}",
            js_sys::encode_uri_component(&session)
        );
        let models = fetch(token(&rc), &model_path, None, None, None);
        let (list, models) = futures_pair(list, models).await;
        {
            let mut s = rc.borrow_mut();
            if s.disposed {
                return;
            }
            if let (Ok(list), Ok(models)) = (list, models) {
                if let Some(p) = list["panes"].as_array() {
                    s.panes = p.clone();
                    let remote = text(&list, "remoteFocus");
                    let local_gone = s
                        .local
                        .as_ref()
                        .is_some_and(|id| !s.panes.iter().any(|p| text(p, "id") == *id));
                    if local_gone {
                        s.local = None;
                    }
                    if !remote.is_empty()
                        && t0 >= s.local_at
                        && (s.local.is_none() || s.local.as_deref() == Some(remote.as_str()))
                    {
                        s.remote = Some(remote);
                        s.local = None
                    } else if remote.is_empty() {
                        s.remote = None
                    }
                }
                if let Some(m) = models["panes"].as_array() {
                    s.models = m.clone();
                }
            }
            s.busy = false;
        }
        let panes = rc.borrow().panes.clone();
        crate::page::panes_seen(&panes);
        draw(&rc);
    });
}
async fn futures_pair<A: std::future::Future, B: std::future::Future>(
    a: A,
    b: B,
) -> (A::Output, B::Output) {
    let a = Box::pin(a);
    let b = Box::pin(b);
    let mut a = Some(a);
    let mut b = Some(b);
    let mut av = None;
    let mut bv = None;
    std::future::poll_fn(move |cx| {
        if let Some(f) = a.as_mut()
            && let std::task::Poll::Ready(v) = f.as_mut().poll(cx)
        {
            av = Some(v);
            a = None
        }
        if let Some(f) = b.as_mut()
            && let std::task::Poll::Ready(v) = f.as_mut().poll(cx)
        {
            bv = Some(v);
            b = None
        }
        if a.is_none() && b.is_none() {
            std::task::Poll::Ready((av.take().unwrap(), bv.take().unwrap()))
        } else {
            std::task::Poll::Pending
        }
    })
    .await
}
fn resize(rc: &Shared) {
    let want = {
        let mut s = rc.borrow_mut();
        if s.inflight || s.want.is_none() || s.want == s.sent || s.disposed {
            return;
        }
        s.sent = s.want.clone();
        s.inflight = true;
        s.sent.clone().unwrap()
    };
    let rc = rc.clone();
    spawn_local(async move {
        let session = rc.borrow().session.clone();
        let body = want.body(&session);
        let result = fetch(token(&rc), "/terminal-panes", Some(body), None, None).await;
        {
            let mut s = rc.borrow_mut();
            if let Ok(data) = result
                && let Some(p) = data["panes"].as_array()
            {
                s.panes = p.clone()
            }
            s.inflight = false;
        }
        draw(&rc);
        resize(&rc);
    });
}
fn later(rc: &Shared) {
    let next = rc.clone();
    rc.borrow_mut().soon = Some(timers::timeout(60, move || {
        draw(&next);
        refresh(&next);
    }));
}
fn delayed_refresh(rc: &Shared, ms: i32) {
    let next = rc.clone();
    timers::timeout(ms, move || refresh(&next)).forget();
}
fn close_accounts(rc: &Shared) {
    let mut s = rc.borrow_mut();
    s.acct = None;
    s.menu.set_inner_html("");
}
fn buttons(box_: &Element, selector: &str, disable: bool) {
    for b in all(box_, selector) {
        let on = b.class_list().contains("on");
        let _ = Reflect::set(
            b.as_ref(),
            &"disabled".into(),
            &JsValue::from_bool(disable || on),
        );
    }
}
fn say(rc: &Shared, box_: &Element, pane: &str, message: &str, err: bool) {
    if rc.borrow().acct.as_deref() != Some(pane) {
        return;
    }
    if let Some(note) = query(box_, ".foot, .msg") {
        note.remove();
    }
    let _ = box_.insert_adjacent_html(
        "beforeend",
        &format!(
            "<p class=\"msg{}\">{}</p>",
            if err { " err" } else { "" },
            escape(message)
        ),
    );
}
fn open_accounts(rc: &Shared, btn: &Element) {
    let pane = dataset(btn, "pane");
    let harness = dataset(btn, "harness");
    let harness = if harness.is_empty() {
        "claude".into()
    } else {
        harness
    };
    if rc.borrow().acct.as_deref() == Some(&pane) {
        close_accounts(rc);
        return;
    }
    let box_ = doc().create_element("div").unwrap();
    box_.set_class_name("pc-acct");
    {
        let mut s = rc.borrow_mut();
        s.acct = Some(pane.clone());
        let x = (rect(btn, "left") - rect(&s.layer, "left"))
            .min(rect(&s.layer, "width") - 308.)
            .max(8.);
        let y = rect(btn, "bottom") - rect(&s.layer, "top") + 6.;
        let _ = box_.set_attribute(
            "style",
            &format!("left:{}px;top:{}px", decimal(x), decimal(y)),
        );
        box_.set_inner_html(
            "<h4>CUENTA DE ESTE PANE</h4><p class=\"msg\">Consultando cuentas…</p>",
        );
        s.menu.set_inner_html("");
        let _ = s.menu.append_child(&box_);
    }
    let current = rc
        .borrow()
        .models
        .iter()
        .find(|m| text(m, "pane") == pane)
        .map(|m| text(m, "hAcct"))
        .unwrap_or_default();
    let rc = rc.clone();
    spawn_local(async move {
        let response = fetch(
            token(&rc),
            &format!(
                "/accounts?harness={}",
                js_sys::encode_uri_component(&harness)
            ),
            None,
            Some("No se pudieron leer las cuentas"),
            None,
        )
        .await;
        match response {
            Ok(data) => {
                if rc.borrow().acct.as_deref() != Some(&pane) || rc.borrow().disposed {
                    return;
                }
                let accounts = data["accounts"].as_array().cloned().unwrap_or_default();
                let mut output = "<h4>CUENTA DE ESTE PANE</h4>".to_string();
                for a in &accounts {
                    let alias = text(a, "alias");
                    let selectable = comandos_web_dom::api::js_truthy(&a["selectable"]);
                    let on = alias == current && selectable;
                    let identity = text(a, "identity");
                    output += &format!(
                        "<button type=\"button\" class=\"acc{}\" data-alias=\"{}\" {} title=\"{}\"><b>{}</b><span class=\"tag\">{}</span>",
                        if on { " on" } else { "" },
                        escape(&alias),
                        if on { "disabled" } else { "" },
                        escape(&identity),
                        escape(&alias),
                        if on {
                            "en uso"
                        } else if selectable {
                            "cambiar →"
                        } else {
                            "iniciar sesión →"
                        }
                    );
                    if !identity.is_empty() {
                        output += &format!("<span>{}</span>", escape(&identity));
                    }
                    let mut bars = String::new();
                    for l in a["limits"].as_array().into_iter().flatten() {
                        let p = number(l, "percent");
                        bars += &format!(
                            "<span>{}</span><span class=\"bar\"><i class=\"{}\" style=\"width:{p}%\"></i></span><span>{p}%</span>",
                            escape(&text(l, "label")),
                            if p >= 95. {
                                "full"
                            } else if p >= 85. {
                                "warn"
                            } else {
                                ""
                            },
                            p = decimal(p)
                        );
                    }
                    if !bars.is_empty() {
                        output += &format!("<span class=\"bars\">{bars}</span>");
                    }
                    output += "</button>";
                }
                output += "<button type=\"button\" class=\"acc\" data-add-account>+ Añadir cuenta</button><p class=\"foot\">Sigue la misma conversación. Si está respondiendo, se interrumpe y cambia al instante.</p>";
                box_.set_inner_html(&output);
                let state = rc.clone();
                let parent_box = box_.clone();
                let pane_ = pane.clone();
                listen(&rc, box_.as_ref(), "click", false, false, move |event| {
                    let Some(b) = target(&event).and_then(|t| t.closest(".acc").ok().flatten())
                    else {
                        return;
                    };
                    if get(b.as_ref(), "disabled").is_truthy() {
                        return;
                    }
                    if b.has_attribute("data-add-account") {
                        add_account(&state, &parent_box, &harness, "");
                        return;
                    }
                    let alias = dataset(&b, "alias");
                    let Some(a) = accounts.iter().find(|a| text(a, "alias") == alias) else {
                        return;
                    };
                    if comandos_web_dom::api::js_truthy(&a["selectable"]) {
                        switch_account(&state, &parent_box, &pane_, &alias)
                    } else {
                        add_account(&state, &parent_box, &harness, &alias)
                    }
                });
            }
            Err(error) => {
                if rc.borrow().acct.as_deref() == Some(&pane) {
                    box_.set_inner_html(&format!(
                        "<h4>CUENTA DE ESTE PANE</h4><p class=\"msg err\">{}</p>",
                        escape(&error)
                    ));
                }
            }
        }
    });
}
fn add_account(rc: &Shared, box_: &Element, harness: &str, alias: &str) {
    buttons(box_, "button", true);
    let Some(note) = query(box_, ".foot, .msg") else {
        return;
    };
    note.set_text_content(Some(&format!("Abriendo el login de {harness}…")));
    let (rc, box_, harness, alias) = (
        rc.clone(),
        box_.clone(),
        harness.to_string(),
        alias.to_string(),
    );
    spawn_local(async move {
        let result = fetch(
            token(&rc),
            "/account/add",
            Some(json!({"provider":harness,"alias":alias,"deviceAuth":true})),
            Some("No se pudo abrir el login"),
            None,
        )
        .await;
        match result {
            Ok(data) => {
                let link = doc().create_element("a").unwrap();
                if let Ok(url) = web_sys::Url::new(&w().location().href().unwrap_or_default()) {
                    url.search_params().set("arg", &text(&data, "session"));
                    let _ = link.set_attribute("href", &url.href());
                }
                let _ = link.set_attribute("target", "_blank");
                let _ = link.set_attribute("rel", "noopener");
                link.set_text_content(Some(&format!("Abrir login · {}", text(&data, "alias"))));
                note.set_text_content(Some("Completa el login y vuelve a Cuenta para elegirla. "));
                let _ = note.append_child(&link);
            }
            Err(error) => note.set_text_content(Some(&error)),
        }
        buttons(&box_, "button", false);
    });
}
async fn pause(ms: i32) {
    let promise = Promise::new(&mut |resolve, _| {
        timers::timeout(ms, move || {
            let _ = resolve.call0(&JsValue::NULL);
        })
        .forget();
    });
    let _ = JsFuture::from(promise).await;
}
async fn wait_switch(
    rc: &Shared,
    key: &str,
    id: &str,
    alias: &str,
    box_: &Element,
    pane: &str,
) -> Value {
    let params = web_sys::UrlSearchParams::new().unwrap();
    params.set("operationKey", key);
    params.set("operationId", id);
    let path = format!("/model/status?{}", params.to_string());
    let mut is_awaiting = false;
    for _ in 0..120 {
        pause(1500).await;
        if rc.borrow().disposed {
            return json!({"ok":false,"detail":"Terminal cerrada"});
        }
        if let Ok(s) = fetch(token(&rc), &path, None, None, None).await {
            if terminal_state(&s) {
                return s;
            }
            is_awaiting = text(&s, "state") == "awaiting_confirmation";
            let progress = stage(&s, alias);
            if !progress.is_empty() {
                say(rc, box_, pane, &progress, false);
            }
        }
    }
    if is_awaiting {
        json!({"ok":false,"awaiting":true,"detail":awaiting(alias)})
    } else {
        json!({"ok":false,"detail":"Sin confirmación en 3 minutos; revisa la terminal del pane."})
    }
}
fn switch_account(rc: &Shared, box_: &Element, pane: &str, alias: &str) {
    buttons(box_, ".acc", true);
    say(rc, box_, pane, &format!("Pasando a {alias}…"), false);
    let (rc, box_, pane, alias) = (
        rc.clone(),
        box_.clone(),
        pane.to_string(),
        alias.to_string(),
    );
    spawn_local(async move {
        let session = rc.borrow().session.clone();
        let result = fetch(
            token(&rc),
            "/account/switch",
            Some(json!({"session":session,"pane":pane,"alias":alias})),
            Some("No se pudo cambiar de cuenta"),
            None,
        )
        .await;
        let result = match result {
            Ok(data) => {
                say(
                    &rc,
                    &box_,
                    &pane,
                    &format!(
                        "Interrumpiendo lo que está haciendo y pasando a {alias} con la misma conversación…"
                    ),
                    false,
                );
                let done = wait_switch(
                    &rc,
                    &text(&data, "operationKey"),
                    &text(&data, "operationId"),
                    &alias,
                    &box_,
                    &pane,
                )
                .await;
                refresh(&rc);
                if comandos_web_dom::api::js_truthy(&done["awaiting"]) {
                    say(&rc, &box_, &pane, &text(&done, "detail"), false);
                    return;
                }
                if !comandos_web_dom::api::js_truthy(&done["ok"]) {
                    let detail = text(&done, "detail");
                    let error = text(&done, "error");
                    Err(if !detail.is_empty() {
                        detail
                    } else if !error.is_empty() {
                        error
                    } else {
                        "El cambio no se confirmó".into()
                    })
                } else {
                    Ok(())
                }
            }
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => {
                say(
                    &rc,
                    &box_,
                    &pane,
                    &format!("Listo: este pane ya sigue la misma conversación en {alias}."),
                    false,
                );
                let state = rc.clone();
                timers::timeout(2200, move || {
                    if state.borrow().acct.as_deref() == Some(&pane) {
                        close_accounts(&state)
                    }
                })
                .forget();
            }
            Err(e) => {
                say(&rc, &box_, &pane, &e, true);
                buttons(&box_, ".acc[data-alias]", false);
            }
        }
    });
}
fn target(e: &Event) -> Option<Element> {
    e.target()?.dyn_into().ok()
}
fn listen(
    rc: &Shared,
    target: &web_sys::EventTarget,
    event: &str,
    capture: bool,
    passive: bool,
    f: impl FnMut(Event) + 'static,
) {
    rc.borrow_mut().listeners.push(events::on_with(
        target,
        event,
        events::Options {
            capture,
            passive,
            once: false,
        },
        f,
    ));
}
fn mouse(e: &Event) -> Option<&MouseEvent> {
    e.dyn_ref()
}
fn layer_point(s: &State, e: &MouseEvent) -> (f64, f64) {
    (
        f64::from(e.client_x()) - rect(&s.layer, "left"),
        f64::from(e.client_y()) - rect(&s.layer, "top"),
    )
}
fn point_cell(s: &State, e: &MouseEvent) -> Option<(f64, f64)> {
    let sc = screen(s)?;
    let (cw, ch) = cell(s);
    if cw <= 0. || ch <= 0. {
        return None;
    }
    Some(((f64::from(e.client_x()) - rect(&sc, "left")) / cw).floor()).zip(Some(
        ((f64::from(e.client_y()) - rect(&sc, "top")) / ch).floor(),
    ))
}
fn gutter_at(s: &State, x: f64, y: f64) -> Option<usize> {
    s.gutters.iter().position(|g| hit(g, x, y, 8.))
}
#[wasm_bindgen::prelude::wasm_bindgen(js_name = release_pane_chrome)]
pub fn dispose() {
    CURRENT.with(|slot| {
        if let Some(rc) = slot.borrow_mut().take() {
            let mut s = rc.borrow_mut();
            s.disposed = true;
            for subscription in s.subscriptions.drain(..) {
                let _ = call(&subscription, "dispose", &[]);
            }
            s.callbacks.clear();
            s.acct = None;
            s.listeners.clear();
            s.timers.clear();
            s.soon = None;
            if let Some(observer) = s.observer.take() {
                observer.disconnect()
            }
            s.observer_cb = None;
            s.layer.set_inner_html("");
        }
    });
}
#[wasm_bindgen::prelude::wasm_bindgen(js_name = attach_pane_chrome)]
pub fn attach() -> Result<(), JsValue> {
    dispose();
    let params = web_sys::UrlSearchParams::new_with_str(&w().location().search()?)?;
    let session = params.get("arg").unwrap_or_default();
    let token = params.get("auth").unwrap_or_default();
    let Some(layer) = doc().get_element_by_id("pane-chrome") else {
        return Ok(());
    };
    if session.is_empty() || token.is_empty() {
        return Ok(());
    }
    let term = get(w().as_ref(), "__comandosTerm");
    if term.is_null() || term.is_undefined() {
        return Err("pane chrome requires live WebTerm adapter".into());
    }
    let frames = doc().create_element("div")?;
    let grip = doc().create_element("div")?;
    let menu = doc().create_element("div")?;
    layer.set_inner_html("");
    layer.append_child(&frames)?;
    layer.append_child(&grip)?;
    layer.append_child(&menu)?;
    let rc = Rc::new(RefCell::new(State {
        session,
        token,
        term,
        layer,
        frames,
        grip,
        menu,
        panes: vec![],
        models: vec![],
        busy: false,
        gutters: vec![],
        hot: None,
        drag: None,
        local: None,
        local_at: 0.,
        remote: None,
        prefix: 0.,
        want: None,
        sent: None,
        inflight: false,
        acct: None,
        disposed: false,
        listeners: vec![],
        timers: vec![],
        soon: None,
        callbacks: vec![],
        subscriptions: vec![],
        observer: None,
        observer_cb: None,
    }));
    let term_el = doc().get_element_by_id("term").ok_or("missing #term")?;
    let state = rc.clone();
    listen(
        &rc,
        term_el.as_ref(),
        "pointerdown",
        true,
        false,
        move |event| {
            let Some(e) = mouse(&event) else { return };
            if e.button() != 0 {
                return;
            }
            let mut s = state.borrow_mut();
            let (x, y) = layer_point(&s, e);
            let i = s
                .hot
                .filter(|i| s.gutters.get(*i).is_some_and(|g| hit(g, x, y, 14.)))
                .or_else(|| gutter_at(&s, x, y));
            let Some(i) = i else {
                let pane = point_cell(&s, e).and_then(|(x, y)| {
                    s.panes
                        .iter()
                        .find(|p| {
                            x >= number(p, "left")
                                && x < number(p, "left") + number(p, "width")
                                && y >= number(p, "top") - 1.
                                && y < number(p, "top") + number(p, "height")
                        })
                        .map(|p| text(p, "id"))
                });
                if let Some(id) = pane
                    && Some(&id) != active(&s.panes, &s.local, &s.remote).as_ref()
                {
                    s.local = Some(id.clone());
                    s.local_at = now();
                    let chosen = s.panes.iter().find(|p| text(p, "id") == id).cloned();
                    if let Some(chosen) = chosen {
                        let selected_at = s.local_at;
                        let owner = state.clone();
                        spawn_local(async move {
                            select_touched_pane(owner, chosen, selected_at).await;
                        });
                    }
                }
                drop(s);
                draw(&state);
                delayed_refresh(&state, 150);
                return;
            };
            e.prevent_default();
            e.stop_propagation();
            let g = s.gutters[i].clone();
            let nb = neighbor(&s.panes, &g).cloned();
            if e.detail() >= 2 {
                if let Some(p) = s.panes.iter().find(|p| text(p, "id") == g.pane)
                    && let Some(nb) = nb
                {
                    let key = if g.vertical { "width" } else { "height" };
                    s.want = Some(ResizeRequest::new(
                        g.pane,
                        g.vertical,
                        ((number(p, key) + number(&nb, key)) / 2.).floor().max(2.),
                    ));
                }
                drop(s);
                resize(&state);
                return;
            }
            s.drag = Some((g, nb));
            s.hot = Some(i);
            drop(s);
            draw_grip(&state);
        },
    );
    let state = rc.clone();
    listen(&rc, w().as_ref(), "mousemove", true, false, move |event| {
        let Some(e) = mouse(&event) else { return };
        let mut s = state.borrow_mut();
        let Some((g, nb)) = s.drag.clone() else {
            return;
        };
        e.prevent_default();
        e.stop_propagation();
        let Some((x, y)) = point_cell(&s, e) else {
            return;
        };
        let Some(p) = s.panes.iter().find(|p| text(p, "id") == g.pane) else {
            return;
        };
        let start = number(p, if g.vertical { "left" } else { "top" });
        let key = if g.vertical { "width" } else { "height" };
        let room = nb
            .as_ref()
            .map(|nb| number(p, key) + number(nb, key))
            .unwrap_or(f64::INFINITY);
        let size = (if g.vertical { x } else { y } - start)
            .min(room - 2.)
            .max(2.);
        s.want = Some(ResizeRequest::new(g.pane, g.vertical, size));
        drop(s);
        resize(&state);
    });
    let state = rc.clone();
    listen(&rc, w().as_ref(), "mouseup", true, false, move |event| {
        let mut s = state.borrow_mut();
        if s.drag.is_none() {
            return;
        }
        event.prevent_default();
        event.stop_propagation();
        s.drag = None;
        drop(s);
        draw_grip(&state);
        later(&state);
    });
    let state = rc.clone();
    listen(
        &rc,
        term_el.as_ref(),
        "dblclick",
        true,
        false,
        move |event| {
            if let Some(e) = mouse(&event) {
                let s = state.borrow();
                let (x, y) = layer_point(&s, e);
                if gutter_at(&s, x, y).is_some() {
                    event.prevent_default();
                    event.stop_propagation();
                }
            }
        },
    );
    let state = rc.clone();
    listen(
        &rc,
        term_el.as_ref(),
        "mousemove",
        false,
        true,
        move |event| {
            let Some(e) = mouse(&event) else { return };
            let mut s = state.borrow_mut();
            if s.drag.is_some() {
                return;
            }
            let (x, y) = layer_point(&s, e);
            if s.hot
                .is_some_and(|i| s.gutters.get(i).is_some_and(|g| hit(g, x, y, 14.)))
            {
                return;
            }
            let next = gutter_at(&s, x, y);
            if next != s.hot {
                s.hot = next;
                drop(s);
                draw_grip(&state);
            }
        },
    );
    let state = rc.clone();
    listen(
        &rc,
        term_el.as_ref(),
        "mouseleave",
        false,
        false,
        move |_| {
            let mut s = state.borrow_mut();
            if s.drag.is_none() && s.hot.is_some() {
                s.hot = None;
                drop(s);
                draw_grip(&state);
            }
        },
    );
    let state = rc.clone();
    let data = Closure::<dyn FnMut(JsValue)>::new(move |d: JsValue| {
        let mut s = state.borrow_mut();
        let now = now();
        if d.as_string().as_deref() == Some("\x02") {
            s.prefix = now + 1500.;
            return;
        }
        if now < s.prefix {
            s.prefix = now + 600.;
            s.local = None;
            drop(s);
            delayed_refresh(&state, 80);
        }
    });
    let subscription = call(&rc.borrow().term, "onData", &[data.as_ref().clone()])?;
    rc.borrow_mut().subscriptions.push(subscription);
    rc.borrow_mut().callbacks.push(data);
    let state = rc.clone();
    let resize_cb = Closure::<dyn FnMut(JsValue)>::new(move |_| later(&state));
    let subscription = call(&rc.borrow().term, "onResize", &[resize_cb.as_ref().clone()])?;
    rc.borrow_mut().subscriptions.push(subscription);
    rc.borrow_mut().callbacks.push(resize_cb);
    let state = rc.clone();
    let layer = rc.borrow().layer.clone();
    listen(&rc, layer.as_ref(), "click", false, false, move |event| {
        let Some(b) = target(&event).and_then(|t| t.closest("button[data-act]").ok().flatten())
        else {
            return;
        };
        if dataset(&b, "act") == "acct" {
            open_accounts(&state, &b)
        } else if dataset(&b, "act") == "ext"
            && let Ok(Some(p)) = w().parent()
        {
            let _ = call(
                p.as_ref(),
                "openPaneExtensions",
                &[
                    state.borrow().session.clone().into(),
                    dataset(&b, "pane").into(),
                    dataset(&b, "harness").into(),
                ],
            );
        }
    });
    let state = rc.clone();
    listen(
        &rc,
        doc().as_ref(),
        "mousedown",
        true,
        false,
        move |event| {
            let s = state.borrow();
            if s.acct.is_none() {
                return;
            }
            let Some(t) = target(&event) else { return };
            let in_menu = s.menu.contains(Some(&t));
            let account = t
                .closest("button[data-act=\"acct\"]")
                .ok()
                .flatten()
                .is_some();
            drop(s);
            if !in_menu && !account {
                close_accounts(&state)
            }
        },
    );
    let state = rc.clone();
    listen(&rc, doc().as_ref(), "keydown", false, false, move |event| {
        if get(event.as_ref(), "key").as_string().as_deref() == Some("Escape")
            && state.borrow().acct.is_some()
        {
            close_accounts(&state)
        }
    });
    let state = rc.clone();
    listen(
        &rc,
        doc().as_ref(),
        "visibilitychange",
        false,
        false,
        move |_| {
            if !doc().hidden() {
                refresh(&state)
            }
        },
    );
    if get(w().as_ref(), "ResizeObserver").is_function() {
        let state = rc.clone();
        let cb = Closure::<dyn FnMut(Array, ResizeObserver)>::new(move |_, _| later(&state));
        let observer = ResizeObserver::new(cb.as_ref().unchecked_ref())?;
        observer.observe(&term_el);
        rc.borrow_mut().observer = Some(observer);
        rc.borrow_mut().observer_cb = Some(cb);
    }
    let state = rc.clone();
    rc.borrow_mut()
        .timers
        .push(timers::interval(2000, move || refresh(&state)));
    CURRENT.with(|s| *s.borrow_mut() = Some(rc.clone()));
    refresh(&rc);
    Ok(())
}
