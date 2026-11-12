use super::*;
use crate::components::web_support::*;
use comandos_web_dom::port::*;
use js_sys::{Array, Function, Reflect};
use wasm_bindgen::{JsCast, JsValue};
#[path = "app_coordinator_interaction.rs"]
mod interaction;
#[path = "app_coordinator_render.rs"]
mod render;
#[path = "app_coordinator_sidebar.rs"]
mod sidebar;
#[path = "app_coordinator_tabs.rs"]
mod tabs;
#[path = "app_coordinator_terminal.rs"]
mod terminal;
#[path = "app_coordinator_viewport.rs"]
mod viewport;

fn publish(
    name: &str,
    f: impl Fn(Array) -> Result<JsValue, JsValue> + 'static,
) -> Result<(), JsValue> {
    set(&js_sys::global(), name, &function(f))
}
fn put(name: &str, v: impl Into<JsValue>) -> Result<(), JsValue> {
    set(&js_sys::global(), name, &v.into())
}
fn run(name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    invoke(&global(name), args)
}
fn call_value(value: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let boxed = invoke(&global("Object"), std::slice::from_ref(value))?;
    call(&boxed, name, args)
}
fn new(name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    Reflect::construct(
        &global(name).dyn_into::<Function>()?,
        &args.iter().cloned().collect::<Array>(),
    )
}
fn values(v: &JsValue) -> Vec<JsValue> {
    Array::from(v).iter().collect()
}
fn entries(map: &JsValue) -> Result<Vec<(JsValue, JsValue)>, JsValue> {
    Ok(values(&Array::from(&call(map, "entries", &[])?).into())
        .into_iter()
        .map(|p| (get(&p, "0"), get(&p, "1")))
        .collect())
}
fn map_get(map: &str, key: &JsValue) -> Result<JsValue, JsValue> {
    call(&global(map), "get", std::slice::from_ref(key))
}
fn body() -> JsValue {
    get(&doc(), "body")
}
fn root() -> JsValue {
    get(&doc(), "documentElement")
}
fn has_class(el: &JsValue, k: &str) -> bool {
    call(&get(el, "classList"), "contains", &[k.into()]).is_ok_and(|v| truthy(&v))
}
fn text(v: &JsValue) -> String {
    if truthy(v) {
        utf16_string(v)
    } else {
        String::new()
    }
}
fn js(text: &str) -> JsValue {
    utf16_value(text)
}
fn state_list() -> Vec<JsValue> {
    let v = get(&state(), "list");
    if truthy(&v) { values(&v) } else { Vec::new() }
}
fn now() -> f64 {
    js_sys::Date::now()
}
fn default(v: JsValue, fallback: JsValue) -> JsValue {
    if truthy(&v) { v } else { fallback }
}
fn item(fields: &[(&str, JsValue)]) -> Result<JsValue, JsValue> {
    let o = object();
    for (k, v) in fields {
        set(&o, k, v)?;
    }
    Ok(o)
}
fn translate(es: &str, en: &str) -> JsValue {
    run("tf", &[js(es), js(en)]).unwrap_or_else(|_| js(es))
}
fn notify_error(err: &JsValue) {
    let msg = default(get(err, "message"), err.clone());
    let _ = run("toast", &[msg, true.into()]);
}
fn fire(v: Result<JsValue, JsValue>, report: bool) {
    match v {
        Ok(v) => {
            let _ = call(
                &js_sys::Promise::resolve(&v).into(),
                "catch",
                &[function(move |a| {
                    if report {
                        notify_error(&a.get(0));
                    }
                    Ok(JsValue::UNDEFINED)
                })],
            );
        }
        Err(e) => {
            if report {
                notify_error(&e);
            }
        }
    }
}
async fn api(path: &str, body: JsValue) -> Result<JsValue, JsValue> {
    let args = if body.is_undefined() {
        vec![js(path)]
    } else {
        vec![js(path), body]
    };
    wait(run("api", &args)).await
}
fn encode(v: &JsValue) -> Result<String, JsValue> {
    run("encodeURIComponent", std::slice::from_ref(v)).map(|v| utf16_string(&v))
}
fn valid_pane(v: &str) -> bool {
    v.strip_prefix('%')
        .is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
}
fn is_app() -> bool {
    run("inApp", &[]).is_ok_and(|v| truthy(&v))
}
fn dock(method: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let d = global("WorkspaceDock");
    if truthy(&d) {
        call(&d, method, args)
    } else {
        Ok(JsValue::UNDEFINED)
    }
}
fn selected_sidebar() -> Result<JsValue, JsValue> {
    if is_app() {
        return Ok(global("ACTIVE_TAB"));
    }
    let active = global("activeTerm");
    let focus = map_get("remotePaneFocus", &active)?;
    item(&[
        ("session", default(active, "".into())),
        ("pane", default(get(&focus, "pane"), "".into())),
        (
            "ts",
            number(&global("activeTermTs"))
                .max(number(&default(get(&focus, "ts"), 0.into())))
                .into(),
        ),
    ])
}
fn remember_focus(list: &JsValue) -> Result<(), JsValue> {
    let live = new("Set", &[])?;
    let map = global("remotePaneFocus");
    for it in values(&default(list.clone(), Array::new().into())) {
        if !truthy(&get(&it, "alive")) {
            continue;
        }
        let sess = get(&it, "session");
        call(&live, "add", std::slice::from_ref(&sess))?;
        let pane = get(&it, "pane");
        if truthy(&get(&it, "paneActive"))
            && truthy(&pane)
            && get(&call(&map, "get", std::slice::from_ref(&sess))?, "pane") != pane
        {
            call(
                &map,
                "set",
                &[
                    sess,
                    item(&[("pane", pane), ("ts", (now() / 1000.0).into())])?,
                ],
            )?;
        }
    }
    for (key, _) in entries(&map)? {
        if !truthy(&call(&live, "has", std::slice::from_ref(&key))?) {
            call(&map, "delete", &[key])?;
        }
    }
    Ok(())
}
pub fn mount_app() -> Result<(), JsValue> {
    for key in ["openTerms", "remotePaneFocus", "termInteraction"] {
        put(key, new("Map", &[])?)?;
    }
    for (key, value) in [
        ("activeTerm", JsValue::NULL),
        ("activeTermTs", 0.into()),
        ("activeView", "panel".into()),
        ("termInteractionSeq", 0.into()),
        ("termPageHidden", false.into()),
        ("appViewportFrame", 0.into()),
        ("appViewportSettle", 0.into()),
        ("tabsPollTs", 0.into()),
        ("wsFocusTimer", 0.into()),
        ("wsFocusSaved", JsValue::NULL),
        ("wsFocusRestored", false.into()),
        ("tabRowsHold", item(&[("n", (-1).into()), ("h", 0.into())])?),
    ] {
        put(key, value)?;
    }
    put("SPLIT_KEY", "cc-split-left")?;
    publish("appEnabled", |_| Ok(global("WEBTERM")))?;
    publish("rememberRemotePaneFocus", |a| {
        remember_focus(&a.get(0))?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("sidebarActiveTab", |_| selected_sidebar())?;

    publish("tabModelShort", |a| {
        Ok(js(&tab_model_short(&text(&a.get(0)))))
    })?;
    viewport::mount()?;
    interaction::mount()?;
    terminal::mount()?;
    tabs::mount()?;

    Ok(())
}

pub fn mount_identity() -> Result<(), JsValue> {
    publish("rowKey", |a| {
        Ok(js(&row_key(
            &utf16_string(&get(&a.get(0), "session")),
            &text(&get(&a.get(0), "pane")),
        )))
    })
}
pub fn mount_sidebar() -> Result<(), JsValue> {
    put(
        "SB_LIMITS",
        item(&[
            ("accounts", JsValue::NULL),
            ("at", 0.into()),
            ("job", JsValue::NULL),
        ])?,
    )?;
    sidebar::mount()
}
pub fn mount_render() -> Result<(), JsValue> {
    put("favColor", "")?;
    render::mount()
}
pub fn mount() -> Result<(), JsValue> {
    mount_app()?;
    mount_identity()?;
    mount_sidebar()?;
    mount_render()
}
