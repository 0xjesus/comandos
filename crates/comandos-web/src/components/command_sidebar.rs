//! Callback-based command sidebar; state and event decisions are Rust.
#[cfg(target_arch = "wasm32")]
#[path = "sidebar_browser.rs"]
mod browser;
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
    use serde_json::{Value, json};
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsCast, JsValue};
    const OPEN: &str = "comandos.commands.open.v2";
    const HIDDEN: &str = "comandos.commands.termsHidden";
    const WAIT: &str = "Espera a que termine de escribir";
    fn nullish(v: &JsValue) -> bool {
        v.is_null() || v.is_undefined()
    }
    fn or(v: JsValue, fallback: JsValue) -> JsValue {
        if truthy(&v) { v } else { fallback }
    }
    fn constructor(name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
        let f = global(name).dyn_into::<js_sys::Function>()?;
        js_sys::Reflect::construct(&f, &args.iter().cloned().collect::<js_sys::Array>())
    }
    fn html(el: &JsValue, value: String) -> Result<(), JsValue> {
        set(el, "innerHTML", &utf16_value(&value))
    }
    fn j(v: &JsValue) -> Value {
        to_utf16_json(v)
    }
    fn s(v: &JsValue) -> String {
        if nullish(v) {
            String::new()
        } else {
            utf16_string(v)
        }
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
    fn message(error: &JsValue, default: &str) -> JsValue {
        let m = get(error, "message");
        if truthy(&m) { m } else { default.into() }
    }
    struct Sidebar {
        opts: JsValue,
        el: JsValue,
        state: JsValue,
        timer: RefCell<JsValue>,
        browser: Rc<super::browser::Browser>,
    }
    impl Sidebar {
        fn callback(&self, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
            let f = get(&self.opts, name);
            if nullish(&f) {
                return Ok(JsValue::UNDEFINED);
            }
            invoke(&f, args)
        }
        fn quiet(&self, name: &str, args: &[JsValue]) {
            let _ = self.callback(name, args);
        }
        fn target(&self) -> JsValue {
            or(
                self.callback("getTarget", &[]).unwrap_or(JsValue::NULL),
                JsValue::NULL,
            )
        }
        fn state(&self, key: &str) -> JsValue {
            get(&self.state, key)
        }
        fn write_state(&self, key: &str, value: JsValue) -> Result<(), JsValue> {
            set(&self.state, key, &value)
        }
        fn read(&self, key: &str) -> JsValue {
            call(&get(&self.opts, "storage"), "getItem", &[key.into()]).unwrap_or(JsValue::NULL)
        }
        fn write(&self, key: &str, value: JsValue) {
            let _ = call(&get(&self.opts, "storage"), "setItem", &[key.into(), value]);
        }
        fn write_open(&self) {
            let value = js_sys::Array::from(&self.state("open"));
            if let Ok(v) = js_sys::JSON::stringify(&value) {
                self.write(OPEN, v.into());
            }
        }
        fn here(&self) -> String {
            view::here_cli(&j(&self.state), &j(&self.target()))
        }
        fn terminals(&self) -> JsValue {
            or(
                self.callback("terminals", &[]).unwrap_or(JsValue::NULL),
                js_sys::Array::new().into(),
            )
        }
        fn current(&self) -> String {
            s(&self.state("curTerm"))
        }
        fn query(&self, selector: &str) -> JsValue {
            query(&self.el, selector)
        }
        fn clis(&self) -> Vec<JsValue> {
            list(&get(&self.state("catalog"), "clis"))
        }
        fn target_title(&self) -> String {
            let t = self.target();
            if !truthy(&t) {
                "Sin destino".into()
            } else {
                view::pick(
                    &j(&t),
                    &["title"],
                    &format!("{} {}", s(&get(&t, "session")), s(&get(&t, "pane"))),
                )
            }
        }
        fn pill(&self) -> String {
            view::catalog_pill(&j(&self.state("catalog")), &self.here())
        }
        fn open_values(&self) -> Value {
            j(&js_sys::Array::from(&self.state("open")).into())
        }
        fn body(&self) -> String {
            let here = self.here();
            let t = self.target();
            let no_target = !truthy(&get(&t, "session")) || !truthy(&get(&t, "pane"));
            self.clis().iter().map(|cli|view::cli_html(&j(cli),&json!({"mode":"run","open":self.open_values(),"here":!here.is_empty()&&s(&get(cli,"id"))==here,"noCli":here.is_empty(),"noTarget":no_target,"q":s(&self.state("q"))}))).collect()
        }
        fn chains_html(&self) -> String {
            format!(
                "{}{}",
                view::runner_html(&j(&self.state("run"))),
                view::saved_html(&j(&self.state("chains")))
            )
        }
        fn apply_catalog(self: &Rc<Self>, payload: JsValue) -> Result<(), JsValue> {
            self.write_state("catalog", or(get(&payload, "catalog"), JsValue::NULL))?;
            self.write_state("cliInPane", or(get(&payload, "cliInPane"), "".into()))?;
            self.write_state("catalogTarget", or(get(&payload, "target"), JsValue::NULL))?;
            if truthy(&self.state("firstRender")) && truthy(&self.state("catalog")) {
                call(&self.state("open"), "add", &["saved".into()])?;
                self.write_state("firstRender", false.into())?;
                self.write_open();
            }
            let t = self.target();
            let applied = or(self.state("catalogTarget"), t);
            let target = if truthy(&applied) {
                let o = object();
                for key in ["session", "pane"] {
                    set(&o, key, &get(&applied, key))?;
                }
                o
            } else {
                JsValue::NULL
            };
            self.write_state("appliedTarget", target)?;
            self.render()
        }
        fn refresh(self: &Rc<Self>) -> JsValue {
            self.browser.sync(if truthy(&self.state("sheet")) {
                ""
            } else if self.state("view") == "usage" {
                "usage"
            } else if self.state("view") == "sessions" {
                "sessions"
            } else {
                "files"
            });
            let owner = self.clone();
            let started = (|| -> Result<JsValue, JsValue> {
                let t = owner.target();
                let path = if truthy(&get(&t, "session")) && truthy(&get(&t, "pane")) {
                    let encode = |v: JsValue| {
                        invoke(&global("encodeURIComponent"), &[v]).map(|v| string(&v))
                    };
                    format!(
                        "/commands/catalog?session={}&pane={}",
                        encode(get(&t, "session"))?,
                        encode(get(&t, "pane"))?
                    )
                } else {
                    "/commands/catalog".into()
                };
                let requests = js_sys::Array::new();
                requests.push(&owner.callback("api", &[path.into()])?);
                requests.push(&owner.callback("api", &["/chains".into()])?);
                Ok(js_sys::Promise::all(&requests).into())
            })();
            promise(async move {
                let result = wait(started).await?;
                let result = js_sys::Array::from(&result);
                let chains = get(&result.get(1), "chains");
                owner.write_state(
                    "chains",
                    if js_sys::Array::is_array(&chains) {
                        chains
                    } else {
                        js_sys::Array::new().into()
                    },
                )?;
                owner.apply_catalog(or(result.get(0), object()))?;
                Ok(JsValue::UNDEFINED)
            })
        }
        fn busy(&self, on: bool) {
            classes(&self.el, "typing", on);
        }
        fn type_into(
            self: &Rc<Self>,
            target: JsValue,
            text: JsValue,
            kind: String,
            after: Option<JsValue>,
        ) -> Result<JsValue, JsValue> {
            let body = object();
            for key in ["session", "pane"] {
                set(&body, key, &get(&target, key))?;
            }
            set(&body, "text", &text)?;
            set(&body, "requestId", &self.callback("makeId", &[])?)?;
            let owner = self.clone();
            let p = promise(async move {
                wait(Ok(JsValue::NULL)).await?;
                let this = owner.state("typing");
                let response = match wait(owner.callback("api", &["/pane/type".into(), body])).await
                {
                    Ok(result) => {
                        if kind == "shell" {
                            let cli = view::launch_cli(&j(&owner.state("catalog")), &s(&text));
                            if !cli.is_empty() {
                                let key = format!(
                                    "comandos.commands.preferred.{}",
                                    s(&or(get(&target, "paneKey"), get(&target, "pane")))
                                );
                                owner.write(&key, utf16_value(&cli));
                            }
                        }
                        let r = object();
                        set(&r, "ok", &true.into())?;
                        set(&r, "result", &result)?;
                        r
                    }
                    Err(error) => {
                        let error = message(&error, "No se pudo escribir en el pane");
                        owner.quiet("toast", &[error.clone(), true.into()]);
                        let r = object();
                        set(&r, "ok", &false.into())?;
                        set(&r, "error", &error)?;
                        r
                    }
                };
                let finished = after
                    .map(|f| invoke(&f, std::slice::from_ref(&response)))
                    .transpose();
                if owner.state("typing") == this {
                    owner.write_state("typing", JsValue::NULL)?;
                    owner.busy(false);
                }
                finished?;
                Ok(response)
            });
            self.write_state("typing", p.clone())?;
            self.busy(true);
            Ok(p)
        }
        fn insert(self: &Rc<Self>, text: JsValue, kind: JsValue) -> Result<JsValue, JsValue> {
            let target = self.target();
            let text = if nullish(&text) {
                String::new()
            } else {
                utf16_string(&text)
            };
            let kind = if kind.is_undefined() {
                "pane".into()
            } else {
                s(&kind)
            };
            match view::admit(
                truthy(&self.state("typing")),
                &j(&target),
                &text,
                &kind,
                &self.here(),
            ) {
                view::Admission::Busy => self.quiet("toast", &[WAIT.into()]),
                view::Admission::NoTarget => {
                    self.quiet("toast", &["Selecciona un pane primero".into(), true.into()])
                }
                view::Admission::Controls => self.quiet(
                    "toast",
                    &[
                        "El comando no puede llevar saltos de línea".into(),
                        true.into(),
                    ],
                ),
                view::Admission::NoCli => {}
                view::Admission::Accepted => {
                    let copy = call(&global("Object"), "assign", &[object(), target])?;
                    return self.type_into(
                        copy,
                        utf16_value(&text),
                        if kind == "shell" {
                            "shell".into()
                        } else {
                            "pane".into()
                        },
                        None,
                    );
                }
            }
            Ok(JsValue::NULL)
        }
        fn start(self: &Rc<Self>, slug: JsValue) -> Result<JsValue, JsValue> {
            let target = self.target();
            if !truthy(&get(&target, "session")) || !truthy(&get(&target, "pane")) {
                self.quiet("toast", &["Selecciona un pane primero".into(), true.into()]);
                return Ok(JsValue::NULL);
            }
            let owner = self.clone();
            Ok(promise(async move {
                if let Ok(ch) = wait(owner.callback("api", &["/chains".into()])).await {
                    let chains = get(&ch, "chains");
                    if js_sys::Array::is_array(&chains) {
                        owner.write_state("chains", chains)?;
                    }
                }
                let chain = list(&owner.state("chains"))
                    .into_iter()
                    .find(|c| get(c, "slug") == slug);
                let Some(chain) = chain
                    .filter(|c| !truthy(&get(c, "error")) && !list(&get(c, "steps")).is_empty())
                else {
                    owner.quiet(
                        "toast",
                        &["Esa cadena no se puede correr".into(), true.into()],
                    );
                    owner.render()?;
                    return Ok(JsValue::NULL);
                };
                let run = object();
                set(&run, "slug", &get(&chain, "slug"))?;
                set(&run, "name", &or(get(&chain, "name"), get(&chain, "slug")))?;
                let steps = js_sys::Array::new();
                for step in list(&get(&chain, "steps")) {
                    steps.push(&call(&global("Object"), "assign", &[object(), step])?);
                }
                set(&run, "steps", &steps)?;
                set(&run, "step", &0.into())?;
                set(
                    &run,
                    "target",
                    &call(&global("Object"), "assign", &[object(), target])?,
                )?;
                set(&run, "error", &"".into())?;
                owner.write_state("run", run.clone())?;
                owner.write_state("sheet", "chains".into())?;
                owner.render()?;
                Ok(run)
            }))
        }
        fn next(self: &Rc<Self>) -> Result<JsValue, JsValue> {
            let run = self.state("run");
            let steps = list(&get(&run, "steps"));
            let index = number(&get(&run, "step")) as usize;
            if !truthy(&run) || index >= steps.len() {
                return Ok(JsValue::NULL);
            }
            if truthy(&self.state("typing")) {
                self.quiet("toast", &[WAIT.into()]);
                return Ok(JsValue::NULL);
            }
            let step = steps.get(index).cloned().unwrap_or(JsValue::NULL);
            let raw = get(&step, "text");
            if view::has_control(&s(&raw)) {
                self.quiet(
                    "toast",
                    &["El paso tiene saltos de línea".into(), true.into()],
                );
                return Ok(JsValue::NULL);
            }
            let target = get(&run, "target");
            let owner = self.clone();
            let after = function(move |a| {
                if owner.state("run") == run {
                    let response = a.get(0);
                    if truthy(&get(&response, "ok")) {
                        set(&run, "step", &(number(&get(&run, "step")) + 1.).into())?;
                        set(&run, "error", &"".into())?;
                    } else {
                        set(&run, "error", &get(&response, "error"))?;
                    }
                    owner.render()?;
                }
                Ok(JsValue::UNDEFINED)
            });
            self.type_into(
                target,
                utf16_value(&utf16_string(&raw)),
                if get(&step, "kind") == "shell" {
                    "shell".into()
                } else {
                    "pane".into()
                },
                Some(after),
            )
        }
        fn stop(self: &Rc<Self>) -> Result<(), JsValue> {
            self.write_state("run", JsValue::NULL)?;
            self.render()
        }
        fn toggle(self: &Rc<Self>, key: JsValue, focus: bool) -> Result<(), JsValue> {
            let open = self.state("open");
            let on = call(&open, "has", std::slice::from_ref(&key))?;
            call(
                &open,
                if truthy(&on) { "delete" } else { "add" },
                std::slice::from_ref(&key),
            )?;
            self.write_open();
            self.render()?;
            if focus {
                let h = all(&self.el, "[data-toggle]")
                    .into_iter()
                    .find(|n| get(&get(n, "dataset"), "toggle") == key);
                if let Some(h) = h {
                    let _ = call(&h, "focus", &[]);
                }
            }
            Ok(())
        }
        fn set_hidden(&self, hidden: bool) -> Result<(), JsValue> {
            self.write_state("termsHidden", hidden.into())?;
            self.write(HIDDEN, if hidden { "1" } else { "0" }.into());
            Ok(())
        }
        fn focus_term(self: &Rc<Self>, id: JsValue) -> Result<(), JsValue> {
            let id = utf16_string(&id);
            self.write_state("curTerm", utf16_value(&id))?;
            if truthy(&self.state("termsHidden")) {
                self.set_hidden(false)?;
            }
            if let Some(x) = list(&self.terminals())
                .iter()
                .find(|x| s(&get(x, "tabId")) == id)
            {
                let t = object();
                set(&t, "kind", &"term".into())?;
                for key in ["tabId", "paneKey", "session", "pane"] {
                    set(&t, key, &get(x, key))?;
                }
                set(&t, "title", &or(get(x, "label"), get(x, "tabId")))?;
                self.callback("focusTarget", &[t])?;
            }
            self.render()
        }
        fn close_term(self: &Rc<Self>, id: JsValue) -> Result<(), JsValue> {
            let id = utf16_string(&id);
            if let Ok(timer) = self.timer.try_borrow() {
                cancel(timer.clone());
            }
            if s(&self.state("closeArm")) != id {
                self.write_state("closeArm", utf16_value(&id))?;
                let owner = self.clone();
                let expected = id.clone();
                let timer = later(
                    function(move |_| {
                        if s(&owner.state("closeArm")) == expected {
                            owner.write_state("closeArm", JsValue::NULL)?;
                            owner.render()?;
                        }
                        Ok(JsValue::UNDEFINED)
                    }),
                    3000.,
                );
                if let Ok(mut slot) = self.timer.try_borrow_mut() {
                    *slot = timer;
                }
                return self.render();
            }
            self.write_state("closeArm", JsValue::NULL)?;
            if self.current() == id {
                self.write_state("curTerm", "".into())?;
            }
            self.render()?;
            let kill = self.callback("killTerm", &[utf16_value(&id)]);
            let owner = self.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(error) = wait(kill).await {
                    owner.quiet("toast", &[message(&error, &string(&error)), true.into()]);
                }
            });
            Ok(())
        }
        fn term_action(self: &Rc<Self>, kind: &str, id: JsValue) -> Result<(), JsValue> {
            match kind {
                "close" if truthy(&id) => self.close_term(id),
                "new" => {
                    if truthy(&self.state("termsHidden")) {
                        self.set_hidden(false)?;
                    }
                    self.callback("newTerm", &[])?;
                    Ok(())
                }
                "toggle" => {
                    self.set_hidden(!truthy(&self.state("termsHidden")))?;
                    self.render()
                }
                "focus" if truthy(&id) => self.focus_term(id),
                _ => Ok(()),
            }
        }
        fn set_sheet(self: &Rc<Self>, key: &str) -> Result<(), JsValue> {
            let next = if ["cmds", "chains", "srv"].contains(&key) {
                key
            } else {
                ""
            };
            if s(&self.state("sheet")) == next {
                return Ok(());
            }
            self.write_state("sheet", next.into())?;
            self.render()?;
            if next == "cmds" {
                let _ = call(&self.query(".cs-search"), "focus", &[]);
            }
            Ok(())
        }
        fn cmds_info(&self) -> Result<JsValue, JsValue> {
            let mut h = 0.;
            let usage = self.query(".cs-empty-terms");
            let usage_visible = !nullish(&usage) && !truthy(&get(&usage, "hidden"));
            // Quotas fill the user's saved sidebar share. Reporting their own
            // bottom as a pinned height creates a resize feedback loop in GTK.
            if !usage_visible
                && let Ok(r) = call(&self.query(".cs-tools"), "getBoundingClientRect", &[])
                && truthy(&get(&r, "height"))
            {
                let win = get(&get(&self.el, "ownerDocument"), "defaultView");
                h = (number(&get(&r, "bottom")) + number(&or(get(&win, "scrollY"), 0.into())))
                    .ceil();
            }
            let info = object();
            set(&info, "open", &truthy(&self.state("sheet")).into())?;
            set(&info, "sheet", &self.state("sheet"))?;
            set(&info, "h", &h.into())?;
            set(
                &info,
                "empty",
                &view::empty_terms(
                    list(&self.terminals()).len(),
                    truthy(&self.state("termsHidden")),
                )
                .is_some()
                .into(),
            )?;
            Ok(info)
        }
        fn paint_sheet(self: &Rc<Self>, count: usize) -> Result<(), JsValue> {
            let cur = s(&self.state("sheet"));
            classes(&self.el, "sheet-open", !cur.is_empty());
            attr(&self.el, "data-panel", &cur);
            for b in all(&self.el, "[data-sheet]") {
                let on = s(&get(&get(&b, "dataset"), "sheet")) == cur;
                classes(&b, "on", on);
                attr(&b, "aria-pressed", if on { "true" } else { "false" });
            }
            let title = match cur.as_str() {
                "cmds" => "Comandos del pane",
                "chains" => "Cadenas guardadas",
                "srv" => "Servidores SSH",
                _ => "",
            };
            let ttl = self.query(".cs-head .sh-t");
            if !nullish(&ttl) {
                set(&ttl, "textContent", &title.into())?;
            }
            let cnt = self.query(".cs-tools .n");
            if !nullish(&cnt) {
                let count = list(&self.state("chains")).len();
                set(
                    &cnt,
                    "textContent",
                    &if count == 0 {
                        "".into()
                    } else {
                        count.to_string().into()
                    },
                )?;
            }
            set(&self.query(".cs-sheet"), "hidden", &cur.is_empty().into())?;
            let empty = view::empty_terms(count, truthy(&self.state("termsHidden")));
            let box_ = self.query(".cs-empty-terms");
            if !nullish(&box_) {
                set(&box_, "hidden", &(!cur.is_empty()).into())?;
                // Account usage belongs to the sidebar even while a quick
                // terminal is visible; only the empty-terminal hint is conditional.
                // Hidden terminals reopen from the arrow in their bar; the footer only offers a new one.
                set(
                    &query(&box_, ".et-foot"),
                    "hidden",
                    &empty.is_none_or(|e| e.2 == "toggle").into(),
                )?;
                let native = invoke(&global("inApp"), &[]).is_ok_and(|v| truthy(&v));
                style(
                    &box_,
                    "flex",
                    if empty.is_none() && !native {
                        "0 1 48%"
                    } else {
                        "1 1 auto"
                    },
                );
                style(&box_, "max-height", "none");
                style(
                    &box_,
                    "min-height",
                    if empty.is_none() && !native {
                        "180px"
                    } else {
                        "0"
                    },
                );
                style(&box_, "order", "1");
                if let Some((text, go, action)) = empty {
                    set(&query(&box_, ".et-t"), "textContent", &text.into())?;
                    let button = query(&box_, ".et-go");
                    set(&button, "textContent", &go.into())?;
                    attr(&button, "data-term-act", action);
                }
                let usage = self.state("view") == "usage";
                // «Sesiones» (ficha viva): the activity feed painted by `session_feed`.
                let sessions = self.state("view") == "sessions";
                set(&query(&box_, ".et-lim"), "hidden", &(!usage).into())?;
                set(
                    &query(&box_, ".cs-explorer"),
                    "hidden",
                    &(usage || sessions).into(),
                )?;
                let feed = query(&box_, ".cs-feed");
                if !nullish(&feed) {
                    set(&feed, "hidden", &(!sessions).into())?;
                }
                for button in all(&self.el, "[data-sidebar-view]") {
                    let on = get(&get(&button, "dataset"), "sidebarView") == self.state("view");
                    classes(&button, "on", on);
                    attr(&button, "aria-selected", if on { "true" } else { "false" });
                }
                if usage && cur.is_empty() {
                    self.quiet("renderLimits", &[query(&box_, ".cs-limits")]);
                }
                self.browser.sync(if !cur.is_empty() {
                    ""
                } else if usage {
                    "usage"
                } else if sessions {
                    "sessions"
                } else {
                    "files"
                });
            }
            if cur == "srv" {
                self.write_state("srvMounted", true.into())?;
                self.quiet("mountServers", &[self.query(".cs-srv-slot")]);
            } else if truthy(&self.state("srvMounted")) {
                self.write_state("srvMounted", false.into())?;
                self.quiet("mountServers", &[JsValue::NULL]);
            }
            let tools = self.query(".cs-tools");
            let height = get(&tools, "offsetHeight");
            if truthy(&height) {
                style(&self.el, "--cs-tools-h", &format!("{}px", string(&height)));
            }
            Ok(())
        }
        fn sync_arrows(&self) -> Result<(), JsValue> {
            let tr = self.query(".cs-terms .tabs");
            if nullish(&tr) || nullish(&get(&tr, "scrollWidth")) {
                return Ok(());
            }
            let parent = get(&tr, "parentNode");
            let width = number(&get(&parent, "clientWidth"));
            let narrow = truthy(&parent) && width > 0. && width < 250.;
            let total = number(&get(&tr, "scrollWidth"));
            let shown = number(&get(&tr, "clientWidth"));
            let left = number(&get(&tr, "scrollLeft"));
            let over = !narrow && total > shown + 2.;
            let at_start = left <= 2.;
            let at_end = left + shown >= total - 2.;
            classes(&tr, "at-start", at_start);
            classes(&tr, "at-end", at_end);
            for b in all(&self.el, ".cs-terms [data-tscroll]") {
                set(&b, "hidden", &(!over).into())?;
                set(
                    &b,
                    "disabled",
                    &if get(&get(&b, "dataset"), "tscroll") == "-1" {
                        at_start
                    } else {
                        at_end
                    }
                    .into(),
                )?;
            }
            Ok(())
        }
        fn wire_track(self: &Rc<Self>) -> Result<(), JsValue> {
            let tr = self.query(".cs-terms .tabs");
            if nullish(&tr) {
                return Ok(());
            }
            let owner = self.clone();
            call(
                &tr,
                "addEventListener",
                &[
                    "scroll".into(),
                    function(move |_| {
                        owner.sync_arrows()?;
                        Ok(JsValue::UNDEFINED)
                    }),
                ],
            )?;
            let track = tr.clone();
            let owner = self.clone();
            let options = object();
            set(&options, "passive", &false.into())?;
            call(
                &tr,
                "addEventListener",
                &[
                    "wheel".into(),
                    function(move |a| {
                        let e = a.get(0);
                        let y = number(&get(&e, "deltaY"));
                        let x = number(&get(&e, "deltaX"));
                        if y.abs() > x.abs()
                            && number(&get(&track, "scrollWidth"))
                                > number(&get(&track, "clientWidth"))
                        {
                            set(
                                &track,
                                "scrollLeft",
                                &(number(&get(&track, "scrollLeft")) + y).into(),
                            )?;
                            call(&e, "preventDefault", &[])?;
                        }
                        let _ = &owner;
                        Ok(JsValue::UNDEFINED)
                    }),
                    options,
                ],
            )?;
            let win = get(&get(&self.el, "ownerDocument"), "defaultView");
            let observer = get(&win, "ResizeObserver");
            if !nullish(&observer) {
                let owner = self.clone();
                if let Ok(f) = observer.dyn_into::<js_sys::Function>()
                    && let Ok(o) = js_sys::Reflect::construct(
                        &f,
                        &[function(move |_| {
                            owner.sync_arrows()?;
                            Ok(JsValue::UNDEFINED)
                        })]
                        .into_iter()
                        .collect::<js_sys::Array>(),
                    )
                {
                    let _ = call(&o, "observe", &[tr]);
                }
            }
            Ok(())
        }
        fn render(self: &Rc<Self>) -> Result<(), JsValue> {
            let head = self.query(".cs-head");
            let body = self.query(".cs-body");
            if !nullish(&head) && !nullish(&body) {
                let title = query(&head, ".cs-target");
                if !nullish(&title) {
                    set(&title, "textContent", &utf16_value(&self.target_title()))?;
                }
                let pill = query(&head, ".cs-pill");
                if !nullish(&pill) {
                    html(&pill, self.pill())?;
                }
                html(&body, self.body())?;
                let chains = self.query(".cs-chains-body");
                if !nullish(&chains) {
                    html(&chains, self.chains_html())?;
                }
            } else {
                html(
                    &self.el,
                    view::shell_html(
                        &view::head_html(&self.target_title(), &self.pill(), &s(&self.state("q"))),
                        &self.body(),
                        &self.chains_html(),
                    ),
                )?;
                self.wire_track()?;
                let clock = id("clock");
                if !nullish(&clock) {
                    let _ = call(&self.query(".cs-footer"), "appendChild", &[clock]);
                }
            }
            let track = or(self.query(".cs-terms .tabs"), self.query(".cs-terms"));
            if !nullish(&track) {
                let left = or(get(&track, "scrollLeft"), 0.into());
                let was = self.state("shownTerm");
                let terms = self.terminals();
                let list = list(&terms);
                let current = self.state("curTerm");
                let shown = if truthy(&current) && list.iter().any(|x| get(x, "tabId") == current) {
                    current
                } else {
                    list.first().map(|x| get(x, "tabId")).unwrap_or("".into())
                };
                if shown != self.state("curTerm") {
                    self.write_state("curTerm", shown.clone())?;
                }
                html(
                    &track,
                    view::terms_html(
                        &j(&terms),
                        &j(&self.target()),
                        &s(&shown),
                        &s(&self.state("closeArm")),
                    ),
                )?;
                set(&track, "scrollLeft", &left)?;
                self.write_state("shownTerm", shown.clone())?;
                if was != shown {
                    let on = query(&track, ".tw.on");
                    let options = from_json(&json!({"block":"nearest","inline":"nearest"}))?;
                    let _ = call(&on, "scrollIntoView", &[options]);
                }
                self.sync_arrows()?;
            }
            let count = list(&self.terminals()).len();
            let slot = self.query(".cs-terms .tog-slot");
            if !nullish(&slot) {
                html(
                    &slot,
                    view::toggle_html(truthy(&self.state("termsHidden")), count > 0),
                )?;
            }
            self.paint_sheet(count)?;
            classes(&self.el, "no-terms", count == 0);
            classes(&self.el, "terms-hidden", truthy(&self.state("termsHidden")));
            for sel in [".sec-cmds", ".sec-terms"] {
                let el = self.query(sel);
                if !nullish(&el) {
                    let _ = set(&get(&el, "style"), "flex", &"".into());
                }
            }
            let mini = self.query(".mini");
            if !nullish(&mini) {
                let info = object();
                set(&info, "session", &or(self.state("curTerm"), "".into()))?;
                set(&info, "hidden", &truthy(&self.state("termsHidden")).into())?;
                set(
                    &info,
                    "tabs",
                    &from_utf16_json(&view::term_tabs(
                        &j(&self.terminals()),
                        &j(&self.target()),
                        &self.current(),
                        &s(&self.state("closeArm")),
                    ))?,
                )?;
                set(&info, "cmds", &self.cmds_info()?)?;
                self.quiet(
                    "mountTerm",
                    &[
                        if truthy(&self.state("termsHidden")) {
                            "".into()
                        } else {
                            or(self.state("curTerm"), "".into())
                        },
                        mini,
                        info,
                    ],
                );
            }
            classes(
                &self.el,
                "searching",
                !s(&self.state("q")).trim().is_empty(),
            );
            self.quiet("hydrate", std::slice::from_ref(&self.el));
            Ok(())
        }
        fn click(self: &Rc<Self>, event: JsValue) -> Result<JsValue, JsValue> {
            let t = get(&event, "target");
            if nullish(&t) || nullish(&get(&t, "closest")) {
                return Ok(JsValue::UNDEFINED);
            }
            for (sel, action) in [
                ("[data-run-next]", "next"),
                ("[data-run-stop]", "stop"),
                ("[data-open-builder]", "builder"),
                ("[data-sidebar-view]", "view"),
                ("[data-sheet]", "sheet"),
                ("[data-sheet-close]", "sheet-close"),
                ("[data-term-act]", "term-act"),
                ("[data-close-term]", "close"),
                ("[data-tscroll]", "scroll"),
                ("[data-new-term]", "new"),
                ("[data-terms-toggle]", "toggle-terms"),
                ("[data-focus-term]", "focus"),
                ("[data-cmd]", "cmd"),
            ] {
                let n = closest(&t, sel);
                if nullish(&n) {
                    continue;
                }
                let data = get(&n, "dataset");
                match action {
                    "view" => {
                        let view = get(&data, "sidebarView");
                        if view == "files" || view == "usage" || view == "sessions" {
                            self.write_state("view", view)?;
                            self.write_state("sheet", "".into())?;
                            self.render()?;
                        }
                    }
                    "next" => {
                        self.next()?;
                    }
                    "stop" => self.stop()?,
                    "builder" => {
                        self.callback("openBuilder", &[])?;
                    }
                    "sheet" => {
                        let _ = set(&self.query(".cs-more"), "open", &false.into());
                        let key = s(&get(&data, "sheet"));
                        self.set_sheet(if key == s(&self.state("sheet")) {
                            ""
                        } else {
                            &key
                        })?;
                    }
                    "sheet-close" => self.set_sheet("")?,
                    "term-act" => {
                        self.term_action(&s(&get(&data, "termAct")), JsValue::UNDEFINED)?
                    }
                    "close" => self.close_term(get(&data, "closeTerm"))?,
                    "scroll" => {
                        let tr = self.query(".cs-terms .tabs");
                        if !nullish(&tr) {
                            set(
                                &tr,
                                "scrollLeft",
                                &(number(&get(&tr, "scrollLeft"))
                                    + number(&get(&data, "tscroll")) * 150.)
                                    .into(),
                            )?;
                            self.sync_arrows()?;
                        }
                    }
                    "new" => self.term_action("new", JsValue::UNDEFINED)?,
                    "toggle-terms" => self.term_action("toggle", JsValue::UNDEFINED)?,
                    "focus" => self.focus_term(get(&data, "focusTerm"))?,
                    "cmd" => {
                        let row = closest(&n, ".cmd");
                        if contains(&row, "dis") {
                            let t = self.target();
                            if !truthy(&get(&t, "session")) || !truthy(&get(&t, "pane")) {
                                self.quiet(
                                    "toast",
                                    &["Selecciona un pane primero".into(), true.into()],
                                );
                            }
                        } else {
                            let result = self.insert(
                                get(&data, "cmd"),
                                if get(&data, "kind") == "shell" {
                                    "shell".into()
                                } else {
                                    "pane".into()
                                },
                            )?;
                            if truthy(&result) {
                                self.set_sheet("")?;
                            }
                        }
                    }
                    _ => {}
                }
                return Ok(JsValue::UNDEFINED);
            }
            let saved = closest(&t, ".cs-saved-item[data-run]");
            if !nullish(&closest(&t, "button")) && !nullish(&saved) {
                self.start(get(&get(&saved, "dataset"), "run"))?;
                return Ok(JsValue::UNDEFINED);
            }
            let toggle = closest(&t, "[data-toggle]");
            if !nullish(&toggle) {
                self.toggle(get(&get(&toggle, "dataset"), "toggle"), false)?;
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
                        if get(&e, "key") == "Escape" && truthy(&owner.state("sheet")) {
                            let _ = call(&e, "preventDefault", &[]);
                            owner.set_sheet("")?;
                            return Ok(JsValue::UNDEFINED);
                        }
                        let t = get(&e, "target");
                        let n = closest(&t, "[data-toggle]");
                        if n == t
                            && !nullish(&n)
                            && view::toggle_key(
                                &s(&get(&e, "key")),
                                truthy(&get(&e, "isComposing")),
                            )
                        {
                            let _ = call(&e, "preventDefault", &[]);
                            owner.toggle(get(&get(&n, "dataset"), "toggle"), true)?;
                        }
                        Ok(JsValue::UNDEFINED)
                    }),
                ],
            )?;
            for event in ["input", "compositionend"] {
                let owner = self.clone();
                call(
                    &self.el,
                    "addEventListener",
                    &[
                        event.into(),
                        function(move |a| {
                            let e = a.get(0);
                            let t = get(&e, "target");
                            if contains(&t, "cs-search")
                                && (event == "compositionend" || !truthy(&get(&e, "isComposing")))
                            {
                                let value = get(&t, "value");
                                owner.write_state("q", utf16_value(&s(&value)))?;
                                owner.render()?;
                            }
                            Ok(JsValue::UNDEFINED)
                        }),
                    ],
                )?;
            }
            Ok(())
        }
        fn create(opts: JsValue) -> Result<JsValue, JsValue> {
            let el = get(&opts, "root");
            let state = from_json(
                &json!({"catalog":null,"cliInPane":"","catalogTarget":null,"chains":[],"run":null,"curTerm":"","typing":null,"q":"","firstRender":true,"appliedTarget":null,"sheet":"","view":"files"}),
            )?;
            set(&state, "open", &constructor("Set", &[])?)?;
            let owner = Rc::new(Self {
                browser: super::browser::Browser::new(opts.clone(), el.clone()),
                opts,
                el,
                state,
                timer: RefCell::new(0.into()),
            });
            let read = owner.read(OPEN);
            if let Some(saved) = read
                .as_string()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .filter(Value::is_array)
            {
                let values = js_sys::Array::new();
                for value in view::arr(&saved) {
                    values.push(&utf16_value(&view::txt(&value)));
                }
                owner.write_state("open", constructor("Set", &[values.into()])?)?;
                owner.write_state("firstRender", false.into())?;
            }
            owner.write_state("termsHidden", (owner.read(HIDDEN) == "1").into())?;
            owner.wire()?;
            let result = object();
            let b = owner.clone();
            method(&result, "refresh", move |_| Ok(b.refresh()))?;
            let b = owner.clone();
            method(&result, "render", move |_| {
                b.render()?;
                Ok(JsValue::UNDEFINED)
            })?;
            let b = owner.clone();
            method(&result, "insert", move |a| b.insert(a.get(0), a.get(1)))?;
            let b = owner.clone();
            method(&result, "startChain", move |a| b.start(a.get(0)))?;
            let b = owner.clone();
            method(&result, "next", move |_| b.next())?;
            let b = owner.clone();
            method(&result, "stop", move |_| {
                b.stop()?;
                Ok(JsValue::UNDEFINED)
            })?;
            let b = owner.clone();
            method(&result, "applyCatalog", move |a| {
                b.apply_catalog(a.get(0))?;
                Ok(JsValue::UNDEFINED)
            })?;
            let b = owner.clone();
            method(&result, "termAction", move |a| {
                b.term_action(&s(&a.get(0)), a.get(1))?;
                Ok(JsValue::UNDEFINED)
            })?;
            let b = owner.clone();
            method(&result, "setCmdsOpen", move |a| {
                let open = truthy(&a.get(0));
                let current = s(&b.state("sheet"));
                b.set_sheet(if !open {
                    ""
                } else if current.is_empty() {
                    "cmds"
                } else {
                    &current
                })?;
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
            let started = if action == "analytics" {
                let cb = get(&self.opts, "onAnalytics");
                if nullish(&cb) {
                    Ok(JsValue::UNDEFINED)
                } else {
                    call(&self.opts, "onAnalytics", std::slice::from_ref(&a))
                }
            } else if switching {
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
    fn options(v: &JsValue) -> Value {
        let mut result = j(v);
        if let Some(o) = result.as_object_mut() {
            o.insert(
                "open".into(),
                j(&js_sys::Array::from(&or(get(v, "open"), js_sys::Array::new().into())).into()),
            );
        }
        result
    }
    pub fn mount() -> Result<(), JsValue> {
        let exports = object();
        method(&exports, "createCommandSidebar", |a| {
            Sidebar::create(a.get(0))
        })?;
        method(&exports, "rowHTML", |a| {
            Ok(utf16_value(&view::row_html(&j(&a.get(0)), &j(&a.get(1)))))
        })?;
        method(&exports, "cliHTML", |a| {
            Ok(utf16_value(&view::cli_html(
                &j(&a.get(0)),
                &options(&a.get(1)),
            )))
        })?;
        method(&exports, "esc", |a| {
            Ok(utf16_value(&comandos_web_view::escape::text(&s(&a.get(0)))))
        })?;
        method(&exports, "isToggleKey", |a| {
            let e = a.get(0);
            Ok(view::toggle_key(&s(&get(&e, "key")), truthy(&get(&e, "isComposing"))).into())
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

#[cfg(all(test, target_arch = "wasm32"))]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen(inline_js = r#"
export async function retainedSidebarChecks(root) {
  const previous=process.cwd();
  process.chdir(root);
  const {createRequire}=await import('node:module');
  const require=createRequire(process.cwd()+'/tests/command_sidebar_checks.cjs');
  const Module=require('node:module');
  const original=Module._load;
  Module._load=function(request,...args){
    if(request===process.cwd()+'/dash/command-sidebar.js') return globalThis.ComandosCommandSidebar;
    return original.call(this,request,...args);
  };
  globalThis.window=globalThis;
  const log=console.log;
  const pending=new Set(['command-sidebar checks ok','LED limits checks ok','LED Analytics wiring checks ok','LED interaction checks ok']);
  let complete;
  const done=new Promise(resolve=>{complete=resolve;});
  console.log=(...args)=>{log(...args);pending.delete(args[0]);if(!pending.size)complete();};
  let timer;
  try {
    require(process.cwd()+'/tests/command_sidebar_checks.cjs');
    await Promise.race([done,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('Incomplete sidebar checks: '+[...pending].join(', '))),10000);})]);
    const assert=require('node:assert/strict');
    const {mkRoot}=require(process.cwd()+'/tests/dom_stub.cjs');
    let release;
    const root=mkRoot();
    const limits=globalThis.ComandosCommandSidebar.createLimitsView(root,{onSwitch(){assert.equal(this.marker,'owner');return new Promise(resolve=>{release=resolve;});},marker:'owner'});
    limits.update([{id:'claude:main',provider:'claude',cli:'Claude',alias:'main',week:20}],'claude:other');
    root.click('.usage-toggle');root.click('[data-limit-action="switch"]');
    assert.equal(root.querySelector('[data-limit-action="switch"]').getAttribute('disabled')!==null,true);
    release();await new Promise(resolve=>setImmediate(resolve));
    assert.equal(root.querySelector('[data-limit-action="switch"]').getAttribute('disabled')!==null,false);
    const calls=[];
    const sidebar=globalThis.ComandosCommandSidebar.createCommandSidebar({root:mkRoot(),getTarget:()=>null,api:(path)=>{calls.push(path);return Promise.resolve(path==='/chains'?{chains:[]}:{catalog:{clis:[]}});}});
    const refreshed=sidebar.refresh();assert.deepEqual(calls,['/commands/catalog','/chains']);await refreshed;
  } finally {clearTimeout(timer);console.log=log;Module._load=original;process.chdir(previous);}
}
"#)]
    extern "C" {
        #[wasm_bindgen(catch,js_name=retainedSidebarChecks)]
        async fn retained_checks(root: &str) -> Result<JsValue, JsValue>;
    }
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn retained_node_contracts_against_rust_exports() {
        super::mount().unwrap();
        retained_checks(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
            .await
            .unwrap();
    }
}
