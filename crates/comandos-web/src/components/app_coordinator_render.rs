use super::*;
fn render(list: JsValue) -> Result<(), JsValue> {
    if !is_app() {
        remember_focus(&list)?;
    }
    if global("renderSessionOverview").is_function() {
        run("renderSessionOverview", std::slice::from_ref(&list))?;
    }
    set(&state(), "list", &list)?;
    let mut seen = Vec::new();
    let mut waiting = 0;
    let mut done = 0;
    let mut working = 0;
    for it in values(&list) {
        let rk = run("rowKey", std::slice::from_ref(&it))?;
        if seen.contains(&rk) {
            continue;
        }
        seen.push(rk);
        let status = get(&it, "status");
        match text(&status).as_str() {
            "waiting" => waiting += 1,
            "done" => done += 1,
            "working" => working += 1,
            _ => {}
        }
        let sess = get(&it, "session");
        let prev = call(&get(&state(), "prev"), "get", std::slice::from_ref(&sess))?;
        if prev != status && !matches!(text(&status).as_str(), "waiting" | "done") {
            run("alertClear", std::slice::from_ref(&sess))?;
        }
        call(&get(&state(), "prev"), "set", &[sess, status])?;
    }
    set(&id("n-waiting"), "textContent", &waiting.into())?;
    set(&id("n-done"), "textContent", &done.into())?;
    set(&id("n-working"), "textContent", &working.into())?;
    set(
        &doc(),
        "title",
        &if waiting > 0 {
            format!("({waiting}) ComandOS")
        } else {
            "ComandOS".into()
        }
        .into(),
    )?;
    favicon(if waiting > 0 {
        "#FFAE1A"
    } else if working > 0 {
        "#7AA5FF"
    } else {
        "#2EE59D"
    })?;
    let alive = new("Set", &[])?;
    let ssh = new("Set", &[])?;
    for it in values(&list) {
        if truthy(&get(&it, "alive")) {
            call(&alive, "add", &[get(&it, "session")])?;
        }
        if get(&it, "sshConnected") == JsValue::TRUE {
            call(&ssh, "add", &[get(&it, "session")])?;
        }
    }
    set(&state(), "liveSess", &alive)?;
    for chip in all(&doc(), ".ssh-chip") {
        classes(
            &chip,
            "live",
            truthy(&call(
                &ssh,
                "has",
                &[js(&format!("ssh-{}", text(&get(&chip, "textContent"))))],
            )?),
        );
    }
    if has_class(&body(), "app") {
        run("refreshDesktopTabs", &[])?;
        run("renderTabbar", &[])?;
    }
    Ok(())
}
fn notify(it: JsValue, verb: JsValue) -> Result<(), JsValue> {
    let notification = global("Notification");
    if !truthy(&get(&get(&state(), "cfg"), "notif"))
        || text(&get(&notification, "permission")) != "granted"
        || !truthy(&get(&doc(), "hidden"))
    {
        return Ok(());
    }
    let sess = get(&it, "session");
    let n = new(
        "Notification",
        &[
            js(&format!("{} {}", text(&get(&it, "project")), text(&verb))),
            item(&[
                ("body", default(get(&it, "detail"), "".into())),
                ("tag", sess.clone()),
            ])?,
        ],
    )?;
    let notice = n.clone();
    set(
        &n,
        "onclick",
        &function(move |_| {
            run("focus", &[])?;
            let escaped = call(&global("CSS"), "escape", std::slice::from_ref(&sess))?;
            let el = query(&doc(), &format!("[data-session=\"{}\"]", text(&escaped)));
            if truthy(&el) {
                call(
                    &el,
                    "scrollIntoView",
                    &[item(&[("block", "center".into())])?],
                )?;
                let input = query(&el, "input");
                if truthy(&input) {
                    call(&input, "focus", &[])?;
                }
            }
            call(&notice, "close", &[])?;
            Ok(JsValue::UNDEFINED)
        }),
    )
}
fn favicon(color: &str) -> Result<(), JsValue> {
    if text(&global("favColor")) == color {
        return Ok(());
    }
    put("favColor", color)?;
    let canvas = call(&doc(), "createElement", &["canvas".into()])?;
    set(&canvas, "width", &32.into())?;
    set(&canvas, "height", &32.into())?;
    let context = call(&canvas, "getContext", &["2d".into()])?;
    set(&context, "fillStyle", &"#0A0D13".into())?;
    call(
        &context,
        "fillRect",
        &[0.into(), 0.into(), 32.into(), 32.into()],
    )?;
    set(&context, "fillStyle", &color.into())?;
    call(&context, "beginPath", &[])?;
    call(
        &context,
        "arc",
        &[16.into(), 16.into(), 9.into(), 0.into(), 7.into()],
    )?;
    call(&context, "fill", &[])?;
    let mut link = query(&doc(), "link[rel=icon]");
    if !truthy(&link) {
        link = call(&doc(), "createElement", &["link".into()])?;
        set(&link, "rel", &"icon".into())?;
        call(
            &get(&doc(), "head"),
            "appendChild",
            std::slice::from_ref(&link),
        )?;
    }
    set(&link, "href", &call(&canvas, "toDataURL", &[])?)
}
pub(super) fn mount() -> Result<(), JsValue> {
    publish("render", |a| {
        render(a.get(0))?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("notify", |a| {
        notify(a.get(0), a.get(1))?;
        Ok(JsValue::UNDEFINED)
    })?;
    publish("favicon", |a| {
        favicon(&text(&a.get(0)))?;
        Ok(JsValue::UNDEFINED)
    })
}
