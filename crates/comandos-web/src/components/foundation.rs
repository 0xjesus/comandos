//! First complete coordinator regions: shared state, i18n and authenticated API.
//! No duplicate component implementation or legacy script evaluation.
use serde_json::{Value, json};

pub fn translations() -> Value {
    serde_json::from_str(include_str!("foundation_translations.json")).unwrap_or_else(|_| json!({}))
}
pub fn translate(text: &str, english: bool) -> String {
    if english {
        translations()
            .get(text)
            .and_then(Value::as_str)
            .unwrap_or(text)
            .into()
    } else {
        text.into()
    }
}
pub fn translated_pair<'a>(es: &'a str, en: &'a str, english: bool) -> &'a str {
    if english { en } else { es }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn mount_state() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_i18n() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_network() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn translation_dictionary_and_fallback_are_preserved() {
        assert_eq!(
            translate("Pregunta pendiente · claude apagado", true),
            "Pending question · claude is off"
        );
        assert_eq!(translate("Guardar", false), "Guardar");
        assert_eq!(translate("unknown <text>", true), "unknown <text>");
        assert_eq!(translated_pair("es", "en", true), "en");
        assert_eq!(translated_pair("es", "en", false), "es");
        assert_eq!(translations().as_object().unwrap().len(), 85);
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{mount_i18n, mount_network, mount_state};
#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use crate::components::web_support::*;
    use comandos_web_dom::port::*;
    use js_sys::{Array, Function, Promise, Reflect};
    use std::cell::RefCell;
    use wasm_bindgen::{JsCast, JsValue};
    thread_local! { static TOKEN_PROMISE: RefCell<Option<JsValue>> = const { RefCell::new(None) }; }

    fn new(name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
        let ctor = global(name).dyn_into::<Function>()?;
        Reflect::construct(&ctor, &args.iter().cloned().collect::<Array>())
    }
    fn publish(
        name: &str,
        f: impl Fn(Array) -> Result<JsValue, JsValue> + 'static,
    ) -> Result<(), JsValue> {
        set(&js_sys::global(), name, &function(f))
    }
    pub fn mount_state() -> Result<(), JsValue> {
        let storage = global("localStorage");
        let stored = call(&storage, "getItem", &["cc-cfg".into()])?;
        let parsed = js_sys::JSON::parse(&if truthy(&stored) {
            string(&stored)
        } else {
            "{}".into()
        })?;
        let cfg = from_json(&json!({"notif":false,"poll":2}))?;
        call(&global("Object"), "assign", &[cfg.clone(), parsed])?;
        let state = object();
        for (name, value) in [
            ("prev", new("Map", &[])?),
            ("cfg", cfg),
            ("favs", new("Set", &[])?),
            ("q", "".into()),
            ("liveSess", new("Set", &[])?),
            ("recover", Array::new().into()),
            ("usage", JsValue::NULL),
            ("usageByPane", new("Map", &[])?),
            ("usageSelected", JsValue::NULL),
        ] {
            set(&state, name, &value)?;
        }
        set(&js_sys::global(), "S", &state)?;
        publish("__comandosState", move |_| Ok(state.clone()))?;
        publish("$", |a| call(&doc(), "querySelector", &[a.get(0)]))
    }
    fn english_now() -> bool {
        global("L").as_string().as_deref() == Some("en")
    }
    pub fn mount_i18n() -> Result<(), JsValue> {
        set(&js_sys::global(), "L", &"es".into())?;
        set(&js_sys::global(), "T_EN", &from_json(&translations())?)?;
        publish("t", |a| {
            let x = a.get(0);
            let translated = get(&global("T_EN"), &string(&x));
            Ok(
                if english_now() && !translated.is_null() && !translated.is_undefined() {
                    translated
                } else {
                    x
                },
            )
        })?;
        let pair = function(|a| Ok(a.get(u32::from(english_now()))));
        set(&js_sys::global(), "tf", &pair)?;
        set(&js_sys::global(), "__comandosTranslate", &pair)?;
        publish("applyI18n", |_| {
            apply_i18n()?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("saveCfg", |_| {
            let cfg = get(&global("S"), "cfg");
            call(
                &global("localStorage"),
                "setItem",
                &["cc-cfg".into(), js_sys::JSON::stringify(&cfg)?.into()],
            )?;
            Ok(JsValue::UNDEFINED)
        })
    }
    fn apply_i18n() -> Result<(), JsValue> {
        if !english_now() {
            return Ok(());
        }
        for el in all(
            &doc(),
            "#settings label, #settings h3, #settings .desc, #settings .test, #servers label, #servers h3, #servers .desc, #servers .test, #srv-cancel, #remote label, #remote h3, #remote .desc, #remote .test, #remote-no-qr, .sec-label, #ssh-bar .lbl, #ssh-manage, #btn-remote, #btn-settings, .pill span",
        ) {
            let key = string(&get(&el, "textContent"));
            let val = get(&global("T_EN"), key.trim());
            if truthy(&val) {
                set(&el, "textContent", &val)?;
            }
        }
        for (selector, key, value) in [
            ("#q", "placeholder", "search session"),
            (
                "#sw-in",
                "placeholder",
                "Jump to...  (Enter opens the first, ↑↓ navigate, Esc closes)",
            ),
            ("#q", "title", "Filter by name or path"),
            (
                "#snd-ctl",
                "title",
                "Global volume (voice and chime). The icon mutes/unmutes.",
            ),
            ("#btn-switch", "title", "Jump to any session (Ctrl+K)"),
            (
                "#btn-theme",
                "title",
                "Theme: Night → Day → Warm (dashboard, tabs and terminals)",
            ),
            (
                "#ssh-manage",
                "title",
                "Add, edit or delete (they live in ~/.ssh/config)",
            ),
        ] {
            let el = query(&doc(), selector);
            if truthy(&el) {
                set(&el, key, &value.into())?;
            }
        }
        Ok(())
    }
    fn auth_token() -> JsValue {
        call(&global("localStorage"), "getItem", &["cc_token".into()])
            .ok()
            .filter(truthy)
            .unwrap_or_else(|| "".into())
    }
    fn consume_token() -> Result<(), JsValue> {
        let url = new("URL", &[get(&global("location"), "href")])?;
        let params = get(&url, "searchParams");
        let token = call(&params, "get", &["token".into()])?;
        if truthy(&token) {
            call(
                &global("localStorage"),
                "setItem",
                &["cc_token".into(), token],
            )?;
            call(&params, "delete", &["token".into()])?;
            let clean = format!(
                "{}{}{}",
                string(&get(&url, "pathname")),
                string(&get(&url, "search")),
                string(&get(&url, "hash"))
            );
            call(
                &global("history"),
                "replaceState",
                &[JsValue::NULL, "".into(), clean.into()],
            )?;
        }
        Ok(())
    }
    fn remote_auth_notice(required: bool) -> Result<(), JsValue> {
        let existing = query(&doc(), "#remote-auth-required");
        if !required {
            if truthy(&existing) {
                call(&existing, "remove", &[])?;
            }
            return Ok(());
        }
        let container = query(&doc(), "#toasts");
        if truthy(&existing) || !truthy(&container) {
            return Ok(());
        }
        let notice = call(&doc(), "createElement", &["div".into()])?;
        set(&notice, "id", &"remote-auth-required".into())?;
        set(&notice, "className", &"toast err".into())?;
        call(&notice, "setAttribute", &["role".into(), "alert".into()])?;
        set(
            &notice,
            "textContent",
            &translated_pair(
                "Acceso remoto no autorizado. En ComandOS de tu equipo, abre Remoto y usa su enlace o QR para autorizar este navegador.",
                "Remote access is not authorized. Open Remote in ComandOS on your computer and use its link or QR code to authorize this browser.",
                english_now(),
            ).into(),
        )?;
        call(&container, "appendChild", &[notice])?;
        Ok(())
    }
    // Starts fetch synchronously, matching async JS's execution before its first await.
    fn api(path: JsValue, body: JsValue) -> Result<JsValue, JsValue> {
        let state_poll = path.as_string().is_some_and(|p| p == "/state" || p.starts_with("/state?"));
        let token = auth_token();
        let headers = object();
        if truthy(&token) {
            set(&headers, "X-Comandos-Token", &token)?;
        }
        let mut args = vec![path];
        let post = truthy(&body);
        if post {
            set(&headers, "Content-Type", &"application/json".into())?;
            let options = object();
            set(&options, "method", &"POST".into())?;
            set(&options, "headers", &headers)?;
            set(&options, "body", &js_sys::JSON::stringify(&body)?.into())?;
            args.push(options);
        } else if truthy(&token) {
            let options = object();
            set(&options, "headers", &headers)?;
            args.push(options);
        }
        let fetched = call(&js_sys::global(), "fetch", &args);
        Ok(promise(async move {
            let response = wait(fetched).await?;
            if get(&response, "status").as_f64() == Some(401.0) {
                let _ = remote_auth_notice(true);
            } else if state_poll && truthy(&get(&response, "ok")) {
                let _ = remote_auth_notice(false);
            }
            let parsed = wait(call(&response, "json", &[]))
                .await
                .unwrap_or_else(|_| object());
            if post && truthy(&get(&response, "ok")) && (parsed.is_null() || parsed.is_undefined())
            {
                return Err(
                    js_sys::TypeError::new("Cannot read properties of null or undefined").into(),
                );
            }
            if !truthy(&get(&response, "ok")) || (post && get(&parsed, "ok") == JsValue::FALSE) {
                if parsed.is_null() || parsed.is_undefined() {
                    return Err(js_sys::TypeError::new(
                        "Cannot read properties of null or undefined",
                    )
                    .into());
                }
                let reason = [
                    get(&parsed, "error"),
                    get(&parsed, "message"),
                    get(&response, "statusText"),
                ]
                .into_iter()
                .find(truthy)
                .unwrap_or_else(|| "La acción no se completó".into());
                return Err(js_sys::Error::new(&string(&reason)).into());
            }
            Ok(parsed)
        }))
    }
    pub fn mount_network() -> Result<(), JsValue> {
        let _ = consume_token(); // original swallows URL/storage/history failures
        publish("authToken", |_| Ok(auth_token()))?;
        publish("api", |a| {
            Ok(api(a.get(0), a.get(1)).unwrap_or_else(|e| Promise::reject(&e).into()))
        })?;
        publish("webtermAccessToken", |_| {
            let remote = auth_token();
            if truthy(&remote) {
                return Ok(Promise::resolve(&remote).into());
            }
            TOKEN_PROMISE.with(|cell| {
                let mut cache = cell
                    .try_borrow_mut()
                    .map_err(|_| js_sys::Error::new("token promise busy"))?;
                if let Some(p) = cache.as_ref() {
                    return Ok(p.clone());
                }
                let fetched = api("/webterm-token".into(), JsValue::UNDEFINED);
                let p = promise(async move {
                    let r = wait(fetched).await?;
                    let token = get(&r, "token");
                    Ok(if truthy(&token) { token } else { "".into() })
                });
                *cache = Some(p.clone());
                Ok(p)
            })
        })?;
        getter(&js_sys::global(), "webtermTokenPromise", || {
            TOKEN_PROMISE.with(|c| {
                c.try_borrow()
                    .ok()
                    .and_then(|p| p.clone())
                    .unwrap_or(JsValue::NULL)
            })
        })
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
export function fixture() {
 const store=new Map([['cc-cfg','{"poll":7,"notif":true,"extra":"kept"}']]);
 globalThis.localStorage={getItem:k=>store.get(k)??null,setItem:(k,v)=>store.set(k,String(v))};
 globalThis.location={href:'https://private.invalid/app?token=secret&x=1#here'};
 globalThis.history={replaceState:(...args)=>globalThis.lastHistory=args};
 globalThis.nodes={'#q':{},'#sw-in':{},'#btn-theme':{}};
 globalThis.labelNode={textContent:' Guardar '};
 globalThis.document={querySelector:s=>globalThis.nodes[s]??null,querySelectorAll:()=>[globalThis.labelNode]};
 globalThis.calls=[]; globalThis.reply={body:{ok:true},ok:true,statusText:''};
 globalThis.fetch=(...args)=>{globalThis.calls.push(args);const r=globalThis.reply; return Promise.resolve({ok:r.ok,status:r.status??(r.ok?200:500),statusText:r.statusText,json:()=>r.rejectJson?Promise.reject(new Error('bad json')):Promise.resolve(r.body)});};
}
export function auth_fixture() {
 fixture();
 const notices=[];
 globalThis.authNotices=notices;
 globalThis.document.querySelector=s=>s==='#toasts'?{appendChild:n=>notices.push(n)}:s==='#remote-auth-required'?notices.find(n=>n.id==='remote-auth-required')??null:globalThis.nodes[s]??null;
 globalThis.document.createElement=()=>({setAttribute(k,v){this[k]=v},remove(){const i=notices.indexOf(this);if(i!==-1)notices.splice(i,1)}});
}
"#)]
    extern "C" {
        fn fixture();
        fn auth_fixture();
    }
    fn global(name: &str) -> JsValue {
        get(&js_sys::global(), name)
    }
    async fn invoke_async(name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
        wait(invoke(&global(name), args)).await
    }
    #[wasm_bindgen_test(async)]
    async fn unauthorized_remote_access_remains_visible_without_clearing_sessions() {
        auth_fixture();
        mount_state().unwrap();
        mount_i18n().unwrap();
        mount_network().unwrap();
        let sessions = from_json(&json!([{"session":"preserved"}])).unwrap();
        set(&global("S"), "list", &sessions).unwrap();
        set(
            &js_sys::global(),
            "reply",
            &from_json(&json!({"ok":false,"status":401,"body":{"error":"Unauthorized"}})).unwrap(),
        )
        .unwrap();
        for path in ["/state", "/tabs", "/state"] {
            assert!(invoke_async("api", &[path.into()]).await.is_err());
        }
        let notices = global("authNotices");
        assert_eq!(
            get(&notices, "length").as_f64(),
            Some(1.0),
            "401 must show one persistent notice"
        );
        let notice = get(&notices, "0");
        let text = get(&notice, "textContent").as_string().unwrap();
        assert!(text.contains("Remoto") && text.contains("QR"));
        assert!(!text.contains("secret"));
        assert_eq!(get(&notice, "role").as_string().as_deref(), Some("alert"));
        assert!(js_sys::Object::is(&get(&global("S"), "list"), &sessions));
        assert_eq!(
            invoke(&global("authToken"), &[])
                .unwrap()
                .as_string()
                .as_deref(),
            Some("secret")
        );
        set(
            &js_sys::global(),
            "reply",
            &from_json(&json!({"ok":true,"status":200,"body":[]})).unwrap(),
        )
        .unwrap();
        invoke_async("api", &["/state".into()]).await.unwrap();
        assert_eq!(
            get(&notices, "length").as_f64(),
            Some(0.0),
            "successful session refresh clears the warning"
        );
    }
    #[wasm_bindgen_test(async)]
    async fn exported_foundation_contracts() {
        fixture();
        mount_state().unwrap();
        mount_i18n().unwrap();
        mount_network().unwrap();
        let state = global("S");
        assert!(js_sys::Object::is(
            &state,
            &invoke(&global("__comandosState"), &[]).unwrap()
        ));
        assert_eq!(get(&get(&state, "cfg"), "poll").as_f64(), Some(7.0));
        assert_eq!(
            get(&get(&state, "cfg"), "extra").as_string().as_deref(),
            Some("kept")
        );
        for key in ["prev", "usageByPane"] {
            assert_eq!(
                call(&get(&state, key), "has", &["x".into()]).unwrap(),
                JsValue::FALSE
            );
        }
        for key in ["favs", "liveSess"] {
            call(&get(&state, key), "add", &["x".into()]).unwrap();
            assert_eq!(
                call(&get(&state, key), "has", &["x".into()]).unwrap(),
                JsValue::TRUE
            );
        }
        set(&get(&state, "cfg"), "poll", &9.into()).unwrap();
        invoke(&global("saveCfg"), &[]).unwrap();
        assert!(
            call(&global("localStorage"), "getItem", &["cc-cfg".into()])
                .unwrap()
                .as_string()
                .unwrap()
                .contains("\"poll\":9")
        );
        set(&js_sys::global(), "L", &"en".into()).unwrap();
        assert_eq!(
            invoke(&global("t"), &["Guardar".into()])
                .unwrap()
                .as_string()
                .as_deref(),
            Some("Save")
        );
        assert_eq!(
            invoke(&global("tf"), &["es".into(), "en".into()])
                .unwrap()
                .as_string()
                .as_deref(),
            Some("en")
        );
        invoke(&global("applyI18n"), &[]).unwrap();
        assert_eq!(
            get(&global("labelNode"), "textContent")
                .as_string()
                .as_deref(),
            Some("Save")
        );
        assert_eq!(
            get(&get(&global("nodes"), "#q"), "placeholder")
                .as_string()
                .as_deref(),
            Some("search session")
        );
        assert_eq!(
            get(&global("lastHistory"), "2").as_string().as_deref(),
            Some("/app?x=1#here")
        );
        assert_eq!(
            invoke_async("webtermAccessToken", &[])
                .await
                .unwrap()
                .as_string()
                .as_deref(),
            Some("secret")
        );
        assert_eq!(get(&global("calls"), "length").as_f64(), Some(0.0));
        invoke_async(
            "api",
            &["/post".into(), from_json(&json!({"value":"safe"})).unwrap()],
        )
        .await
        .unwrap();
        let options = get(&get(&global("calls"), "0"), "1");
        assert_eq!(get(&options, "method").as_string().as_deref(), Some("POST"));
        assert_eq!(
            get(&get(&options, "headers"), "X-Comandos-Token")
                .as_string()
                .as_deref(),
            Some("secret")
        );
        assert_eq!(
            get(&get(&options, "headers"), "Content-Type")
                .as_string()
                .as_deref(),
            Some("application/json")
        );
        assert_eq!(
            get(&options, "body").as_string().as_deref(),
            Some("{\"value\":\"safe\"}")
        );
        set(
            &js_sys::global(),
            "reply",
            &from_json(&json!({"ok":true,"body":{"ok":false,"error":"contract failure"}})).unwrap(),
        )
        .unwrap();
        let err = invoke_async("api", &["/post".into(), true.into()])
            .await
            .unwrap_err();
        assert_eq!(
            get(&err, "message").as_string().as_deref(),
            Some("contract failure")
        );
        assert_eq!(
            get(&invoke_async("api", &["/get".into()]).await.unwrap(), "ok"),
            JsValue::FALSE
        );
        set(
            &js_sys::global(),
            "reply",
            &from_json(&json!({"ok":false,"body":{},"statusText":"HTTP failed"})).unwrap(),
        )
        .unwrap();
        assert_eq!(
            get(
                &invoke_async("api", &["/get".into()]).await.unwrap_err(),
                "message"
            )
            .as_string()
            .as_deref(),
            Some("HTTP failed")
        );
        set(
            &js_sys::global(),
            "reply",
            &from_json(&json!({"ok":false,"body":{}})).unwrap(),
        )
        .unwrap();
        assert_eq!(
            get(
                &invoke_async("api", &["/get".into()]).await.unwrap_err(),
                "message"
            )
            .as_string()
            .as_deref(),
            Some("La acción no se completó")
        );
        set(
            &js_sys::global(),
            "reply",
            &from_json(&json!({"ok":true,"rejectJson":true})).unwrap(),
        )
        .unwrap();
        assert_eq!(
            to_json(&invoke_async("api", &["/get".into()]).await.unwrap()),
            json!({})
        );
        call(
            &global("localStorage"),
            "setItem",
            &["cc_token".into(), "".into()],
        )
        .unwrap();
        set(&js_sys::global(), "calls", &js_sys::Array::new().into()).unwrap();
        set(
            &js_sys::global(),
            "reply",
            &from_json(&json!({"ok":true,"body":{"token":"local"}})).unwrap(),
        )
        .unwrap();
        let a = invoke(&global("webtermAccessToken"), &[]).unwrap();
        let b = invoke(&global("webtermAccessToken"), &[]).unwrap();
        assert_eq!(
            wait(Ok(a)).await.unwrap().as_string().as_deref(),
            Some("local")
        );
        assert_eq!(
            wait(Ok(b)).await.unwrap().as_string().as_deref(),
            Some("local")
        );
        assert_eq!(get(&global("calls"), "length").as_f64(), Some(1.0));
        assert_eq!(
            get(&get(&global("calls"), "0"), "length").as_f64(),
            Some(1.0)
        );
        assert!(
            invoke_async("api", &["/null".into(), true.into()])
                .await
                .is_ok()
        );
        set(
            &js_sys::global(),
            "reply",
            &from_json(&json!({"ok":true,"body":null})).unwrap(),
        )
        .unwrap();
        assert!(
            invoke_async("api", &["/null".into(), true.into()])
                .await
                .is_err()
        );
        assert!(
            invoke_async("api", &["/null".into()])
                .await
                .unwrap()
                .is_null()
        );
        set(&js_sys::global(), "L", &"es".into()).unwrap();
        invoke(&global("applyI18n"), &[]).unwrap();
        assert_eq!(
            get(&global("labelNode"), "textContent")
                .as_string()
                .as_deref(),
            Some("Save")
        );
        call(
            &global("localStorage"),
            "setItem",
            &["cc-cfg".into(), "invalid json".into()],
        )
        .unwrap();
        assert!(mount_state().is_err());
    }
}
