//! Browser adapter for the original helpers region; decisions are Rust.
use super::view::*;
use comandos_web_dom::{bridge::global_set, port::*};
use serde_json::{Value, json};
use wasm_bindgen::JsValue;
#[path = "foundation_helpers_notices.rs"]
mod notices;
#[path = "foundation_helpers_wizard.rs"]
mod wizard;
pub fn global(k: &str) -> JsValue {
    get(&js_sys::global(), k)
}
pub fn doc() -> JsValue {
    global("document")
}
pub fn id(k: &str) -> JsValue {
    call(&doc(), "getElementById", &[k.into()]).unwrap_or(JsValue::NULL)
}
pub fn query(root: &JsValue, q: &str) -> JsValue {
    call(root, "querySelector", &[q.into()]).unwrap_or(JsValue::NULL)
}
pub fn all(root: &JsValue, q: &str) -> Vec<JsValue> {
    let v = call(root, "querySelectorAll", &[q.into()]).unwrap_or(JsValue::NULL);
    (0..number(&get(&v, "length")) as u32)
        .map(|i| get(&v, &i.to_string()))
        .collect()
}
pub fn classes(el: &JsValue, k: &str, on: bool) {
    let _ = call(&get(el, "classList"), "toggle", &[k.into(), on.into()]);
}
pub fn has_class(el: &JsValue, k: &str) -> bool {
    truthy(&call(&get(el, "classList"), "contains", &[k.into()]).unwrap_or(JsValue::FALSE))
}
pub fn attr(el: &JsValue, k: &str, v: &str) {
    let _ = call(el, "setAttribute", &[k.into(), v.into()]);
}
pub fn style(el: &JsValue, k: &str, v: &str) {
    let _ = call(&get(el, "style"), "setProperty", &[k.into(), v.into()]);
}
pub fn html(el: &JsValue, v: String) {
    let _ = set(el, "innerHTML", &utf16_value(&v));
}
pub fn txt(el: &JsValue, v: String) {
    let _ = set(el, "textContent", &utf16_value(&v));
}
pub fn event(f: impl Fn(JsValue) -> Result<(), JsValue> + 'static) -> JsValue {
    function(move |a| {
        f(a.get(0))?;
        Ok(JsValue::UNDEFINED)
    })
}
pub fn listen(el: &JsValue, k: &str, f: JsValue) {
    let _ = call(el, "addEventListener", &[k.into(), f]);
}
pub fn later(f: JsValue, ms: f64) -> JsValue {
    call(&js_sys::global(), "setTimeout", &[f, ms.into()]).unwrap_or(JsValue::NULL)
}
pub fn cancel(t: JsValue) {
    let _ = call(&js_sys::global(), "clearTimeout", &[t]);
}
pub fn interval(ms: f64, f: impl Fn() -> Result<(), JsValue> + 'static) {
    let _ = call(
        &js_sys::global(),
        "setInterval",
        &[event(move |_| f()), ms.into()],
    );
}
pub fn closest(el: &JsValue, k: &str) -> JsValue {
    call(el, "closest", &[k.into()]).unwrap_or(JsValue::NULL)
}
pub fn data(el: &JsValue, k: &str) -> String {
    let v = get(&get(el, "dataset"), k);
    if v.is_undefined() || v.is_null() {
        String::new()
    } else {
        utf16_string(&v)
    }
}
pub fn state() -> JsValue {
    invoke(&global("__comandosState"), &[]).unwrap_or_else(|_| global("S"))
}
pub fn en() -> bool {
    global("L").as_string().as_deref() == Some("en")
}
pub fn tf(es: &str, en: &str) -> String {
    invoke(&global("tf"), &[utf16_value(es), utf16_value(en)])
        .map(|v| utf16_string(&v))
        .unwrap_or(es.into())
}
pub fn icon(k: &str, size: f64) -> String {
    invoke(&global("svg"), &[k.into(), size.into()])
        .map(|v| utf16_string(&v))
        .unwrap_or_default()
}
pub fn toast(message: String, bad: bool) {
    let _ = invoke(&global("toast"), &[utf16_value(&message), bad.into()]);
}
pub fn json_global(k: &str) -> Value {
    to_utf16_json(&global(k))
}
pub fn set_global(k: &str, v: &Value) {
    if let Ok(v) = from_utf16_json(v) {
        let _ = global_set(k, &v);
    }
}
pub async fn api(path: &str, body: Option<Value>) -> Result<JsValue, JsValue> {
    let mut args = vec![utf16_value(path)];
    if let Some(body) = body {
        args.push(from_utf16_json(&body)?);
    }
    wait(invoke(&global("api"), &args)).await
}
pub fn err(e: JsValue) -> String {
    let m = get(&e, "message");
    if truthy(&m) {
        utf16_string(&m)
    } else {
        utf16_string(&e)
    }
}
pub fn promise(
    f: impl std::future::Future<Output = Result<JsValue, JsValue>> + 'static,
) -> JsValue {
    wasm_bindgen_futures::future_to_promise(f).into()
}
pub fn call_global(k: &str, args: &[JsValue]) -> JsValue {
    invoke(&global(k), args).unwrap_or(JsValue::UNDEFINED)
}
pub fn render_list() {
    call_global("render", &[get(&state(), "list")]);
}
pub fn now() -> f64 {
    js_sys::Date::now()
}
pub fn usage_key(session: &JsValue, pane: &JsValue) -> String {
    format!(
        "{}\u{1f}{}",
        if truthy(session) {
            utf16_string(session)
        } else {
            String::new()
        },
        if truthy(pane) {
            utf16_string(pane)
        } else {
            String::new()
        }
    )
}
fn fixed(n: f64, d: u8) -> String {
    js_sys::Number::from(n)
        .to_fixed(d)
        .map(|v| v.into())
        .unwrap_or_default()
}
fn money(n: f64) -> String {
    if n == 0.0 || n.is_nan() {
        "$0.00".into()
    } else {
        format!("${}", fixed(n, if n < 0.01 { 4 } else { 2 }))
    }
}
fn tokens(n: f64) -> String {
    if n >= 1e9 {
        format!("{}B tok", fixed(n / 1e9, 2))
    } else if n >= 1e6 {
        format!("{}M tok", fixed(n / 1e6, 1))
    } else {
        fmt_tokens(n)
    }
}
pub fn reset(ts: f64) -> String {
    if ts == 0.0 || ts.is_nan() {
        return String::new();
    }
    let d: JsValue = js_sys::Date::new(&(ts * 1000.0).into()).into();
    let now: JsValue = js_sys::Date::new_0().into();
    let options = from_json(&json!({"hour":"2-digit","minute":"2-digit"})).unwrap_or(JsValue::NULL);
    let hm = call(
        &d,
        "toLocaleTimeString",
        &[js_sys::Array::new().into(), options],
    )
    .ok()
    .map(|v| utf16_string(&v))
    .unwrap_or_default();
    if call(&d, "toDateString", &[]).ok() == call(&now, "toDateString", &[]).ok() {
        return format!("{} {hm}", tf("hoy", "today"));
    }
    let opts = if (ts * 1000.0 - number(&now)) / 86400000.0 < 6.0 {
        json!({"weekday":"short"})
    } else {
        json!({"day":"numeric","month":"short"})
    };
    let date = call(
        &d,
        "toLocaleDateString",
        &[
            js_sys::Array::new().into(),
            from_json(&opts).unwrap_or(JsValue::NULL),
        ],
    )
    .ok()
    .map(|v| utf16_string(&v))
    .unwrap_or_default();
    format!("{date} {hm}")
}
fn index_usage(value: JsValue) -> Result<(), JsValue> {
    let state = state();
    set(
        &state,
        "usage",
        &if truthy(&value) {
            value.clone()
        } else {
            JsValue::NULL
        },
    )?;
    let map: JsValue = js_sys::Map::new().into();
    for p in js_rows(&get(&value, "panes")) {
        call(
            &map,
            "set",
            &[
                usage_key(&get(&p, "tmux_session"), &get(&p, "tmux_pane")).into(),
                p,
            ],
        )?;
    }
    for project in js_rows(&get(&value, "projects")) {
        for p in js_rows(&get(&project, "panes")) {
            call(
                &map,
                "set",
                &[
                    usage_key(&get(&p, "tmux_session"), &get(&p, "tmux_pane")).into(),
                    p.clone(),
                ],
            )?;
            let key = usage_key(&get(&p, "tmux_session"), &"".into());
            if !truthy(&call(&map, "has", &[key.clone().into()])?) {
                call(&map, "set", &[key.into(), p])?;
            }
        }
    }
    set(&state, "usageByPane", &map)
}
pub fn js_rows(v: &JsValue) -> Vec<JsValue> {
    if js_sys::Array::is_array(v) {
        js_sys::Array::from(v).iter().collect()
    } else {
        vec![]
    }
}
fn usage_item(item: &JsValue) -> JsValue {
    if !truthy(item) {
        return JsValue::NULL;
    }
    call(
        &get(&state(), "usageByPane"),
        "get",
        &[usage_key(&get(item, "session"), &get(item, "pane")).into()],
    )
    .ok()
    .filter(truthy)
    .unwrap_or(JsValue::NULL)
}
fn chip(u: &Value, title: bool) -> String {
    if u.is_null() {
        return String::new();
    }
    if title {
        return [
            if truth(at(u, "provider")) {
                s(at(u, "provider"))
            } else {
                s(at(u, "agent"))
            },
            short_model(&s(at(u, "model"))),
            confidence(&s(at(u, "confidence"))).into(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    }
    let mut bits = vec![];
    if truth(at(u, "model")) {
        bits.push(short_model(&s(at(u, "model"))));
    }
    let effort = if truth(at(u, "reasoning_effort")) {
        at(u, "reasoning_effort")
    } else {
        at(u, "effort")
    };
    if truth(effort) {
        bits.push(s(effort));
    }
    if num(at(u, "cost_usd")) > 0.0 {
        bits.push(money(num(at(u, "cost_usd"))));
    } else if num(at(u, "total_tokens")) > 0.0 {
        bits.push(format!(
            "{}{}",
            tokens(num(at(u, "total_tokens"))),
            if at(u, "usage_window").as_str() == Some("24h") {
                tf(" hoy", " today")
            } else {
                String::new()
            }
        ));
    }
    bits.into_iter().take(3).collect::<Vec<_>>().join(" · ")
}
pub fn provider(agent: &str, model: &str) -> JsValue {
    for p in js_rows(&get(&global("MT"), "providers")) {
        let regex = js_sys::Reflect::construct(
            &global("RegExp").into(),
            &[get(&p, "match"), "i".into()]
                .into_iter()
                .collect::<js_sys::Array>(),
        );
        if let Ok(r) = regex
            && truthy(
                &call(&r, "test", &[utf16_value(&format!("{agent} {model}"))])
                    .unwrap_or(JsValue::FALSE),
            )
        {
            return p;
        }
    }
    JsValue::NULL
}
pub fn provider_mark(agent: &str, model: &str, size: f64) -> String {
    let p = provider(agent, model);
    let name = get(&p, "icon").as_string().unwrap_or_default();
    if truthy(&p) && truthy(&get(&global("ICON"), &name)) {
        return format!(
            "<span class=\"pmark\" title=\"{}\">{}</span>",
            get(&p, "label").as_string().unwrap_or_default(),
            icon(&name, size)
        );
    }
    let input = if !agent.is_empty() { agent } else { model };
    let units = comandos_web_view::utf16::decode(&short_model(input));
    let mono = comandos_web_view::utf16::encode(units.into_iter().take(2)).to_uppercase();
    if mono.is_empty() {
        String::new()
    } else {
        format!("<span class=\"pmark pmono\" title=\"{input}\">{mono}</span>")
    }
}
fn tier(model: &str) -> JsValue {
    let mt = global("MT");
    if !truthy(&mt) || model.is_empty() {
        return "".into();
    }
    for p in js_rows(&get(&mt, "patterns")) {
        if let Ok(r) = js_sys::Reflect::construct(
            &global("RegExp").into(),
            &[get(&p, "match"), "i".into()]
                .into_iter()
                .collect::<js_sys::Array>(),
        ) && truthy(&call(&r, "test", &[utf16_value(model)]).unwrap_or(JsValue::FALSE))
        {
            return if truthy(&get(&p, "tier")) {
                get(&p, "tier")
            } else {
                "".into()
            };
        }
    }
    if truthy(&get(&mt, "defaultTier")) {
        get(&mt, "defaultTier")
    } else {
        "".into()
    }
}
pub fn pick(items: JsValue) -> JsValue {
    let state = state();
    let active = call_global("sidebarActiveTab", &[]);
    let selected = get(&state, "sel").as_string().unwrap_or_default();
    let result = pick_sel(
        &to_utf16_json(&items),
        &selected,
        number(&get(&state, "selTs")),
        &to_utf16_json(&active),
    );
    let key = row_key(&result);
    js_rows(&items)
        .into_iter()
        .find(|it| row_key(&to_utf16_json(it)) == key)
        .unwrap_or(JsValue::NULL)
}
fn limits(state: &Value) {
    let strip = id("limits-strip");
    if strip.is_null() {
        return;
    }
    let limits = list(at(state, "limits"));
    let mut pills = vec![];
    for prov in ["claude", "codex"] {
        let mine: Vec<_> = limits
            .iter()
            .filter(|l| !truth(at(l, "account")) || at(l, "account").as_str() == Some("main"))
            .collect();
        let session = mine
            .iter()
            .find(|l| {
                at(l, "provider").as_str() == Some(prov) && at(l, "window").as_str() == Some("5h")
            })
            .copied();
        let week = mine
            .iter()
            .find(|l| {
                at(l, "provider").as_str() == Some(prov)
                    && at(l, "window").as_str() == Some("7d")
                    && !truth(at(l, "scope"))
            })
            .copied();
        if session.is_none() && week.is_none() {
            continue;
        }
        let worst = session
            .map(|v| num(at(v, "percent")))
            .unwrap_or(0.0)
            .max(week.map(|v| num(at(v, "percent"))).unwrap_or(0.0));
        let mut tip = vec![];
        for (l, label) in [
            (session, tf("Sesion 5h", "5h session")),
            (week, tf("Semana", "Week")),
        ] {
            if let Some(l) = l {
                tip.push(format!(
                    "{label}: {} ({} {})",
                    fmt_percent(at(l, "percent")),
                    tf("resetea", "resets"),
                    reset(num(at(l, "resets_at")))
                ));
            }
        }
        pills.push(format!(
            "<span class=\"pill lim {}\" title=\"{} — {}\">{} <b>{}</b> · sem <b>{}</b></span>",
            limit_class(worst),
            attr_esc(prov),
            attr_esc(&tip.join(" · ")),
            md_esc(prov),
            fmt_percent(session.map(|l| at(l, "percent")).unwrap_or(&Value::Null)),
            fmt_percent(week.map(|l| at(l, "percent")).unwrap_or(&Value::Null))
        ));
    }
    if let Some(g) = limits
        .iter()
        .find(|l| at(l, "provider").as_str() == Some("grok"))
    {
        if !at(g, "percent").is_null() {
            pills.push(format!(
                "<span class=\"pill lim {}\" title=\"grok — {}\">grok <b>{}</b> sem</span>",
                limit_class(num(at(g, "percent"))),
                attr_esc(&format!(
                    "{}: {} ({} {})",
                    tf("Semana", "Week"),
                    fmt_percent(at(g, "percent")),
                    tf("resetea", "resets"),
                    reset(num(at(g, "resets_at")))
                )),
                fmt_percent(at(g, "percent"))
            ));
        } else if truth(at(g, "tokens_today")) || truth(at(g, "tokens_7d")) {
            let today = truth(at(g, "tokens_today"));
            pills.push(format!(
                "<span class=\"pill lim lim-ok\" title=\"grok — {}\">grok <b>{}</b> {}</span>",
                attr_esc(&tf(
                    &format!("medido local: {} en 7d", tokens(num(at(g, "tokens_7d")))),
                    &format!(
                        "measured locally: {} in 7d",
                        tokens(num(at(g, "tokens_7d")))
                    )
                )),
                md_esc(&tokens(num(if today {
                    at(g, "tokens_today")
                } else {
                    at(g, "tokens_7d")
                }))),
                if today {
                    tf("hoy", "today")
                } else {
                    "7d".into()
                }
            ));
        }
    }
    html(&strip, pills.join(""));
    classes(&strip, "hidden", pills.is_empty());
}
fn limit_class(n: f64) -> &'static str {
    if n >= 90.0 {
        "lim-danger"
    } else if n >= 70.0 {
        "lim-warn"
    } else {
        "lim-ok"
    }
}
pub fn apply_favorites(favorites: JsValue) -> Result<(), JsValue> {
    let next: JsValue = js_sys::Set::new(&JsValue::UNDEFINED).into();
    for sess in js_rows(&favorites) {
        if sess.as_string().as_deref() != Some("local") {
            call(&next, "add", &[sess])?;
        }
    }
    let pending = js_sys::Array::from(&global("favoritePending"));
    for row in pending.iter() {
        let row = js_sys::Array::from(&row);
        call(
            &next,
            if truthy(&row.get(1)) { "add" } else { "delete" },
            &[row.get(0)],
        )?;
    }
    let current = get(&state(), "favs");
    let changed = get(&next, "size") != get(&current, "size")
        || js_sys::Array::from(&next)
            .iter()
            .any(|s| !truthy(&call(&current, "has", &[s]).unwrap_or(JsValue::FALSE)));
    set(&state(), "favs", &next)?;
    if changed {
        render_list();
    }
    Ok(())
}
fn update_favorite(button: JsValue, session: String, label: String) -> Result<(), JsValue> {
    let on = truthy(
        &call(&get(&state(), "favs"), "has", &[utf16_value(&session)]).unwrap_or(JsValue::FALSE),
    );
    classes(&button, "on", on);
    let wm = global("ComandosWorkMarks");
    let want = if on && truthy(&wm) { "svg" } else { "☆" };
    if data(&button, "glyph") != want {
        set(&get(&button, "dataset"), "glyph", &want.into())?;
        if want == "svg" {
            set(
                &button,
                "innerHTML",
                &call(&wm, "iconSvg", &["favorite".into(), 14.into()])?,
            )?;
        } else {
            txt(&button, "☆".into());
        }
    }
    classes(&button, "wm-animated", on && truthy(&wm));
    set(
        &button,
        "disabled",
        &(session == "local"
            || truthy(
                &call(&global("favoritePending"), "has", &[utf16_value(&session)])
                    .unwrap_or(JsValue::FALSE),
            ))
        .into(),
    )?;
    let title = if session == "local" {
        tf("LOCAL siempre va primero", "LOCAL always comes first")
    } else if on {
        tf("Quitar de favoritos", "Remove from favorites")
    } else {
        tf("Marcar como favorita", "Add to favorites")
    };
    set(&button, "title", &utf16_value(&title))?;
    attr(&button, "aria-label", &format!("{title} · {label}"));
    attr(&button, "aria-pressed", if on { "true" } else { "false" });
    Ok(())
}
fn favorite(session: String, on: bool) -> Result<JsValue, JsValue> {
    if session.is_empty()
        || session == "local"
        || truthy(
            &call(&global("favoritePending"), "has", &[utf16_value(&session)])
                .unwrap_or(JsValue::FALSE),
        )
    {
        return Ok(js_sys::Promise::resolve(&JsValue::UNDEFINED).into());
    }
    let before = truthy(
        &call(&get(&state(), "favs"), "has", &[utf16_value(&session)]).unwrap_or(JsValue::FALSE),
    );
    call(
        &global("favoritePending"),
        "set",
        &[utf16_value(&session), on.into()],
    )?;
    bump_favorite();
    apply_favorites(js_sys::Array::from(&get(&state(), "favs")).into())?;
    let previous = global("favoriteQueue");
    let write = promise(async move {
        let _ = wait(Ok(previous)).await;
        let result = api(
            "/prefs-set",
            Some(json!({"favorite":{"session":session,"enabled":on}})),
        )
        .await;
        call(
            &global("favoritePending"),
            "delete",
            &[utf16_value(&session)],
        )?;
        bump_favorite();
        match result {
            Ok(r) => {
                apply_favorites(get(&r, "favorites"))?;
                render_list();
                Ok(JsValue::UNDEFINED)
            }
            Err(e) => {
                call(
                    &get(&state(), "favs"),
                    if before { "add" } else { "delete" },
                    &[utf16_value(&session)],
                )?;
                render_list();
                Err(e)
            }
        }
    });
    global_set("favoriteQueue", &write)?;
    Ok(write)
}
fn bump_favorite() {
    let _ = global_set(
        "favoriteVersion",
        &(number(&global("favoriteVersion")) + 1.0).into(),
    );
}
async fn refresh_favorites() -> Result<JsValue, JsValue> {
    if truthy(&global("favoriteReadPending")) || now() - number(&global("favoriteReadAt")) < 5000.0
    {
        return Ok(JsValue::UNDEFINED);
    }
    global_set("favoriteReadAt", &now().into())?;
    global_set("favoriteReadPending", &true.into())?;
    let version = global("favoriteVersion");
    if let Ok(prefs) = api("/prefs", None).await
        && version == global("favoriteVersion")
    {
        let _ = apply_favorites(get(&prefs, "favorites"));
    }
    global_set("favoriteReadPending", &false.into())?;
    Ok(JsValue::UNDEFINED)
}
async fn copy_text(value: JsValue) -> Result<JsValue, JsValue> {
    if wait(call(
        &get(&global("navigator"), "clipboard"),
        "writeText",
        std::slice::from_ref(&value),
    ))
    .await
    .is_ok()
    {
        return Ok(true.into());
    }
    let ta = call(&doc(), "createElement", &["textarea".into()])?;
    set(&ta, "value", &value)?;
    style(&ta, "position", "fixed");
    style(&ta, "opacity", "0");
    call(
        &get(&doc(), "body"),
        "appendChild",
        std::slice::from_ref(&ta),
    )?;
    let _ = call(&ta, "select", &[]);
    let ok = call(&doc(), "execCommand", &["copy".into()]).unwrap_or(JsValue::FALSE);
    let _ = call(&ta, "remove", &[]);
    Ok(ok)
}
async fn tick_usage(force: bool) -> Result<JsValue, JsValue> {
    let n = now();
    if !force && n - number(&global("usagePollTs")) < 10000.0 {
        return Ok(JsValue::UNDEFINED);
    }
    global_set("usagePollTs", &n.into())?;
    if let Ok(usage) = api("/usage/state", None).await {
        index_usage(usage.clone())?;
        limits(&to_utf16_json(&usage));
        render_list();
    }
    Ok(JsValue::UNDEFINED)
}
pub fn mount() -> Result<(), JsValue> {
    let scope: JsValue = js_sys::global().into();
    for (k, v) in [
        (
            "LABEL",
            json!({"waiting":"Esperando respuesta","done":"Termino","working":"Trabajando","idle":"Viva, sin actividad","dead":"Apagada"}),
        ),
        ("NS", ns_defaults()),
        ("MT", Value::Null),
        ("PROVIDERS", Value::Null),
        ("MODEL_NEWS", Value::Null),
        ("ACTIVE_TAB", json!({"session":"","ts":0})),
        (
            "MODEL_CHOICES",
            serde_json::from_str(include_str!("foundation_helpers_models.json"))
                .map_err(|e| JsValue::from_str(&e.to_string()))?,
        ),
    ] {
        set_global(k, &v)
    }
    for name in ["MOTOR_PENDING", "SWITCH_SEEN", "favoritePending"] {
        global_set(name, &js_sys::Map::new().into())?;
    }
    for name in [
        "usagePollTs",
        "nsFsTimer",
        "nsFsSeq",
        "favoriteVersion",
        "favoriteReadAt",
    ] {
        global_set(name, &0.into())?;
    }
    for name in [
        "MOTOR_STATUS_BUSY",
        "favoriteReadPending",
        "ocModelsLoading",
        "termFallbackNotified",
    ] {
        global_set(name, &false.into())?;
    }
    for name in ["mdlMenuBtn", "ocModels"] {
        global_set(name, &JsValue::NULL)?;
    }
    global_set(
        "favoriteQueue",
        &js_sys::Promise::resolve(&JsValue::UNDEFINED).into(),
    )?;
    global_set(
        "FS_HOME_RE",
        &js_sys::RegExp::new("^/home/[^/]+", "").into(),
    )?;
    global_set(
        "PATH_LINK_RE",
        &js_sys::RegExp::new(
            "(~/[\\w.+@%,=-]+(?:/[\\w.+@%,=-]+)*|/[\\w.+@%,=-]+(?:/[\\w.+@%,=-]+)+)((?::\\d+)?)",
            "g",
        )
        .into(),
    )?;
    let location = global("location");
    let hostname = get(&location, "hostname").as_string().unwrap_or_default();
    let loopback = hostname.starts_with("127.")
        || hostname.starts_with("localhost")
        || hostname.starts_with("::1");
    let native_page = call(
        &global("document"),
        "querySelector",
        &["meta[name='comandos-web-mode'][content='native']".into()],
    )
    .is_ok_and(|meta| !meta.is_null() && !meta.is_undefined());
    let native_browser = native_page
        && !invoke(&global("inApp"), &[]).is_ok_and(|value| truthy(&value));
    let webterm = native_browser || (get(&location, "protocol").as_string().as_deref() == Some("https:")
        && !loopback)
        || (loopback && global("__COMANDOS_DEV_WEBTERM") == JsValue::TRUE);
    for (k, v) in [
        ("LOOPBACK", json!(loopback)),
        ("WEBTERM", json!(webterm)),
        (
            "TERM_BASE",
            json!(if webterm {
                format!(
                    "{}/term",
                    get(&location, "origin").as_string().unwrap_or_default()
                )
            } else {
                String::new()
            }),
        ),
        (
            "TERM_FALLBACK_BASE",
            json!(if webterm {
                format!("https://{hostname}:8443")
            } else {
                String::new()
            }),
        ),
        ("TERM_PRIMARY_ATTEMPTS", json!(3)),
        ("TERM_PRIMARY_RETRY_MS", json!(400)),
        ("TERM_PRIMARY_PROBE_TIMEOUT_MS", json!(800)),
    ] {
        set_global(k, &v)
    }
    for name in [
        "mdEsc",
        "attrEsc",
        "mdInline",
        "mdHtml",
        "linkifyPaths",
        "mdStrip",
        "shortPath",
        "shortModel",
        "confidenceLabel",
        "sessionStageLabel",
    ] {
        method(&scope, name, move |a| {
            let input = if truthy(&a.get(0)) {
                utf16_string(&a.get(0))
            } else {
                String::new()
            };
            let out = match name {
                "mdEsc" => md_esc(&input),
                "attrEsc" => attr_esc(&input),
                "mdInline" => md_inline(&input),
                "mdHtml" => md_html(&input, en()),
                "linkifyPaths" => linkify_paths(&input, en()),
                "mdStrip" => md_strip(&input),
                "shortPath" => short_path(&input),
                "shortModel" => short_model(&input),
                "confidenceLabel" => confidence(&input).into(),
                _ => stage_label(&to_utf16_json(&a.get(0))),
            };
            Ok(utf16_value(&out))
        })?;
    }
    method(&scope, "agoTxt", |a| {
        Ok(utf16_value(&ago_txt(
            number(&a.get(0)),
            now() / 1000.0,
            en(),
        )))
    })?;
    for name in [
        "fmtMoney",
        "fmtTokens",
        "fmtPercent",
        "fmtBytes",
        "fmtReset",
    ] {
        method(&scope, name, move |a| {
            let value = a.get(0);
            let n = if truthy(&value) { number(&value) } else { 0.0 };
            Ok(utf16_value(&match name {
                "fmtMoney" => money(n),
                "fmtTokens" => tokens(n),
                "fmtBytes" => fmt_bytes(n),
                "fmtReset" => reset(n),
                _ => {
                    if value.is_null() || value.is_undefined() {
                        "--".into()
                    } else {
                        format!("{}%", fixed(n, if n % 1.0 != 0.0 { 1 } else { 0 }))
                    }
                }
            }))
        })?;
    }
    method(&scope, "usagePaneKey", |a| {
        Ok(utf16_value(&usage_key(&a.get(0), &a.get(1))))
    })?;
    method(&scope, "indexUsage", |a| {
        index_usage(a.get(0))?;
        Ok(JsValue::UNDEFINED)
    })?;
    method(&scope, "usageForItem", |a| Ok(usage_item(&a.get(0))))?;
    for (name, title) in [("usageChipText", false), ("usageChipTitle", true)] {
        method(&scope, name, move |a| {
            Ok(utf16_value(&chip(&to_utf16_json(&a.get(0)), title)))
        })?;
    }
    method(&scope, "renderLimitsStrip", |a| {
        limits(&to_utf16_json(&a.get(0)));
        Ok(JsValue::UNDEFINED)
    })?;
    method(&scope, "renderUsage", |a| {
        index_usage(a.get(0))?;
        limits(&to_utf16_json(&a.get(0)));
        render_list();
        Ok(JsValue::UNDEFINED)
    })?;
    method(&scope, "providerOf", |a| {
        Ok(provider(
            &a.get(0).as_string().unwrap_or_default(),
            &a.get(1).as_string().unwrap_or_default(),
        ))
    })?;
    method(&scope, "providerMark", |a| {
        Ok(utf16_value(&provider_mark(
            &a.get(0).as_string().unwrap_or_default(),
            &a.get(1).as_string().unwrap_or_default(),
            a.get(2).as_f64().unwrap_or(12.0),
        )))
    })?;
    method(&scope, "tierOf", |a| {
        Ok(tier(&a.get(0).as_string().unwrap_or_default()))
    })?;
    method(&scope, "tierStyleOf", |a| {
        let v = get(
            &get(&global("MT"), "tiers"),
            &a.get(0).as_string().unwrap_or_default(),
        );
        Ok(if truthy(&v) { v } else { JsValue::NULL })
    })?;
    method(&scope, "pickSel", |a| Ok(pick(a.get(0))))?;
    method(&scope, "copyText", |a| Ok(promise(copy_text(a.get(0)))))?;
    method(&scope, "tickUsage", |a| {
        Ok(promise(tick_usage(truthy(&a.get(0)))))
    })?;
    method(&scope, "applyFavorites", |a| {
        apply_favorites(a.get(0))?;
        Ok(JsValue::UNDEFINED)
    })?;
    method(&scope, "setSessionFavorite", |a| {
        favorite(a.get(0).as_string().unwrap_or_default(), truthy(&a.get(1)))
    })?;
    method(&scope, "refreshFavorites", |_| {
        Ok(promise(refresh_favorites()))
    })?;
    method(&scope, "updateFavoriteButton", |a| {
        let session = a.get(1).as_string().unwrap_or_default();
        let label = a.get(2).as_string().unwrap_or(session.clone());
        update_favorite(a.get(0), session, label)?;
        Ok(JsValue::UNDEFINED)
    })?;
    wizard::mount()?;
    notices::mount()?;
    Ok(())
}
pub fn attach() -> Result<(), JsValue> {
    for (path, name) in [("/model-tiers", "MT"), ("/providers", "PROVIDERS")] {
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(v) = api(path, None).await {
                let _ = global_set(name, &if truthy(&v) { v } else { JsValue::NULL });
            }
        });
    }
    listen(
        &doc(),
        "click",
        event(|e| {
            let link = closest(&get(&e, "target"), ".pathlink");
            if !link.is_null() {
                call(&e, "preventDefault", &[])?;
                call(&e, "stopPropagation", &[])?;
                let path = data(&link, "path");
                wasm_bindgen_futures::spawn_local(async move {
                    if let Err(e) = api("/open-path", Some(json!({"path":path}))).await {
                        toast(err(e), true);
                    }
                });
            }
            Ok(())
        }),
    );
    wizard::attach()?;
    notices::attach()?;
    Ok(())
}
