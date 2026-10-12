use super::*;
use wasm_bindgen::prelude::wasm_bindgen;
#[path = "foundation_helpers_model_controls.rs"]
mod models;
#[wasm_bindgen(
    inline_js = "export function helpersTerminal(){return {activeView:typeof activeView==='undefined'?undefined:activeView};}"
)]
extern "C" {
    fn helpersTerminal() -> JsValue;
}
fn ns() -> JsValue {
    global("NS")
}
fn ns_s(k: &str) -> String {
    get(&ns(), k).as_string().unwrap_or_default()
}
fn put(k: &str, v: JsValue) {
    let _ = set(&ns(), k, &v);
}
fn harness(id: &str) -> JsValue {
    get(&get(&global("PROVIDERS"), "harnesses"), id)
}
fn motor(id: &str) -> JsValue {
    get(&get(&global("PROVIDERS"), "motors"), id)
}
fn matrix() -> Vec<JsValue> {
    js_rows(&get(&global("PROVIDERS"), "matrix"))
}
fn route(h: Option<String>, m: Option<String>) -> JsValue {
    let h = h.unwrap_or_else(|| ns_s("harness"));
    let m = m.unwrap_or_else(|| ns_s("motor"));
    matrix()
        .into_iter()
        .find(|r| {
            get(r, "harness").as_string().as_deref() == Some(&h)
                && get(r, "motor").as_string().as_deref() == Some(&m)
        })
        .unwrap_or(JsValue::UNDEFINED)
}
fn model_list() -> Vec<JsValue> {
    js_rows(&get(&motor(&ns_s("motor")), "models"))
}
fn model() -> JsValue {
    let list = model_list();
    list.iter()
        .find(|m| get(m, "id").as_string().as_deref() == Some(&ns_s("model")))
        .cloned()
        .or_else(|| list.first().cloned())
        .unwrap_or_else(object)
}
fn account_items(provider: &str, role: &str) -> Vec<Value> {
    let acp =
        role == "motor" && get(&route(None, None), "driver").as_string().as_deref() == Some("acp");
    js_rows(&get(&harness(provider),"accounts")).into_iter().map(|a|{let enabled=truthy(&get(&a,if role=="motor"&&!acp{"motorSelectable"}else{"selectable"}));let identity=get(&a,"identity").as_string().unwrap_or_default();let title=if !identity.is_empty(){identity}else if !enabled&&truthy(&get(&a,"selectable")){tf("cuenta de motor nombrada: gateway aislado aún no disponible","named motor account: isolated gateway not available yet")}else{let state=get(&a,"state").as_string().unwrap_or_default();match state.as_str(){"ready"=>"suscripción lista","login_required"=>"requiere login","unsupported_auth"=>"auth no compatible con suscripción",_=>&state}.into()};json!({"id":get(&a,"alias").as_string().unwrap_or_default(),"label":get(&a,"alias").as_string().unwrap_or_default(),"title":title,"disabled":!enabled,"icon":provider})}).collect()
}
fn ensure_account(provider: &str, field: &str, role: &str) {
    if truthy(&get(&ns(), "profileId")) {
        return;
    }
    let ready: Vec<_> = account_items(provider, role)
        .into_iter()
        .filter(|a| !truth(at(a, "disabled")))
        .collect();
    if !ready.iter().any(|a| s(at(a, "id")) == ns_s(field)) {
        put(
            field,
            ready
                .first()
                .map(|a| s(at(a, "id")))
                .filter(|s| !s.is_empty())
                .unwrap_or("main".into())
                .into(),
        );
    }
}
fn chips(el: JsValue, items: Vec<Value>, cur: String, cb: JsValue) {
    let rendered = items
        .iter()
        .map(|x| {
            format!(
                "<button type=\"button\" class=\"{}\" data-id=\"{}\" {} {}>{}{}</button>",
                if s(at(x, "id")) == cur { "on" } else { "" },
                attr_esc(&s(at(x, "id"))),
                if truth(at(x, "disabled")) {
                    "disabled aria-disabled=\"true\""
                } else {
                    ""
                },
                if truth(at(x, "title")) {
                    format!("title=\"{}\"", attr_esc(&s(at(x, "title"))))
                } else {
                    String::new()
                },
                if truth(at(x, "icon")) {
                    provider_mark(&s(at(x, "icon")), &s(at(x, "model")), 13.0)
                } else {
                    String::new()
                },
                md_esc(&s(at(x, "label")))
            )
        })
        .collect();
    html(&el, rendered);
    for button in all(&el, "button:not(:disabled)") {
        let root = el.clone();
        let cb = cb.clone();
        let b = button.clone();
        listen(
            &button,
            "click",
            event(move |_| {
                for o in all(&root, "button") {
                    classes(&o, "on", false)
                }
                classes(&b, "on", true);
                invoke(&cb, &[utf16_value(&data(&b, "id"))])?;
                Ok(())
            }),
        );
    }
}
fn select_defaults() {
    let r = route(None, None);
    put(
        "routeId",
        if truthy(&get(&r, "selectable")) {
            get(&r, "id")
        } else {
            "".into()
        },
    );
    let list = model_list();
    if (!truthy(&get(&ns(), "profileId")) || ns_s("model").is_empty())
        && !list
            .iter()
            .any(|m| get(m, "id").as_string().as_deref() == Some(&ns_s("model")))
    {
        put(
            "model",
            list.first().map(|m| get(m, "id")).unwrap_or("".into()),
        );
    }
    let m = model();
    let allowed = js_rows(&get(&m, "efforts"));
    if (!truthy(&get(&ns(), "profileId")) || ns_s("effort").is_empty())
        && !allowed
            .iter()
            .any(|e| e.as_string().as_deref() == Some(&ns_s("effort")))
    {
        let default = get(&m, "defaultEffort");
        put(
            "effort",
            if truthy(&default) {
                default
            } else {
                allowed.first().cloned().unwrap_or("".into())
            },
        );
    }
}
fn selectable(h: &str) -> Vec<JsValue> {
    matrix()
        .into_iter()
        .filter(|r| {
            get(r, "harness").as_string().as_deref() == Some(h)
                && (truthy(&get(r, "selectable"))
                    || get(&get(r, "reason"), "code").as_string().as_deref() != Some("not_routed"))
        })
        .collect()
}
fn callback(f: impl Fn(String) + 'static) -> JsValue {
    function(move |a| {
        f(a.get(0).as_string().unwrap_or_default());
        Ok(JsValue::UNDEFINED)
    })
}
fn render() {
    let shell = ns_s("harness") == "shell";
    let source = get(&global("PROVIDERS"), "matrixHarnesses");
    let ids = if truthy(&source) {
        js_rows(&source)
    } else {
        vec!["claude".into(), "codex".into(), "grok".into()]
    };
    let mut choices:Vec<_>=ids.iter().map(|id|{let id=id.as_string().unwrap_or_default();let h=harness(&id);json!({"id":id,"label":get(&h,"label").as_string().unwrap_or(id.clone()),"icon":to_utf16_json(&get(&h,"icon"))})}).collect();
    choices.push(json!({"id":"shell","label":"Shell","icon":"terminal"}));
    chips(
        id("ns-harness"),
        choices,
        ns_s("harness"),
        callback(|h| {
            put("harness", h.clone().into());
            put("motor", if h == "shell" { "".into() } else { h.into() });
            put("harnessAccount", "main".into());
            put("motorAccount", "main".into());
            select_defaults();
            render();
        }),
    );
    for name in ["ns-motor-wrap", "ns-model-wrap", "ns-effort-wrap"] {
        classes(&id(name), "hidden", shell)
    }
    if !shell {
        let choices=selectable(&ns_s("harness")).iter().map(|r|{let id=get(r,"motor").as_string().unwrap_or_default();let m=motor(&id);let enabled=truthy(&get(r,"selectable"));let reason=get(&get(r,"reason"),"message");json!({"id":id,"label":get(&m,"label").as_string().unwrap_or(id.clone()),"icon":to_utf16_json(&get(&m,"icon")),"disabled":!enabled,"title":if enabled{String::new()}else{if truthy(&reason){utf16_string(&reason)}else{get(r,"productState").as_string().unwrap_or_default()}}})}).collect();
        chips(
            id("ns-motor"),
            choices,
            ns_s("motor"),
            callback(|id| {
                put("motor", id.into());
                put("motorAccount", "main".into());
                select_defaults();
                render();
            }),
        );
        chips(id("ns-model"),model_list().iter().map(|m|{let id=get(m,"id").as_string().unwrap_or_default();json!({"id":id,"label":get(m,"name").as_string().unwrap_or(id.clone()),"model":id,"icon":ns_s("motor")})}).collect(),ns_s("model"),callback(|id|{put("model",id.into());select_defaults();render();}));
        chips(id("ns-effort"),js_rows(&get(&model(),"efforts")).iter().map(|e|json!({"id":e.as_string().unwrap_or_default(),"label":e.as_string().unwrap_or_default()})).collect(),ns_s("effort"),callback(|id|put("effort",id.into())));
    }
    let caps = get(&harness(&ns_s("harness")), "capabilities");
    let r = route(None, None);
    let accounts = truthy(&get(&caps, "accounts"));
    classes(&id("ns-acct-wrap"), "hidden", shell || !accounts);
    if !shell && accounts {
        ensure_account(&ns_s("harness"), "harnessAccount", "harness");
        chips(
            id("ns-acct"),
            account_items(&ns_s("harness"), "harness"),
            ns_s("harnessAccount"),
            callback(|id| {
                put("harnessAccount", id.clone().into());
                if ns_s("motor") == ns_s("harness") {
                    put("motorAccount", id.into());
                }
                render();
            }),
        );
    }
    let separate = !shell
        && truthy(&r)
        && js_rows(&get(&r, "accountScopes"))
            .iter()
            .any(|s| s.as_string().as_deref() == Some("motor"))
        && ns_s("motor") != ns_s("harness");
    classes(&id("ns-motor-acct-wrap"), "hidden", !separate);
    if separate {
        ensure_account(&ns_s("motor"), "motorAccount", "motor");
        chips(
            id("ns-motor-acct"),
            account_items(&ns_s("motor"), "motor"),
            ns_s("motorAccount"),
            callback(|id| put("motorAccount", id.into())),
        );
        txt(
            &id("ns-motor-acct-note"),
            tf(
                "Esta suscripción paga la inferencia; la cuenta del harness conserva transcript y herramientas.",
                "This subscription runs inference; the harness account keeps transcript and tools.",
            ),
        );
    } else if !shell {
        put("motorAccount", get(&ns(), "harnessAccount"));
    }
    let all = matrix();
    let unavailable: Vec<_> = all
        .iter()
        .filter(|r| {
            !truthy(&get(r, "selectable"))
                && get(&get(r, "reason"), "code").as_string().as_deref() != Some("not_routed")
        })
        .collect();
    classes(
        &id("ns-unavailable"),
        "hidden",
        shell || unavailable.is_empty(),
    );
    txt(
        &query(&id("ns-unavailable"), "summary"),
        format!(
            "{} combinaciones listas · {} bloqueadas (login / gateway)",
            all.iter().filter(|r| truthy(&get(r, "selectable"))).count(),
            unavailable.len()
        ),
    );
    html(
        &id("ns-matrix"),
        unavailable
            .into_iter()
            .map(|r| {
                let h = get(r, "harness").as_string().unwrap_or_default();
                let m = get(r, "motor").as_string().unwrap_or_default();
                format!(
                    "<div><b>{} → {}</b><small>{}</small></div>",
                    md_esc(&get(&harness(&h), "label").as_string().unwrap_or(h)),
                    md_esc(&get(&motor(&m), "label").as_string().unwrap_or(m)),
                    md_esc(
                        &get(&get(r, "reason"), "message")
                            .as_string()
                            .or_else(|| get(r, "productState").as_string())
                            .unwrap_or_default()
                    )
                )
            })
            .collect(),
    );
    let route_id = if truthy(&get(&r, "selectable")) {
        get(&r, "id")
    } else {
        "".into()
    };
    put("routeId", route_id.clone());
    let profile_error = if truthy(&get(&ns(), "profileId")) && !shell {
        let draft = object();
        let _ = call(&global("Object"), "assign", &[draft.clone(), ns()]);
        let _ = set(&draft, "toHarness", &get(&ns(), "harness"));
        call(
            &global("SessionConfig"),
            "validate",
            &[global("PROVIDERS"), draft],
        )
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
    } else {
        String::new()
    };
    let disabled = !profile_error.is_empty() || (!shell && !truthy(&route_id));
    let go = id("ns-go");
    let _ = set(&go, "disabled", &disabled.into());
    let _ = set(
        &go,
        "title",
        &utf16_value(&if disabled {
            tf(
                "Esta combinación no está disponible — elige un motor habilitado",
                "This combination is unavailable — pick an enabled engine",
            )
        } else {
            tf(
                "Crear la sesión con este setup",
                "Create the session with this setup",
            )
        }),
    );
    txt(
        &id("ns-note"),
        if !profile_error.is_empty() {
            profile_error
        } else if shell {
            String::new()
        } else if truthy(&route_id) {
            format!(
                "{} → {} · {}{}",
                get(&harness(&ns_s("harness")), "label")
                    .as_string()
                    .unwrap_or_default(),
                get(&motor(&ns_s("motor")), "label")
                    .as_string()
                    .unwrap_or_default(),
                ns_s("model"),
                if ns_s("effort").is_empty() {
                    String::new()
                } else {
                    format!(" · {}", ns_s("effort"))
                }
            )
        } else {
            tf("combinación no disponible", "combination unavailable")
        },
    );
}
async fn open() -> Result<JsValue, JsValue> {
    let body = get(&doc(), "body");
    if has_class(&body, "app") && !has_class(&body, "split") && !has_class(&body, "inapp") {
        let active = get(&helpersTerminal(), "activeView");
        if !active.is_undefined() && active.as_string().as_deref() != Some("panel") {
            call_global("showView", &["panel".into()]);
        }
    }
    classes(&id("newsess"), "hidden", false);
    if !truthy(&global("PROVIDERS"))
        && let Ok(p) = api("/providers", None).await
    {
        let _ = global_set("PROVIDERS", &p);
    }
    let mut dirs: Vec<_> = js_rows(&get(&state(), "list"))
        .into_iter()
        .filter_map(|it| get(&it, "cwd").as_string())
        .filter(|s| !s.is_empty())
        .collect();
    dirs.sort();
    dirs.dedup();
    html(
        &id("ns-dirs"),
        dirs.iter()
            .map(|d| format!("<option value=\"{}\">", attr_esc(d)))
            .collect(),
    );
    let selected = pick(get(&state(), "list"));
    let cwd = get(&selected, "cwd");
    if truthy(&cwd) && !truthy(&get(&id("ns-cwd"), "value")) {
        let _ = set(&id("ns-cwd"), "value", &cwd);
    }
    if let Ok(options) = api("/optimization/plans", None).await {
        let active = get(&options, "active");
        if let Some(plan) = js_rows(&get(&options, "plans"))
            .iter()
            .find(|p| get(p, "id") == active)
        {
            let variant = get(&get(plan, "variants"), &ns_s("harness"));
            if truthy(&get(&variant, "selectable")) {
                let route = get(&variant, "routeId").as_string().unwrap_or_default();
                if let Some(m) = route.split(':').nth(1).filter(|s| !s.is_empty()) {
                    put("motor", m.into());
                }
                put("model", get(&variant, "model"));
                put("effort", get(&variant, "effort"));
            }
        }
    }
    select_defaults();
    render();
    let _ = call(&id("ns-cwd"), "focus", &[]);
    let path = get(&id("ns-cwd"), "value").as_string().unwrap_or_default();
    let path = if path.trim().is_empty() {
        "~/codebase".into()
    } else {
        path.trim().into()
    };
    wasm_bindgen_futures::spawn_local(async move {
        let _ = browse(path).await;
    });
    Ok(JsValue::UNDEFINED)
}
fn short_home(path: &str) -> String {
    let regex = global("FS_HOME_RE");
    call(
        &get(&get(&global("String"), "prototype"), "replace"),
        "call",
        &[utf16_value(path), regex, "~".into()],
    )
    .ok()
    .map(|v| utf16_string(&v))
    .unwrap_or(path.into())
}
async fn browse(path: String) -> Result<JsValue, JsValue> {
    let seq = number(&global("nsFsSeq")) + 1.0;
    global_set("nsFsSeq", &seq.into())?;
    let encoded = call_global(
        "encodeURIComponent",
        &[utf16_value(if path.is_empty() { "~" } else { &path })],
    );
    let Ok(d) = api(&format!("/fs/dirs?path={}", string(&encoded)), None).await else {
        return Ok(JsValue::UNDEFINED);
    };
    if number(&global("nsFsSeq")) != seq || truthy(&get(&d, "error")) {
        return Ok(JsValue::UNDEFINED);
    }
    if let Some(home) = get(&d, "home").as_string().filter(|s| !s.is_empty()) {
        let escaped = home
            .chars()
            .map(|c| {
                if ".*+?^${}()|[]\\".contains(c) {
                    format!("\\{c}")
                } else {
                    c.to_string()
                }
            })
            .collect::<String>();
        global_set(
            "FS_HOME_RE",
            &js_sys::RegExp::new(&format!("^{escaped}"), "").into(),
        )?;
    }
    let mut acc = String::new();
    let crumbs = get(&d, "base")
        .as_string()
        .unwrap_or_default()
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|p| {
            acc.push('/');
            acc += p;
            format!(
                "<button type=\"button\" class=\"ns-crumb\" data-p=\"{}\">{}</button>",
                attr_esc(&acc),
                md_esc(p)
            )
        })
        .collect::<Vec<_>>();
    html(
        &id("ns-crumbs"),
        crumbs.join("<span class=\"ns-crumb-sep\">/</span>"),
    );
    let dirs = js_rows(&get(&d, "dirs"));
    html(
        &id("ns-dirlist"),
        if dirs.is_empty() {
            format!(
                "<span class=\"ns-fs-empty\">{}</span>",
                tf("sin subcarpetas", "no subfolders")
            )
        } else {
            dirs.iter()
                .map(|x| {
                    format!(
                        "<button type=\"button\" class=\"ns-dir\" data-p=\"{}\">{} {}</button>",
                        attr_esc(&get(x, "path").as_string().unwrap_or_default()),
                        icon("layers", 11.0),
                        md_esc(&get(x, "name").as_string().unwrap_or_default())
                    )
                })
                .collect()
        },
    );
    let path = get(&d, "path").as_string().unwrap_or_default();
    if truthy(&get(&d, "exists")) {
        html(
            &id("ns-fs-state"),
            format!(
                "<span class=\"ns-ok\">{} {}</span>",
                icon("check", 11.0),
                tf("carpeta existente", "folder exists")
            ),
        );
    } else {
        html(
            &id("ns-fs-state"),
            format!(
                "<span class=\"ns-new\">{} {} {}</span><button type=\"button\" class=\"test\" id=\"ns-mkdir\" data-p=\"{}\" title=\"{}\">{} {}</button>",
                icon("sparkles", 11.0),
                md_esc(&short_home(&path)),
                tf("no existe", "doesn't exist"),
                attr_esc(&path),
                attr_esc(&tf(
                    "Crear la carpeta (y las intermedias) dentro de tu HOME",
                    "Create the folder (and parents) inside your HOME"
                )),
                icon("plus", 11.0),
                tf("Crear carpeta", "Create folder")
            ),
        );
        listen(
            &id("ns-mkdir"),
            "click",
            event(|_| {
                let path = data(&id("ns-mkdir"), "p");
                wasm_bindgen_futures::spawn_local(async move {
                    match api("/fs/mkdir", Some(json!({"path":path}))).await {
                        Ok(r) => {
                            let path = get(&r, "path").as_string().unwrap_or_default();
                            toast(
                                tf(
                                    &format!("Carpeta creada: {}", short_home(&path)),
                                    &format!("Folder created: {}", short_home(&path)),
                                ),
                                false,
                            );
                            let _ = set(&id("ns-cwd"), "value", &utf16_value(&path));
                            let _ = browse(path).await;
                        }
                        Err(e) => toast(err(e), true),
                    }
                });
                Ok(())
            }),
        );
    }
    for b in all(&doc(), "#ns-crumbs .ns-crumb, #ns-dirlist .ns-dir") {
        let button = b.clone();
        listen(
            &b,
            "click",
            event(move |_| {
                let path = data(&button, "p");
                let _ = set(&id("ns-cwd"), "value", &utf16_value(&path));
                wasm_bindgen_futures::spawn_local(async move {
                    let _ = browse(path).await;
                });
                Ok(())
            }),
        );
    }
    Ok(JsValue::UNDEFINED)
}
async fn go() -> Result<JsValue, JsValue> {
    let cwd = get(&id("ns-cwd"), "value")
        .as_string()
        .unwrap_or_default()
        .trim()
        .to_string();
    if cwd.is_empty() {
        toast(tf("Falta la carpeta", "Missing folder"), true);
        return Ok(JsValue::UNDEFINED);
    }
    let result=async{let into=global("NS");let into=get(&into,"intoPane");let danger=has_class(&id("ns-danger"),"on");if truthy(&into)&&ns_s("harness")!="shell"{let session=get(&into,"session").as_string().unwrap_or_default();let pane=get(&into,"pane").as_string().unwrap_or_default();let r=api("/session/configure",Some(json!({"session":session,"pane":pane,"toHarness":ns_s("harness"),"motor":ns_s("motor"),"model":ns_s("model"),"effort":ns_s("effort"),"account":ns_s("harnessAccount"),"harnessAccount":ns_s("harnessAccount"),"motorAccount":ns_s("motorAccount"),"danger":danger}))).await?;let key=get(&r,"operationKey").as_string().filter(|s|!s.is_empty()).unwrap_or(format!("{session}|{pane}"));let pending=json!({"motor":if ns_s("motor").is_empty(){ns_s("harness")}else{ns_s("motor")},"model":ns_s("model"),"effort":ns_s("effort"),"since":now(),"queued":truthy(&get(&r,"queued")),"stageTxt":tf("arrancando CLI en esta terminal…","starting CLI in this terminal…")});call(&global("MOTOR_PENDING"),"set",&[utf16_value(&key),from_utf16_json(&pending)?])?;put("intoPane",JsValue::NULL);classes(&id("newsess"),"hidden",true);toast(tf(&format!("Arrancando {} en esta terminal",ns_s("harness")),&format!("Starting {} in this terminal",ns_s("harness"))),false);call_global("tick",&[]);return Ok(JsValue::UNDEFINED)}let mut payload=if ns_s("harness")=="shell"{json!({"cwd":cwd,"agent":"shell"})}else{json!({"cwd":cwd,"routeId":ns_s("routeId"),"model":ns_s("model"),"effort":ns_s("effort"),"harnessAccount":ns_s("harnessAccount"),"motorAccount":ns_s("motorAccount")})};if let Some(m)=payload.as_object_mut(){if truthy(&get(&ns(),"profileId")){m.insert("profileId".into(),to_utf16_json(&get(&ns(),"profileId")));}m.insert("danger".into(),json!(danger));}let r=api("/session-new",Some(payload)).await?;put("intoPane",JsValue::NULL);classes(&id("newsess"),"hidden",true);if truthy(&call_global("inApp",&[])){call_global("openInApp",&[get(&r,"session"),"claude".into(),get(&r,"label")]);}else if truthy(&global("WEBTERM")){call_global("openTerm",&[get(&r,"session"),get(&r,"label")]);}else{let _=api("/focus",Some(json!({"session":to_utf16_json(&get(&r,"session"))}))).await;}toast(format!("{} {}{}",get(&r,"label").as_string().unwrap_or_default(),tf("creada","created"),if truthy(&get(&r,"worktree")){" · ⎇ worktree"}else{""}),false);call_global("tick",&[]);Ok(JsValue::UNDEFINED)}.await;
    match result {
        Ok(v) => Ok(v),
        Err(e) => {
            let message = err(e);
            if message.contains("no existe") {
                let _ = browse(cwd).await;
                toast(
                    tf(
                        "Esa carpeta no existe — usa el botón Crear carpeta de abajo.",
                        "That folder doesn't exist — use the Create folder button below.",
                    ),
                    true,
                );
            } else {
                toast(message, true);
            }
            Ok(JsValue::UNDEFINED)
        }
    }
}
async fn open_for(session: String, pane: String, cwd: String) -> Result<JsValue, JsValue> {
    let item = js_rows(&get(&state(), "list")).into_iter().find(|it| {
        truthy(&get(it, "alive"))
            && get(it, "operable") != JsValue::FALSE
            && get(it, "session").as_string().as_deref() == Some(&session)
            && (pane.is_empty() || get(it, "pane").as_string().as_deref() == Some(&pane))
    });
    let Some(it) = item else {
        toast(
            tf(
                "El panel ya no está disponible; actualiza la sesión",
                "Pane no longer available; refresh the session",
            ),
            true,
        );
        return Ok(JsValue::UNDEFINED);
    };
    put(
        "intoPane",
        from_json(
            &json!({"session":session,"pane":get(&it,"pane").as_string().unwrap_or_default()}),
        )?,
    );
    call_global(
        "ulog",
        &["click".into(), "open-ai-here".into(), utf16_value(&session)],
    );
    open().await?;
    let input = id("ns-cwd");
    if !input.is_null() {
        let path = if !cwd.is_empty() {
            cwd
        } else {
            get(&it, "cwd")
                .as_string()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| get(&input, "value").as_string().unwrap_or_default())
        };
        set(&input, "value", &utf16_value(&path))?;
    }
    toast(
        tf(
            "Elige CLI y motor: arranca en ESTA terminal (como el +).",
            "Pick CLI and engine: starts in THIS terminal (same as +).",
        ),
        false,
    );
    Ok(JsValue::UNDEFINED)
}
pub fn mount() -> Result<(), JsValue> {
    let scope: JsValue = js_sys::global().into();
    method(&scope, "nsMatrix", |_| {
        Ok(matrix().into_iter().collect::<js_sys::Array>().into())
    })?;
    for (name, is_harness) in [("nsHarness", true), ("nsMotor", false)] {
        method(&scope, name, move |a| {
            let id = a.get(0).as_string().unwrap_or_default();
            let out = if is_harness { harness(&id) } else { motor(&id) };
            Ok(if truthy(&out) { out } else { object() })
        })?;
    }
    method(&scope, "nsRoute", |a| {
        Ok(route(a.get(0).as_string(), a.get(1).as_string()))
    })?;
    method(&scope, "nsModels", |_| {
        Ok(model_list().into_iter().collect::<js_sys::Array>().into())
    })?;
    method(&scope, "nsModel", |_| Ok(model()))?;
    method(&scope, "nsAccountItems", |a| {
        from_utf16_json(&json!(account_items(
            &a.get(0).as_string().unwrap_or_default(),
            a.get(1).as_string().as_deref().unwrap_or("harness")
        )))
    })?;
    method(&scope, "nsEnsureAccount", |a| {
        ensure_account(
            &a.get(0).as_string().unwrap_or_default(),
            &a.get(1).as_string().unwrap_or_default(),
            a.get(2).as_string().as_deref().unwrap_or("harness"),
        );
        Ok(JsValue::UNDEFINED)
    })?;
    method(&scope, "nsChips", |a| {
        chips(
            a.get(0),
            list(&to_utf16_json(&a.get(1))),
            a.get(2).as_string().unwrap_or_default(),
            a.get(3),
        );
        Ok(JsValue::UNDEFINED)
    })?;
    method(&scope, "nsSelectDefaults", |_| {
        select_defaults();
        Ok(JsValue::UNDEFINED)
    })?;
    method(&scope, "nsSelectableMotors", |a| {
        Ok(selectable(&a.get(0).as_string().unwrap_or_default())
            .into_iter()
            .collect::<js_sys::Array>()
            .into())
    })?;
    method(&scope, "nsRender", |_| {
        render();
        Ok(JsValue::UNDEFINED)
    })?;
    method(&scope, "nsOpen", |_| Ok(promise(open())))?;
    method(&scope, "nsShortHome", |a| {
        Ok(utf16_value(&short_home(
            &a.get(0).as_string().unwrap_or_default(),
        )))
    })?;
    method(&scope, "nsBrowse", |a| {
        Ok(promise(browse(a.get(0).as_string().unwrap_or_default())))
    })?;
    for name in ["nsOpenForPane", "openAiHere"] {
        method(&scope, name, |a| {
            Ok(promise(open_for(
                a.get(0).as_string().unwrap_or_default(),
                a.get(1).as_string().unwrap_or_default(),
                a.get(2).as_string().unwrap_or_default(),
            )))
        })?;
    }
    method(&scope, "openHarnessFor", |a| {
        let session = a.get(0).as_string().unwrap_or_default();
        let it = js_rows(&get(&state(), "list"))
            .into_iter()
            .find(|it| get(it, "session").as_string().as_deref() == Some(&session))
            .unwrap_or(JsValue::NULL);
        call_global(
            "ulog",
            &[
                "click".into(),
                "harness-from-terminal".into(),
                utf16_value(&session),
            ],
        );
        wasm_bindgen_futures::spawn_local(async move {
            let _ = open_for(
                session,
                get(&it, "pane").as_string().unwrap_or_default(),
                get(&it, "cwd").as_string().unwrap_or_default(),
            )
            .await;
        });
        Ok(JsValue::UNDEFINED)
    })?;
    models::mount()
}
pub fn attach() -> Result<(), JsValue> {
    listen(
        &id("ns-cwd"),
        "input",
        event(|_| {
            cancel(global("nsFsTimer"));
            let timer = later(
                event(|_| {
                    let path = get(&id("ns-cwd"), "value")
                        .as_string()
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    wasm_bindgen_futures::spawn_local(async move {
                        let _ = browse(if path.is_empty() { "~".into() } else { path }).await;
                    });
                    Ok(())
                }),
                220.0,
            );
            global_set("nsFsTimer", &timer)?;
            Ok(())
        }),
    );
    listen(
        &id("ns-cancel"),
        "click",
        event(|_| {
            put("intoPane", JsValue::NULL);
            classes(&id("newsess"), "hidden", true);
            Ok(())
        }),
    );
    listen(
        &id("newsess"),
        "click",
        event(|e| {
            if get(&get(&e, "target"), "id").as_string().as_deref() == Some("newsess") {
                classes(&id("newsess"), "hidden", true)
            }
            Ok(())
        }),
    );
    listen(
        &id("ns-danger"),
        "click",
        event(|_| {
            let b = id("ns-danger");
            let on = !has_class(&b, "on");
            classes(&b, "on", on);
            attr(&b, "aria-checked", if on { "true" } else { "false" });
            Ok(())
        }),
    );
    listen(
        &id("ns-term"),
        "click",
        event(|_| {
            wasm_bindgen_futures::spawn_local(async {
                // Plain shell in the chosen folder; the agent choice is kept for next time.
                let previous = get(&ns(), "harness");
                put("harness", "shell".into());
                let _ = go().await;
                put("harness", previous);
            });
            Ok(())
        }),
    );
    listen(
        &id("ns-go"),
        "click",
        event(|_| {
            wasm_bindgen_futures::spawn_local(async {
                let _ = go().await;
            });
            Ok(())
        }),
    );
    interval(1000.0, || {
        if !truthy(&call_global("inApp", &[])) {
            return Ok(());
        }
        wasm_bindgen_futures::spawn_local(async {
            if let Ok(d) = api("/active-tab", None).await {
                let current = global("ACTIVE_TAB");
                let session = get(&d, "session");
                let changed = truthy(&session)
                    && (session != get(&current, "session")
                        || get(&d, "pane").as_string().unwrap_or_default()
                            != get(&current, "pane").as_string().unwrap_or_default());
                if changed {
                    let _ = set(&d, "ts", &(now() / 1000.0).into());
                    let _ = global_set("ACTIVE_TAB", &d);
                    let pane = get(&d, "pane").as_string().unwrap_or_default();
                    let key = format!(
                        "{}{}",
                        session.as_string().unwrap_or_default(),
                        if pane.is_empty() {
                            String::new()
                        } else {
                            format!("|{pane}")
                        }
                    );
                    let _ = set(&state(), "sel", &utf16_value(&key));
                    let _ = set(&state(), "selTs", &now().into());
                    render_list();
                } else if truthy(&session) {
                    let _ = call(&global("Object"), "assign", &[current, d]);
                }
            }
        });
        Ok(())
    });
    models::attach()
}
