use super::view::*;
use comandos_web_dom::port::*;
use serde_json::{Value, json};
use std::{cell::Cell, rc::Rc};
use wasm_bindgen::JsValue;
fn g(k: &str) -> JsValue {
    get(&js_sys::global(), k)
}
fn doc() -> JsValue {
    g("document")
}
fn id(k: &str) -> JsValue {
    call(&doc(), "getElementById", &[k.into()]).unwrap_or(JsValue::NULL)
}
fn query(el: &JsValue, k: &str) -> JsValue {
    call(el, "querySelector", &[k.into()]).unwrap_or(JsValue::NULL)
}
fn cls(el: &JsValue, k: &str, on: bool) {
    let _ = call(&get(el, "classList"), "toggle", &[k.into(), on.into()]);
}
fn has(el: &JsValue, k: &str) -> bool {
    truthy(&call(&get(el, "classList"), "contains", &[k.into()]).unwrap_or(false.into()))
}
fn attr(el: &JsValue, k: &str, v: &str) {
    let _ = call(el, "setAttribute", &[k.into(), v.into()]);
}
fn txt(el: &JsValue, v: &str) {
    let _ = set(el, "textContent", &utf16_value(v));
}
fn html(el: &JsValue, v: &str) {
    let _ = set(el, "innerHTML", &utf16_value(v));
}
fn style(el: &JsValue, k: &str, v: &str) {
    let _ = call(&get(el, "style"), "setProperty", &[k.into(), v.into()]);
}
fn event(f: impl Fn(JsValue) -> Result<(), JsValue> + 'static) -> JsValue {
    function(move |a| {
        f(a.get(0))?;
        Ok(JsValue::UNDEFINED)
    })
}
fn listen(el: &JsValue, k: &str, handler: JsValue, capture: bool) {
    let _ = call(el, "addEventListener", &[k.into(), handler, capture.into()]);
}
fn tf(es: &str, en: &str) -> String {
    invoke(&g("tf"), &[utf16_value(es), utf16_value(en)])
        .map(|v| utf16_string(&v))
        .unwrap_or(es.into())
}
fn t(s: &str) -> String {
    invoke(&g("t"), &[utf16_value(s)])
        .map(|v| utf16_string(&v))
        .unwrap_or(s.into())
}
fn toast(s: String, bad: bool) {
    let _ = invoke(&g("toast"), &[utf16_value(&s), bad.into()]);
}
fn err(e: JsValue) -> String {
    let message = get(&e, "message");
    if truthy(&message) {
        utf16_string(&message)
    } else {
        utf16_string(&e)
    }
}
fn fire(f: impl std::future::Future<Output = Result<JsValue, JsValue>> + 'static) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = f.await {
            toast(err(e), true)
        }
    });
}
fn promise(f: impl std::future::Future<Output = Result<JsValue, JsValue>> + 'static) -> JsValue {
    wasm_bindgen_futures::future_to_promise(f).into()
}
async fn api(path: &str, body: Option<Value>) -> Result<JsValue, JsValue> {
    let mut args = vec![utf16_value(path)];
    if let Some(body) = body {
        args.push(from_utf16_json(&body)?);
    }
    wait(invoke(&g("api"), &args)).await
}
fn app() -> bool {
    truthy(&invoke(&g("inApp"), &[]).unwrap_or(false.into()))
}
async fn connect(host: String) -> Result<JsValue, JsValue> {
    let r = api("/ssh-connect", Some(json!({"host":host}))).await?;
    if app() {
        invoke(&g("openInApp"), &[get(&r, "session"), "ssh".into()])?;
    } else {
        api(
            "/focus",
            Some(json!({"session":to_utf16_json(&get(&r,"session"))})),
        )
        .await?;
    }
    toast(
        if truthy(&get(&r, "connected")) {
            tf(
                &format!("Conectado a {host}"),
                &format!("Connected to {host}"),
            )
        } else if truthy(&get(&r, "note")) {
            utf16_string(&get(&r, "note"))
        } else {
            tf(
                &format!("{host} sin conexion todavia"),
                &format!("{host} not connected yet"),
            )
        },
        !truthy(&get(&r, "connected")),
    );
    Ok(JsValue::UNDEFINED)
}
async fn new_tab(host: String) -> Result<JsValue, JsValue> {
    let r = api("/ssh-new-tab", Some(json!({"host":host}))).await?;
    let label = if truthy(&get(&r, "label")) {
        get(&r, "label")
    } else {
        utf16_value(&host)
    };
    if app() {
        invoke(&g("openInApp"), &[get(&r, "session"), "ssh".into(), label])?;
    } else if truthy(&g("WEBTERM")) {
        invoke(&g("openTerm"), &[get(&r, "session"), label])?;
    } else {
        api(
            "/focus",
            Some(json!({"session":to_utf16_json(&get(&r,"session"))})),
        )
        .await?;
    }
    toast(
        if truthy(&get(&r, "connected")) {
            tf(
                &format!("Nueva pestana SSH: {host}"),
                &format!("New SSH tab: {host}"),
            )
        } else if truthy(&get(&r, "note")) {
            utf16_string(&get(&r, "note"))
        } else {
            tf(
                &format!("{host} abierto en nueva pestana"),
                &format!("{host} opened in a new tab"),
            )
        },
        !truthy(&get(&r, "connected")),
    );
    Ok(JsValue::UNDEFINED)
}
async fn setup(host: String) -> Result<JsValue, JsValue> {
    let r = api("/ssh-key-setup", Some(json!({"host":host}))).await?;
    if app() {
        invoke(&g("openInApp"), &[get(&r, "session"), "ssh".into()])?;
    } else if truthy(&g("WEBTERM")) {
        invoke(
            &g("openTerm"),
            &[get(&r, "session"), utf16_value(&format!("setup {host}"))],
        )?;
    } else {
        api(
            "/focus",
            Some(json!({"session":to_utf16_json(&get(&r,"session"))})),
        )
        .await?;
    }
    toast(
        tf(
            &format!("Teclea el password de {host} una sola vez"),
            &format!("Type {host} password once"),
        ),
        false,
    );
    Ok(JsValue::UNDEFINED)
}
fn fill(h: JsValue) -> Result<(), JsValue> {
    let f = id("srv-form");
    for k in ["host", "hostname", "user", "port", "identity"] {
        let v = get(&h, k);
        set(
            &get(&f, k),
            "value",
            &if truthy(&v) { v } else { "".into() },
        )?;
    }
    let host = get(&h, "host");
    set(
        &get(&f, "orig"),
        "value",
        &if truthy(&host) {
            host.clone()
        } else {
            "".into()
        },
    )?;
    set(&get(&f, "remember"), "checked", &(!truthy(&host)).into())?;
    let title = if truthy(&host) {
        let host = utf16_string(&host);
        tf(&format!("Editar {host}"), &format!("Edit {host}"))
    } else {
        t("Agregar servidor")
    };
    txt(&id("srv-form-title"), &title);
    cls(&id("srv-cancel"), "hidden", !truthy(&host));
    Ok(())
}
async fn load() -> Result<JsValue, JsValue> {
    let _ = load_inner().await;
    Ok(JsValue::UNDEFINED)
}
async fn load_inner() -> Result<JsValue, JsValue> {
    let Ok(hosts) = api("/ssh", None).await else {
        return Ok(JsValue::UNDEFINED);
    };
    let hosts = js_sys::Array::from(&hosts);
    let chips = id("ssh-chips");
    html(&chips, "");
    txt(
        &id("ssh-count"),
        &format!("{} ({})", t("Servidores"), hosts.length()),
    );
    let list = id("srv-list");
    html(&list, "");
    for h in hosts.iter() {
        let host = utf16_string(&get(&h, "host"));
        let target = target(&to_utf16_json(&h));
        let chip = call(&doc(), "createElement", &["button".into()])?;
        set(&chip, "className", &"ssh-chip".into())?;
        txt(&chip, &host);
        set(&chip, "title", &utf16_value(&target))?;
        let name = host.clone();
        listen(
            &chip,
            "click",
            event(move |_| {
                fire(new_tab(name.clone()));
                Ok(())
            }),
            false,
        );
        call(&chips, "appendChild", &[chip])?;
        let row = call(&doc(), "createElement", &["div".into()])?;
        set(&row, "className", &"srv-row".into())?;
        html(
            &row,
            "<div class=\"srv-info\"><div class=\"srv-name\"></div><div class=\"srv-tgt\"></div></div><button class=\"con\">Conectar</button><button class=\"key\">Recordar</button><button class=\"ed\">Editar</button><button class=\"del\">Borrar</button>",
        );
        txt(&query(&row, ".srv-name"), &host);
        txt(&query(&row, ".srv-tgt"), &target);
        let name = host.clone();
        listen(
            &query(&row, ".con"),
            "click",
            event(move |_| {
                fire(connect(name.clone()));
                Ok(())
            }),
            false,
        );
        let name = host.clone();
        set(
            &query(&row, ".key"),
            "title",
            &utf16_value(&tf(
                "Instalar llave para entrar sin password",
                "Install key for passwordless login",
            )),
        )?;
        listen(
            &query(&row, ".key"),
            "click",
            event(move |_| {
                fire(setup(name.clone()));
                Ok(())
            }),
            false,
        );
        listen(
            &query(&row, ".ed"),
            "click",
            event(move |_| fill(h.clone())),
            false,
        );
        let del = query(&row, ".del");
        let button = del.clone();
        listen(
            &del,
            "click",
            event(move |_| {
                if !has(&button, "arm") {
                    cls(&button, "arm", true);
                    txt(&button, &t("Seguro?"));
                    let b = button.clone();
                    call(
                        &js_sys::global(),
                        "setTimeout",
                        &[
                            event(move |_| {
                                cls(&b, "arm", false);
                                txt(&b, &t("Borrar"));
                                Ok(())
                            }),
                            3000.into(),
                        ],
                    )?;
                } else {
                    let host = host.clone();
                    fire(async move {
                        api("/ssh-del", Some(json!({"host":host}))).await?;
                        toast(
                            tf(&format!("{host} borrado"), &format!("{host} deleted")),
                            false,
                        );
                        fire(load());
                        Ok(JsValue::UNDEFINED)
                    });
                }
                Ok(())
            }),
            false,
        );
        call(&list, "appendChild", &[row])?;
    }
    if hosts.length() == 0 {
        html(
            &list,
            "<div class=\"desc\">Sin servidores. Agrega el primero abajo.</div>",
        );
    }
    Ok(JsValue::UNDEFINED)
}
fn manager() {
    let modal = id("servers");
    let opening = !has(&modal, "open");
    cls(&modal, "open", opening);
    for other in ["settings", "remote"] {
        cls(&id(other), "open", false)
    }
    if opening {
        let _ = set(&modal, "scrollTop", &0.into());
        let _ = set(&id("srv-list"), "scrollTop", &0.into());
    }
}
fn apply_hidden(hidden: bool) -> Result<(), JsValue> {
    cls(&get(&doc(), "body"), "panel-hidden", hidden);
    attr(
        &id("tab-left"),
        "aria-pressed",
        if hidden { "true" } else { "false" },
    );
    let _ = call(
        &g("localStorage"),
        "setItem",
        &[
            "cc-panel-hidden".into(),
            if hidden { "1".into() } else { "0".into() },
        ],
    );
    let resize = js_sys::Reflect::construct(
        &g("Event").into(),
        &[JsValue::from_str("resize")]
            .into_iter()
            .collect::<js_sys::Array>(),
    )?;
    call(&js_sys::global(), "dispatchEvent", &[resize])?;
    Ok(())
}
fn animation(
    el: &JsValue,
    frames: Value,
    duration: u32,
    easing: Option<&str>,
) -> Result<JsValue, JsValue> {
    let mut options = json!({"duration":duration});
    if let Some(easing) = easing
        && let Some(m) = options.as_object_mut()
    {
        m.insert("easing".into(), json!(easing));
    }
    call(
        el,
        "animate",
        &[from_utf16_json(&frames)?, from_utf16_json(&options)?],
    )
}
fn panel_hidden(hidden: bool, animate: bool) -> Result<(), JsValue> {
    let vp = id("view-panel");
    let sp = id("splitter");
    let old = get(&vp, "_fx");
    if truthy(&old) {
        let _ = call(&old, "finish", &[]);
    }
    let reduced = truthy(&get(
        &invoke(
            &g("matchMedia"),
            &["(prefers-reduced-motion: reduce)".into()],
        )
        .unwrap_or(JsValue::NULL),
        "matches",
    ));
    let plan = panel_plan(
        hidden,
        animate,
        !vp.is_null(),
        get(&vp, "animate").is_function(),
        reduced,
        number(&get(&vp, "offsetWidth")),
    );
    if plan == PanelPlan::Immediate {
        return apply_hidden(hidden);
    }
    let away =
        json!({"transform":"translateX(-100%)","opacity":0.55,"boxShadow":"0 0 0 transparent"});
    let here =
        json!({"transform":"translateX(0)","opacity":1,"boxShadow":"8px 0 18px rgba(0,0,0,.28)"});
    if plan == PanelPlan::Hide {
        let a = animation(
            &vp,
            json!([here, away]),
            220,
            Some("cubic-bezier(.4,0,.2,1)"),
        )?;
        set(&vp, "_fx", &a)?;
        let _ = animation(&sp, json!([{"opacity":1},{"opacity":0}]), 220, None);
        let done = Rc::new(Cell::new(false));
        let end = event(move |_| {
            if done.replace(true) {
                return Ok(());
            }
            set(&vp, "_fx", &JsValue::NULL)?;
            apply_hidden(true)
        });
        set(&a, "onfinish", &end)?;
        set(&a, "oncancel", &end)?;
    } else {
        apply_hidden(false)?;
        let here = json!({"transform":"translateX(0)","opacity":1,"boxShadow":"0 0 0 transparent"});
        let a = animation(
            &vp,
            json!([away, here]),
            240,
            Some("cubic-bezier(.2,0,0,1)"),
        )?;
        set(&vp, "_fx", &a)?;
        let _ = animation(&sp, json!([{"opacity":0},{"opacity":1}]), 240, None);
        let end = event(move |_| set(&vp, "_fx", &JsValue::NULL));
        set(&a, "onfinish", &end)?;
        set(&a, "oncancel", &end)?;
    }
    Ok(())
}
fn bridge(body: Value) -> Result<(), JsValue> {
    call(
        &get(&get(&g("webkit"), "messageHandlers"), "centro"),
        "postMessage",
        &[from_utf16_json(&body).and_then(|v| js_sys::JSON::stringify(&v).map(JsValue::from))?],
    )?;
    Ok(())
}
fn header() -> Result<(), JsValue> {
    for (id_, action) in [
        ("btn-terminal", "quickTerminal"),
        ("btn-newsess", "newSession"),
    ] {
        listen(
            &id(id_),
            "click",
            event(move |e| {
                if app() {
                    call(&e, "stopImmediatePropagation", &[])?;
                    call(&e, "preventDefault", &[])?;
                    bridge(json!({"headerAction":action}))?;
                }
                Ok(())
            }),
            true,
        );
    }
    listen(
        &id("tab-term"),
        "click",
        event(|_| {
            let body = get(&doc(), "body");
            if has(&body, "app")
                && !has(&body, "split")
                && !has(&body, "inapp")
                && g("activeView").as_string().as_deref() != Some("panel")
            {
                invoke(&g("showView"), &["panel".into()])?;
            }
            call(&id("btn-terminal"), "click", &[])?;
            Ok(())
        }),
        false,
    );
    if call(&g("localStorage"), "getItem", &["cc-panel-hidden".into()])
        .ok()
        .and_then(|v| v.as_string())
        .as_deref()
        == Some("1")
        && !app()
    {
        panel_hidden(true, false)?;
    }
    listen(
        &id("tab-left"),
        "click",
        event(|_| {
            if app() {
                bridge(json!({"leftPanel":"toggle"}))
            } else {
                panel_hidden(!has(&get(&doc(), "body"), "panel-hidden"), true)
            }
        }),
        false,
    );
    listen(
        &id("tab-new"),
        "click",
        event(|_| {
            call(&id("btn-newsess"), "click", &[])?;
            Ok(())
        }),
        false,
    );
    listen(
        &id("tab-rows"),
        "click",
        event(|_| {
            let rows = !has(&get(&doc(), "body"), "tabs-rows");
            invoke(
                &g("applyTabsLayout"),
                &[if rows { "rows".into() } else { "row".into() }],
            )?;
            fire(async move {
                if let Err(e) = api(
                    "/prefs-set",
                    Some(json!({"tabs_layout":if rows{"rows"}else{"row"}})),
                )
                .await
                {
                    invoke(
                        &g("applyTabsLayout"),
                        &[if rows { "row".into() } else { "rows".into() }],
                    )?;
                    toast(err(e), true);
                }
                Ok(JsValue::UNDEFINED)
            });
            Ok(())
        }),
        false,
    );
    let new = id("btn-newsess");
    set(
        &new,
        "title",
        &utf16_value(&tf(
            "Nueva sesión: carpeta, agente y cuenta",
            "New session: folder, agent and account",
        )),
    )?;
    attr(&new, "aria-label", &tf("Nueva sesión", "New session"));
    listen(
        &new,
        "click",
        event(|_| {
            if has(&get(&doc(), "body"), "app")
                && g("activeView").as_string().as_deref() != Some("panel")
            {
                invoke(&g("showView"), &["panel".into()])?;
            }
            set(&g("NS"), "intoPane", &JsValue::NULL)?;
            let _ = invoke(&g("nsOpen"), &[]);
            Ok(())
        }),
        false,
    );
    let term = id("btn-terminal");
    set(
        &term,
        "title",
        &utf16_value(&tf(
            "Terminal en carpeta nueva fechada (se abre también en el escritorio)",
            "Terminal in a new dated folder (also opens on desktop)",
        )),
    )?;
    let button = term.clone();
    listen(
        &term,
        "click",
        event(move |_| {
            let quick = invoke(&g("quickTerminalInstance"), &[])?;
            if !truthy(&quick) {
                toast(
                    tf(
                        "Terminal rápida no disponible",
                        "Quick terminal unavailable",
                    ),
                    true,
                );
                return Ok(());
            }
            attr(&button, "aria-busy", "true");
            set(&button, "disabled", &true.into())?;
            let b = button.clone();
            let opening = call(&quick, "open", &[]);
            wasm_bindgen_futures::spawn_local(async move {
                let _ = wait(opening).await;
                let _ = set(&b, "disabled", &false.into());
                let _ = call(&b, "removeAttribute", &["aria-busy".into()]);
            });
            Ok(())
        }),
        false,
    );
    set(&id("btn-menu"), "title", &utf16_value(&tf("Más", "More")))?;
    attr(&id("btn-menu"), "aria-label", &tf("Más", "More"));
    set(
        &id("btn-servers"),
        "title",
        &utf16_value(&tf("Servidores", "Servers")),
    )?;
    Ok(())
}
fn set_menu(open: bool) -> Result<(), JsValue> {
    let menu = id("menu-panel");
    let button = id("btn-menu");
    cls(&menu, "hidden", !open);
    attr(
        &button,
        "aria-expanded",
        if open { "true" } else { "false" },
    );
    if open {
        let r = call(&button, "getBoundingClientRect", &[])?;
        style(
            &menu,
            "top",
            &format!(
                "{}px",
                if number(&get(&r, "height")) != 0.0 {
                    number(&get(&r, "bottom")) + 8.0
                } else {
                    8.0
                }
            ),
        );
        style(
            &menu,
            "left",
            &format!(
                "{}px",
                menu_left(
                    number(&get(&r, "left")),
                    number(&g("innerWidth")),
                    number(&get(&menu, "offsetWidth"))
                )
            ),
        );
    }
    Ok(())
}
fn popovers() -> Result<(), JsValue> {
    let menu = id("menu-panel");
    let button = id("btn-menu");
    if get(&menu, "parentNode") != get(&doc(), "body") {
        call(
            &get(&doc(), "body"),
            "appendChild",
            std::slice::from_ref(&menu),
        )?;
    }
    listen(
        &button,
        "click",
        event(|_| set_menu(has(&id("menu-panel"), "hidden"))),
        false,
    );
    listen(
        &menu,
        "click",
        event(|e| {
            if truthy(&call(&get(&e,"target"),"closest",&["#toggle-overview,#open-session-profiles,#open-extension-usage,#btn-sov,#btn-theme,#limits-strip .pill".into()]).unwrap_or(JsValue::NULL)){set_menu(false)?;}
            Ok(())
        }),
        false,
    );
    let m = menu.clone();
    let b = button.clone();
    listen(
        &doc(),
        "click",
        event(move |e| {
            if has(&m, "hidden") {
                return Ok(());
            }
            let path = call(&e, "composedPath", &[]).unwrap_or(js_sys::Array::new().into());
            if js_sys::Array::from(&path).iter().any(|v| v == m || v == b) {
                return Ok(());
            }
            let target = get(&e, "target");
            if truthy(&call(&m, "contains", std::slice::from_ref(&target)).unwrap_or(false.into()))
                || truthy(
                    &call(&b, "contains", std::slice::from_ref(&target)).unwrap_or(false.into()),
                )
            {
                return Ok(());
            }
            set_menu(false)
        }),
        false,
    );
    listen(
        &doc(),
        "keydown",
        event(|e| {
            if get(&e, "key").as_string().as_deref() == Some("Escape")
                && !has(&id("menu-panel"), "hidden")
            {
                set_menu(false)?;
                call(&id("btn-menu"), "focus", &[])?;
            }
            Ok(())
        }),
        false,
    );
    listen(
        &id("btn-servers"),
        "click",
        event(|_| {
            let open = has(&id("servers-panel"), "hidden");
            cls(&id("servers-panel"), "hidden", !open);
            attr(
                &id("btn-servers"),
                "aria-expanded",
                if open { "true" } else { "false" },
            );
            if open {
                cls(&id("ssh-bar"), "open", true);
                attr(&id("ssh-toggle"), "aria-expanded", "true");
                fire(load());
            }
            Ok(())
        }),
        false,
    );
    Ok(())
}
async fn submit(form: JsValue) -> Result<JsValue, JsValue> {
    let remember = truthy(&get(&get(&form, "remember"), "checked"));
    let mut fields = json!({});
    for k in ["host", "hostname", "user", "port", "identity"] {
        if let Some(m) = fields.as_object_mut() {
            m.insert(k.into(), to_utf16_json(&get(&get(&form, k), "value")));
        }
    }
    let mut payload = form_payload(&fields);
    let host = payload
        .get("host")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let orig = get(&get(&form, "orig"), "value");
    let update = truthy(&orig);
    if update && let Some(m) = payload.as_object_mut() {
        m.insert("orig".into(), to_utf16_json(&orig));
    }
    api(
        if update { "/ssh-update" } else { "/ssh-add" },
        Some(payload),
    )
    .await?;
    toast(
        if update {
            tf(&format!("{host} actualizado"), &format!("{host} updated"))
        } else {
            tf(&format!("{host} guardado"), &format!("{host} saved"))
        },
        false,
    );
    call(&form, "reset", &[])?;
    fill(object())?;
    fire(load());
    if remember {
        setup(host).await?;
    }
    Ok(JsValue::UNDEFINED)
}
pub fn mount() -> Result<(), JsValue> {
    let scope: JsValue = js_sys::global().into();
    method(&scope, "connectHost", |a| {
        Ok(promise(connect(utf16_string(&a.get(0)))))
    })?;
    method(&scope, "openSshTab", |a| {
        Ok(promise(new_tab(utf16_string(&a.get(0)))))
    })?;
    method(&scope, "setupSshKey", |a| {
        Ok(promise(setup(utf16_string(&a.get(0)))))
    })?;
    method(&scope, "fillSrvForm", |a| {
        fill(a.get(0))?;
        Ok(JsValue::UNDEFINED)
    })?;
    method(&scope, "loadSsh", |_| Ok(promise(load())))?;
    method(&scope, "toggleSshManager", |_| {
        manager();
        Ok(JsValue::UNDEFINED)
    })?;
    method(&scope, "initHeaderActions", |_| {
        header()?;
        Ok(JsValue::UNDEFINED)
    })?;
    method(&scope, "initHeaderPopovers", |_| {
        popovers()?;
        Ok(JsValue::UNDEFINED)
    })?;
    Ok(())
}
pub fn attach() -> Result<(), JsValue> {
    let bar = id("ssh-bar");
    let toggle = id("ssh-toggle");
    if !bar.is_null() && !toggle.is_null() {
        let open = call(&g("localStorage"), "getItem", &["cc-ssh-open".into()])
            .ok()
            .and_then(|v| v.as_string())
            .as_deref()
            == Some("1");
        cls(&bar, "open", open);
        attr(
            &toggle,
            "aria-expanded",
            if open { "true" } else { "false" },
        );
        let toggle_handler = toggle.clone();
        listen(
            &toggle,
            "click",
            event(move |_| {
                let open = !has(&bar, "open");
                call(
                    &g("localStorage"),
                    "setItem",
                    &[
                        "cc-ssh-open".into(),
                        if open { "1".into() } else { "0".into() },
                    ],
                )?;
                cls(&bar, "open", open);
                attr(
                    &toggle_handler,
                    "aria-expanded",
                    if open { "true" } else { "false" },
                );
                Ok(())
            }),
            false,
        );
    }
    listen(
        &id("ssh-manage"),
        "click",
        event(|_| {
            manager();
            Ok(())
        }),
        false,
    );
    header()?;
    popovers()?;
    listen(
        &id("srv-cancel"),
        "click",
        event(|_| {
            call(&id("srv-form"), "reset", &[])?;
            fill(object())
        }),
        false,
    );
    listen(
        &id("srv-form"),
        "submit",
        event(|e| {
            call(&e, "preventDefault", &[])?;
            fire(submit(get(&e, "target")));
            Ok(())
        }),
        false,
    );
    Ok(())
}
