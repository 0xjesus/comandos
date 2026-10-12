use super::super::web_support::*;
use super::super::web_support::{utf16_attr as attr, utf16_toast as toast};
use super::*;
use comandos_web_dom::port::utf16_string as string;
use comandos_web_dom::port::{
    from_utf16_json as from_json, to_utf16_json as to_json, utf16_get as get, utf16_set as set,
};
use comandos_web_dom::{bridge::global_set, port::*};
use comandos_web_view::work_marks as view;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::JsValue;
struct Marks {
    marks: JsValue,
    panes: JsValue,
    activity: JsValue,
    loading: JsValue,
    menu: Option<Menu>,
}
struct Menu {
    el: JsValue,
    button: JsValue,
    outside: JsValue,
}
thread_local! {static STATE:Rc<RefCell<Marks>>=Rc::new(RefCell::new(Marks {marks:js_sys::Map::new().into(),panes:js_sys::Array::new().into(),activity:object(),loading:JsValue::NULL,menu:None}));}
fn with_state() -> Rc<RefCell<Marks>> {
    STATE.with(Clone::clone)
}
fn key(scope: &str, name: &str) -> String {
    format!("{scope}\0{name}")
}
fn index(list: JsValue) -> Result<JsValue, JsValue> {
    let map = js_sys::Map::new();
    for row in js_sys::Array::from(&list).iter() {
        map.set(
            &utf16_value(&key(
                &string(&get(&row, "scope")),
                &string(&get(&row, "key")),
            )),
            &row,
        );
    }
    Ok(map.into())
}
fn row(scope: &str, name: &str) -> JsValue {
    let st = with_state();
    let Ok(s) = st.try_borrow() else {
        return JsValue::NULL;
    };
    let r = call(&s.marks, "get", &[utf16_value(&key(scope, name))]).unwrap_or(JsValue::NULL);
    if truthy(&r) {
        r
    } else {
        from_json(&json!({"scope":scope,"key":name,"mark":"none","favorite":false,"revision":0}))
            .unwrap_or(JsValue::NULL)
    }
}
fn favorite(name: &str) -> bool {
    truthy(&call(&get(&state(), "favs"), "has", &[utf16_value(name)]).unwrap_or(JsValue::FALSE))
}
fn adopt(body: JsValue) -> Result<JsValue, JsValue> {
    let marks = get(&body, "marks");
    if !js_sys::Array::is_array(&marks) {
        return Ok(JsValue::UNDEFINED);
    }
    let st = with_state();
    {
        let mut s = st
            .try_borrow_mut()
            .map_err(|_| js_sys::Error::new("marks state busy"))?;
        s.marks = index(marks)?;
        let panes = get(&body, "panes");
        s.panes = if js_sys::Array::is_array(&panes) {
            panes
        } else {
            js_sys::Array::new().into()
        };
        let a = get(&body, "activity");
        s.activity = if a.is_object() { a } else { object() };
    }
    decorate()?;
    Ok(JsValue::UNDEFINED)
}
fn load() -> Result<JsValue, JsValue> {
    let st = with_state();
    let active = st
        .try_borrow()
        .map_err(|_| js_sys::Error::new("marks state busy"))?
        .loading
        .clone();
    if truthy(&active) {
        return Ok(active);
    }
    let memory = st.clone();
    let pending = promise(async move {
        if let Ok(r) = request("GET", "/work-marks", JsValue::NULL).await
            && number(&get(&r, "status")) == 200.
        {
            adopt(get(&r, "body"))?;
        }
        if let Ok(mut s) = memory.try_borrow_mut() {
            s.loading = JsValue::NULL;
        }
        Ok(JsValue::UNDEFINED)
    });
    st.try_borrow_mut()
        .map_err(|_| js_sys::Error::new("marks state busy"))?
        .loading = pending.clone();
    Ok(pending)
}
fn set_mark(scope: String, name: String, value: JsValue) -> JsValue {
    promise(async move {
        let current = row(&scope, &name);
        let body = object();
        set(&body, "scope", &utf16_value(&scope))?;
        set(&body, "key", &utf16_value(&name))?;
        set(&body, "value", &value)?;
        set(&body, "expectedRevision", &get(&current, "revision"))?;
        let r = request("POST", "/work-marks", body).await?;
        let body = get(&r, "body");
        let status = number(&get(&r, "status"));
        let changed = if status == 200. {
            get(&body, "mark")
        } else if status == 409. {
            get(&body, "current")
        } else {
            JsValue::NULL
        };
        if truthy(&changed) {
            let st = with_state();
            {
                let s = st
                    .try_borrow()
                    .map_err(|_| js_sys::Error::new("marks state busy"))?;
                call(
                    &s.marks,
                    "set",
                    &[utf16_value(&key(&scope, &name)), changed.clone()],
                )?;
            }
            decorate()?;
            if status == 200. {
                return Ok(changed);
            }
            return Err(js_sys::Error::new(&t(
                "Otro dispositivo cambió esta marca; se muestra la actual.",
                "Another device changed this mark; showing the current one.",
            ))
            .into());
        }
        let message = get(&body, "error");
        let message = if message.is_string() {
            message
        } else {
            "No se pudo guardar la marca".into()
        };
        Err(invoke(&global("Error"), &[message])?)
    })
}
fn paint(button: JsValue, target: JsValue) -> Result<JsValue, JsValue> {
    let scope = string(&get(&target, "scope"));
    let r = row(&scope, &string(&get(&target, "key")));
    let st = with_state();
    let activity = st
        .try_borrow()
        .map_err(|_| js_sys::Error::new("marks state busy"))?
        .activity
        .clone();
    let activity = activity_for(&to_json(&target), &to_json(&activity));
    let mark = string(&get(&r, "mark"));
    let c = channels(&mark, activity.as_str().unwrap_or_default());
    let ai = c.get("ai").and_then(Value::as_str).unwrap_or("idle");
    // Hecho / Aparcado / Esperando and «¿Hecho? ✓» are retired (11-oct): the tab only
    // shows the AI state and the star.
    let sticker: Option<&str> = None;
    let suggest = false;
    let fav = scope == "pane" && truthy(&get(&r, "favorite"));
    let signature = format!("{ai}|{}|{suggest}|{fav}", sticker.unwrap_or_default());
    set(&button, "_wmTarget", &target)?;
    if get(&button, "_wmSignature").as_string().as_deref() == Some(&signature) {
        return Ok(JsValue::UNDEFINED);
    }
    set(&button, "_wmSignature", &signature.into())?;
    set(
        &button,
        "innerHTML",
        &format!(
            "{}{}",
            view::ai_icon(ai, 12., english()),
            if fav {
                view::icon("favorite", 12.)
            } else {
                String::new()
            }
        )
        .into(),
    )?;
    classes(&button, "wm-animated", ai != "idle" || fav);
    set(&get(&button, "dataset"), "wmState", &ai.into())?;
    let d = view::data();
    let ai_label = d
        .get("AI_LABELS")
        .and_then(|v| v.get(ai))
        .and_then(|v| v.get(usize::from(english())))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let title = format!(
        "IA: {ai_label}{}{}",
        sticker
            .map(|s| format!(
                " · {}: {s}",
                if scope == "pane" {
                    t("Marca del pane", "Pane mark")
                } else {
                    t("Marca de la pestaña", "Tab mark")
                }
            ))
            .unwrap_or_default(),
        if fav {
            format!(" · {}", view::label("favorite", english()))
        } else {
            String::new()
        }
    );
    attr(&button, "aria-label", &title);
    set(&button, "title", &utf16_value(&title))?;
    let host = get(&button, "parentNode");
    if !truthy(&host) {
        return Ok(JsValue::UNDEFINED);
    }
    let mut extra = query(&host, ":scope > .wm-extra");
    if !truthy(&extra) {
        extra = call(&doc(), "createElement", &["span".into()])?;
        set(&extra, "className", &"wm-extra".into())?;
        let name = get(&button, "_wmBefore");
        let before = if get(&name, "parentNode") == host {
            get(&name, "nextSibling")
        } else {
            JsValue::NULL
        };
        call(&host, "insertBefore", &[extra.clone(), before])?;
        let b = button.clone();
        listen(
            &extra,
            "click",
            function(move |a| {
                let e = a.get(0);
                if truthy(&query_closest(&get(&e, "target"), "[data-wm-suggest]")) {
                    stop(&e);
                    let t = get(&b, "_wmTarget");
                    let _ = set_mark(
                        string(&get(&t, "scope")),
                        string(&get(&t, "key")),
                        "resolved".into(),
                    );
                }
                Ok(JsValue::UNDEFINED)
            }),
        );
    }
    let html = format!(
        "{}{}",
        sticker
            .map(|s| format!("<span class=\"wm-sticker wm-st-{mark}\">{s}</span>"))
            .unwrap_or_default(),
        if suggest {
            "<button type=\"button\" class=\"wm-suggest\" data-wm-suggest title=\"La IA terminó y no preguntó nada\">¿Hecho? ✓</button>"
        } else {
            ""
        }
    );
    set(&extra, "innerHTML", &utf16_value(&html))?;
    Ok(JsValue::UNDEFINED)
}
fn query_closest(el: &JsValue, sel: &str) -> JsValue {
    call(el, "closest", &[sel.into()]).unwrap_or(JsValue::NULL)
}
fn close_menu(focus: bool) -> Result<JsValue, JsValue> {
    let st = with_state();
    let menu = st
        .try_borrow_mut()
        .map_err(|_| js_sys::Error::new("marks state busy"))?
        .menu
        .take();
    if let Some(m) = menu {
        call(&m.el, "remove", &[])?;
        attr(&m.button, "aria-expanded", "false");
        call(
            &doc(),
            "removeEventListener",
            &["pointerdown".into(), m.outside, true.into()],
        )?;
        if focus && truthy(&get(&m.button, "isConnected")) {
            let o = object();
            set(&o, "preventScroll", &true.into())?;
            let _ = call(&m.button, "focus", &[o]);
        }
    }
    Ok(JsValue::UNDEFINED)
}
fn choose(items: JsValue, target: JsValue, index: u32) -> JsValue {
    promise(async move {
        let item = js_sys::Array::from(&items).get(index);
        close_menu(true)?;
        let scope = string(&get(&target, "scope"));
        let name = string(&get(&target, "key"));
        let is_fav = get(&item, "kind").as_string().as_deref() == Some("favorite");
        let result = if is_fav && scope == "session" && global("setSessionFavorite").is_function() {
            wait(invoke(
                &global("setSessionFavorite"),
                &[utf16_value(&name), get(&item, "value")],
            ))
            .await
        } else {
            let value = if is_fav {
                let o = object();
                set(&o, "favorite", &get(&item, "value"))?;
                o
            } else {
                get(&item, "value")
            };
            wait(Ok(set_mark(scope, name, value))).await
        };
        if let Err(e) = result {
            toast(&string(&get(&e, "message")));
        }
        decorate()?;
        Ok(JsValue::UNDEFINED)
    })
}
fn open_menu(button: JsValue) -> Result<JsValue, JsValue> {
    let target = get(&button, "_wmTarget");
    if !truthy(&target) {
        return Ok(JsValue::UNDEFINED);
    }
    let same = with_state()
        .try_borrow()
        .map_err(|_| js_sys::Error::new("marks state busy"))?
        .menu
        .as_ref()
        .is_some_and(|m| m.button == button);
    if same {
        return close_menu(true);
    }
    close_menu(false)?;
    let scope = string(&get(&target, "scope"));
    let name = string(&get(&target, "key"));
    let items = view::menu_items(&scope, &to_json(&row(&scope, &name)), favorite(&name), english());
    let items = from_json(&Value::Array(
        items.as_array().into_iter().flatten().filter(|i| i["kind"] != "mark").cloned().collect(),
    ))?;
    let el = call(&doc(), "createElement", &["div".into()])?;
    set(&el, "className", &"wm-menu".into())?;
    attr(&el, "role", "menu");
    attr(
        &el,
        "aria-label",
        &if scope == "pane" {
            t("Marca del pane", "Pane mark")
        } else {
            t("Marca de la pestaña", "Tab mark")
        },
    );
    set(
        &el,
        "innerHTML",
        &utf16_value(&view::menu(&to_json(&items))),
    )?;
    call(
        &get(&doc(), "body"),
        "appendChild",
        std::slice::from_ref(&el),
    )?;
    let r = call(&button, "getBoundingClientRect", &[])?;
    let w = number(&get(&el, "offsetWidth"));
    let h = number(&get(&el, "offsetHeight"));
    let left = number(&get(&r, "left"))
        .min(number(&global("innerWidth")) - w - 8.)
        .max(8.);
    let bottom = number(&get(&r, "bottom"));
    let top = if bottom + h + 8. > number(&global("innerHeight")) {
        (number(&get(&r, "top")) - h - 4.).max(8.)
    } else {
        bottom + 4.
    };
    style(&el, "left", &format!("{left}px"));
    style(&el, "top", &format!("{top}px"));
    let buttons = all(&el, ".wm-item");
    for (i, b) in buttons.iter().enumerate() {
        let it = items.clone();
        let t = target.clone();
        listen(
            b,
            "click",
            function(move |a| {
                let _ = call(&a.get(0), "stopPropagation", &[]);
                Ok(choose(it.clone(), t.clone(), i as u32))
            }),
        );
    }
    let bs = buttons.clone();
    let it = items.clone();
    let t = target.clone();
    listen(
        &el,
        "keydown",
        function(move |a| {
            let e = a.get(0);
            let k = string(&get(&e, "key"));
            let i = bs
                .iter()
                .position(|b| b == &get(&doc(), "activeElement"))
                .map(|i| i as i32)
                .unwrap_or(-1);
            if k == "Escape" {
                stop(&e);
                return close_menu(true);
            }
            if k == "Tab" {
                return close_menu(false);
            }
            if k == "Enter" || k == " " {
                let _ = call(&e, "preventDefault", &[]);
                if i >= 0 {
                    return Ok(choose(it.clone(), t.clone(), i as u32));
                }
                return Ok(JsValue::UNDEFINED);
            }
            let n = next_index(&k, i, bs.len() as i32);
            if n != i {
                let _ = call(&e, "preventDefault", &[]);
                if let Some(b) = bs.get(n as usize) {
                    let _ = call(b, "focus", &[]);
                }
            }
            Ok(JsValue::UNDEFINED)
        }),
    );
    let outside_el = el.clone();
    let outside_button = button.clone();
    let outside = function(move |a| {
        let t = get(&a.get(0), "target");
        if !truthy(&call(&outside_el, "contains", std::slice::from_ref(&t))?) && t != outside_button
        {
            return close_menu(false);
        }
        Ok(JsValue::UNDEFINED)
    });
    call(
        &doc(),
        "addEventListener",
        &["pointerdown".into(), outside.clone(), true.into()],
    )?;
    with_state()
        .try_borrow_mut()
        .map_err(|_| js_sys::Error::new("marks state busy"))?
        .menu = Some(Menu {
        el,
        button: button.clone(),
        outside,
    });
    attr(&button, "aria-expanded", "true");
    let selected = js_sys::Array::from(&items)
        .iter()
        .position(|x| truthy(&get(&x, "checked")))
        .unwrap_or(0);
    if let Some(b) = buttons.get(selected) {
        let opts = object();
        set(&opts, "preventScroll", &true.into())?;
        call(b, "focus", &[opts])?;
    }
    Ok(JsValue::UNDEFINED)
}
fn indicator(host: JsValue, target: JsValue, before: JsValue) -> Result<(), JsValue> {
    let mut button = query(&host, ":scope > .wm-ind");
    if !truthy(&button) {
        button = call(&doc(), "createElement", &["button".into()])?;
        set(&button, "type", &"button".into())?;
        set(&button, "className", &"wm-ind".into())?;
        attr(&button, "aria-haspopup", "menu");
        attr(&button, "aria-expanded", "false");
        let b = button.clone();
        listen(
            &button,
            "click",
            function(move |a| {
                stop(&a.get(0));
                open_menu(b.clone())
            }),
        );
        let b = button.clone();
        listen(
            &button,
            "keydown",
            function(move |a| {
                let e = a.get(0);
                if ["ArrowDown", "Enter", " "].contains(&string(&get(&e, "key")).as_str()) {
                    stop(&e);
                    return open_menu(b.clone());
                }
                Ok(JsValue::UNDEFINED)
            }),
        );
        call(&host, "insertBefore", &[button.clone(), before.clone()])?;
        set(&button, "_wmBefore", &before)?;
        if global("IntersectionObserver").is_function() {
            let f = function(|a| {
                for entry in js_sys::Array::from(&a.get(0)).iter() {
                    classes(
                        &get(&entry, "target"),
                        "wm-offscreen",
                        !truthy(&get(&entry, "isIntersecting")),
                    );
                }
                Ok(JsValue::UNDEFINED)
            });
            let observer = js_sys::Reflect::construct(
                &js_sys::Function::from(global("IntersectionObserver")),
                &js_sys::Array::of1(&f),
            )?;
            call(&observer, "observe", &[button.clone()])?;
        }
    }
    paint(button, target)?;
    Ok(())
}
fn decorate() -> Result<JsValue, JsValue> {
    // Status lights and favorites are retired (11-oct): tabs and rows carry no indicator.
    if true {
        return Ok(JsValue::UNDEFINED);
    }
    for tab in all(&doc(), "#tabbar .apptab") {
        let session = get(&tab, "_session");
        let k = string(&get(&get(&tab, "dataset"), "tabKey"));
        if !truthy(&session)
            || session == "local"
            || !(k.starts_with("term:") || k.starts_with("group:"))
        {
            continue;
        }
        let t = object();
        set(&t, "scope", &"session".into())?;
        set(&t, "key", &session)?;
        set(&t, "session", &session)?;
        indicator(tab.clone(), t, query(&tab, ".lbl"))?;
        set(&get(&tab, "dataset"), "wmInd", &"1".into())?;
        if !truthy(&get(&tab, "_wmContext")) {
            set(&tab, "_wmContext", &true.into())?;
            let t = tab.clone();
            listen(
                &tab,
                "contextmenu",
                function(move |a| {
                    let b = query(&t, ":scope > .wm-ind");
                    if truthy(&b) {
                        stop(&a.get(0));
                        return open_menu(b);
                    }
                    Ok(JsValue::UNDEFINED)
                }),
            );
        }
    }
    let panes = with_state()
        .try_borrow()
        .map_err(|_| js_sys::Error::new("marks state busy"))?
        .panes
        .clone();
    for r in all(&doc(), ".row[data-rk]") {
        let target = target_for_row(&string(&get(&get(&r, "dataset"), "rk")), &to_json(&panes));
        let host = query(&r, ".ident");
        if !target.is_null() && truthy(&host) {
            indicator(host.clone(), from_json(&target)?, query(&host, ".name"))?;
        }
    }
    Ok(JsValue::UNDEFINED)
}
pub fn mount() -> Result<(), JsValue> {
    let api = object();
    for (k, v) in [
        ("MARKS", json!(MARKS)),
        (
            "AI_STATES",
            json!(["work", "need", "done", "error", "idle"]),
        ),
    ] {
        set(&api, k, &from_json(&v)?)?;
    }
    let d = view::data();
    for k in ["ICONS", "AI_COLORS", "AI_LABELS", "STICKERS"] {
        set(&api, k, &from_json(d.get(k).unwrap_or(&Value::Null))?)?;
    }
    method(&api, "label", |a| {
        Ok(utf16_value(&view::label(
            &string(&a.get(0)),
            if a.get(1).is_undefined() {
                english()
            } else {
                number(&a.get(1)) != 0.
            },
        )))
    })?;
    method(&api, "iconSvg", |a| {
        Ok(utf16_value(&view::icon(
            &string(&a.get(0)),
            a.get(1).as_f64().unwrap_or(16.),
        )))
    })?;
    method(&api, "aiIconSvg", |a| {
        Ok(view::ai_icon(
            &string(&a.get(0)),
            if truthy(&a.get(1)) {
                number(&a.get(1))
            } else {
                12.
            },
            english(),
        )
        .into())
    })?;
    method(&api, "aiCycle", |a| {
        Ok(if ai_state(&string(&a.get(0))) == "need" {
            1.4
        } else {
            0.
        }
        .into())
    })?;
    method(&api, "channels", |a| {
        from_json(&channels(&string(&a.get(0)), &string(&a.get(1))))
    })?;
    method(&api, "display", |a| {
        from_json(&display(&string(&a.get(0)), &string(&a.get(1)), english()))
    })?;
    method(&api, "targetForRow", |a| {
        from_json(&target_for_row(
            &if truthy(&a.get(0)) {
                string(&a.get(0))
            } else {
                String::new()
            },
            &to_json(&a.get(1)),
        ))
    })?;
    method(&api, "activityFor", |a| {
        from_json(&activity_for(&to_json(&a.get(0)), &to_json(&a.get(1))))
    })?;
    method(&api, "menuItems", |a| {
        from_json(&view::menu_items(
            &string(&a.get(0)),
            &to_json(&a.get(1)),
            truthy(&a.get(2)),
            english(),
        ))
    })?;
    method(&api, "nextIndex", |a| {
        Ok(next_index(
            &string(&a.get(0)),
            number(&a.get(1)) as i32,
            number(&a.get(2)) as i32,
        )
        .into())
    })?;
    method(&api, "indexMarks", |a| index(a.get(0)))?;
    method(&api, "load", |_| load())?;
    method(&api, "setMark", |a| {
        Ok(set_mark(string(&a.get(0)), string(&a.get(1)), a.get(2)))
    })?;
    method(&api, "adopt", |a| adopt(a.get(0)))?;
    method(&api, "decorate", |_| decorate())?;
    method(&api, "paint", |a| paint(a.get(0), a.get(1)))?;
    global_set("WorkMarks", &api)
}
pub fn attach() -> Result<(), JsValue> {
    let sync = || {
        classes(
            &get(&doc(), "body"),
            "wm-paused",
            truthy(&get(&doc(), "hidden")),
        );
    };
    sync();
    listen(
        &doc(),
        "visibilitychange",
        function(move |_| {
            sync();
            if !truthy(&get(&doc(), "hidden")) {
                return load();
            }
            Ok(JsValue::UNDEFINED)
        }),
    );
    let observer = js_sys::Reflect::construct(
        &js_sys::Function::from(global("MutationObserver")),
        &js_sys::Array::of1(&function(|_| decorate())),
    )?;
    let watch = move || {
        for name in ["tabbar", "rows"] {
            let el = id(name);
            if truthy(&el) && !truthy(&get(&el, "_wmObserved")) {
                let _ = set(&el, "_wmObserved", &true.into());
                if let Ok(o) = from_json(&json!({"childList":true,"subtree":true})) {
                    let _ = call(&observer, "observe", &[el, o]);
                }
            }
        }
    };
    watch();
    load()?;
    call(
        &js_sys::global(),
        "setInterval",
        &[
            function(move |_| {
                watch();
                if !truthy(&get(&doc(), "hidden")) {
                    return load();
                }
                Ok(JsValue::UNDEFINED)
            }),
            5000.into(),
        ],
    )?;
    Ok(())
}
