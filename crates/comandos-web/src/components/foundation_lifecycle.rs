//! Complete native-app, local usage, event notices and polling coordinator regions.
//! Other coordinator regions remain explicit dependencies through global exports.
pub fn only_panel(query: &str) -> String {
    query.chars().filter(char::is_ascii_lowercase).collect()
}
pub fn poll_seconds(value: f64, hidden: bool) -> f64 {
    let seconds = if value == 0.0 || value.is_nan() {
        2.0
    } else {
        value
    };
    if hidden { seconds.max(15.0) } else { seconds }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn mount_app() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach_app() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_log() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach_log() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_notices() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_poll() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach_poll() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
/// This inventory region contains only its separator; toast is in avisos-de-eventos.
pub fn retire_toasts_separator() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn panel_filter_keeps_only_original_lowercase_ascii() {
        assert_eq!(
            only_panel("appearance<script>ÉCHAINs-1"),
            "appearancescripts"
        );
        assert_eq!(only_panel("news"), "news");
    }
    #[test]
    fn poll_visibility_preserves_default_and_original_numeric_behavior() {
        assert_eq!(poll_seconds(f64::NAN, false), 2.0);
        assert_eq!(poll_seconds(0.0, false), 2.0);
        assert_eq!(poll_seconds(-2.0, false), -2.0);
        assert_eq!(poll_seconds(-2.0, true), 15.0);
        assert_eq!(poll_seconds(30.0, true), 30.0);
        assert_eq!(poll_seconds(2.0, true), 15.0);
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{
    attach_app, attach_log, attach_poll, mount_app, mount_log, mount_notices, mount_poll,
};
#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use crate::components::web_support::*;
    use comandos_web_dom::port::*;
    use js_sys::{Array, Function, Promise, Reflect};
    use serde_json::json;
    use wasm_bindgen::{JsCast, JsValue};
    fn new(name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
        Reflect::construct(
            &global(name).dyn_into::<Function>()?,
            &args.iter().cloned().collect::<Array>(),
        )
    }
    fn publish(
        name: &str,
        f: impl Fn(Array) -> Result<JsValue, JsValue> + 'static,
    ) -> Result<(), JsValue> {
        set(&js_sys::global(), name, &function(f))
    }
    fn exported(name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
        invoke(&global(name), args)
    }
    fn now() -> f64 {
        js_sys::Date::now()
    }
    fn in_app() -> bool {
        truthy(&get(&get(&global("webkit"), "messageHandlers"), "centro"))
    }
    fn native(message: JsValue) -> Result<JsValue, JsValue> {
        call(
            &get(&get(&global("webkit"), "messageHandlers"), "centro"),
            "postMessage",
            &[js_sys::JSON::stringify(&message)?.into()],
        )
    }
    fn query_arg(name: &str) -> Result<JsValue, JsValue> {
        let params = new("URLSearchParams", &[get(&global("location"), "search")])?;
        call(&params, "get", &[name.into()]).map(|v| if truthy(&v) { v } else { "".into() })
    }
    fn open_native(session: JsValue, win: JsValue, label: JsValue) -> Result<JsValue, JsValue> {
        let value = object();
        set(&value, "session", &session)?;
        set(&value, "win", &win)?;
        set(&value, "label", &label)?;
        native(value)
    }
    fn args(it: &JsValue, win: Option<JsValue>) -> Result<JsValue, JsValue> {
        if it.is_null() || it.is_undefined() {
            return Err(js_sys::TypeError::new("session is null or undefined").into());
        }
        let body = object();
        set(&body, "session", &get(it, "session"))?;
        set(&body, "cwd", &get(it, "cwd"))?;
        if let Some(win) = win {
            set(&body, "win", &win)?;
        }
        Ok(body)
    }
    fn open_session(it: JsValue, win: JsValue) -> Result<JsValue, JsValue> {
        let session = get(&it, "session");
        let project = get(&it, "project");
        if truthy(&it) && truthy(&session) && global("alertResolve").is_function() {
            exported("alertResolve", std::slice::from_ref(&session))?;
        }
        exported(
            "ulog",
            &[
                "open".into(),
                if truthy(&it) && truthy(&project) {
                    project.clone()
                } else if truthy(&it) {
                    session.clone()
                } else {
                    "".into()
                },
                if truthy(&win) { win.clone() } else { "".into() },
            ],
        )?;
        let app = in_app();
        let remote = if app {
            false
        } else {
            truthy(&exported("appEnabled", &[])?)
        };
        let path = if app || remote {
            "/ensure"
        } else if win.as_string().as_deref() == Some("shell") {
            "/shell"
        } else {
            "/focus"
        };
        let first = exported(
            "api",
            &[
                path.into(),
                args(
                    &it,
                    if app || remote {
                        Some(win.clone())
                    } else {
                        None
                    },
                )?,
            ],
        );
        Ok(promise(async move {
            let mut target = session.clone();
            if app {
                let result = wait(first).await?;
                let selected = get(&result, "session");
                if truthy(&selected) {
                    target = selected;
                }
                open_native(target, win, JsValue::UNDEFINED)?;
                return exported(
                    "tf",
                    &[
                        format!("{} en su pestana", string(&project)).into(),
                        format!("{} is in its tab", string(&project)).into(),
                    ],
                );
            }
            if remote {
                match wait(first).await {
                    Ok(r) => {
                        let selected = get(&r, "session");
                        if truthy(&selected) {
                            target = selected;
                        }
                    }
                    Err(_) => {
                        if let Ok(r) =
                            wait(exported("api", &["/up".into(), args(&it, None)?])).await
                        {
                            let selected = get(&r, "session");
                            if truthy(&selected) {
                                target = selected;
                            }
                        }
                    }
                }
                exported("openTerm", &[target, project.clone()])?;
                return exported(
                    "tf",
                    &[
                        format!("{} abierto", string(&project)).into(),
                        format!("{} opened", string(&project)).into(),
                    ],
                );
            }
            if wait(first).await.is_err() {
                wait(exported("api", &["/up".into(), args(&it, None)?])).await?;
            }
            exported(
                "tf",
                &[
                    format!("Tu terminal esta en {}", string(&project)).into(),
                    format!("Your terminal is now on {}", string(&project)).into(),
                ],
            )
        }))
    }
    pub fn mount_app() -> Result<(), JsValue> {
        publish("inApp", |_| Ok(in_app().into()))?;
        publish("openInApp", |a| {
            open_native(a.get(0), a.get(1), a.get(2))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("openSession", |a| {
            Ok(open_session(a.get(0), a.get(1)).unwrap_or_else(|e| Promise::reject(&e).into()))
        })?;
        let panels = from_json(
            &json!({"settings":"#settings","remote":"#remote","servers":"#servers","sovereignty":"#sovereignty"}),
        )?;
        let request = if query_arg("panel")?.as_string().as_deref() == Some("chains") {
            (|| -> Result<JsValue, JsValue> {
                let stored = call(
                    &global("localStorage"),
                    "getItem",
                    &["cc-center-panel".into()],
                )?;
                let parsed = js_sys::JSON::parse(&if truthy(&stored) {
                    string(&stored)
                } else {
                    "null".into()
                })?;
                call(
                    &global("localStorage"),
                    "removeItem",
                    &["cc-center-panel".into()],
                )?;
                if truthy(&parsed)
                    && now() - number(&get(&parsed, "at")) < 10000.0
                    && truthy(&get(&panels, &string(&get(&parsed, "panel"))))
                {
                    Ok(parsed)
                } else {
                    Ok(JsValue::NULL)
                }
            })()
            .unwrap_or(JsValue::NULL)
        } else {
            JsValue::NULL
        };
        let only = if truthy(&request) {
            get(&request, "panel")
        } else {
            only_panel(&string(&query_arg("panel")?)).into()
        };
        set(&js_sys::global(), "CENTER_PANELS", &panels)?;
        set(&js_sys::global(), "CENTER_REQ", &request)?;
        set(&js_sys::global(), "ONLY_PANEL", &only)?;
        let root = get(&doc(), "documentElement");
        if in_app() {
            classes(&root, "inapp", true);
            classes(&root, "gtkapp", true);
        }
        if truthy(&only) {
            classes(&root, "only-panel", true);
            set(&get(&root, "dataset"), "onlyPanel", &only)?;
        }
        Ok(())
    }
    pub fn attach_app() -> Result<(), JsValue> {
        let body = get(&doc(), "body");
        if in_app() {
            classes(&body, "inapp", true);
        }
        if truthy(&global("ONLY_PANEL")) {
            classes(&body, "only-panel", true);
            later(
                function(|_| {
                    let only = string(&global("ONLY_PANEL"));
                    if only == "appearance" {
                        classes(&query(&doc(), "#settings"), "open", true);
                        exported(
                            "activateMtab",
                            &[query(&doc(), "#settings .modal-panel"), "appearance".into()],
                        )?;
                        exported("renderThemeGallery", &[])?;
                        exported("renderButtonStyleGallery", &[])?;
                    } else if truthy(&global("CENTER_REQ")) {
                        exported("openCenterPanel", &[global("CENTER_REQ")])?;
                    }
                    Ok(JsValue::UNDEFINED)
                }),
                300.0,
            );
        }
        Ok(())
    }
    fn slice_string(value: &JsValue, limit: u32) -> JsValue {
        js_sys::JsString::from(utf16_value(&utf16_string(value)))
            .slice(0, limit)
            .into()
    }
    fn ulog(a: Array) -> Result<(), JsValue> {
        if !truthy(&a.get(1)) {
            return Ok(());
        }
        let selected = if global("activePaneTarget").is_function() {
            get(&exported("activePaneTarget", &[])?, "session")
        } else {
            JsValue::UNDEFINED
        };
        let row = object();
        set(&row, "ts", &(now() / 1000.0).into())?;
        set(&row, "k", &a.get(0))?;
        set(&row, "n", &slice_string(&a.get(1), 80))?;
        set(
            &row,
            "c",
            &slice_string(
                &if truthy(&a.get(2)) {
                    a.get(2)
                } else {
                    "".into()
                },
                80,
            ),
        )?;
        set(
            &row,
            "d",
            &call(
                &global("Math"),
                "round",
                &[if truthy(&a.get(3)) {
                    a.get(3)
                } else {
                    0.into()
                }],
            )?,
        )?;
        set(
            &row,
            "s",
            &if truthy(&selected) {
                selected
            } else {
                "".into()
            },
        )?;
        let buf = get(&global("ULOG"), "buf");
        call(&buf, "push", &[row])?;
        if number(&get(&buf, "length")) >= 40.0 {
            flush()?;
        }
        Ok(())
    }
    fn flush() -> Result<(), JsValue> {
        let buf = get(&global("ULOG"), "buf");
        if !truthy(&get(&buf, "length")) {
            return Ok(());
        }
        let events = call(&buf, "splice", &[0.into(), 200.into()])?;
        let value = object();
        set(&value, "events", &events)?;
        let body: JsValue = js_sys::JSON::stringify(&value)?.into();
        let send = (|| -> Result<(), JsValue> {
            let nav = global("navigator");
            if truthy(&get(&nav, "sendBeacon")) && !in_app() {
                let list = Array::new();
                list.push(&body);
                call(
                    &nav,
                    "sendBeacon",
                    &[
                        "/ui-log".into(),
                        new(
                            "Blob",
                            &[list.into(), from_json(&json!({"type":"application/json"}))?],
                        )?,
                    ],
                )?;
            } else {
                let opts = from_json(
                    &json!({"method":"POST","headers":{"Content-Type":"application/json"},"keepalive":true}),
                )?;
                set(&opts, "body", &body)?;
                let request = call(&js_sys::global(), "fetch", &["/ui-log".into(), opts])?;
                call(&request, "catch", &[function(|_| Ok(JsValue::UNDEFINED))])?;
            }
            Ok(())
        })();
        let _ = send;
        Ok(())
    }
    fn name_of(el: JsValue) -> Result<JsValue, JsValue> {
        let target = call(
            &el,
            "closest",
            &["[data-ulog],button,a,[role=tab],[role=switch],.row,.ssh-chip".into()],
        )?;
        if !truthy(&target) {
            return Ok(JsValue::NULL);
        }
        let data = get(&get(&target, "dataset"), "ulog");
        if truthy(&data) {
            return Ok(data);
        }
        let id = get(&target, "id");
        if truthy(&id) {
            return Ok(format!("#{}", string(&id)).into());
        }
        let list = call(&global("Array"), "from", &[get(&target, "classList")])?;
        let classes = js_sys::Array::from(&list);
        let selected = classes.iter().map(|x| string(&x)).find(|c| {
            !["on", "hidden", "open", "cfg", "test", "arm", "running"].contains(&c.as_str())
        });
        let cls = selected.unwrap_or_else(|| string(&get(&target, "tagName")).to_lowercase());
        let aria = call(&target, "getAttribute", &["aria-label".into()])?;
        let text = get(&target, "textContent");
        let raw = if truthy(&aria) {
            aria
        } else if truthy(&text) {
            text
        } else {
            "".into()
        };
        let text: JsValue = js_sys::JsString::from(raw).trim().slice(0, 24).into();
        if truthy(&text) {
            Ok(js_sys::JsString::from(cls)
                .concat(&JsValue::from(":"))
                .concat(&text)
                .into())
        } else {
            Ok(cls.into())
        }
    }
    fn screen_start(name: JsValue) -> Result<(), JsValue> {
        let screens = get(&global("ULOG"), "screens");
        if !truthy(&call(&screens, "has", std::slice::from_ref(&name))?) {
            call(&screens, "set", &[name, now().into()])?;
        }
        Ok(())
    }
    fn screen_end(name: JsValue) -> Result<(), JsValue> {
        let screens = get(&global("ULOG"), "screens");
        let names = if truthy(&name) {
            let a = Array::new();
            a.push(&name);
            a
        } else {
            Array::from(&call(
                &global("Array"),
                "from",
                &[call(&screens, "keys", &[])?],
            )?)
        };
        for name in names.iter() {
            let t0 = call(&screens, "get", std::slice::from_ref(&name))?;
            if truthy(&t0) {
                exported(
                    "ulog",
                    &[
                        "screen".into(),
                        name.clone(),
                        "".into(),
                        ((now() - number(&t0)) / 1000.0).into(),
                    ],
                )?;
                call(&screens, "delete", &[name])?;
            }
        }
        Ok(())
    }
    pub fn mount_log() -> Result<(), JsValue> {
        let log = object();
        set(&log, "buf", &Array::new().into())?;
        set(&log, "screens", &new("Map", &[])?)?;
        set(&log, "visibleSince", &now().into())?;
        set(&js_sys::global(), "ULOG", &log)?;
        publish("ulog", |a| {
            ulog(a)?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("ulogFlush", |_| {
            flush()?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("ulogNameOf", |a| name_of(a.get(0)))?;
        publish("ulogScreenStart", |a| {
            screen_start(a.get(0))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("ulogScreenEnd", |a| {
            screen_end(a.get(0))?;
            Ok(JsValue::UNDEFINED)
        })
    }
    pub fn attach_log() -> Result<(), JsValue> {
        call(
            &js_sys::global(),
            "setInterval",
            &[global("ulogFlush"), 5000.into()],
        )?;
        let header = query(&doc(), "header");
        if truthy(&header) {
            let update = function(move |_| {
                let rect = call(&header, "getBoundingClientRect", &[])?;
                let px = format!("{}px", string(&get(&rect, "bottom")));
                style(&get(&doc(), "documentElement"), "--hdr-h", &px);
                Ok(JsValue::UNDEFINED)
            });
            invoke(&update, &[])?;
            if truthy(&global("ResizeObserver")) {
                let observer = new("ResizeObserver", std::slice::from_ref(&update))?;
                call(&observer, "observe", &[query(&doc(), "header")])?;
            }
            listen(&js_sys::global(), "resize", update);
        }
        listen(
            &js_sys::global(),
            "pagehide",
            function(|_| {
                screen_end(JsValue::UNDEFINED)?;
                flush()?;
                Ok(JsValue::UNDEFINED)
            }),
        );
        call(
            &doc(),
            "addEventListener",
            &[
                "click".into(),
                function(|a| {
                    let target = get(&a.get(0), "target");
                    let name = name_of(target.clone())?;
                    if !truthy(&name) {
                        return Ok(JsValue::UNDEFINED);
                    }
                    let modal = call(&target, "closest", &[".modal".into()])?;
                    let where_ = if truthy(&modal) {
                        format!("modal:{}", string(&get(&modal, "id")))
                    } else {
                        let mut selected = "tablero";
                        for (selector, label) in [
                            ("#command-sidebar", "comandos"),
                            ("header", "header"),
                        ] {
                            if truthy(&call(&target, "closest", &[selector.into()])?) {
                                selected = label;
                                break;
                            }
                        }
                        selected.into()
                    };
                    exported("ulog", &["click".into(), name, where_.into()])?;
                    Ok(JsValue::UNDEFINED)
                }),
                true.into(),
            ],
        )?;
        let observer = new(
            "MutationObserver",
            &[function(|a| {
                for m in Array::from(&a.get(0)).iter() {
                    let el = get(&m, "target");
                    if !el.is_instance_of::<web_sys::Element>() {
                        continue;
                    }
                    let cls = get(&el, "classList");
                    let id = string(&get(&el, "id"));
                    if truthy(&call(&cls, "contains", &["modal".into()])?) {
                        let name = format!("modal:{id}");
                        if truthy(&call(&cls, "contains", &["open".into()])?) {
                            screen_start(name.into())?;
                        } else {
                            screen_end(name.into())?;
                        }
                    }
                }
                Ok(JsValue::UNDEFINED)
            })],
        )?;
        call(
            &observer,
            "observe",
            &[
                get(&doc(), "documentElement"),
                from_json(&json!({"attributes":true,"attributeFilter":["class"],"subtree":true}))?,
            ],
        )?;
        listen(
            &doc(),
            "visibilitychange",
            function(|_| {
                if truthy(&get(&doc(), "hidden")) {
                    exported(
                        "ulog",
                        &[
                            "screen".into(),
                            "tablero-visible".into(),
                            "".into(),
                            ((now() - number(&get(&global("ULOG"), "visibleSince"))) / 1000.0)
                                .into(),
                        ],
                    )?;
                    screen_end(JsValue::UNDEFINED)?;
                    flush()?;
                } else {
                    set(&global("ULOG"), "visibleSince", &now().into())?;
                }
                Ok(JsValue::UNDEFINED)
            }),
        );
        Ok(())
    }
    fn alert_layout() -> Result<(), JsValue> {
        let container = id("toasts");
        if !truthy(&container) {
            return Ok(());
        }
        let cards = all(&container, ".alert");
        for (i, c) in cards.iter().enumerate() {
            classes(c, "hidden", i > 0);
        }
        let mut more = query(&container, ".alert-more");
        if cards.len() > 1 {
            if !truthy(&more) {
                more = call(&doc(), "createElement", &["button".into()])?;
                set(&more, "type", &"button".into())?;
                set(&more, "className", &"alert-more".into())?;
                listen(
                    &more,
                    "click",
                    // No notification drawer: the button reveals the queued alerts.
                    function(|_| {
                        for c in all(&id("toasts"), ".alert") {
                            classes(&c, "hidden", false);
                        }
                        Ok(JsValue::UNDEFINED)
                    }),
                );
                call(&container, "appendChild", &[more.clone()])?;
            }
            let n = cards.len() - 1;
            let text = exported(
                "tf",
                &[
                    format!("y {n} más →").into(),
                    format!("and {n} more →").into(),
                ],
            )?;
            set(&more, "textContent", &text)?;
        } else if truthy(&more) {
            call(&more, "remove", &[])?;
        }
        Ok(())
    }
    fn alert_clear(session: JsValue) -> Result<(), JsValue> {
        let alerts = global("ALERTS");
        let alert = call(&alerts, "get", std::slice::from_ref(&session))?;
        if truthy(&alert) {
            call(&alert, "remove", &[])?;
            call(&alerts, "delete", &[session])?;
            alert_layout()?;
        }
        Ok(())
    }
    pub fn mount_notices() -> Result<(), JsValue> {
        set(&js_sys::global(), "ALERTS", &new("Map", &[])?)?;
        set(&js_sys::global(), "ATTENDED", &new("Map", &[])?)?;
        publish("alertPush", |_| Ok(JsValue::UNDEFINED))?;
        publish("alertLayout", |_| {
            alert_layout()?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("alertClear", |a| {
            alert_clear(a.get(0))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("alertResolve", |a| {
            let s = a.get(0);
            alert_clear(s.clone())?;
            call(&global("ATTENDED"), "set", &[s, now().into()])?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("toast", |a| {
            let node = call(&doc(), "createElement", &["div".into()])?;
            set(
                &node,
                "className",
                &if truthy(&a.get(1)) {
                    "toast err"
                } else {
                    "toast"
                }
                .into(),
            )?;
            set(&node, "textContent", &a.get(0))?;
            call(&id("toasts"), "appendChild", std::slice::from_ref(&node))?;
            later(
                function(move |_| {
                    call(&node, "remove", &[])?;
                    Ok(JsValue::UNDEFINED)
                }),
                3500.0,
            );
            Ok(JsValue::UNDEFINED)
        })
    }
    /// Only the sessions the page draws (see the server's `live_items`).
    const LIVE_STATE: &str = "/state?live=1";
    fn tick() -> Result<JsValue, JsValue> {
        if truthy(&global("statePollInFlight")) {
            return Ok(Promise::resolve(&JsValue::UNDEFINED).into());
        }
        exported("refreshFavorites", &[])?;
        set(&js_sys::global(), "statePollInFlight", &true.into())?;
        let requested = exported("api", &[LIVE_STATE.into()]);
        Ok(promise(async move {
            if let Ok(state) = wait(requested).await {
                let _ = exported("render", &[state]);
            }
            set(&js_sys::global(), "statePollInFlight", &false.into())?;
            exported("tickUsage", &[])?;
            Ok(JsValue::UNDEFINED)
        }))
    }
    pub fn mount_poll() -> Result<(), JsValue> {
        set(&js_sys::global(), "statePollInFlight", &false.into())?;
        set(&js_sys::global(), "timer", &JsValue::NULL)?;
        publish("tick", |_| {
            Ok(tick().unwrap_or_else(|e| Promise::reject(&e).into()))
        })?;
        publish("remotePollSeconds", |_| {
            Ok(poll_seconds(
                number(&get(&get(&global("S"), "cfg"), "poll")),
                truthy(&get(&doc(), "hidden")),
            )
            .into())
        })?;
        publish("arm", |_| {
            call(&js_sys::global(), "clearInterval", &[global("timer")])?;
            let seconds = exported("remotePollSeconds", &[])?;
            let timer = call(
                &js_sys::global(),
                "setInterval",
                &[global("tick"), (number(&seconds) * 1000.0).into()],
            )?;
            set(&js_sys::global(), "timer", &timer)?;
            Ok(JsValue::UNDEFINED)
        })
    }
    pub fn attach_poll() -> Result<(), JsValue> {
        listen(&doc(), "visibilitychange", global("arm"));
        Ok(())
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
#[allow(clippy::unwrap_used)]
mod wasm_tests {
    use super::*;
    use comandos_web_dom::port::*;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_test::*;
    #[wasm_bindgen(inline_js = r#"
export function lifecycle_fixture() {
 globalThis.window=globalThis; globalThis.times=[]; globalThis.intervals=[];globalThis.cleared=[];
 globalThis.setTimeout=(f,ms)=>{times.push({f,ms});return times.length};globalThis.setInterval=(f,ms)=>{intervals.push({f,ms});return intervals.length};globalThis.clearInterval=x=>cleared.push(x);
 globalThis.listeners={};globalThis.addEventListener=(name,f)=>{(listeners[name]??=[]).push(f)};
 globalThis.Element=class {
 constructor(id='',classes=[]){this.id=id;this.nodeType=1;this.dataset={};this.tagName='BUTTON';this.textContent='';this.children=[];this.attrs={};this.events={};this.style={setProperty:(k,v)=>this.style[k]=v};
 this.classList=new Set(classes);this.classList.contains=this.classList.has;this.classList.toggle=(c,on)=>on?this.classList.add(c):this.classList.delete(c);}
 addEventListener(n,f){this.events[n]=f} appendChild(n){this.children.push(n);n.parent=this;return n} remove(){if(this.parent)this.parent.children=this.parent.children.filter(x=>x!==this);this.removed=true}
 querySelectorAll(s){return this.children.filter(n=>s==='.alert'&&n.classList.has('alert'))} querySelector(s){return this.children.find(n=>s==='.alert-more'&&n.className==='alert-more')??null}
 closest(s){return this.closestMap?.[s]??null} getAttribute(k){return this.attrs[k]??null}click(){this.clicked=true} getBoundingClientRect(){return {bottom:66}}
 };
 globalThis.nodes={};for(const id of ['toasts','btn-notif','btn-pomo','settings'])nodes['#'+id]=new Element(id);
 globalThis.header=new Element('head'); globalThis.document={documentElement:new Element('html'),body:new Element('body'),hidden:false,events:{},querySelector:s=>s==='header'?header:nodes[s]??null,querySelectorAll:()=>[],getElementById:k=>nodes['#'+k]??null,createElement:t=>{const x=new Element();x.tagName=t.toUpperCase();return x},addEventListener(n,f,opt){(this.events[n]??=[]).push({f,opt})}};
 globalThis.ResizeObserver=class{constructor(f){this.f=f}observe(){}};globalThis.observers=[];globalThis.MutationObserver=class{constructor(f){this.f=f;observers.push(this)}observe(){}};
 const store=new Map();globalThis.localStorage={getItem:k=>store.get(k)??null,setItem:(k,v)=>store.set(k,String(v)),removeItem:k=>store.delete(k)};
 globalThis.location={search:'',origin:'https://private.invalid'};globalThis.webkit=undefined;globalThis.actions=[];globalThis.pending=[];
 globalThis.api=(path,body)=>{actions.push({path,body});return new Promise((resolve,reject)=>pending.push({resolve,reject}));};
 globalThis.tf=(es,en)=>es;globalThis.appEnabled=()=>false;globalThis.openTerm=(...a)=>actions.push({term:a});globalThis.notifBadge=()=>actions.push({badge:true});globalThis.activePaneTarget=()=>({session:'selected'});
 globalThis.refreshFavorites=()=>actions.push({favorites:true});globalThis.render=r=>actions.push({render:r});globalThis.tickUsage=()=>actions.push({usage:true});
 globalThis.fetches=[];globalThis.fetch=(...args)=>{fetches.push(args);return Promise.resolve({})};Object.defineProperty(globalThis,'navigator',{configurable:true,value:{beacons:[],sendBeacon(...args){this.beacons.push(args);return false}}});
}
export async function contracts() {
 const eq=(a,b,msg)=>{if(JSON.stringify(a)!==JSON.stringify(b))throw Error(msg+': '+JSON.stringify(a)+' != '+JSON.stringify(b));}; const ok=(x,msg)=>{if(!x)throw Error(msg)};
 const ready=async()=>{for(let i=0;i<32;i++){if(pending.length)return;await Promise.resolve();}throw Error('expected pending fallback');};
 const it={session:'sess',cwd:'/private/path',project:'Project'};
 let p=openSession(it,'shell');eq(actions.slice(-1)[0],{path:'/shell',body:{session:'sess',cwd:'/private/path'}},'local request starts synchronously');pending.shift().resolve({});eq(await p,'Tu terminal esta en Project','local result');
 p=openSession(it,'claude');pending.shift().reject(Error('gone'));await ready();eq(actions.slice(-1)[0].path,'/up','local fallback');pending.shift().resolve({});await p;
 globalThis.appEnabled=()=>true;p=openSession(it,'claude');eq(actions.slice(-1)[0].path,'/ensure','remote ensure');pending.shift().reject(Error('ensure'));await ready();eq(actions.slice(-1)[0].path,'/up','remote up');pending.shift().resolve({session:'revived'});eq(await p,'Project abierto','remote result');eq(actions.slice(-1)[0],{term:['revived','Project']},'remote target');
 p=openSession(it,'claude');pending.shift().reject(Error('ensure'));await ready();pending.shift().reject(Error('up'));await p;eq(actions.slice(-1)[0],{term:['sess','Project']},'remote both failure fallback');
 globalThis.nativeMessages=[];globalThis.webkit={messageHandlers:{centro:{postMessage:s=>nativeMessages.push(JSON.parse(s))}}};p=openSession(it,'claude');pending.shift().resolve({session:'native'});eq(await p,'Project en su pestana','native result');eq(nativeMessages.slice(-1)[0],{session:'native',win:'claude'},'native payload omission');
 openInApp('one','shell','Label');eq(nativeMessages.slice(-1)[0],{session:'one',win:'shell',label:'Label'},'native explicit label');
 localStorage.setItem('cc-center-panel',JSON.stringify({panel:'settings',at:Date.now(),extra:'keep'}));location.search='?panel=chains';__mountApp();eq(ONLY_PANEL,'settings','fresh center override');eq(CENTER_REQ.extra,'keep','unknown request fields');eq(localStorage.getItem('cc-center-panel'),null,'request consumed');ok(document.documentElement.classList.has('gtkapp'),'native root');__attachApp();eq(times.slice(-1)[0].ms,300,'center delay');globalThis.openCenterPanel=r=>actions.push({center:r});times.slice(-1)[0].f();eq(actions.slice(-1)[0].center.panel,'settings','center open');
 localStorage.setItem('cc-center-panel',JSON.stringify({panel:'remote',at:Date.now()-20000}));__mountApp();eq(CENTER_REQ,null,'expired center ignored');eq(ONLY_PANEL,'chains','query fallback');
 location.search='?panel=UsAgE1-news';__mountApp();eq(ONLY_PANEL,'sgnews','query strips all but lowercase');
 globalThis.webkit=undefined;location.search='';
 ULOG.buf=[];ulog('ignored','');eq(ULOG.buf.length,0,'empty name ignored');ulog('click','😀'.repeat(50),'x'.repeat(90),-1.5);eq(ULOG.buf.length,1,'buffer one');eq(ULOG.buf[0].n.length,80,'UTF16 limit');eq(ULOG.buf[0].c.length,80,'context limit');eq(ULOG.buf[0].d,-1,'JS round ties');eq(ULOG.buf[0].s,'selected','active pane');
 ulogFlush();eq(ULOG.buf.length,0,'flush drains');eq(navigator.beacons[0][0],'/ui-log','beacon route');const delivered=JSON.parse(await navigator.beacons[0][1].text());eq(delivered.events.length,1,'beacon body');
 globalThis.webkit={messageHandlers:{centro:{postMessage(){}}}};for(let i=0;i<40;i++)ulog('click','name');eq(ULOG.buf.length,0,'40 threshold');eq(fetches.slice(-1)[0][0],'/ui-log','native raw fetch');ok(fetches.slice(-1)[0][1].keepalive,'keepalive');ok(!fetches.slice(-1)[0][1].headers['X-Comandos-Token'],'original raw log headers');
 const button=new Element('', ['on','hidden','primary']);button.textContent='  Hello  ';button.closestMap={'[data-ulog],button,a,[role=tab],[role=switch],.row,.ssh-chip':button};eq(ulogNameOf(button),'primary:Hello','fallback control name');button.id='exact';eq(ulogNameOf(button),'#exact','id priority');button.dataset.ulog='stable';eq(ulogNameOf(button),'stable','data priority');
 ulogScreenStart('modal:settings');const t0=ULOG.screens.get('modal:settings');ulogScreenStart('modal:settings');eq(ULOG.screens.get('modal:settings'),t0,'screen starts once');ulogScreenEnd();eq(ULOG.screens.size,0,'all screens end');ok(ULOG.buf.some(x=>x.k==='screen'&&x.n==='modal:settings'),'duration logged');
 __attachLog();eq(intervals.slice(-1)[0].ms,5000,'flush interval');eq(document.documentElement.style['--hdr-h'],'66px','header geometry');ok(document.events.click[0].opt===true,'capture clicks');
 const modal=new Element('settings',['modal','open']);observers.slice(-1)[0].f([{target:modal}]);ok(ULOG.screens.has('modal:settings'),'modal observer starts');modal.classList.delete('open');observers.slice(-1)[0].f([{target:modal}]);ok(!ULOG.screens.has('modal:settings'),'modal observer ends');
 toast('<b>safe</b>',true);eq(nodes['#toasts'].children.slice(-1)[0].textContent,'<b>safe</b>','toast uses text');eq(times.slice(-1)[0].ms,3500,'toast timeout');times.slice(-1)[0].f();
 const card=new Element('', ['alert']);nodes['#toasts'].appendChild(card);ALERTS.set('sess',card);alertResolve('sess');ok(card.removed,'resolve removes old alert');ok(ATTENDED.has('sess'),'attendance mark');
 globalThis.S={cfg:{poll:2}};document.hidden=true;eq(remotePollSeconds(),15,'hidden minimum');document.hidden=false;S.cfg.poll='7';eq(remotePollSeconds(),7,'number string');__attachPoll();arm();eq(intervals.slice(-1)[0].ms,7000,'arm interval');ok(cleared.length,'old interval cleared');
 actions=[];p=tick();ok(statePollInFlight,'poll marks synchronously');const extra=await tick();eq(extra,undefined,'duplicate resolves');eq(actions.filter(x=>x.path==='/state').length,1,'one state request');pending.shift().resolve({list:[]});await p;ok(!statePollInFlight,'in flight cleared');ok(actions.some(x=>x.render),'renders');ok(actions.some(x=>x.usage),'usage tick');
 p=tick();pending.shift().reject(Error('offline'));await p;ok(!statePollInFlight,'failure clears flag');
}
"#)]
    extern "C" {
        fn lifecycle_fixture();
        #[wasm_bindgen(catch)]
        async fn contracts() -> Result<JsValue, JsValue>;
    }
    #[wasm_bindgen_test(async)]
    async fn lifecycle_exports_preserve_native_remote_logs_notices_and_polling() {
        lifecycle_fixture();
        mount_app().unwrap();
        mount_log().unwrap();
        mount_notices().unwrap();
        mount_poll().unwrap();
        for (name, f) in [
            ("__mountApp", mount_app as fn() -> Result<(), JsValue>),
            ("__attachApp", attach_app),
            ("__attachLog", attach_log),
            ("__attachPoll", attach_poll),
        ] {
            set(
                &js_sys::global(),
                name,
                &function(move |_| {
                    f()?;
                    Ok(JsValue::UNDEFINED)
                }),
            )
            .unwrap();
        }
        contracts().await.unwrap();
    }
}
