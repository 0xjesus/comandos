//! Remote controls from the main coordinator, implemented without script evaluation.
#[cfg(not(target_arch = "wasm32"))]
pub fn mount() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(target_arch = "wasm32")]
pub use web::{attach, mount};
#[cfg(target_arch = "wasm32")]
mod web {
    use crate::components::web_support::*;
    use comandos_web_dom::port::*;
    use js_sys::{Array, Function, Reflect};
    use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
    #[wasm_bindgen(
        inline_js = "export function remoteTerms(){return typeof openTerms==='undefined'?undefined:openTerms;} export function remoteActiveTerm(){return typeof activeTerm==='undefined'?undefined:activeTerm;}"
    )]
    extern "C" {
        fn remoteTerms() -> JsValue;
        fn remoteActiveTerm() -> JsValue;
    }
    fn publish(
        name: &str,
        f: impl Fn(Array) -> Result<JsValue, JsValue> + 'static,
    ) -> Result<(), JsValue> {
        set(&js_sys::global(), name, &function(f))
    }
    fn tr(es: &str, en: &str) -> JsValue {
        invoke(&global("tf"), &[es.into(), en.into()]).unwrap_or_else(|_| es.into())
    }
    fn say(message: JsValue, error: bool) -> Result<(), JsValue> {
        invoke(&global("toast"), &[message, error.into()]).map(|_| ())
    }
    fn error(e: JsValue) -> Result<(), JsValue> {
        say(get(&e, "message"), true)
    }
    fn query_node(sel: &str) -> JsValue {
        query(&doc(), sel)
    }
    fn new(name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
        Reflect::construct(
            &global(name).dyn_into::<Function>()?,
            &args.iter().cloned().collect::<Array>(),
        )
    }
    fn qr_path() -> Result<JsValue, JsValue> {
        let params = object();
        set(&params, "ts", &string(&js_sys::Date::now().into()).into())?;
        let qs = new("URLSearchParams", &[params])?;
        let token = invoke(&global("authToken"), &[])?;
        if truthy(&token) {
            call(&qs, "set", &["token".into(), token])?;
        }
        Ok(format!("/remote-qr.png?{}", string(&call(&qs, "toString", &[])?)).into())
    }
    fn button_state(state: JsValue, busy: bool) -> Result<JsValue, JsValue> {
        let primary = truthy(&get(&state, "primaryHealthy"));
        let fallback = truthy(&get(&state, "fallbackHealthy"));
        let reachable = truthy(&get(&state, "webtermReachable"));
        let on = truthy(&get(&state, "remoteOn"));
        let out = object();
        for (key, val) in [
            ("remoteOnDisabled", busy || on),
            ("remoteOffDisabled", busy || !on),
            ("webtermOnDisabled", busy || (primary && fallback)),
            ("webtermOffDisabled", busy || !(primary || fallback)),
            ("openTerminalDisabled", busy || !reachable),
            ("remoteOnActive", on),
            ("remoteOffActive", false),
            ("webtermOnActive", primary && fallback),
            ("webtermOffActive", false),
            ("openTerminalActive", reachable),
        ] {
            set(&out, key, &val.into())?;
        }
        Ok(out)
    }
    fn set_button(
        sel: JsValue,
        disabled: JsValue,
        active: JsValue,
        warn: JsValue,
    ) -> Result<(), JsValue> {
        let b = query_node(&string(&sel));
        let disabled = truthy(&disabled);
        set(&b, "disabled", &disabled.into())?;
        classes(&b, "on", truthy(&active));
        classes(&b, "warn", truthy(&warn) && !disabled);
        Ok(())
    }
    fn apply_buttons() -> Result<(), JsValue> {
        let state = button_state(global("REMOTE"), truthy(&global("REMOTE_BUSY")))?;
        for (sel, disabled, active, warn) in [
            ("#remote-on", "remoteOnDisabled", "remoteOnActive", false),
            ("#remote-off", "remoteOffDisabled", "remoteOffActive", true),
            (
                "#remote-webterm-on",
                "webtermOnDisabled",
                "webtermOnActive",
                false,
            ),
            (
                "#remote-webterm-off",
                "webtermOffDisabled",
                "webtermOffActive",
                true,
            ),
            (
                "#remote-open-terminal",
                "openTerminalDisabled",
                "openTerminalActive",
                false,
            ),
        ] {
            set_button(
                sel.into(),
                get(&state, disabled),
                get(&state, active),
                warn.into(),
            )?;
        }
        Ok(())
    }
    fn busy(on: JsValue) -> Result<(), JsValue> {
        set(&js_sys::global(), "REMOTE_BUSY", &truthy(&on).into())?;
        apply_buttons()
    }
    fn or(value: JsValue, fallback: JsValue) -> JsValue {
        if truthy(&value) { value } else { fallback }
    }
    fn render(state: JsValue) -> Result<(), JsValue> {
        let state = or(state, object());
        set(&js_sys::global(), "REMOTE", &state)?;
        let status = query_node("#remote-status");
        let on = truthy(&get(&state, "remoteOn"));
        let terminal = or(get(&state, "terminalState"), "off".into());
        let active = terminal == JsValue::from_str("active");
        let degraded = terminal == JsValue::from_str("degraded");
        classes(&status, "on", active);
        classes(&status, "degraded", degraded);
        classes(&query_node("#btn-remote"), "on", on);
        let label = query(&status, "span:last-child");
        let text = if active {
            tr("Terminal: Activo", "Terminal: Active")
        } else if degraded {
            tr("Terminal: Degradado", "Terminal: Degraded")
        } else {
            tr("Terminal: Apagado", "Terminal: Off")
        };
        set(&label, "textContent", &text)?;
        let urls = or(get(&state, "urls"), object());
        for (selector, key) in [
            ("#remote-dashboard-url", "dashboard"),
            ("#remote-term-url", "terminal"),
            ("#remote-fallback-url", "terminalFallback"),
        ] {
            set(
                &query_node(selector),
                "value",
                &or(get(&urls, key), "".into()),
            )?;
        }
        let dashboard = truthy(&get(&urls, "dashboard"));
        let has_qr = dashboard && truthy(&get(&state, "qrAvailable"));
        classes(&query_node("#remote-qr"), "hidden", !has_qr);
        classes(&query_node("#remote-no-qr"), "hidden", has_qr || !dashboard);
        if has_qr {
            set(&query_node("#remote-qr"), "src", &qr_path()?)?;
        }
        apply_buttons()
    }
    fn load() -> JsValue {
        let fetched = invoke(&global("api"), &["/remote-state".into()]);
        promise(async move {
            let result = wait(fetched).await.and_then(render);
            if let Err(e) = result {
                error(e)?;
            }
            Ok(JsValue::UNDEFINED)
        })
    }
    fn trimmed(sel: &str) -> JsValue {
        js_sys::JsString::from(get(&query_node(sel), "value"))
            .trim()
            .into()
    }
    fn app_enabled() -> Result<bool, JsValue> {
        invoke(&global("appEnabled"), &[]).map(|v| truthy(&v))
    }
    fn open_url(href: JsValue) -> Result<(), JsValue> {
        call(
            &global("window"),
            "open",
            &[href, "_blank".into(), "noopener".into()],
        )
        .map(|_| ())
    }
    fn ensure() -> Result<JsValue, JsValue> {
        for selector in ["#remote", "#settings", "#servers"] {
            classes(&query_node(selector), "open", false);
        }
        if !app_enabled()? {
            let url = trimmed("#remote-term-url");
            if truthy(&url) {
                open_url(url)?;
            }
            return Ok(js_sys::Promise::resolve(&JsValue::UNDEFINED).into());
        }
        let tabs = invoke(&global("loadDesktopTabs"), &[]);
        Ok(promise(async move {
            wait(tabs).await?;
            let terms = remoteTerms();
            let mut session = or(
                remoteActiveTerm(),
                Array::from(&call(&terms, "keys", &[])?).get(0),
            );
            if !truthy(&session)
                && let Ok(rows) = wait(invoke(&global("api"), &["/state?live=1".into()])).await
            {
                let candidate = Array::from(&rows).iter().find(|row| {
                    let s = get(row, "session");
                    truthy(&get(row, "alive"))
                        && truthy(&s)
                        && !["local", "hub", "control"]
                            .iter()
                            .any(|v| s == JsValue::from_str(v))
                });
                if let Some(row) = candidate {
                    let s = get(&row, "session");
                    if invoke(
                        &global("addTermTab"),
                        &[s.clone(), or(get(&row, "project"), s.clone())],
                    )
                    .is_ok()
                    {
                        session = s;
                    }
                }
            }
            if truthy(&session) {
                let current = call(&terms, "get", std::slice::from_ref(&session))?;
                invoke(
                    &global("openTerm"),
                    &[session.clone(), or(get(&current, "label"), session)],
                )?;
            } else {
                let url = trimmed("#remote-term-url");
                if truthy(&url) {
                    set(&global("location"), "href", &url)?;
                }
            }
            Ok(JsValue::UNDEFINED)
        }))
    }
    fn action(path: JsValue, message: JsValue, after: JsValue) -> Result<JsValue, JsValue> {
        busy(true.into())?;
        let fetched = invoke(&global("api"), &[path, object()]);
        Ok(promise(async move {
            let result = async {
                render(wait(fetched).await?)?;
                invoke(&global("toast"), &[message])?;
                invoke(&global("arm"), &[])?;
                if truthy(&after) {
                    wait(invoke(&after, &[])).await?;
                }
                Ok::<_, JsValue>(())
            }
            .await;
            let toasted = result.or_else(error);
            busy(false.into())?;
            toasted?;
            Ok(JsValue::UNDEFINED)
        }))
    }
    fn copy(selector: JsValue) -> Result<JsValue, JsValue> {
        let value = trimmed(&string(&selector));
        if !truthy(&value) {
            return invoke(
                &global("toast"),
                &[tr("No hay URL todavia", "No URL yet"), true.into()],
            );
        }
        let pending = invoke(&global("copyText"), &[value])?;
        let then = function(|a| {
            let ok = truthy(&a.get(0));
            invoke(
                &global("toast"),
                &[
                    if ok {
                        tr("URL copiada", "URL copied")
                    } else {
                        tr("No pude copiar", "Could not copy")
                    },
                    (!ok).into(),
                ],
            )
        });
        call(&pending, "then", &[then])?;
        Ok(JsValue::UNDEFINED)
    }
    pub fn mount() -> Result<(), JsValue> {
        set(&js_sys::global(), "REMOTE", &JsValue::NULL)?;
        set(&js_sys::global(), "REMOTE_BUSY", &false.into())?;
        publish("remoteQrPath", |_| qr_path())?;
        publish("remoteButtonState", |a| {
            button_state(a.get(0), truthy(&a.get(1)))
        })?;
        publish("setRemoteButton", |a| {
            set_button(a.get(0), a.get(1), a.get(2), a.get(3))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("applyRemoteButtonState", |_| {
            apply_buttons()?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("setRemoteBusy", |a| {
            busy(a.get(0))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("renderRemote", |a| {
            render(a.get(0))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("loadRemote", |_| Ok(load()))?;
        publish("ensureRemoteTerminalVisible", |_| {
            Ok(ensure().unwrap_or_else(|e| js_sys::Promise::reject(&e).into()))
        })?;
        publish("remoteAction", |a| {
            Ok(action(a.get(0), a.get(1), a.get(2))
                .unwrap_or_else(|e| js_sys::Promise::reject(&e).into()))
        })?;
        publish("copyRemote", |a| copy(a.get(0)))
    }
    pub fn attach() -> Result<(), JsValue> {
        listen(
            &query_node("#btn-remote"),
            "click",
            function(|_| {
                let el = query_node("#remote");
                let on = truthy(&call(&get(&el, "classList"), "toggle", &["open".into()])?);
                for selector in ["#settings", "#servers"] {
                    classes(&query_node(selector), "open", false);
                }
                if on {
                    let _ = load();
                }
                Ok(JsValue::UNDEFINED)
            }),
        );
        for (selector, path, es, en, show) in [
            (
                "#remote-on",
                "/remote-on",
                "Remoto prendido",
                "Remote on",
                false,
            ),
            (
                "#remote-off",
                "/remote-off",
                "Remoto apagado",
                "Remote off",
                false,
            ),
            (
                "#remote-webterm-on",
                "/remote-webterm-on",
                "Terminal web prendida",
                "Web terminal on",
                true,
            ),
            (
                "#remote-webterm-off",
                "/remote-webterm-off",
                "Terminal web apagada",
                "Web terminal off",
                false,
            ),
        ] {
            listen(
                &query_node(selector),
                "click",
                function(move |_| {
                    let after = if show {
                        function(|_| ensure())
                    } else {
                        JsValue::UNDEFINED
                    };
                    action(path.into(), tr(es, en), after)
                }),
            );
        }
        listen(
            &query_node("#remote-open-terminal"),
            "click",
            function(|_| {
                let fetched = ensure();
                Ok(promise(async move {
                    if let Err(e) = wait(fetched).await {
                        error(e)?;
                    }
                    Ok(JsValue::UNDEFINED)
                }))
            }),
        );
        for (button, input) in [
            ("#remote-copy-dashboard", "#remote-dashboard-url"),
            ("#remote-copy-term", "#remote-term-url"),
            ("#remote-copy-fallback", "#remote-fallback-url"),
        ] {
            listen(
                &query_node(button),
                "click",
                function(move |_| copy(input.into())),
            );
        }
        Ok(())
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
#[allow(clippy::unwrap_used)]
mod wasm_tests {
    use super::*;
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen(inline_js = r#"
let remoteContracts;
export async function nativeRemoteReference(root){
  const {createRequire}=await import('node:module');
  remoteContracts=createRequire(root+'/tests/remote_contract.cjs')(root+'/crates/comandos-web/tests/foundation_remote_contract.cjs');
  return JSON.stringify(await remoteContracts.reference(root));
}
export function nativeRemoteFixture(){remoteContracts.install();}
export async function nativeRemoteActual(){return JSON.stringify(await remoteContracts.native());}
"#)]
    extern "C" {
        #[wasm_bindgen(catch)]
        async fn nativeRemoteReference(root: &str) -> Result<JsValue, JsValue>;
        fn nativeRemoteFixture();
        #[wasm_bindgen(catch)]
        async fn nativeRemoteActual() -> Result<JsValue, JsValue>;
    }
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn remote_controls_match_original_coordinator() {
        let expected = nativeRemoteReference(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
            .await
            .unwrap();
        nativeRemoteFixture();
        mount().unwrap();
        attach().unwrap();
        let actual = nativeRemoteActual().await.unwrap();
        assert_eq!(actual.as_string(), expected.as_string());
    }
}
