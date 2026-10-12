use super::*;
fn ordered() -> Result<Vec<(JsValue, JsValue)>, JsValue> {
    let mut all = entries(&global("openTerms"))?;
    let rank = |s: &JsValue| {
        if text(s) == "local" {
            0
        } else if call(&get(&state(), "favs"), "has", std::slice::from_ref(s))
            .is_ok_and(|v| truthy(&v))
        {
            1
        } else {
            2
        }
    };
    all.sort_by_key(|(s, _)| rank(s));
    Ok(all)
}
fn row_for(sess: &JsValue) -> JsValue {
    let list = state_list();
    list.iter()
        .find(|it| get(it, "session") == *sess && truthy(&get(it, "alive")))
        .or_else(|| list.iter().find(|it| get(it, "session") == *sess))
        .cloned()
        .unwrap_or(JsValue::UNDEFINED)
}
fn make_tab(
    t: &JsValue,
    label: JsValue,
    key: JsValue,
    closable: bool,
    status: JsValue,
    entry: JsValue,
    selected: &JsValue,
) -> Result<(), JsValue> {
    let source = truthy(&entry);
    set(
        t,
        "_target",
        &if source {
            get(&entry, "target")
        } else {
            key.clone()
        },
    )?;
    let session = if source {
        get(&entry, "session")
    } else {
        js(text(&key).strip_prefix("term:").unwrap_or(&text(&key)))
    };
    set(t, "_session", &session)?;
    set(t, "_group", &default(get(&entry, "groupId"), JsValue::NULL))?;
    if source {
        set(&get(t, "dataset"), "wsSource", &get(&entry, "source"))?;
    } else {
        Reflect::delete_property(
            &get(t, "dataset").unchecked_into::<js_sys::Object>(),
            &"wsSource".into(),
        )?;
    }
    if !truthy(&query(t, ".lbl")) || get(t, "_closable") != closable {
        set(t, "_closable", &closable.into())?;
        set(t,"innerHTML",&format!("<span class=\"dot\"></span><span class=\"lbl\"></span><span class=\"mdl\"></span><span class=\"ws-count\" hidden></span>{}",if closable{"<span class=\"x\">×</span>"}else{""}).into())?;
        let tab = t.clone();
        set(
            t,
            "onclick",
            &function(move |_| {
                run("showView", &[get(&tab, "_target"), true.into()])?;
                Ok(JsValue::UNDEFINED)
            }),
        )?;
        let tab = t.clone();
        set(
            t,
            "onkeydown",
            &function(move |a| {
                let e = a.get(0);
                if get(&e, "target") == tab
                    && matches!(text(&get(&e, "key")).as_str(), "Enter" | " ")
                {
                    call(&e, "preventDefault", &[])?;
                    run("showView", &[get(&tab, "_target"), true.into()])?;
                }
                Ok(JsValue::UNDEFINED)
            }),
        )?;
        let favorite = query(t, ".tab-fav");
        if truthy(&favorite) {
            let tab = t.clone();
            set(
                &favorite,
                "onclick",
                &function(move |a| {
                    call(&a.get(0), "stopPropagation", &[])?;
                    let sess = get(&tab, "_session");
                    let has = truthy(&call(
                        &get(&state(), "favs"),
                        "has",
                        std::slice::from_ref(&sess),
                    )?);
                    fire(run("setSessionFavorite", &[sess, (!has).into()]), true);
                    Ok(JsValue::UNDEFINED)
                }),
            )?;
        }
        let close = query(t, ".x");
        if truthy(&close) {
            let tab = t.clone();
            set(
                &close,
                "onclick",
                &function(move |a| {
                    call(&a.get(0), "stopPropagation", &[])?;
                    let group = get(&tab, "_group");
                    if truthy(&group) {
                        run("askCloseGroup", &[group])?;
                    } else {
                        run("askCloseTab", &[get(&tab, "_session")])?;
                    }
                    Ok(JsValue::UNDEFINED)
                }),
            )?;
        }
    }
    let on = *selected == key;
    set(
        t,
        "className",
        &if on { "apptab on" } else { "apptab" }.into(),
    )?;
    attr(t, "role", "tab");
    attr(t, "aria-selected", if on { "true" } else { "false" });
    set(t, "tabIndex", &if on { 0 } else { -1 }.into())?;
    let lbl = query(t, ".lbl");
    if get(&lbl, "textContent") != label {
        set(&lbl, "textContent", &label)?;
    }
    let mdl = query(t, ".mdl");
    if truthy(&mdl) {
        let models = state_list()
            .into_iter()
            .filter(|it| {
                get(it, "session") == session
                    && get(it, "alive") != JsValue::FALSE
                    && truthy(&get(it, "model"))
            })
            .collect::<Vec<_>>();
        let short = if let Some(first) = models.first() {
            let first = text(&run("tabModelShort", &[get(first, "model")])?);
            format!(
                "{first}{}",
                if models.len() > 1 {
                    format!(" +{}", models.len() - 1)
                } else {
                    String::new()
                }
            )
        } else {
            String::new()
        };
        if get(&mdl, "textContent") != js(&short) {
            set(&mdl, "textContent", &js(&short))?;
        }
        set(&mdl, "hidden", &short.is_empty().into())?;
    }
    let dot = query(t, ".dot");
    set(&dot, "hidden", &(!truthy(&status)).into())?;
    let color = match text(&status).as_str() {
        "waiting" => "var(--waiting)",
        "working" => "var(--working)",
        "done" => "var(--done)",
        _ => "var(--faint)",
    };
    set(&get(&dot, "style"), "background", &color.into())?;
    set(&get(&dot, "style"), "color", &color.into())?;
    let close = query(t, ".x");
    if truthy(&close) {
        set(
            &close,
            "title",
            &if truthy(&get(t, "_group")) {
                translate("Cerrar grupo", "Close group")
            } else {
                translate("Cerrar pestaña", "Close tab")
            },
        )?;
    }
    let favorite = query(t, ".tab-fav");
    if truthy(&favorite) {
        run("updateFavoriteButton", &[favorite, session, label])?;
    }
    let count = query(t, ".ws-count");
    let counted = source && number(&get(&entry, "count")) > 1.0;
    set(&count, "hidden", &(!counted).into())?;
    if counted {
        set(
            &count,
            "textContent",
            &format!("{} tabs", string(&get(&entry, "count"))).into(),
        )?;
    }
    classes(t, "ws-unavailable", source && !truthy(&get(&entry, "live")));
    Ok(())
}
fn render_bar() -> Result<(), JsValue> {
    let bar = id("tabbar");
    let split = has_class(&body(), "split");
    let dock_entries = dock("stripEntries", &[])?;
    let mut selected =
        if split && text(&global("activeView")) == "panel" && truthy(&global("activeTerm")) {
            js(&format!("term:{}", text(&global("activeTerm"))))
        } else {
            global("activeView")
        };
    if truthy(&dock_entries) && text(&selected).starts_with("term:") {
        for e in values(&dock_entries) {
            if truthy(&dock("isSelected", std::slice::from_ref(&e))?) {
                selected = default(get(&e, "key"), selected);
                break;
            }
        }
    }
    let mut existing = values(&get(&bar, "children"))
        .into_iter()
        .map(|el| (get(&get(&el, "dataset"), "tabKey"), el))
        .collect::<Vec<_>>();
    let mut wanted = Vec::new();
    let mut rows = if truthy(&dock_entries) {
        values(&dock_entries)
            .into_iter()
            .map(|e| {
                let it = row_for(&get(&e, "session"));
                (
                    get(&e, "label"),
                    get(&e, "key"),
                    truthy(&get(&e, "closable")),
                    get(&it, "status"),
                    e,
                )
            })
            .collect::<Vec<_>>()
    } else {
        ordered()?
            .into_iter()
            .map(|(sess, o)| {
                let it = row_for(&sess);
                (
                    default(get(&o, "label"), default(get(&it, "project"), sess.clone())),
                    js(&format!("term:{}", text(&sess))),
                    text(&sess) != "local",
                    get(&it, "status"),
                    JsValue::UNDEFINED,
                )
            })
            .collect()
    };
    rows.sort_by_key(|(_, key, _, _, entry)| {
        text(&get(entry, "session")) != "local" && text(key) != "term:local"
    });
    for (label, key, close, status, entry) in rows {
        let t = if let Some(index) = existing.iter().position(|(k, _)| *k == key) {
            existing.remove(index).1
        } else {
            let el = call(&doc(), "createElement", &["div".into()])?;
            set(&get(&el, "dataset"), "tabKey", &key)?;
            el
        };
        make_tab(&t, label, key, close, status, entry, &selected)?;
        wanted.push(t);
    }
    for (_, el) in existing {
        call(&el, "remove", &[])?;
    }
    for (index, el) in wanted.into_iter().enumerate() {
        let next = get(&get(&bar, "children"), &index.to_string());
        if next != el {
            call(&bar, "insertBefore", &[el, default(next, JsValue::NULL)])?;
        }
    }
    // Center the active tab whenever it changes, whatever changed it (arrows, swipe,
    // another device); repeated renders of the same tab leave manual scrolling alone.
    let key = text(&selected);
    if CENTERED.with(|c| c.replace(key.clone())) != key {
        reveal(bar)?;
    }
    navigation()
}
thread_local! {
    static CENTERED: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}
