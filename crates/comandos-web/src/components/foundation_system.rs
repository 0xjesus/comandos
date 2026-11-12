//! System notification preferences and centered native panels.
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_notifications() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}

#[cfg(all(test, target_arch = "wasm32"))]
#[allow(clippy::unwrap_used)]
mod wasm_tests {
    use super::*;
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen(inline_js = r#"
let systemContracts;
export async function nativeSystemReference(root){
 const {createRequire}=await import('node:module');
 systemContracts=createRequire(root+'/tests/system_contract.cjs')(root+'/crates/comandos-web/tests/foundation_system_contract.cjs');
 return JSON.stringify(await systemContracts.reference(root));
}
export function nativeSystemFixture(){systemContracts.install();}
export async function nativeSystemActual(){return JSON.stringify(await systemContracts.native());}
"#)]
    extern "C" {
        #[wasm_bindgen(catch)]
        async fn nativeSystemReference(root: &str) -> Result<JsValue, JsValue>;
        fn nativeSystemFixture();
        #[wasm_bindgen(catch)]
        async fn nativeSystemActual() -> Result<JsValue, JsValue>;
    }
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn system_notification_and_centered_panel_contracts_match_original() {
        let expected = nativeSystemReference(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
            .await
            .unwrap();
        nativeSystemFixture();
        mount_notifications().unwrap();
        mount_panels().unwrap();
        attach_notifications().unwrap();
        attach_panels().unwrap();
        let actual = nativeSystemActual().await.unwrap();
        assert_eq!(actual.as_string(), expected.as_string());
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach_notifications() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_panels() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach_panels() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(target_arch = "wasm32")]
pub use web::{attach_notifications, attach_panels, mount_notifications, mount_panels};
#[cfg(target_arch = "wasm32")]
mod web {
    use crate::components::web_support::*;
    use comandos_web_dom::port::*;
    use js_sys::{Array, Function, Reflect};
    use wasm_bindgen::{JsCast, JsValue};
    fn publish(
        name: &str,
        f: impl Fn(Array) -> Result<JsValue, JsValue> + 'static,
    ) -> Result<(), JsValue> {
        set(&js_sys::global(), name, &function(f))
    }
    fn query_node(selector: &str) -> JsValue {
        query(&doc(), selector)
    }
    fn on(el: &JsValue, key: &str) -> bool {
        truthy(&call(&get(el, "classList"), "contains", &[key.into()]).unwrap_or(JsValue::FALSE))
    }
    fn tr(es: JsValue, en: JsValue) -> JsValue {
        invoke(&global("tf"), &[es.clone(), en]).unwrap_or(es)
    }
    fn bool_attr(el: &JsValue, key: &str, v: bool) {
        attr(el, key, if v { "true" } else { "false" });
    }
    fn or(v: JsValue, otherwise: JsValue) -> JsValue {
        if truthy(&v) { v } else { otherwise }
    }
    fn notice_error(e: JsValue) -> Result<(), JsValue> {
        invoke(&global("toast"), &[get(&e, "message"), true.into()]).map(|_| ())
    }
    fn load_conf() -> JsValue {
        let fetched = invoke(&global("api"), &["/conf".into()]);
        promise(async move {
            let _ = async {
                let conf = wait(fetched).await?;
                set(
                    &js_sys::global(),
                    "DESKTOP_POPUPS",
                    &(get(&conf, "DESKTOP_NOTIFY") != JsValue::from_str("0")).into(),
                )?;
                for b in all(&doc(), ".cfg[data-key]") {
                    let key = string(&get(&get(&b, "dataset"), "key"));
                    let enabled = get(&conf, &key) != JsValue::from_str("0");
                    classes(&b, "on", enabled);
                    bool_attr(&b, "aria-checked", enabled);
                }
                let volume = number(&get(&conf, "VOLUME"));
                let volume = if volume.is_nan() || volume == 0.0 {
                    60.0
                } else {
                    volume
                };
                for selector in ["#vol", "#vol-top"] {
                    set(&query_node(selector), "value", &volume.into())?;
                }
                let sound = get(&conf, "SOUND_ENABLED") != JsValue::from_str("0");
                let mute = query_node("#btn-mute");
                let svg = invoke(
                    &global("svg"),
                    &[
                        if sound {
                            "volumeOn".into()
                        } else {
                            "volumeOff".into()
                        },
                        15.0.into(),
                    ],
                )?;
                set(&mute, "innerHTML", &svg)?;
                set(
                    &get(&mute, "dataset"),
                    "on",
                    &if sound { "1".into() } else { "0".into() },
                )?;
                classes(&mute, "on", sound);
                for b in all(&doc(), ".lang-btn") {
                    classes(
                        &b,
                        "on",
                        or(get(&conf, "CC_LANG"), "auto".into())
                            == get(&get(&b, "dataset"), "lang"),
                    );
                }
                let voice = get(&conf, "SPEAK_DONE") != JsValue::from_str("0")
                    || get(&conf, "SPEAK_ATTENTION") != JsValue::from_str("0");
                let b = query_node("#sw-voice");
                classes(&b, "on", voice);
                bool_attr(&b, "aria-checked", voice);
                Ok::<_, JsValue>(())
            }
            .await;
            Ok(JsValue::UNDEFINED)
        })
    }
    fn conf_set(key: JsValue, value: JsValue) -> Result<JsValue, JsValue> {
        let patch = object();
        set(&patch, "key", &key)?;
        set(&patch, "value", &value)?;
        invoke(&global("api"), &["/conf-set".into(), patch])
    }
    pub fn mount_notifications() -> Result<(), JsValue> {
        set(&js_sys::global(), "DESKTOP_POPUPS", &false.into())?;
        let ctor = global("Set").dyn_into::<Function>()?;
        let cats = Array::new();
        for name in ["done", "attention", "usage"] {
            cats.push(&name.into());
        }
        let args = Array::new();
        args.push(&cats);
        set(
            &js_sys::global(),
            "SYSTEM_POPUP_CATS",
            &Reflect::construct(&ctor, &args)?,
        )?;
        publish("loadConf", |_| Ok(load_conf()))
    }
    pub fn attach_notifications() -> Result<(), JsValue> {
        let mute = query_node("#btn-mute");
        listen(
            &mute,
            "click",
            function(|_| {
                let enabled =
                    get(&get(&query_node("#btn-mute"), "dataset"), "on") == JsValue::from_str("1");
                let fetched = conf_set(
                    "SOUND_ENABLED".into(),
                    if enabled { "0".into() } else { "1".into() },
                );
                Ok(promise(async move {
                    let result = async {
                        wait(fetched).await?;
                        invoke(
                            &global("toast"),
                            &[tr(
                                if enabled {
                                    "Sonido silenciado".into()
                                } else {
                                    "Sonido activado".into()
                                },
                                if enabled {
                                    "Sound muted".into()
                                } else {
                                    "Sound on".into()
                                },
                            )],
                        )?;
                        let _ = load_conf();
                        Ok::<_, JsValue>(())
                    }
                    .await;
                    if let Err(e) = result {
                        notice_error(e)?;
                    }
                    Ok(JsValue::UNDEFINED)
                }))
            }),
        );
        listen(
            &query_node("#vol-top"),
            "change",
            function(|_| {
                let fetched = conf_set(
                    "VOLUME".into(),
                    string(&get(&query_node("#vol-top"), "value")).into(),
                );
                Ok(promise(async move {
                    let result = async {
                        wait(fetched).await?;
                        let current = string(&get(&query_node("#vol-top"), "value"));
                        invoke(
                            &global("toast"),
                            &[tr(
                                format!("Volumen {current}% (todo el sistema)").into(),
                                format!("Volume {current}% (system-wide)").into(),
                            )],
                        )?;
                        let _ = load_conf();
                        Ok::<_, JsValue>(())
                    }
                    .await;
                    if let Err(e) = result {
                        notice_error(e)?;
                    }
                    Ok(JsValue::UNDEFINED)
                }))
            }),
        );
        for b in all(&doc(), ".cfg[data-key]") {
            let el = b.clone();
            listen(
                &b,
                "click",
                function(move |_| {
                    let enabled = !on(&el, "on");
                    let switch = call(&el, "closest", &[".switch".into()]).unwrap_or(JsValue::NULL);
                    let label = query(&switch, "label");
                    let name = or(get(&label, "textContent"), get(&get(&el, "dataset"), "key"));
                    let fetched = conf_set(
                        get(&get(&el, "dataset"), "key"),
                        if enabled { "1".into() } else { "0".into() },
                    );
                    let el = el.clone();
                    Ok(promise(async move {
                        let result = async {
                            wait(fetched).await?;
                            if get(&get(&el, "dataset"), "key")
                                == JsValue::from_str("DESKTOP_NOTIFY")
                            {
                                set(&js_sys::global(), "DESKTOP_POPUPS", &enabled.into())?;
                            }
                            classes(&el, "on", enabled);
                            bool_attr(&el, "aria-checked", enabled);
                            let status = tr(
                                if enabled {
                                    "activado".into()
                                } else {
                                    "apagado".into()
                                },
                                if enabled { "on".into() } else { "off".into() },
                            );
                            let message =
                                format!("{}: {}", utf16_string(&name), utf16_string(&status));
                            invoke(&global("toast"), &[utf16_value(&message)])?;
                            Ok::<_, JsValue>(())
                        }
                        .await;
                        if let Err(e) = result {
                            notice_error(e)?;
                        }
                        Ok(JsValue::UNDEFINED)
                    }))
                }),
            );
        }
        Ok(())
    }
    fn native_message(value: JsValue) -> Result<(), JsValue> {
        let handler = get(
            &get(&get(&global("window"), "webkit"), "messageHandlers"),
            "centro",
        );
        call(
            &handler,
            "postMessage",
            &[js_sys::JSON::stringify(&value)?.into()],
        )
        .map(|_| ())
    }
    fn shown(el: &JsValue) -> bool {
        on(el, "open")
    }
    fn observe(el: JsValue, f: JsValue) -> Result<(), JsValue> {
        let args = Array::new();
        args.push(&f);
        let observer =
            Reflect::construct(&global("MutationObserver").dyn_into::<Function>()?, &args)?;
        let options =
            from_json(&serde_json::json!({"attributes":true,"attributeFilter":["class"]}))?;
        call(&observer, "observe", &[el, options])?;
        Ok(())
    }
    fn click(selector: &str) -> Result<(), JsValue> {
        let el = query_node(selector);
        if !el.is_null() && !el.is_undefined() {
            call(&el, "click", &[])?;
        }
        Ok(())
    }
    fn open_panel(request: JsValue) -> Result<(), JsValue> {
        let panel = string(&get(&request, "panel"));
        let sel = get(&global("CENTER_PANELS"), &panel);
        let el = query_node(&string(&sel));
        if !truthy(&el) {
            return Ok(());
        }
        match panel.as_str() {
            "servers" => {
                invoke(&global("toggleSshManager"), &[])?;
            }
            "sovereignty" => click("#btn-sov")?,
            "remote" => click("#btn-remote")?,
            _ => {
                click("#btn-settings")?;
                let tab = get(&request, "tab");
                if truthy(&tab) {
                    invoke(
                        &global("activateMtab"),
                        &[query_node("#settings .modal-panel"), tab],
                    )?;
                }
            }
        }
        let watched = el.clone();
        observe(
            el,
            function(move |_| {
                if !shown(&watched) {
                    let _ = native_message(from_json(&serde_json::json!({"chainModal":"close"}))?);
                }
                Ok(JsValue::UNDEFINED)
            }),
        )
    }
    pub fn mount_panels() -> Result<(), JsValue> {
        publish("openCenterPanel", |a| {
            open_panel(a.get(0))?;
            Ok(JsValue::UNDEFINED)
        })
    }
    pub fn attach_panels() -> Result<(), JsValue> {
        if !truthy(&invoke(&global("inApp"), &[])?) || truthy(&global("ONLY_PANEL")) {
            return Ok(());
        }
        let panels = js_sys::Object::entries(&global("CENTER_PANELS").into());
        for row in panels.iter() {
            let pair = Array::from(&row);
            let panel = string(&pair.get(0));
            let el = query_node(&string(&pair.get(1)));
            if !truthy(&el) {
                continue;
            }
            let watched = el.clone();
            observe(
                el,
                function(move |_| {
                    if !shown(&watched) {
                        return Ok(JsValue::UNDEFINED);
                    }
                    let tab = if panel == "settings" {
                        or(
                            get(&get(&query(&watched, ".mtab.active"), "dataset"), "mtab"),
                            "".into(),
                        )
                    } else {
                        "".into()
                    };
                    let request = object();
                    set(&request, "panel", &panel.clone().into())?;
                    set(&request, "tab", &tab)?;
                    set(&request, "at", &js_sys::Date::now().into())?;
                    let saved = call(
                        &global("localStorage"),
                        "setItem",
                        &[
                            "cc-center-panel".into(),
                            js_sys::JSON::stringify(&request)?.into(),
                        ],
                    );
                    if saved.is_ok() {
                        native_message(from_json(&serde_json::json!({"headerAction":"chains"}))?)?;
                        classes(&watched, "open", false);
                    }
                    Ok(JsValue::UNDEFINED)
                }),
            )?;
        }
        Ok(())
    }
}
