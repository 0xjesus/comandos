//! Complete theme/settings coordinator region. No legacy scripts are evaluated.
#[cfg(not(target_arch = "wasm32"))]
pub fn mount() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}

#[cfg(all(test, target_arch = "wasm32"))]
#[allow(clippy::unwrap_used)]
mod wasm_tests {
    use super::*;
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen(inline_js = r#"
let themeContracts;
export async function themePreferencesReference(root) {
  const {createRequire}=await import('node:module');
  themeContracts=createRequire(root+'/tests/theme_preferences.cjs')(root+'/crates/comandos-web/tests/theme_preferences_contract.cjs');
  return JSON.stringify(await themeContracts.reference(root));
}
export function themePreferencesFixture(){themeContracts.install();}
export async function themePreferencesActual(){return JSON.stringify(await themeContracts.native());}
"#)]
    extern "C" {
        #[wasm_bindgen(catch)]
        async fn themePreferencesReference(root: &str) -> Result<JsValue, JsValue>;
        fn themePreferencesFixture();
        #[wasm_bindgen(catch)]
        async fn themePreferencesActual() -> Result<JsValue, JsValue>;
    }
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn complete_theme_preferences_match_original_coordinator() {
        let expected = themePreferencesReference(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
            .await
            .unwrap();
        themePreferencesFixture();
        mount().unwrap();
        attach().unwrap();
        let actual = themePreferencesActual().await.unwrap();
        assert_eq!(actual.as_string(), expected.as_string());
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_theme() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_favorites() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_prefs() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_tabs() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_ranges() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_preview() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach_theme() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach_tabs() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach_ranges() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach_preview() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(target_arch = "wasm32")]
pub use web::{
    attach, attach_preview, attach_ranges, attach_tabs, attach_theme, mount, mount_favorites,
    mount_prefs, mount_preview, mount_ranges, mount_tabs, mount_theme,
};
#[cfg(target_arch = "wasm32")]
mod web {
    use crate::components::web_support::*;
    use comandos_web_dom::port::*;
    use comandos_web_view::theme_preferences as model;
    use comandos_web_view::user_themes as ut;
    use js_sys::{Array, Reflect};
    use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
    // Temporary access to bindings owned by the helper/terminal coordinator.
    // These accessors also work against authoritative globals after its port.
    #[wasm_bindgen(
        inline_js = "export function themeTerms(){return typeof openTerms==='undefined'?undefined:openTerms;} export function themeFavoriteVersion(){return typeof favoriteVersion==='undefined'?undefined:favoriteVersion;} export function themeFavoriteReadAt(v){if(typeof favoriteReadAt!=='undefined')favoriteReadAt=v;else globalThis.favoriteReadAt=v;} export function themeRowsHold(v){if(typeof tabRowsHold!=='undefined')tabRowsHold=v;else globalThis.tabRowsHold=v;}"
    )]
    extern "C" {
        fn themeTerms() -> JsValue;
        fn themeFavoriteVersion() -> JsValue;
        fn themeFavoriteReadAt(v: &JsValue);
        fn themeRowsHold(v: &JsValue);
    }
    fn publish(
        name: &str,
        f: impl Fn(Array) -> Result<JsValue, JsValue> + 'static,
    ) -> Result<(), JsValue> {
        set(&js_sys::global(), name, &function(f))
    }
    fn void(f: impl Fn(Array) -> Result<(), JsValue> + 'static) -> JsValue {
        function(move |a| {
            f(a)?;
            Ok(JsValue::UNDEFINED)
        })
    }
    fn value(el: &JsValue) -> JsValue {
        get(el, "value")
    }
    fn text(el: &JsValue, v: &JsValue) -> Result<(), JsValue> {
        set(el, "textContent", v)
    }
    fn html(el: &JsValue, v: String) -> Result<(), JsValue> {
        set(el, "innerHTML", &v.into())
    }
    fn tr(es: String, en: String) -> String {
        invoke(&global("tf"), &[es.clone().into(), en.into()])
            .map(|v| string(&v))
            .unwrap_or(es)
    }
    fn invoke_void(name: &str, args: &[JsValue]) -> Result<(), JsValue> {
        invoke(&global(name), args).map(|_| ())
    }
    fn escaped(v: &JsValue, attribute: bool) -> Result<String, JsValue> {
        invoke(
            &global(if attribute { "attrEsc" } else { "mdEsc" }),
            std::slice::from_ref(v),
        )
        .map(|v| utf16_string(&v))
    }
    fn current_theme() -> String {
        let id = global("curThemeId");
        if truthy(&id) {
            string(&id)
        } else {
            string(&global("curTheme"))
        }
    }
    fn current_buttons() -> String {
        string(&global("curButtonStyle"))
    }
    fn message(kind: &str, key: &str, v: JsValue) -> Result<JsValue, JsValue> {
        let out = object();
        set(&out, "source", &"comandos".into())?;
        set(&out, "type", &kind.into())?;
        set(&out, key, &v)?;
        Ok(out)
    }
    fn frames(mut f: impl FnMut(JsValue) -> Result<(), JsValue>) -> Result<(), JsValue> {
        let terms = themeTerms();
        let values = call(&terms, "values", &[])?;
        for item in Array::from(&values).iter() {
            let frame = get(&item, "frame");
            if truthy(&frame) && get(&get(&frame, "dataset"), "compat") != JsValue::from_str("1") {
                f(frame)?;
            }
        }
        Ok(())
    }
    fn notify_app(key: &str, v: JsValue, tell: bool) -> Result<(), JsValue> {
        if tell && truthy(&invoke(&global("inApp"), &[])?) {
            let msg = object();
            set(&msg, key, &v)?;
            let handler = get(
                &get(&get(&global("window"), "webkit"), "messageHandlers"),
                "centro",
            );
            call(
                &handler,
                "postMessage",
                &[js_sys::JSON::stringify(&msg)?.into()],
            )?;
        }
        Ok(())
    }
    fn broadcast(name: JsValue) -> Result<(), JsValue> {
        let r = ut::resolve(&prefs(), &string(&name));
        let msg = message("theme", "theme", r.base.clone().into())?;
        if r.user {
            set(&msg, "palette", &from_json(&ut::term_palette(&r.colors))?)?;
        }
        frames(|frame| {
            invoke_void("styleTermFrame", std::slice::from_ref(&frame))?;
            let window = get(&frame, "contentWindow");
            if !window.is_null() && !window.is_undefined() {
                call(
                    &window,
                    "postMessage",
                    &[msg.clone(), get(&global("location"), "origin")],
                )?;
            }
            Ok(())
        })
    }
    fn broadcast_cursor(prefs: &JsValue) -> Result<(), JsValue> {
        let shape = get(prefs, "cursor_shape");
        if !matches!(
            shape.as_string().as_deref(),
            Some("block" | "ibeam" | "underline")
        ) {
            return Ok(());
        }
        frames(|frame| {
            let window = get(&frame, "contentWindow");
            if !window.is_null() && !window.is_undefined() {
                call(
                    &window,
                    "postMessage",
                    &[
                        message("cursor-style", "style", shape.clone())?,
                        get(&global("location"), "origin"),
                    ],
                )?;
            }
            Ok(())
        })
    }
    thread_local! {
        static PREFS: std::cell::RefCell<serde_json::Value> =
            const { std::cell::RefCell::new(serde_json::Value::Null) };
        static DRAFT: std::cell::RefCell<Option<ut::Draft>> =
            const { std::cell::RefCell::new(None) };
        static DELETE_ARMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    fn prefs() -> serde_json::Value {
        PREFS.with(|p| p.borrow().clone())
    }
    fn remember(p: &JsValue) {
        let v = to_json(p);
        if v.is_object() {
            PREFS.with(|slot| *slot.borrow_mut() = v);
        }
    }
    /// A user theme that exists, or the built-in name (unknown → noche).
    fn known(name: &str) -> String {
        if ut::is_user_id(name) && ut::find(&prefs(), name).is_some() {
            name.into()
        } else {
            model::theme(name).into()
        }
    }
    const CSS_VARS: [&str; 10] = [
        "--bg",
        "--panel",
        "--panel2",
        "--line",
        "--line2",
        "--text",
        "--dim",
        "--faint",
        "--brand",
        "--on-brand",
    ];
    /// Paint the page: the built-in base, plus a user theme's colours (or a
    /// draft being edited) as inline variables over it.
    fn paint(
        base: &str,
        colors: Option<&serde_json::Map<String, serde_json::Value>>,
        treatment: &str,
    ) -> Result<(), JsValue> {
        let meta = model::metadata();
        let m = meta.get(base).unwrap_or(&serde_json::Value::Null);
        let docel = get(&doc(), "documentElement");
        let data = get(&docel, "dataset");
        if base == "noche" {
            Reflect::delete_property(&data.clone().into(), &"theme".into())?;
        } else {
            set(&data, "theme", &base.into())?;
        }
        let treatment = if treatment == "solid" { "" } else { treatment };
        set(&data, "treatment", &treatment.into())?;
        let st = get(&docel, "style");
        match colors {
            Some(c) => {
                for (k, v) in ut::css_vars(c) {
                    call(&st, "setProperty", &[k.into(), v.into()])?;
                }
                let bg = c
                    .get("bg")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("#000000");
                let scheme = if ut::luminance(bg) > 0.35 {
                    "light"
                } else {
                    "dark"
                };
                set(&st, "colorScheme", &scheme.into())?;
            }
            None => {
                for k in CSS_VARS {
                    call(&st, "removeProperty", &[k.into()])?;
                }
                set(
                    &st,
                    "colorScheme",
                    &m["scheme"].as_str().unwrap_or_default().into(),
                )?;
            }
        }
        Ok(())
    }
    fn apply_theme(name: JsValue, tell: bool) -> Result<(), JsValue> {
        let input = name.as_string().unwrap_or_default();
        let name = known(&input);
        let r = ut::resolve(&prefs(), &name);
        // `curTheme` stays a built-in name: terminal pages and URLs read it.
        set(&js_sys::global(), "curTheme", &r.base.clone().into())?;
        set(&js_sys::global(), "curThemeId", &name.clone().into())?;
        let meta = model::metadata();
        let m = meta.get(&r.base).unwrap_or(&serde_json::Value::Null);
        paint(&r.base, r.user.then_some(&r.colors), &r.treatment)?;
        let color = query(&doc(), "meta[name=\"theme-color\"]");
        if truthy(&color) {
            let first = if r.user {
                r.colors.get("bg").and_then(serde_json::Value::as_str)
            } else {
                m.get("sw")
                    .and_then(|sw| sw.get(0))
                    .and_then(serde_json::Value::as_str)
            };
            attr(&color, "content", first.unwrap_or_default());
        }
        let btn = query(&doc(), "#btn-theme");
        html(&btn, icon(m["icon"].as_str().unwrap_or_default(), 15.0))?;
        let label = &r.label;
        set(
            &btn,
            "title",
            &tr(
                format!("Apariencia: {label}"),
                format!("Appearance: {label}"),
            )
            .into(),
        )?;
        render_themes()?;
        broadcast(name.clone().into())?;
        notify_app("theme", name.into(), tell)
    }
    fn apply_buttons(style: JsValue, tell: bool) -> Result<(), JsValue> {
        let input = style.as_string().unwrap_or_default();
        let name = model::button_style(&input);
        set(&js_sys::global(), "curButtonStyle", &name.into())?;
        set(
            &get(&get(&doc(), "documentElement"), "dataset"),
            "btnStyle",
            &name.into(),
        )?;
        render_buttons()?;
        frames(|frame| {
            let window = get(&frame, "contentWindow");
            if !window.is_null() && !window.is_undefined() {
                call(
                    &window,
                    "postMessage",
                    &[
                        message("button-style", "style", name.into())?,
                        get(&global("location"), "origin"),
                    ],
                )?;
            }
            Ok(())
        })?;
        notify_app("buttonStyle", name.into(), tell)
    }
    fn error_toast(e: JsValue) {
        let _ = invoke_void("toast", &[get(&e, "message"), true.into()]);
    }
    fn select_theme(name: JsValue) -> JsValue {
        let previous = current_theme();
        let applied = apply_theme(name.clone(), true);
        let user = ut::resolve(&prefs(), &string(&name));
        let fetched = applied.and_then(|()| {
            let patch = object();
            set(&patch, "theme", &name)?;
            if let Some(style) = user.button_style.as_deref().filter(|_| user.user) {
                set(&patch, "button_style", &style.into())?;
                apply_buttons(style.into(), true)?;
            }
            invoke(&global("api"), &["/prefs-set".into(), patch])
        });
        promise(async move {
            let result = async {
                wait(fetched).await?;
                let label = user.label.as_str();
                invoke_void(
                    "toast",
                    &[tr(format!("Tema: {label}"), format!("Theme: {label}")).into()],
                )
            }
            .await;
            if let Err(e) = result {
                apply_theme(previous.into(), true)?;
                error_toast(e);
            }
            Ok(JsValue::UNDEFINED)
        })
    }
    fn select_buttons(style: JsValue) -> JsValue {
        let previous = current_buttons();
        let fetched = apply_buttons(style.clone(), true).and_then(|()| {
            let patch = object();
            set(&patch, "button_style", &style)?;
            invoke(&global("api"), &["/prefs-set".into(), patch])
        });
        promise(async move {
            if let Err(e) = wait(fetched).await {
                apply_buttons(previous.into(), true)?;
                error_toast(e);
            }
            Ok(JsValue::UNDEFINED)
        })
    }
    fn render_buttons() -> Result<(), JsValue> {
        let g = query(&doc(), "#button-style-gallery");
        if !truthy(&g) {
            return Ok(());
        }
        html(
            &g,
            model::button_gallery(&current_buttons(), |n, s| icon(n, s.into())),
        )?;
        for b in all(&g, "[data-btn-style-id]") {
            let item = b.clone();
            listen(
                &b,
                "click",
                void(move |_| {
                    let _ = select_buttons(get(&get(&item, "dataset"), "btnStyleId"));
                    Ok(())
                }),
            );
        }
        Ok(())
    }
    fn render_themes() -> Result<(), JsValue> {
        let g = query(&doc(), "#theme-gallery");
        if !truthy(&g) {
            return Ok(());
        }
        let en = english();
        let markup = DRAFT.with(|d| {
            ut::gallery(
                &prefs(),
                &current_theme(),
                d.borrow().as_ref(),
                en,
                &|n, s| icon(n, s.into()),
            )
        });
        html(&g, markup)?;
        if !truthy(&get(&g, "_utWired")) {
            set(&g, "_utWired", &true.into())?;
            wire_gallery(&g)?;
        }
        Ok(())
    }
    /// Show the draft live on the page and in the preview, without
    /// re-rendering (a colour dialog may still be open).
    fn paint_draft() -> Result<(), JsValue> {
        let Some(d) = DRAFT.with(|d| d.borrow().clone()) else {
            return Ok(());
        };
        let show = d.from.user || d.edited();
        paint(&d.from.base, show.then_some(&d.colors), &d.treatment)?;
        let pv = query(&doc(), "[data-ut-pv]");
        if truthy(&pv) {
            let fresh = ut::preview(&d);
            let holder = call(&doc(), "createElement", &["div".into()])?;
            html(&holder, fresh)?;
            let next = get(&holder, "firstElementChild");
            if truthy(&next) {
                call(&pv, "replaceWith", &[next])?;
            }
        }
        for el in all(&doc(), "[data-ut-hex]") {
            let key = string(&get(&get(&el, "dataset"), "utHex"));
            let v = color_of(&d.colors, &key);
            text(&el, &v.into())?;
        }
        for el in all(&doc(), "[data-ut-cr]") {
            let key = string(&get(&get(&el, "dataset"), "utCr"));
            if let Some(r) = ut::contrast(&d.colors, &key) {
                let (cls, sym) = ut::mark(r);
                attr(&el, "class", &format!("ut-cr {cls}"));
                attr(&el, "title", &ut::mark_text(r, english()));
                text(&el, &format!("{sym} {r:.1}").into())?;
            }
        }
        for el in all(&doc(), "[data-ut-edited]") {
            set(&el, "hidden", &(!d.edited()).into())?;
        }
        Ok(())
    }
    fn color_of(c: &serde_json::Map<String, serde_json::Value>, key: &str) -> String {
        if let Some(i) = key
            .strip_prefix("pal")
            .and_then(|i| i.parse::<usize>().ok())
        {
            return c["pal"]
                .get(i)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .into();
        }
        c.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .into()
    }
    fn open_draft(id: &str) {
        let d = ut::Draft::new(ut::resolve(&prefs(), id), &current_buttons());
        DRAFT.with(|slot| *slot.borrow_mut() = Some(d));
        DELETE_ARMED.with(|a| a.set(false));
    }
    fn close_draft() -> Result<(), JsValue> {
        DRAFT.with(|slot| *slot.borrow_mut() = None);
        apply_theme(current_theme().into(), false)
    }
    fn data_of(el: &JsValue, key: &str) -> Option<String> {
        let v = get(&get(el, "dataset"), key);
        (!v.is_undefined() && !v.is_null()).then(|| string(&v))
    }
    fn wire_gallery(g: &JsValue) -> Result<(), JsValue> {
        listen(
            g,
            "click",
            void(move |a| {
                let target = get(&a.get(0), "target");
                let hit = call(&target, "closest", &["button".into()])?;
                if !truthy(&hit) {
                    return Ok(());
                }
                if let Some(id) = data_of(&hit, "themeId") {
                    let same = DRAFT.with(|d| d.borrow().as_ref().is_some_and(|d| d.from.id == id));
                    if same {
                        return close_draft();
                    }
                    open_draft(&id);
                    if id == current_theme() {
                        return render_themes();
                    }
                    let _ = select_theme(id.into());
                    return Ok(());
                }
                if hit_has(&hit, "data-ut-close") {
                    return close_draft();
                }
                if let Some(tab) = data_of(&hit, "utTab") {
                    DRAFT.with(|d| {
                        if let Some(d) = d.borrow_mut().as_mut() {
                            d.tab = tab;
                        }
                    });
                    return render_themes();
                }
                if let Some(t) = data_of(&hit, "utTreatment") {
                    DRAFT.with(|d| {
                        if let Some(d) = d.borrow_mut().as_mut() {
                            d.treatment = t;
                        }
                    });
                    render_themes()?;
                    return paint_draft();
                }
                if let Some(style) = data_of(&hit, "utButtons") {
                    let user = DRAFT.with(|d| {
                        let mut d = d.borrow_mut();
                        let Some(d) = d.as_mut() else { return false };
                        d.button_style = style.clone();
                        d.from.user
                    });
                    if user {
                        apply_buttons(style.into(), false)?;
                        render_themes()?;
                        return paint_draft();
                    }
                    let _ = select_buttons(style.into());
                    return render_themes();
                }
                if hit_has(&hit, "data-ut-undo") {
                    let id = DRAFT.with(|d| d.borrow().as_ref().map(|d| d.from.id.clone()));
                    if let Some(id) = id {
                        open_draft(&id);
                        apply_theme(current_theme().into(), false)?;
                        apply_buttons(current_buttons().into(), false)?;
                    }
                    return Ok(());
                }
                if let Some(kind) = data_of(&hit, "utSave") {
                    let _ = save_draft(kind == "here");
                    return Ok(());
                }
                if hit_has(&hit, "data-ut-delete") {
                    if !DELETE_ARMED.with(|a| a.replace(true)) {
                        text(
                            &hit,
                            &tr(
                                "¿Borrar? Clic otra vez".into(),
                                "Delete? Click again".into(),
                            )
                            .into(),
                        )?;
                        return Ok(());
                    }
                    let _ = delete_draft();
                }
                Ok(())
            }),
        );
        listen(
            g,
            "input",
            void(move |a| {
                let target = get(&a.get(0), "target");
                let Some(key) = data_of(&target, "utColor") else {
                    return Ok(());
                };
                let value = string(&get(&target, "value"));
                let changed = DRAFT.with(|d| {
                    d.borrow_mut()
                        .as_mut()
                        .is_some_and(|d| d.set_color(&key, &value))
                });
                if changed {
                    paint_draft()?;
                }
                Ok(())
            }),
        );
        listen(
            g,
            "keydown",
            void(move |a| {
                let e = a.get(0);
                let key = string(&get(&e, "key"));
                let target = get(&e, "target");
                if !truthy(&get(&get(&target, "dataset"), "themeId")) {
                    return Ok(());
                }
                if !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].contains(&key.as_str()) {
                    return Ok(());
                }
                call(&e, "preventDefault", &[])?;
                let cards = all(&doc(), "#theme-gallery [data-theme-id]");
                let at = cards.iter().position(|c| c == &target).unwrap_or(0) as i32;
                let delta = if ["ArrowLeft", "ArrowUp"].contains(&key.as_str()) {
                    -1
                } else {
                    1
                };
                let n = cards.len() as i32;
                if n > 0 {
                    call(&cards[((at + delta + n) % n) as usize], "focus", &[])?;
                }
                Ok(())
            }),
        );
        Ok(())
    }
    fn hit_has(el: &JsValue, name: &str) -> bool {
        call(el, "hasAttribute", &[name.into()]).is_ok_and(|v| truthy(&v))
    }
    async fn refresh_prefs() -> Result<(), JsValue> {
        let p = wait(invoke(&global("api"), &["/prefs".into()])).await?;
        remember(&p);
        Ok(())
    }
    fn save_draft(here: bool) -> JsValue {
        promise(async move {
            let result = async {
                let Some(d) = DRAFT.with(|d| d.borrow().clone()) else {
                    return Ok(());
                };
                let input = query(&doc(), "#ut-name");
                let label = string(&get(&input, "value")).trim().to_string();
                let id = if here && d.from.user {
                    d.from.id.clone()
                } else {
                    ut::new_id(js_sys::Date::now() as u64)
                };
                let theme = d.theme(&id, &label);
                if let Err(e) = ut::validate(&theme) {
                    invoke_void("toast", &[e.into(), true.into()])?;
                    return Ok(());
                }
                let patch = from_json(&serde_json::json!({
                    "user_theme": theme,
                    "theme": id,
                    "button_style": d.button_style,
                }))?;
                wait(invoke(&global("api"), &["/prefs-set".into(), patch])).await?;
                refresh_prefs().await?;
                open_draft(&id);
                apply_theme(id.clone().into(), true)?;
                apply_buttons(d.button_style.clone().into(), true)?;
                invoke_void(
                    "toast",
                    &[tr(format!("Guardado: {label}"), format!("Saved: {label}")).into()],
                )
            }
            .await;
            if let Err(e) = result {
                error_toast(e);
            }
            Ok(JsValue::UNDEFINED)
        })
    }
    fn delete_draft() -> JsValue {
        promise(async move {
            let result = async {
                let Some(d) = DRAFT.with(|d| d.borrow().clone()) else {
                    return Ok(());
                };
                let patch = from_json(&serde_json::json!({"user_theme_delete": d.from.id}))?;
                wait(invoke(&global("api"), &["/prefs-set".into(), patch])).await?;
                refresh_prefs().await?;
                DRAFT.with(|slot| *slot.borrow_mut() = None);
                let now = prefs()["theme"].as_str().unwrap_or("noche").to_string();
                apply_theme(now.into(), true)?;
                invoke_void(
                    "toast",
                    &[tr(
                        format!("Borrado: {}", d.from.label),
                        format!("Deleted: {}", d.from.label),
                    )
                    .into()],
                )
            }
            .await;
            if let Err(e) = result {
                error_toast(e);
            }
            Ok(JsValue::UNDEFINED)
        })
    }
    fn set_pref(patch: JsValue, ok: JsValue) -> JsValue {
        let fetched = invoke(&global("api"), &["/prefs-set".into(), patch.clone()]);
        promise(async move {
            match wait(fetched).await {
                Ok(_) => {
                    broadcast_cursor(&patch)?;
                    if truthy(&ok) {
                        invoke_void("toast", &[ok])?;
                    }
                }
                Err(e) => error_toast(e),
            }
            Ok(JsValue::UNDEFINED)
        })
    }
    fn apply_tabs(mode: JsValue) -> Result<(), JsValue> {
        let rows = mode == JsValue::from_str("rows");
        themeRowsHold(&from_json(&serde_json::json!({"n":-1,"h":0}))?);
        let bar = id("tabbar");
        if truthy(&bar) {
            set(&get(&bar, "style"), "minHeight", &"".into())?;
        }
        classes(&get(&doc(), "body"), "tabs-rows", rows);
        let b = query(&doc(), "#tab-rows");
        if truthy(&b) {
            attr(&b, "aria-pressed", if rows { "true" } else { "false" });
            let title = if rows {
                tr("Una fila".into(), "One row".into())
            } else {
                tr("Varias filas".into(), "Several rows".into())
            };
            set(&b, "title", &title.clone().into())?;
            attr(&b, "aria-label", &title);
        }
        let _ = (|| {
            invoke_void("updateTabNavigation", &[])?;
            let ctor = global("Event").dyn_into::<js_sys::Function>()?;
            let args = Array::new();
            args.push(&"resize".into());
            call(
                &global("window"),
                "dispatchEvent",
                &[Reflect::construct(&ctor, &args)?],
            )?;
            Ok::<_, JsValue>(())
        })();
        Ok(())
    }
    fn or(v: JsValue, fallback: JsValue) -> JsValue {
        if truthy(&v) { v } else { fallback }
    }
    fn nullish(v: JsValue, fallback: JsValue) -> JsValue {
        if v.is_null() || v.is_undefined() {
            fallback
        } else {
            v
        }
    }
    fn hydrate(p: JsValue) -> Result<(), JsValue> {
        let ff = query(&doc(), "#pf-font-family");
        if truthy(&ff) {
            let current = or(get(&p, "font_family"), "Ubuntu Sans Mono".into());
            let fonts = get(&p, "fonts");
            if Array::is_array(&fonts) && number(&get(&fonts, "length")) > 0.0 {
                let rows = Array::from(&fonts);
                if !rows.iter().any(|f| get(&f, "family") == current) {
                    let missing = object();
                    set(&missing, "family", &current)?;
                    set(
                        &missing,
                        "label",
                        &format!(
                            "{} · {}",
                            string(&current),
                            tr("no instalada".into(), "not installed".into())
                        )
                        .into(),
                    )?;
                    set(&missing, "a11y", &false.into())?;
                    rows.unshift(&missing);
                }
                let mut out = String::new();
                for f in rows.iter() {
                    let family = get(&f, "family");
                    let esc = escaped(&family, true)?;
                    let label = escaped(&or(get(&f, "label"), family), false)?;
                    let accessible = if truthy(&get(&f, "a11y")) {
                        format!(" · {}", tr("accesible".into(), "accessible".into()))
                    } else {
                        String::new()
                    };
                    out.push_str(&format!("<option value=\"{esc}\" style=\"font-family:'{esc}',monospace\">{label}{accessible}</option>"));
                }
                set(&ff, "innerHTML", &utf16_value(&out))?;
            }
            set(&ff, "value", &current)?;
        }
        for (field, id, label, fallback, is_nullish) in [
            ("font_size", "pf-font-size", "pf-font-size-v", 13.0, false),
            ("terminal_padding", "pf-padding", "pf-padding-v", 8.0, true),
            (
                "terminal_opacity",
                "pf-opacity",
                "pf-opacity-v",
                100.0,
                true,
            ),
        ] {
            let el = query(&doc(), &format!("#{id}"));
            if truthy(&el) {
                let v = get(&p, field);
                set(
                    &el,
                    "value",
                    &if is_nullish {
                        nullish(v, fallback.into())
                    } else {
                        or(v, fallback.into())
                    },
                )?;
                let label = query(&doc(), &format!("#{label}"));
                if truthy(&label) {
                    text(&label, &value(&el))?;
                }
            }
        }
        for b in all(&doc(), ".cur-shape") {
            classes(
                &b,
                "on",
                or(get(&p, "cursor_shape"), "block".into()) == get(&get(&b, "dataset"), "shape"),
            );
        }
        for (sel, key) in [
            ("#sw-cursor-blink", "cursor_blink"),
            ("#sw-ligatures", "ligatures"),
        ] {
            let el = query(&doc(), sel);
            if truthy(&el) {
                classes(&el, "on", get(&p, key) != JsValue::FALSE);
            }
        }
        for b in all(&doc(), ".npos") {
            classes(
                &b,
                "on",
                or(get(&p, "notif_pos"), "free".into()) == get(&get(&b, "dataset"), "npos"),
            );
            if !truthy(&get(&b, "_wired")) {
                set(&b, "_wired", &true.into())?;
                let el = b.clone();
                listen(
                    &b,
                    "click",
                    function(move |_| {
                        let pos = get(&get(&el, "dataset"), "npos");
                        invoke_void(
                            "ulog",
                            &[
                                "click".into(),
                                format!("notif-pos:{}", string(&pos)).into(),
                                "ajustes".into(),
                            ],
                        )?;
                        let patch = object();
                        set(&patch, "notif_pos", &pos)?;
                        let pending = set_pref(
                            patch,
                            tr(
                                "Posición de avisos guardada".into(),
                                "Notification position saved".into(),
                            )
                            .into(),
                        );
                        let el = el.clone();
                        Ok(promise(async move {
                            wait(Ok(pending)).await?;
                            for b in all(&doc(), ".npos") {
                                classes(&b, "on", b == el);
                            }
                            Ok(JsValue::UNDEFINED)
                        }))
                    }),
                );
            }
        }
        Ok(())
    }
    fn load_prefs() -> JsValue {
        let version = themeFavoriteVersion();
        let fetched = invoke(&global("api"), &["/prefs".into()]);
        promise(async move {
            let _ = async {
                let p = wait(fetched).await?;
                remember(&p);
                themeFavoriteReadAt(&js_sys::Date::now().into());
                if version == themeFavoriteVersion() {
                    invoke_void("applyFavorites", &[get(&p, "favorites")])?;
                }
                apply_theme(or(get(&p, "theme"), "noche".into()), true)?;
                apply_buttons(or(get(&p, "button_style"), "sutil".into()), true)?;
                apply_tabs(or(get(&p, "tabs_layout"), "row".into()))?;
                broadcast_cursor(&p)?;
                hydrate(p)
            }
            .await;
            Ok(JsValue::UNDEFINED)
        })
    }
    fn activate(scope: JsValue, name: JsValue) -> Result<(), JsValue> {
        for b in all(&scope, ".mtab") {
            classes(&b, "active", get(&get(&b, "dataset"), "mtab") == name);
        }
        for b in all(&scope, ".mtab-pane") {
            classes(&b, "active", get(&get(&b, "dataset"), "mpane") == name);
        }
        Ok(())
    }
    fn wire_tabs(root: JsValue) -> Result<(), JsValue> {
        for b in all(&root, ".mtab") {
            if truthy(&get(&b, "_wired")) {
                continue;
            }
            set(&b, "_wired", &true.into())?;
            let el = b.clone();
            listen(
                &b,
                "click",
                void(move |a| {
                    call(&a.get(0), "stopPropagation", &[])?;
                    let scope = or(call(&el, "closest", &[".modal-panel".into()])?, doc());
                    activate(scope, get(&get(&el, "dataset"), "mtab"))
                }),
            );
        }
        Ok(())
    }
    fn range(input: JsValue) -> Result<(), JsValue> {
        let min = number(&get(&input, "min"));
        let min = if min.is_nan() || min == 0.0 { 0.0 } else { min };
        let max = number(&get(&input, "max"));
        let max = if max.is_nan() || max == 0.0 {
            100.0
        } else {
            max
        };
        let pct = (number(&value(&input)) - min) / (max - min) * 100.0;
        style(&input, "--fill", &format!("{}%", string(&pct.into())));
        Ok(())
    }
    fn refresh_ranges() -> Result<(), JsValue> {
        for r in all(&doc(), "input[type=range].solid") {
            range(r)?;
        }
        Ok(())
    }
    fn wire_ranges() -> Result<(), JsValue> {
        for r in all(&doc(), "input[type=range].solid") {
            if truthy(&get(&r, "_wired")) {
                continue;
            }
            set(&r, "_wired", &true.into())?;
            range(r.clone())?;
            for event in ["input", "change"] {
                let el = r.clone();
                listen(&r, event, void(move |_| range(el.clone())));
            }
        }
        Ok(())
    }
    fn preview() -> Result<(), JsValue> {
        let family = or(
            value(&query(&doc(), "#pf-font-family")),
            "Ubuntu Sans Mono".into(),
        );
        let size = or(value(&query(&doc(), "#pf-font-size")), 13.0.into());
        let p = query(&doc(), "#font-preview");
        if truthy(&p) {
            set(
                &get(&p, "style"),
                "fontFamily",
                &format!("\"{}\", monospace", string(&family)).into(),
            )?;
            set(
                &get(&p, "style"),
                "fontSize",
                &format!("{}px", string(&size)).into(),
            )?;
        }
        Ok(())
    }
    pub fn mount_theme() -> Result<(), JsValue> {
        set(
            &js_sys::global(),
            "THEME_SEQ",
            &from_json(&serde_json::json!(model::THEMES))?,
        )?;
        set(
            &js_sys::global(),
            "THEME_META",
            &from_json(&model::metadata())?,
        )?;
        set(
            &js_sys::global(),
            "BUTTON_STYLES",
            &from_json(&serde_json::json!(model::BUTTONS))?,
        )?;
        set(&js_sys::global(), "curTheme", &"noche".into())?;
        set(&js_sys::global(), "curButtonStyle", &"sutil".into())?;
        publish("broadcastTermTheme", |a| {
            broadcast(a.get(0))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("applyTheme", |a| {
            apply_theme(a.get(0), truthy(&a.get(1)))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("applyButtonStyle", |a| {
            apply_buttons(a.get(0), truthy(&a.get(1)))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("selectTheme", |a| Ok(select_theme(a.get(0))))?;
        publish("selectButtonStyle", |a| Ok(select_buttons(a.get(0))))?;
        publish("renderThemeGallery", |_| {
            render_themes()?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("renderButtonStyleGallery", |_| {
            render_buttons()?;
            Ok(JsValue::UNDEFINED)
        })?;
        Ok(())
    }
    pub fn mount_favorites() -> Result<(), JsValue> {
        publish("loadPrefs", |_| Ok(load_prefs()))?;
        Ok(())
    }
    pub fn mount_prefs() -> Result<(), JsValue> {
        publish("applyTabsLayout", |a| {
            apply_tabs(a.get(0))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("hydrateTerminalPrefs", |a| {
            hydrate(a.get(0))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("setPref", |a| Ok(set_pref(a.get(0), a.get(1))))?;
        Ok(())
    }
    pub fn mount_tabs() -> Result<(), JsValue> {
        // Ajustes › Consumos vive en las pestañas de Ajustes: se monta con ellas.
        crate::components::usage_board_web::mount()?;
        publish("activateMtab", |a| {
            activate(a.get(0), a.get(1))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("wireMtabs", |a| {
            wire_tabs(if a.length() == 0 { doc() } else { a.get(0) })?;
            Ok(JsValue::UNDEFINED)
        })?;
        Ok(())
    }
    pub fn mount_ranges() -> Result<(), JsValue> {
        publish("updateRangeFill", |a| {
            range(a.get(0))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("refreshAllRangeFills", |_| {
            refresh_ranges()?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("wireRangeFills", |_| {
            wire_ranges()?;
            Ok(JsValue::UNDEFINED)
        })?;
        Ok(())
    }
    pub fn mount_preview() -> Result<(), JsValue> {
        publish("updateFontPreview", |_| {
            preview()?;
            Ok(JsValue::UNDEFINED)
        })
    }
    pub fn mount() -> Result<(), JsValue> {
        mount_theme()?;
        mount_favorites()?;
        mount_prefs()?;
        mount_tabs()?;
        mount_ranges()?;
        mount_preview()?;
        Ok(())
    }
    pub fn attach_theme() -> Result<(), JsValue> {
        let btn = query(&doc(), "#btn-theme");
        listen(
            &btn,
            "click",
            void(|_| {
                classes(&query(&doc(), "#settings"), "open", true);
                activate(query(&doc(), "#settings .modal-panel"), "appearance".into())?;
                render_themes()?;
                render_buttons()?;
                later(
                    void(|_| {
                        let el = query(&doc(), "#theme-gallery [aria-checked=\"true\"]");
                        if truthy(&el) {
                            call(&el, "focus", &[])?;
                        }
                        Ok(())
                    }),
                    0.0,
                );
                Ok(())
            }),
        );
        Ok(())
    }
    pub fn attach_tabs() -> Result<(), JsValue> {
        crate::components::usage_board_web::attach()?;
        wire_tabs(doc())
    }
    pub fn attach_ranges() -> Result<(), JsValue> {
        wire_ranges()?;
        call(
            &js_sys::global(),
            "setInterval",
            &[void(|_| refresh_ranges()), 250.0.into()],
        )?;
        Ok(())
    }
    pub fn attach_preview() -> Result<(), JsValue> {
        preview()?;
        for (selector, label, key, ok) in [
            (
                "#pf-font-family",
                "",
                "font_family",
                Some(("Tipografía guardada", "Font saved")),
            ),
            ("#pf-font-size", "#pf-font-size-v", "font_size", None),
            ("#pf-padding", "#pf-padding-v", "terminal_padding", None),
            ("#pf-opacity", "#pf-opacity-v", "terminal_opacity", None),
        ] {
            let el = query(&doc(), selector);
            if !truthy(&el) {
                continue;
            }
            let font = key == "font_family";
            let size = key == "font_size";
            if !font {
                listen(
                    &el,
                    "input",
                    void(move |a| {
                        text(&query(&doc(), label), &value(&get(&a.get(0), "target")))?;
                        if size {
                            preview()?;
                        }
                        Ok(())
                    }),
                );
            }
            listen(
                &el,
                "change",
                void(move |a| {
                    let v = value(&get(&a.get(0), "target"));
                    let patch = object();
                    set(&patch, key, &if font { v } else { number(&v).into() })?;
                    let _ = set_pref(
                        patch,
                        ok.map(|(es, en)| tr(es.into(), en.into()).into())
                            .unwrap_or(JsValue::UNDEFINED),
                    );
                    if font {
                        preview()?;
                    }
                    Ok(())
                }),
            );
        }
        for b in all(&doc(), ".cur-shape") {
            let el = b.clone();
            listen(
                &b,
                "click",
                void(move |_| {
                    for x in all(&doc(), ".cur-shape") {
                        classes(&x, "on", false);
                    }
                    classes(&el, "on", true);
                    let patch = object();
                    set(&patch, "cursor_shape", &get(&get(&el, "dataset"), "shape"))?;
                    let _ = set_pref(patch, JsValue::UNDEFINED);
                    Ok(())
                }),
            );
        }
        for (selector, key) in [
            ("#sw-cursor-blink", "cursor_blink"),
            ("#sw-ligatures", "ligatures"),
        ] {
            let el = query(&doc(), selector);
            if truthy(&el) {
                listen(
                    &el,
                    "click",
                    void(move |a| {
                        let target = get(&a.get(0), "currentTarget");
                        let on = !truthy(&call(
                            &get(&target, "classList"),
                            "contains",
                            &["on".into()],
                        )?);
                        classes(&target, "on", on);
                        let patch = object();
                        set(&patch, key, &on.into())?;
                        let _ = set_pref(patch, JsValue::UNDEFINED);
                        Ok(())
                    }),
                );
            }
        }
        Ok(())
    }
    pub fn attach() -> Result<(), JsValue> {
        attach_theme()?;
        attach_tabs()?;
        attach_ranges()?;
        attach_preview()
    }
}