// Compatibility hook: CSS owns the row height, including viewport shrinkage.
fn hold_height() -> Result<(), JsValue> {
    let bar = id("tabbar");
    if truthy(&bar) {
        set(&get(&bar, "style"), "minHeight", &"".into())?;
    }
    Ok(())
}
fn navigation() -> Result<(), JsValue> {
    let tabs = values(&get(&id("tabbar"), "children"));
    let current = tabs.iter().position(|tab| has_class(tab, "on"));
    for (name, disabled) in [
        ("tab-prev", current.is_none_or(|index| index == 0)),
        (
            "tab-next",
            tabs.is_empty() || current.is_some_and(|index| index + 1 >= tabs.len()),
        ),
    ] {
        let button = id(name);
        if truthy(&button) {
            set(&button, "disabled", &disabled.into())?;
        }
    }
    Ok(())
}
fn reveal(bar: JsValue) -> Result<(), JsValue> {
    run(
        "requestAnimationFrame",
        &[function(move |_| {
            let tab = query(&bar, ".apptab.on");
            if !truthy(&tab) {
                return Ok(JsValue::UNDEFINED);
            }
            let target = call(&tab, "getBoundingClientRect", &[])?;
            let bounds = call(&bar, "getBoundingClientRect", &[])?;
            if number(&get(&bounds, "width")) < 1.0 {
                // Not laid out yet: let the next render center it.
                CENTERED.with(|c| c.borrow_mut().clear());
                return Ok(JsValue::UNDEFINED);
            }
            let (start, end, scroll) = if has_class(&body(), "tabs-rows") {
                ("top", "bottom", "scrollTop")
            } else {
                ("left", "right", "scrollLeft")
            };
            // A single row keeps the active tab in the middle; the grid only scrolls it into view.
            let shift = if scroll == "scrollLeft" {
                (number(&get(&target, start)) + number(&get(&target, end))) / 2.0
                    - (number(&get(&bounds, start)) + number(&get(&bounds, end))) / 2.0
            } else if number(&get(&target, start)) < number(&get(&bounds, start)) {
                number(&get(&target, start)) - number(&get(&bounds, start))
            } else if number(&get(&target, end)) > number(&get(&bounds, end)) {
                number(&get(&target, end)) - number(&get(&bounds, end))
            } else {
                0.0
            };
            if shift.abs() >= 1.0 {
                set(&bar, scroll, &(number(&get(&bar, scroll)) + shift).into())?;
            }
            navigation()?;
            Ok(JsValue::UNDEFINED)
        })],
    )?;
    Ok(())
}
fn show_view(key: JsValue, reveal_tab: bool) -> Result<(), JsValue> {
    let previous = global("activeTerm");
    put("activeView", key.clone())?;
    let split = has_class(&body(), "split");
    let key_s = text(&key);
    if let Some(sess) = key_s.strip_prefix("term:") {
        put("activeTerm", js(sess))?;
    }
    let shown = if let Some(sess) = key_s.strip_prefix("term:") {
        js(sess)
    } else if split {
        let active = global("activeTerm");
        if truthy(&active)
            && truthy(&call(
                &global("openTerms"),
                "has",
                std::slice::from_ref(&active),
            )?)
        {
            active
        } else {
            entries(&global("openTerms"))?
                .first()
                .map(|(s, _)| s.clone())
                .unwrap_or(JsValue::UNDEFINED)
        }
    } else {
        JsValue::NULL
    };
    let visible = if truthy(&shown) {
        default(
            dock("visibleTabs", std::slice::from_ref(&shown))?,
            [shown.clone()].into_iter().collect::<Array>().into(),
        )
    } else {
        Array::new().into()
    };
    if truthy(&shown) {
        put("activeTerm", shown.clone())?;
        for sess in values(&visible) {
            run("ensureFrame", &[sess])?;
        }
        run("saveDeviceFocus", std::slice::from_ref(&shown))?;
    }
    run(
        "restoreInactiveTermInteractions",
        std::slice::from_ref(&shown),
    )?;
    classes(&id("view-panel"), "on", split || key_s == "panel");
    classes(
        &id("term-area"),
        "on",
        if split {
            truthy(&shown)
        } else {
            key_s.starts_with("term:")
        },
    );
    for (sess, o) in entries(&global("openTerms"))? {
        let frame = get(&o, "frame");
        if truthy(&frame) {
            classes(&frame, "on", values(&visible).contains(&sess));
        }
    }
    dock("render", std::slice::from_ref(&shown))?;
    let mouse = get(&map_get("termInteraction", &shown)?, "mouse");
    if truthy(&shown) && (shown != previous || mouse.is_null() || mouse.is_undefined()) {
        fire(
            run("syncTermInteraction", std::slice::from_ref(&shown)),
            false,
        );
    }
    let active = global("activeTerm");
    let sel = text(&get(&state(), "sel"));
    if !is_app()
        && truthy(&active)
        && (active != previous || sel.split('|').next().unwrap_or_default() != text(&active))
    {
        put("activeTermTs", now() / 1000.0)?;
        set(&state(), "sel", &active)?;
        set(&state(), "selTs", &now().into())?;
        run(
            "render",
            &[default(get(&state(), "list"), Array::new().into())],
        )?;
    }
    render_bar()?;
    if reveal_tab {
        reveal(id("tabbar"))?;
    }
    Ok(())
}
fn add(sess: JsValue, label: JsValue, mirrored: bool) -> Result<(), JsValue> {
    let o = map_get("openTerms", &sess)?;
    if truthy(&o) {
        if truthy(&label) {
            set(&o, "label", &label)?;
        }
        if mirrored {
            set(&o, "mirrored", &true.into())?;
        }
        return Ok(());
    }
    call(
        &global("openTerms"),
        "set",
        &[
            sess.clone(),
            item(&[
                ("frame", JsValue::NULL),
                ("label", default(label, sess)),
                ("mirrored", mirrored.into()),
            ])?,
        ],
    )?;
    Ok(())
}
fn close(sess: JsValue) -> Result<(), JsValue> {
    let o = map_get("openTerms", &sess)?;
    if truthy(&o) {
        let frame = get(&o, "frame");
        if truthy(&frame) {
            call(&frame, "remove", &[])?;
        }
        call(&global("openTerms"), "delete", std::slice::from_ref(&sess))?;
    }
    let s = sess.clone();
    let restored = run("restoreTermInteraction", std::slice::from_ref(&sess))?;
    call(
        &restored,
        "finally",
        &[function(move |_| {
            run("cleanupTermInteraction", std::slice::from_ref(&s))?;
            Ok(JsValue::UNDEFINED)
        })],
    )?;
    run("cleanupTermInteraction", std::slice::from_ref(&sess))?;
    if global("activeTerm") == sess {
        put(
            "activeTerm",
            entries(&global("openTerms"))?
                .first()
                .map(|(s, _)| s.clone())
                .unwrap_or(JsValue::NULL),
        )?;
    }
    let view = global("activeView");
    show_view(
        if text(&view) == format!("term:{}", text(&sess)) {
            "panel".into()
        } else {
            view
        },
        false,
    )
}
fn ask_close(sess: JsValue) -> Result<(), JsValue> {
    if text(&sess) == "local" {
        return Ok(());
    }
    let o = map_get("openTerms", &sess)?;
    set(
        &id("tabclose-name"),
        "textContent",
        &default(get(&o, "label"), sess.clone()),
    )?;
    let modal = id("tabclose");
    classes(&modal, "open", true);
    set(
        &id("tabclose-yes"),
        "onclick",
        &function(move |_| {
            classes(&modal, "open", false);
            let sess = sess.clone();
            Ok(promise(async move {
                let _ = api("/tab-close", item(&[("session", sess.clone())])?).await;
                close(sess)?;
                put("tabsPollTs", 0)?;
                Ok(JsValue::UNDEFINED)
            }))
        }),
    )
}
async fn ask_group(group: JsValue) -> Result<JsValue, JsValue> {
    let preview = match api(
        &format!("/workspace/close-group?groupId={}", encode(&group)?),
        JsValue::UNDEFINED,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            notify_error(&e);
            return Ok(JsValue::UNDEFINED);
        }
    };
    let modal = id("groupclose");
    let list = id("groupclose-list");
    let status = id("groupclose-status");
    let yes = id("groupclose-yes");
    set(
        &id("groupclose-title"),
        "textContent",
        &translate(
            "¿Cerrar este grupo de pestañas?",
            "Close this group of tabs?",
        ),
    )?;
    set(
        &id("groupclose-desc"),
        "textContent",
        &translate(
            "Se cierran aquí y en el escritorio. Sus sesiones tmux siguen vivas y puedes recuperarlas en Recientes.",
            "They close here and on the desktop. Their tmux sessions stay alive in Recents.",
        ),
    )?;
    set(
        &id("groupclose-cancel"),
        "textContent",
        &translate("Cancelar", "Cancel"),
    )?;
    let mut closing = 0;
    let mut nodes = Vec::new();
    for member in values(&get(&preview, "members")) {
        let kept = truthy(&get(&member, "kept"));
        if !kept {
            closing += 1;
        }
        let li = call(&doc(), "createElement", &["li".into()])?;
        set(
            &li,
            "textContent",
            &js(&format!(
                "{}{}",
                text(&get(&member, "label")),
                if kept {
                    text(&translate(" — se queda abierta", " — stays open"))
                } else {
                    String::new()
                }
            )),
        )?;
        nodes.push(li);
    }
    call(&list, "replaceChildren", &nodes)?;
    set(&status, "textContent", &"".into())?;
    set(
        &yes,
        "textContent",
        &translate(
            &format!("Cerrar {closing} pestañas"),
            &format!("Close {closing} tabs"),
        ),
    )?;
    set(&yes, "disabled", &(closing == 0).into())?;
    let request_id = random_id("close-", 8)?;
    classes(&modal, "open", true);
    let button = yes.clone();
    set(
        &yes,
        "onclick",
        &function(move |_| {
            set(&button, "disabled", &true.into())?;
            set(&status, "textContent", &translate("Cerrando…", "Closing…"))?;
            let (button, status, modal, preview, group, request_id) = (
                button.clone(),
                status.clone(),
                modal.clone(),
                preview.clone(),
                group.clone(),
                request_id.clone(),
            );
            let payload = item(&[
                ("requestId", request_id.into()),
                ("groupId", group),
                ("expectedRevision", get(&preview, "revision")),
                ("members", get(&preview, "members")),
            ])?;
            Ok(promise(async move {
                let result = async {
                    let headers = item(&[("Content-Type", "application/json".into())])?;
                    let token = run("authToken", &[])?;
                    if truthy(&token) {
                        set(&headers, "X-Comandos-Token", &token)?;
                    }
                    let options = item(&[
                        ("method", "POST".into()),
                        ("headers", headers),
                        ("body", js_sys::JSON::stringify(&payload)?.into()),
                    ])?;
                    let response =
                        wait(run("fetch", &["/workspace/close-group".into(), options])).await?;
                    let parsed = wait(call(&response, "json", &[]))
                        .await
                        .unwrap_or_else(|_| object());
                    if !truthy(&get(&response, "ok")) {
                        return Err(js_sys::Error::new(&string(&default(
                            get(&parsed, "error"),
                            get(&response, "statusText"),
                        )))
                        .into());
                    }
                    Ok::<_, JsValue>(parsed)
                }
                .await;
                let result = match result {
                    Ok(v) => v,
                    Err(e) => {
                        set(&button, "disabled", &false.into())?;
                        set(
                            &status,
                            "textContent",
                            &js(&format!(
                                "{}{}",
                                text(&get(&e, "message")),
                                text(&translate(" No se cerró nada.", " Nothing was closed."))
                            )),
                        )?;
                        return Ok(JsValue::UNDEFINED);
                    }
                };
                for sess in values(&default(get(&result, "closed"), Array::new().into())) {
                    close(sess)?;
                }
                put("tabsPollTs", 0)?;
                if truthy(&get(&result, "ok")) {
                    classes(&modal, "open", false);
                    return Ok(JsValue::UNDEFINED);
                }
                let count = number(&get(&get(&result, "closed"), "length"));
                let remaining = call(&get(&result, "remaining"), "join", &[", ".into()])?;
                let error = text(&get(&result, "error"));
                set(
                    &status,
                    "textContent",
                    &translate(
                        &format!(
                            "Se cerraron {count}; quedan abiertas: {}. {error}",
                            text(&remaining)
                        ),
                        &format!("Closed {count}; still open: {}. {error}", text(&remaining)),
                    ),
                )?;
                Ok(JsValue::UNDEFINED)
            }))
        }),
    )?;
    Ok(JsValue::UNDEFINED)
}
fn random_id(prefix: &str, count: usize) -> Result<String, JsValue> {
    let date = call_value(&now().into(), "toString", &[36.into()])?;
    let random = call_value(
        &JsValue::from(js_sys::Math::random()),
        "toString",
        &[36.into()],
    )?;
    let suffix = call_value(&random, "slice", &[2.into(), (2 + count).into()])?;
    Ok(format!("{prefix}{}-{}", text(&date), text(&suffix)))
}
fn device_id() -> JsValue {
    let result = (|| -> Result<JsValue, JsValue> {
        let storage = global("localStorage");
        let mut id = call(&storage, "getItem", &["comandos.deviceId".into()])?;
        let v = text(&id);
        let valid = (8..=120).contains(&v.len())
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b));
        if !valid {
            let crypto = global("crypto");
            let suffix = if truthy(&get(&crypto, "randomUUID")) {
                call(&crypto, "randomUUID", &[])?
            } else {
                let d = call_value(&now().into(), "toString", &[36.into()])?;
                let r = call_value(
                    &JsValue::from(js_sys::Math::random()),
                    "toString",
                    &[36.into()],
                )?;
                js(&format!(
                    "{}{}",
                    text(&d),
                    text(&call_value(&r, "slice", &[2.into()])?)
                ))
            };
            id = js(&format!("web-{}", text(&suffix)));
            call(
                &storage,
                "setItem",
                &["comandos.deviceId".into(), id.clone()],
            )?;
        }
        Ok(id)
    })();
    result.unwrap_or(JsValue::NULL)
}
fn save_focus(sess: JsValue) -> Result<(), JsValue> {
    if !truthy(&global("WS_DEVICE"))
        || !truthy(&global("wsFocusRestored"))
        || !truthy(&sess)
        || sess == global("wsFocusSaved")
    {
        return Ok(());
    }
    cancel(global("wsFocusTimer"));
    put(
        "wsFocusTimer",
        later(
            function(move |_| {
                put("wsFocusSaved", sess.clone())?;
                let body = item(&[
                    ("deviceId", global("WS_DEVICE")),
                    ("activeTabId", sess.clone()),
                ])?;
                let promise = run("api", &["/workspace/client".into(), body])?;
                call(
                    &promise,
                    "catch",
                    &[function(|_| {
                        put("wsFocusSaved", JsValue::NULL)?;
                        Ok(JsValue::UNDEFINED)
                    })],
                )?;
                Ok(JsValue::UNDEFINED)
            }),
            800.0,
        ),
    )
}
fn restore_focus() -> Result<JsValue, JsValue> {
    if truthy(&global("wsFocusRestored")) {
        return Ok(js_sys::Promise::resolve(&JsValue::UNDEFINED).into());
    }
    put("wsFocusRestored", true)?;
    if !truthy(&global("WS_DEVICE")) || text(&global("activeView")).starts_with("term:") {
        return Ok(js_sys::Promise::resolve(&JsValue::UNDEFINED).into());
    }
    let request = run(
        "api",
        &[js(&format!(
            "/workspace/client?deviceId={}",
            encode(&global("WS_DEVICE"))?
        ))],
    );
    Ok(promise(async move {
        if let Ok(mine) = wait(request).await {
            let tab = get(&mine, "activeTabId");
            put("wsFocusSaved", default(tab.clone(), JsValue::NULL))?;
            if truthy(&tab)
                && truthy(&call(
                    &global("openTerms"),
                    "has",
                    std::slice::from_ref(&tab),
                )?)
                && !text(&global("activeView")).starts_with("term:")
            {
                put("activeTerm", tab.clone())?;
                put("activeView", js(&format!("term:{}", text(&tab))))?;
            }
        }
        Ok(JsValue::UNDEFINED)
    }))
}
async fn load_tabs() -> Result<(), JsValue> {
    let result: Result<(), JsValue> = async {
        let tabs = api("/tabs", JsValue::UNDEFINED).await?;
        let tabs = values(&tabs);
        for t in &tabs {
            add(get(t, "session"), get(t, "label"), true)?;
        }
        let wanted = tabs.iter().map(|t| get(t, "session")).collect::<Vec<_>>();
        for (sess, o) in entries(&global("openTerms"))? {
            if truthy(&get(&o, "mirrored")) && !wanted.contains(&sess) {
                close(sess)?;
            }
        }
        let mut ordered = Vec::new();
        for t in &tabs {
            let sess = get(t, "session");
            let o = map_get("openTerms", &sess)?;
            if truthy(&o) {
                ordered.push((sess, o));
            }
        }
        for (sess, o) in entries(&global("openTerms"))? {
            if !wanted.contains(&sess) {
                ordered.push((sess, o));
            }
        }
        let map = global("openTerms");
        call(&map, "clear", &[])?;
        for (sess, o) in ordered {
            call(&map, "set", &[sess, o])?;
        }
        if !truthy(&global("activeTerm"))
            && let Some(t) = tabs.first()
        {
            put("activeTerm", get(t, "session"))?;
        }
        if let Ok(workspace) = api("/workspace", JsValue::UNDEFINED).await {
            let _ = dock("adopt", &[workspace]);
        }
        wait(restore_focus()).await?;
        show_view(global("activeView"), false)?;
        Ok(())
    }
    .await;
    let _ = result;
    Ok(())
}
fn refresh_tabs() -> Result<(), JsValue> {
    let time = now();
    if time - number(&global("tabsPollTs")) < 5000.0 {
        return Ok(());
    }
    put("tabsPollTs", time)?;
    fire(run("loadDesktopTabs", &[]), false);
    Ok(())
}
fn init_navigation() -> Result<(), JsValue> {
    let bar = id("tabbar");
    call(
        &bar,
        "addEventListener",
        &[
            "scroll".into(),
            global("updateTabNavigation"),
            item(&[("passive", true.into())])?,
        ],
    )?;
    for (id_name, delta) in [("tab-prev", -1i64), ("tab-next", 1i64)] {
        let b = bar.clone();
        set(
            &id(id_name),
            "onclick",
            &function(move |_| {
                let tabs = values(&get(&b, "children"));
                if !tabs.is_empty() {
                    let current = tabs.iter().position(|tab| has_class(tab, "on"));
                    let next = current.map_or(0, |index| {
                        (index as i64 + delta).clamp(0, tabs.len() as i64 - 1) as usize
                    });
                    if current != Some(next)
                        && let Some(tab) = tabs.get(next)
                    {
                        run("showView", &[get(tab, "_target"), true.into()])?;
                    }
                }
                Ok(JsValue::UNDEFINED)
            }),
        )?;
    }
    set(
        &id("tab-panel"),
        "onclick",
        &function(|_| {
            let key = if text(&global("activeView")) == "panel" && truthy(&global("activeTerm")) {
                js(&format!("term:{}", text(&global("activeTerm"))))
            } else {
                "panel".into()
            };
            show_view(key, false)?;
            Ok(JsValue::UNDEFINED)
        }),
    )?;
    set(&id("tab-open"), "onclick", &global("swOpen"))?;
    init_sort()?;
    if !truthy(&global("quickTerminal")) && truthy(&global("ComandosQuickTerminal")) {
        let options = item(&[
            ("api", global("api")),
            ("openTerm", global("openTerm")),
            ("toast", global("toast")),
            ("storage", global("sessionStorage")),
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
    Ok(())
}
fn init_sort() -> Result<(), JsValue> {
    let button = id("tab-sort");
    let state = item(&[("previous", JsValue::NULL), ("menu", JsValue::NULL)])?;
    let b = button.clone();
    set(
        &button,
        "onclick",
        &function(move |a| {
            call(&a.get(0), "stopPropagation", &[])?;
            let menu = get(&state, "menu");
            if truthy(&menu) {
                call(&menu, "remove", &[])?;
                set(&state, "menu", &JsValue::NULL)?;
                return Ok(JsValue::UNDEFINED);
            }
            let menu = call(&doc(), "createElement", &["div".into()])?;
            set(&menu, "className", &"tab-sort-menu".into())?;
            attr(&menu, "role", "menu");
            set(&state, "menu", &menu)?;
            set(&menu,"innerHTML",&format!("{}{}",[("recent","⏱ Actividad reciente"),("need","▶ Te necesita primero"),("alpha","A→Z")].iter().map(|(k,label)|format!("<button type=\"button\" role=\"menuitem\" data-sort=\"{k}\">{label}</button>")).collect::<String>(),if truthy(&get(&state,"previous")){format!("<button type=\"button\" role=\"menuitem\" data-sort=\"undo\">↶ {}</button>",text(&translate("Deshacer","Undo")))}else{String::new()}).into())?;
            let rect = call(&b, "getBoundingClientRect", &[])?;
            set(
                &get(&menu, "style"),
                "top",
                &format!("{}px", number(&get(&rect, "bottom")) + 4.0).into(),
            )?;
            set(
                &get(&menu, "style"),
                "left",
                &format!(
                    "{}px",
                    number(&get(&rect, "left"))
                        .min(number(&global("innerWidth")) - 230.0)
                        .max(8.0)
                )
                .into(),
            )?;
            let st = state.clone();
            listen(
                &menu,
                "click",
                function(move |a| {
                    let target = call(
                        &get(&a.get(0), "target"),
                        "closest",
                        &["[data-sort]".into()],
                    )?;
                    if !truthy(&target) {
                        return Ok(JsValue::UNDEFINED);
                    }
                    let by = get(&get(&target, "dataset"), "sort");
                    let payload = if text(&by) == "undo" {
                        item(&[("restore", get(&st, "previous"))])?
                    } else {
                        item(&[("by", by)])?
                    };
                    call(&get(&st, "menu"), "remove", &[])?;
                    set(&st, "menu", &JsValue::NULL)?;
                    let s = st.clone();
                    let request = run("api", &["/workspace/sort".into(), payload.clone()]);
                    Ok(promise(async move {
                        match wait(request).await {
                            Ok(r) => {
                                set(
                                    &s,
                                    "previous",
                                    &if truthy(&get(&payload, "restore")) {
                                        JsValue::NULL
                                    } else {
                                        get(&r, "previous")
                                    },
                                )?;
                                if truthy(&dock("adopt", &[r])?) {
                                    render_bar()?;
                                }
                                fire(run("loadDesktopTabs", &[]), false);
                                run(
                                    "toast",
                                    &[if truthy(&get(&payload, "restore")) {
                                        translate(
                                            "Orden anterior restaurado",
                                            "Previous order restored",
                                        )
                                    } else {
                                        translate(
                                            "Pestañas ordenadas · Deshacer en ⇅",
                                            "Tabs sorted · Undo in ⇅",
                                        )
                                    }],
                                )?;
                            }
                            Err(e) => notify_error(&e),
                        }
                        Ok(JsValue::UNDEFINED)
                    }))
                }),
            );
            call(&body(), "appendChild", &[menu])?;
            let st = state.clone();
            later(
                function(move |_| {
                    let cleanup = object();
                    let s = st.clone();
                    let c = cleanup.clone();
                    let f = function(move |_| {
                        let menu = get(&s, "menu");
                        if truthy(&menu) {
                            call(&menu, "remove", &[])?;
                            set(&s, "menu", &JsValue::NULL)?;
                        }
                        call(
                            &doc(),
                            "removeEventListener",
                            &["click".into(), get(&c, "f")],
                        )?;
                        Ok(JsValue::UNDEFINED)
                    });
                    set(&cleanup, "f", &f)?;
                    listen(&doc(), "click", f);
                    Ok(JsValue::UNDEFINED)
                }),
                0.0,
            );
            Ok(JsValue::UNDEFINED)
        }),
    )
}
fn init_app() -> Result<(), JsValue> {
    if !truthy(&global("WEBTERM")) {
        return Ok(());
    }
    classes(&body(), "app", true);
    crate::components::chat_view::install()?;
    run("restoreSplitLeft", &[])?;
    run("initSplitDrag", &[])?;
    init_navigation()?;
    let update = function(|_| {
        run("scheduleAppViewport", &[])?;
        run("applyAppLayout", &[])?;
        Ok(JsValue::UNDEFINED)
    });
    let vv = global("visualViewport");
    if truthy(&vv) {
        listen(&vv, "resize", update.clone());
        listen(&vv, "scroll", global("scheduleAppViewport"));
    }
    call(
        &js_sys::global(),
        "addEventListener",
        &[
            "scroll".into(),
            global("pinAppScroll"),
            item(&[("passive", true.into())])?,
        ],
    )?;
    listen(&js_sys::global(), "resize", update.clone());
    listen(&js_sys::global(), "orientationchange", update.clone());
    invoke(&update, &[])?;
    fire(run("loadDesktopTabs", &[]), false);
    Ok(())
}
pub(super) fn mount() -> Result<(), JsValue> {
    put("WS_DEVICE", device_id())?;
    publish("orderedTermTabs", |_| {
        Ok(ordered()?
            .into_iter()
            .map(|(s, o)| JsValue::from([s, o].into_iter().collect::<Array>()))
            .collect::<Array>()
            .into())
    })?;
    publish("renderTabbar", |_| {
        render_bar()?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("holdTabRowsHeight", |_| {
        hold_height()?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("updateTabNavigation", |_| {
        navigation()?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("revealActiveTab", |a| {
        reveal(a.get(0))?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("initTabNavigation", |_| {
        init_navigation()?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("showView", |a| {
        show_view(a.get(0), truthy(&a.get(1)))?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("addTermTab", |a| {
        add(a.get(0), a.get(1), truthy(&a.get(2)))?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("askCloseTab", |a| {
        ask_close(a.get(0))?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("askCloseGroup", |a| Ok(promise(ask_group(a.get(0)))))?;
    publish("openTerm", |a| {
        let sess = a.get(0);
        let label = a.get(1);
        if truthy(&global("WEBTERM")) && truthy(&sess) {
            add(sess.clone(), label.clone(), true)?;
            fire(
                run(
                    "api",
                    &[
                        "/tab-register".into(),
                        item(&[
                            ("session", sess.clone()),
                            ("label", default(label, sess.clone())),
                        ])?,
                    ],
                ),
                false,
            );
            show_view(js(&format!("term:{}", text(&sess))), true)?;
        }
        Ok(JsValue::UNDEFINED)
    })?;
    publish("closeTerm", |a| {
        close(a.get(0))?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("saveDeviceFocus", |a| {
        save_focus(a.get(0))?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("restoreDeviceFocus", |_| restore_focus())?;
    publish("loadDesktopTabs", |_| {
        Ok(promise(async {
            load_tabs().await?;
            Ok(JsValue::UNDEFINED)
        }))
    })?;
    publish("refreshDesktopTabs", |_| {
        refresh_tabs()?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("initApp", |_| {
        init_app()?;
        Ok(JsValue::UNDEFINED)
    })
}
