//! Left sidebar: «Sesiones» feed (default), «Consumos» quota cards and the Servers sheet.
#[cfg(not(target_arch = "wasm32"))]
pub fn mount() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(target_arch = "wasm32")]
pub use web::mount;
#[cfg(target_arch = "wasm32")]
mod web {
    use crate::components::web_support::*;
    use comandos_web_dom::port::*;
    use comandos_web_view::command_sidebar as view;
    use comandos_web_view::escape::text as esc;
    use serde_json::{Value, json};
    use std::{
        cell::Cell,
        rc::Rc,
    };
    use wasm_bindgen::JsValue;
    fn nullish(v: &JsValue) -> bool {
        v.is_null() || v.is_undefined()
    }
    fn or(v: JsValue, fallback: JsValue) -> JsValue {
        if truthy(&v) { v } else { fallback }
    }
    fn s(v: &JsValue) -> String {
        if nullish(v) {
            String::new()
        } else {
            utf16_string(v)
        }
    }
    fn j(v: &JsValue) -> Value {
        to_utf16_json(v)
    }
    fn list(v: &JsValue) -> Vec<JsValue> {
        if js_sys::Array::is_array(v) {
            js_sys::Array::from(v).iter().collect()
        } else {
            Vec::new()
        }
    }
    fn closest(el: &JsValue, selector: &str) -> JsValue {
        call(el, "closest", &[selector.into()]).unwrap_or(JsValue::NULL)
    }
    fn contains(el: &JsValue, cls: &str) -> bool {
        call(&get(el, "classList"), "contains", &[cls.into()]).is_ok_and(|v| truthy(&v))
    }
    fn html(el: &JsValue, value: String) -> Result<(), JsValue> {
        set(el, "innerHTML", &utf16_value(&value))
    }
    fn error_text(e: &JsValue) -> String {
        get(e, "message")
            .as_string()
            .or_else(|| e.as_string())
            .unwrap_or_else(|| "Vuelve a intentarlo".into())
    }
    struct Sidebar {
        opts: JsValue,
        el: JsValue,
        state: JsValue,
        credits_at: Cell<f64>,
        credits_busy: Cell<bool>,
    }
    impl Sidebar {
        fn quiet(&self, name: &str, args: &[JsValue]) {
            let f = get(&self.opts, name);
            if !nullish(&f) {
                let _ = invoke(&f, args);
            }
        }
        fn state(&self, key: &str) -> String {
            s(&get(&self.state, key))
        }
        fn write_state(&self, key: &str, value: &str) -> Result<(), JsValue> {
            set(&self.state, key, &value.into())
        }
        fn query(&self, selector: &str) -> JsValue {
            query(&self.el, selector)
        }
        fn set_sheet(self: &Rc<Self>, key: &str) -> Result<(), JsValue> {
            let next = view::sheet_name(key);
            if self.state("sheet") == next {
                return Ok(());
            }
            self.write_state("sheet", next)?;
            self.render()
        }
        /// OpenRouter balance in the Consumos view; at most one read a minute.
        fn load_credits(self: &Rc<Self>, key: Option<String>) {
            let at = js_sys::Date::now();
            if self.credits_busy.get() || (key.is_none() && at - self.credits_at.get() < 60000.) {
                return;
            }
            self.credits_busy.set(true);
            self.credits_at.set(at);
            let mut args = vec![JsValue::from_str("/credits/openrouter")];
            if let Some(key) = key
                && let Ok(body) = from_json(&json!({"key":key}))
            {
                args.push(body);
            }
            let request = invoke(&get(&self.opts, "api"), &args);
            let b = self.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let r = wait(request).await.map(|v| j(&v)).and_then(|v| {
                    match v.get("error").and_then(Value::as_str) {
                        Some(e) => Err(JsValue::from_str(e)),
                        None => Ok(v),
                    }
                });
                b.credits_busy.set(false);
                let content = match r {
                    Ok(v) if v["connected"] == true => format!(
                        "<div class=\"sf-credit-numbers\"><div><small>Disponible · USD</small><strong>${:.2}</strong></div><div><small>Comprado</small><b>${:.2}</b></div><div><small>Consumido</small><b>${:.2}</b></div></div>",
                        v["remaining"].as_f64().unwrap_or(0.),
                        v["total_credits"].as_f64().unwrap_or(0.),
                        v["total_usage"].as_f64().unwrap_or(0.)
                    ),
                    result => {
                        let message = match result {
                            Err(e) => error_text(&e),
                            Ok(_) => "Conecta tu clave de administración para consultar el saldo real."
                                .into(),
                        };
                        format!(
                            "<p>{}</p><details><summary>Conectar OpenRouter</summary><label>Clave de administración<input class=\"sf-credit-key\" type=\"password\" autocomplete=\"off\" placeholder=\"sk-or-…\"></label><button data-flat data-sf-credits-connect>Guardar y conectar</button><small>Se guarda solo en el servidor. No uses aquí una clave de modelos.</small></details>",
                            esc(&message)
                        )
                    }
                };
                let slot = b.query(".cs-credits");
                if !nullish(&slot) {
                    let _ = set(
                        &slot,
                        "innerHTML",
                        &format!(
                            "<div class=\"sf-credit-head\"><strong>OpenRouter</strong><button data-flat data-sf-credits-refresh aria-label=\"Actualizar créditos\">↻</button></div>{content}"
                        )
                        .into(),
                    );
                }
            });
        }
        fn paint(self: &Rc<Self>) -> Result<(), JsValue> {
            let cur = self.state("sheet");
            classes(&self.el, "sheet-open", !cur.is_empty());
            attr(&self.el, "data-panel", &cur);
            for b in all(&self.el, "[data-sheet]") {
                let on = s(&get(&get(&b, "dataset"), "sheet")) == cur;
                classes(&b, "on", on);
                attr(&b, "aria-pressed", if on { "true" } else { "false" });
            }
            let title = self.query(".cs-head .sh-t");
            if !nullish(&title) {
                set(
                    &title,
                    "textContent",
                    &if cur == "srv" { "Servidores SSH" } else { "" }.into(),
                )?;
            }
            let sheet = self.query(".cs-sheet");
            if !nullish(&sheet) {
                set(&sheet, "hidden", &cur.is_empty().into())?;
            }
            let view = view::view_name(&self.state("view"));
            let usage = view == "usage";
            let box_ = self.query(".cs-empty-terms");
            if !nullish(&box_) {
                set(&box_, "hidden", &(!cur.is_empty()).into())?;
                // The active view takes all the height below the toolbar.
                style(&box_, "flex", "1 1 auto");
                style(&box_, "min-height", "0");
                style(&box_, "max-height", "none");
                let feed = query(&box_, ".cs-feed");
                if !nullish(&feed) {
                    set(&feed, "hidden", &usage.into())?;
                }
                set(&query(&box_, ".et-lim"), "hidden", &(!usage).into())?;
                if usage && cur.is_empty() {
                    self.quiet("renderLimits", &[query(&box_, ".cs-limits")]);
                    self.load_credits(None);
                }
            }
            for button in all(&self.el, "[data-sidebar-view]") {
                let on = s(&get(&get(&button, "dataset"), "sidebarView")) == view;
                classes(&button, "on", on);
                attr(&button, "aria-selected", if on { "true" } else { "false" });
            }
            if cur == "srv" {
                set(&self.state, "srvMounted", &true.into())?;
                self.quiet("mountServers", &[self.query(".cs-srv-slot")]);
            } else if truthy(&get(&self.state, "srvMounted")) {
                set(&self.state, "srvMounted", &false.into())?;
                self.quiet("mountServers", &[JsValue::NULL]);
            }
            let height = get(&self.query(".cs-tools"), "offsetHeight");
            if truthy(&height) {
                style(&self.el, "--cs-tools-h", &format!("{}px", string(&height)));
            }
            Ok(())
        }
        fn render(self: &Rc<Self>) -> Result<(), JsValue> {
            if nullish(&self.query(".cs-tools")) {
                html(&self.el, view::shell_html())?;
            }
            self.paint()?;
            self.quiet("hydrate", std::slice::from_ref(&self.el));
            Ok(())
        }
        fn click(self: &Rc<Self>, event: JsValue) -> Result<JsValue, JsValue> {
            let t = get(&event, "target");
            if nullish(&t) || nullish(&get(&t, "closest")) {
                return Ok(JsValue::UNDEFINED);
            }
            let n = closest(&t, "[data-sidebar-view]");
            if !nullish(&n) {
                let view = view::view_name(&s(&get(&get(&n, "dataset"), "sidebarView")));
                self.write_state("view", view)?;
                self.write_state("sheet", "")?;
                self.render()?;
                return Ok(JsValue::UNDEFINED);
            }
            let n = closest(&t, "[data-sheet]");
            if !nullish(&n) {
                let key = s(&get(&get(&n, "dataset"), "sheet"));
                self.set_sheet(if key == self.state("sheet") { "" } else { &key })?;
                return Ok(JsValue::UNDEFINED);
            }
            if !nullish(&closest(&t, "[data-sheet-close]")) {
                self.set_sheet("")?;
                return Ok(JsValue::UNDEFINED);
            }
            if !nullish(&closest(&t, "[data-sf-credits-refresh]")) {
                self.credits_at.set(0.);
                self.load_credits(None);
                return Ok(JsValue::UNDEFINED);
            }
            if !nullish(&closest(&t, "[data-sf-credits-connect]")) {
                let input = self.query(".sf-credit-key");
                let key = s(&get(&input, "value"));
                let _ = set(&input, "value", &"".into());
                if !key.trim().is_empty() {
                    self.load_credits(Some(key));
                }
            }
            Ok(JsValue::UNDEFINED)
        }
        fn wire(self: &Rc<Self>) -> Result<(), JsValue> {
            let owner = self.clone();
            call(
                &self.el,
                "addEventListener",
                &["click".into(), function(move |a| owner.click(a.get(0)))],
            )?;
            let owner = self.clone();
            call(
                &self.el,
                "addEventListener",
                &[
                    "keydown".into(),
                    function(move |a| {
                        let e = a.get(0);
                        if get(&e, "key") == "Escape" && !owner.state("sheet").is_empty() {
                            let _ = call(&e, "preventDefault", &[]);
                            owner.set_sheet("")?;
                        }
                        Ok(JsValue::UNDEFINED)
                    }),
                ],
            )?;
            Ok(())
        }
        fn create(opts: JsValue) -> Result<JsValue, JsValue> {
            let el = get(&opts, "root");
            let state = object();
            set(&state, "sheet", &"".into())?;
            set(&state, "view", &"sessions".into())?;
            let owner = Rc::new(Self {
                opts,
                el,
                state,
                credits_at: Cell::new(0.),
                credits_busy: Cell::new(false),
            });
            owner.wire()?;
            let result = object();
            let b = owner.clone();
            method(&result, "render", move |_| {
                b.render()?;
                Ok(JsValue::UNDEFINED)
            })?;
            let b = owner.clone();
            method(&result, "setSheet", move |a| {
                b.set_sheet(&s(&a.get(0)))?;
                Ok(JsValue::UNDEFINED)
            })?;
            getter(&result, "state", move || owner.state.clone())?;
            Ok(result)
        }
    }
    struct Limits {
        slot: JsValue,
        opts: JsValue,
        state: JsValue,
    }
    impl Limits {
        fn paint(&self) -> Result<(), JsValue> {
            let active = get(&get(&self.slot, "ownerDocument"), "activeElement");
            let card = closest(&active, ".card[data-account]");
            let account = if call(&self.slot, "contains", std::slice::from_ref(&card))
                .is_ok_and(|v| truthy(&v))
            {
                get(&get(&card, "dataset"), "account")
            } else {
                JsValue::NULL
            };
            let action = get(&get(&active, "dataset"), "limitAction");
            html(
                &self.slot,
                view::limits_html(
                    &j(&get(&self.state, "accounts")),
                    &s(&get(&self.state, "curId")),
                    &j(&self.state),
                ),
            )?;
            if truthy(&account) {
                self.focus(&account, &action);
            }
            Ok(())
        }
        fn focus(&self, account: &JsValue, action: &JsValue) {
            if let Some(card) = all(&self.slot, ".card")
                .into_iter()
                .find(|c| get(&get(c, "dataset"), "account") == *account)
            {
                let toggle = query(&card, ".usage-toggle");
                let button = if truthy(action) {
                    all(&card, "[data-limit-action]")
                        .into_iter()
                        .find(|b| get(&get(b, "dataset"), "limitAction") == *action)
                        .unwrap_or(JsValue::NULL)
                } else {
                    toggle.clone()
                };
                let disabled = truthy(&get(&button, "disabled"))
                    || call(&button, "getAttribute", &["disabled".into()])
                        .is_ok_and(|v| !v.is_null());
                let target = if nullish(&button) || disabled {
                    toggle
                } else {
                    button
                };
                let _ = call(&target, "focus", &[]);
            }
        }
        fn click(self: &Rc<Self>, event: JsValue) -> Result<JsValue, JsValue> {
            let t = get(&event, "target");
            let card = closest(&t, ".card[data-account]");
            if nullish(&card) {
                return Ok(JsValue::UNDEFINED);
            }
            let account = get(&get(&card, "dataset"), "account");
            let found = list(&get(&self.state, "accounts"))
                .into_iter()
                .find(|a| get(a, "id") == account);
            let Some(a) = found else {
                return Ok(JsValue::UNDEFINED);
            };
            let button = closest(&t, "button");
            if nullish(&button) {
                return Ok(JsValue::UNDEFINED);
            }
            if contains(&button, "usage-toggle") {
                let open = get(&self.state, "openId");
                set(
                    &self.state,
                    "openId",
                    &if open == get(&a, "id") {
                        "".into()
                    } else {
                        get(&a, "id")
                    },
                )?;
                self.paint()?;
                self.focus(&get(&a, "id"), &JsValue::UNDEFINED);
                return Ok(JsValue::UNDEFINED);
            }
            let action = s(&get(&get(&button, "dataset"), "limitAction"));
            let switching = action == "switch"
                && !truthy(&get(&self.state, "pending"))
                && view::can_switch(&j(&a), &s(&get(&self.state, "curId")));
            if switching {
                set(&self.state, "pending", &true.into())?;
                self.paint()?;
            }
            let started = if switching {
                let cb = get(&self.opts, "onSwitch");
                if nullish(&cb) {
                    Ok(JsValue::UNDEFINED)
                } else {
                    call(&self.opts, "onSwitch", &[a, get(&self.state, "curId")])
                }
            } else {
                Ok(JsValue::UNDEFINED)
            };
            let owner = self.clone();
            Ok(promise(async move {
                let result = wait(started).await;
                if switching {
                    set(&owner.state, "pending", &false.into())?;
                    owner.paint()?;
                }
                if let Err(error) = result {
                    let cb = get(&owner.opts, "onError");
                    if !nullish(&cb) {
                        call(&owner.opts, "onError", &[error])?;
                    }
                }
                Ok(JsValue::UNDEFINED)
            }))
        }

        fn create(slot: JsValue, opts: JsValue) -> Result<JsValue, JsValue> {
            let owner = Rc::new(Self {
                slot,
                opts,
                state: from_json(&json!({"accounts":[],"curId":"","openId":"","pending":false}))?,
            });
            let view = object();
            let b = owner.clone();
            method(&view, "update", move |a| {
                let accounts = a.get(0);
                set(
                    &b.state,
                    "accounts",
                    &if js_sys::Array::is_array(&accounts) {
                        accounts
                    } else {
                        js_sys::Array::new().into()
                    },
                )?;
                set(&b.state, "curId", &or(a.get(1), "".into()))?;
                let open = get(&b.state, "openId");
                if !list(&get(&b.state, "accounts"))
                    .iter()
                    .any(|a| get(a, "id") == open)
                {
                    set(&b.state, "openId", &"".into())?;
                }
                b.paint()?;
                Ok(JsValue::UNDEFINED)
            })?;
            let slot = owner.slot.clone();
            call(
                &slot,
                "addEventListener",
                &["click".into(), function(move |a| owner.click(a.get(0)))],
            )?;
            Ok(view)
        }
    }
    pub fn mount() -> Result<(), JsValue> {
        let exports = object();
        method(&exports, "createCommandSidebar", |a| {
            Sidebar::create(a.get(0))
        })?;
        method(&exports, "limitsHTML", |a| {
            Ok(utf16_value(&view::limits_html(
                &j(&a.get(0)),
                &s(&a.get(1)),
                &j(&a.get(2)),
            )))
        })?;
        method(&exports, "createLimitsView", |a| {
            Limits::create(a.get(0), or(a.get(1), object()))
        })?;
        set(&js_sys::global(), "ComandosCommandSidebar", &exports)?;
        // «Sesiones» feed and K5 marks (`window.ComandosFeed`); never blocks the sidebar.
        let _ = crate::components::session_feed::install();
        Ok(())
    }
}
