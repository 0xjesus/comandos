use super::*;
fn quick(sess: &JsValue) -> bool {
    text(sess).starts_with("term-q")
}
fn active_target() -> Result<JsValue, JsValue> {
    if !truthy(&get(&selected_sidebar()?, "session")) && !truthy(&get(&state(), "sel")) {
        return Ok(JsValue::NULL);
    }
    let it = run(
        "pickSel",
        &[default(get(&state(), "list"), Array::new().into())],
    )?;
    let sess = get(&it, "session");
    if !truthy(&it) || !truthy(&sess) {
        return Ok(JsValue::NULL);
    }
    let label = default(
        get(&map_get("openTerms", &sess)?, "label"),
        default(get(&it, "project"), sess.clone()),
    );
    let q = quick(&sess);
    let pane = default(get(&it, "pane"), "".into());
    item(&[
        ("session", sess.clone()),
        ("pane", pane.clone()),
        ("kind", if q { "term" } else { "pane" }.into()),
        ("paneKey", if q { sess } else { "".into() }),
        (
            "title",
            js(&format!(
                "{} · {}",
                text(&label),
                text(&default(pane, "panel".into()))
            )),
        ),
    ])
}
fn limits_current() -> Result<String, JsValue> {
    if !truthy(&get(&selected_sidebar()?, "session")) && !truthy(&get(&state(), "sel")) {
        return Ok(String::new());
    }
    let it = run(
        "pickSel",
        &[default(get(&state(), "list"), Array::new().into())],
    )?;
    let agent = get(&it, "agent");
    if !truthy(&it) || !truthy(&agent) || matches!(text(&agent).as_str(), "acp" | "shell") {
        return Ok(String::new());
    }
    Ok(format!(
        "{}:{}",
        text(&agent),
        text(&default(
            get(&it, "harnessAccount"),
            default(get(&it, "account"), "main".into())
        ))
    ))
}
fn limits_paint(slot: &JsValue) -> Result<(), JsValue> {
    if !truthy(slot) {
        return Ok(());
    }
    let limits = global("SB_LIMITS");
    let accounts = get(&limits, "accounts");
    if !truthy(&accounts) {
        if !truthy(&get(slot, "firstChild")) {
            set(
                slot,
                "innerHTML",
                &js(&format!(
                    "<div class=\"wait\">{}</div>",
                    text(&translate("Leyendo el uso…", "Reading usage…"))
                )),
            )?;
        }
        return Ok(());
    }
    let key = js(&format!(
        "{}|{}",
        string(&get(&limits, "at")),
        limits_current()?
    ));
    if get(&get(slot, "dataset"), "v") == key {
        return Ok(());
    }
    let mut view = get(slot, "_limitsView");
    if !truthy(&view) {
        let options = item(&[
            (
                "onSwitch",
                function(|a| {
                    let account = a.get(0);
                    let from = a.get(1);
                    let target = active_target()?;
                    if !truthy(&target) || js(&limits_current()?) != from {
                        return Ok(js_sys::Promise::reject(
                            &js_sys::Error::new(&string(&translate(
                                "El pane seleccionado cambió; revisa la cuenta antes de continuar",
                                "The selected pane changed; check the account before continuing",
                            )))
                            .into(),
                        )
                        .into());
                    }
                    let request = run(
                        "api",
                        &[
                            "/account/switch".into(),
                            item(&[
                                ("session", get(&target, "session")),
                                ("pane", default(get(&target, "pane"), "".into())),
                                ("alias", get(&account, "alias")),
                                ("interrupt", true.into()),
                            ])?,
                        ],
                    );
                    Ok(promise(async move {
                        let result = wait(request).await?;
                        let alias = text(&get(&account, "alias"));
                        run(
                            "toast",
                            &[if truthy(&get(&result, "queued")) {
                                translate("Cambio de cuenta en cola", "Account switch queued")
                            } else {
                                translate(
                                    &format!("Reanudando la conversación en {alias}"),
                                    &format!("Resuming the conversation in {alias}"),
                                )
                            }],
                        )?;
                        set(&global("SB_LIMITS"), "at", &0.into())?;
                        if global("tick").is_function() {
                            run("tick", &[])?;
                        }
                        Ok(JsValue::UNDEFINED)
                    }))
                }),
            ),
            (
                "onError",
                function(|a| {
                    notify_error(&a.get(0));
                    Ok(JsValue::UNDEFINED)
                }),
            ),
        ])?;
        view = call(
            &global("ComandosCommandSidebar"),
            "createLimitsView",
            &[slot.clone(), options],
        )?;
        set(slot, "_limitsView", &view)?;
    }
    call(&view, "update", &[accounts, js(&limits_current()?)])?;
    set(&get(slot, "dataset"), "v", &key)?;
    run("hydrateIcons", std::slice::from_ref(slot))?;
    Ok(())
}
fn limits(slot: &JsValue) -> Result<(), JsValue> {
    limits_paint(slot)?;
    let limits = global("SB_LIMITS");
    if truthy(&get(&limits, "job")) || now() - number(&get(&limits, "at")) < 60000.0 {
        return Ok(());
    }
    let request = run("api", &["/analytics/week?offset=0&sidebar=1".into()]);
    let state = limits.clone();
    let job = promise(async move {
        let result: Result<(), JsValue> = async {
            let m = wait(request).await?;
            set(
                &state,
                "accounts",
                &default(get(&m, "accounts"), Array::new().into()),
            )?;
            set(&state, "at", &now().into())?;
            let bytes = js_sys::JSON::stringify(&item(&[
                ("accounts", get(&state, "accounts")),
                ("at", get(&state, "at")),
            ])?)?;
            let _ = call(
                &global("localStorage"),
                "setItem",
                &["cc-sb-limits3".into(), bytes.into()],
            );
            limits_paint(&query(&doc(), "#command-sidebar .cs-limits"))?;
            Ok(())
        }
        .await;
        if result.is_err() {
            set(&state, "at", &(now() - 45000.0).into())?;
        }
        set(&state, "job", &JsValue::NULL)?;
        Ok(JsValue::UNDEFINED)
    });
    set(&limits, "job", &job)
}
fn initial_limits() {
    let result = (|| -> Result<(), JsValue> {
        let raw = call(
            &global("localStorage"),
            "getItem",
            &["cc-sb-limits3".into()],
        )?;
        let c = js_sys::JSON::parse(&string(&default(raw, "null".into())))?;
        if truthy(&c)
            && Array::is_array(&get(&c, "accounts"))
            && now() - number(&get(&c, "at")) < 30.0 * 60000.0
        {
            set(&global("SB_LIMITS"), "accounts", &get(&c, "accounts"))?;
        }
        Ok(())
    })();
    let _ = result;
}
fn open_any(sess: JsValue, label: JsValue) -> Result<(), JsValue> {
    if !truthy(&sess) {
        return Ok(());
    }
    if is_app() {
        run("openInApp", &[sess, "claude".into(), label])?;
    } else {
        run("openTerm", &[sess.clone(), default(label, sess)])?;
    }
    Ok(())
}
fn quick_instance() -> Result<JsValue, JsValue> {
    if !truthy(&global("quickTerminal")) && truthy(&global("ComandosQuickTerminal")) {
        let options = item(&[
            ("api", global("api")),
            ("toast", global("toast")),
            ("storage", global("sessionStorage")),
            ("openTerm", global("openTabAnywhere")),
            (
                "onOpened",
                function(|_| {
                    put("tabsPollTs", 0)?;
                    Ok(JsValue::UNDEFINED)
                }),
            ),
        ])?;
        put(
            "quickTerminal",
            call(
                &global("ComandosQuickTerminal"),
                "createQuickTerminal",
                &[options],
            )?,
        )?;
    }
    Ok(default(global("quickTerminal"), JsValue::NULL))
}
fn mount_servers(slot: JsValue) -> Result<(), JsValue> {
    let panel = id("servers-panel");
    let button = id("btn-servers");
    if !truthy(&panel) {
        return Ok(());
    }
    if !truthy(&get(&panel, "_home")) {
        set(
            &panel,
            "_home",
            &item(&[
                ("parent", get(&panel, "parentNode")),
                ("next", get(&panel, "nextSibling")),
            ])?,
        )?;
    }
    if truthy(&slot) {
        if get(&panel, "parentNode") != slot {
            call(&slot, "appendChild", std::slice::from_ref(&panel))?;
        }
        classes(&panel, "hidden", false);
        classes(&id("ssh-bar"), "open", true);
        attr(&id("ssh-toggle"), "aria-expanded", "true");
        if truthy(&button) {
            attr(&button, "aria-expanded", "true");
        }
        run("loadSsh", &[])?;
    } else {
        let home = get(&panel, "_home");
        let parent = get(&home, "parent");
        if get(&panel, "parentNode") != parent {
            call(
                &parent,
                "insertBefore",
                &[panel.clone(), get(&home, "next")],
            )?;
        }
        classes(&panel, "hidden", true);
        if truthy(&button) {
            attr(&button, "aria-expanded", "false");
        }
    }
    Ok(())
}
fn mount_sidebar() -> Result<(), JsValue> {
    if truthy(&global("ONLY_PANEL")) {
        return Ok(());
    }
    let root = id("command-sidebar");
    if !truthy(&root)
        || truthy(&global("commandSidebar"))
        || !truthy(&global("ComandosCommandSidebar"))
    {
        return Ok(());
    }
    let options = item(&[
        ("api", global("api")),
        ("root", root),
        ("storage", global("localStorage")),
        ("hydrate", global("hydrateIcons")),
        ("toast", global("toast")),
        ("renderLimits", global("sidebarLimits")),
        (
            "mountServers",
            function(|a| {
                mount_servers(a.get(0))?;
                Ok(JsValue::UNDEFINED)
            }),
        ),
    ])?;
    let sidebar = call(
        &global("ComandosCommandSidebar"),
        "createCommandSidebar",
        &[options],
    )?;
    put("commandSidebar", sidebar.clone())?;
    call(&sidebar, "render", &[])?;
    Ok(())
}
pub(super) fn mount() -> Result<(), JsValue> {
    publish("isQuickTermSession", |a| Ok(quick(&a.get(0)).into()))?;
    publish("activePaneTarget", |_| active_target())?;
    publish("openTabAnywhere", |a| {
        open_any(a.get(0), a.get(1))?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("quickTerminalInstance", |_| quick_instance())?;
    publish("mountCommandSidebar", |_| {
        mount_sidebar()?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("sidebarLimitsCur", |_| Ok(js(&limits_current()?)))?;
    publish("sidebarLimitsPaint", |a| {
        limits_paint(&a.get(0))?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("sidebarLimits", |a| {
        limits(&a.get(0))?;
        Ok(JsValue::UNDEFINED)
    })?;
    initial_limits();
    // Quotas refresh only while «Consumos» is on screen and the page is visible.
    run(
        "setInterval",
        &[
            function(|_| {
                let slot = query(
                    &doc(),
                    "#command-sidebar .cs-empty-terms:not([hidden]) .et-lim:not([hidden]) .cs-limits",
                );
                if truthy(&slot) && text(&get(&doc(), "visibilityState")) == "visible" {
                    limits(&slot)?;
                }
                Ok(JsValue::UNDEFINED)
            }),
            60000.into(),
        ],
    )?;
    for key in ["cc-sb-limits", "cc-sb-limits2"] {
        let _ = call(&global("localStorage"), "removeItem", &[key.into()]);
    }
    Ok(())
}
