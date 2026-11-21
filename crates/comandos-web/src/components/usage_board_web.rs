//! Ajustes › Consumos: pide `/usage/board` y lo dibuja mientras está a la vista.
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
    use comandos_web_view::usage_board as view;
    use std::cell::{Cell, RefCell};
    use wasm_bindgen::JsValue;

    /// Cada cuánto se vuelve a pedir el tablero mientras está a la vista.
    const REFRESH_MS: f64 = 60_000.0;
    /// Cada cuánto se comprueba si el tablero está a la vista.
    const WATCH_MS: f64 = 1_500.0;
    thread_local! {
        static BOARD: RefCell<serde_json::Value> = const { RefCell::new(serde_json::Value::Null) };
        static VIEW: RefCell<view::View> = RefCell::new(view::View::default());
        static LOADED: Cell<f64> = const { Cell::new(0.0) };
        static LOADING: Cell<bool> = const { Cell::new(false) };
    }
    fn host() -> JsValue {
        id("usage-board")
    }
    fn visible() -> bool {
        let h = host();
        truthy(&h) && truthy(&get(&h, "offsetParent"))
    }
    fn render() -> Result<(), JsValue> {
        let h = host();
        let board = BOARD.with(|b| b.borrow().clone());
        if !truthy(&h) || board.is_null() {
            return Ok(());
        }
        let html = VIEW.with(|v| view::board(&board, &v.borrow(), english()));
        set(&h, "innerHTML", &html.into())
    }
    fn failed() -> Result<(), JsValue> {
        // Con un tablero ya dibujado se conserva; el siguiente intento lo renueva.
        if BOARD.with(|b| !b.borrow().is_null()) {
            return Ok(());
        }
        let h = host();
        if truthy(&h) {
            set(
                &h,
                "innerHTML",
                &format!(
                    "<div class=\"desc\">{}</div>",
                    t(
                        "No se pudieron cargar los consumos. Se vuelve a intentar en un minuto.",
                        "Usage could not be loaded. Trying again in a minute."
                    )
                )
                .into(),
            )?;
        }
        Ok(())
    }
    fn load() -> JsValue {
        if LOADING.with(|l| l.replace(true)) {
            return JsValue::UNDEFINED;
        }
        let fetched = invoke(&global("api"), &["/usage/board".into()]);
        promise(async move {
            let result = wait(fetched).await;
            LOADING.with(|l| l.set(false));
            LOADED.with(|l| l.set(js_sys::Date::now()));
            let board = result.as_ref().map(to_json).unwrap_or_default();
            if board.get("projects").is_none() {
                failed()?;
                return Ok(JsValue::UNDEFINED);
            }
            // La primera vez se abre el proyecto que más pesa.
            if BOARD.with(|b| b.borrow().is_null())
                && let Some(name) = board
                    .get("projects")
                    .and_then(|p| p.get(0))
                    .and_then(|p| p.get("name"))
                    .and_then(serde_json::Value::as_str)
            {
                VIEW.with(|v| v.borrow_mut().open_projects.insert(name.to_owned()));
            }
            BOARD.with(|b| *b.borrow_mut() = board);
            render()?;
            Ok(JsValue::UNDEFINED)
        })
    }
    fn watch() {
        let stale = js_sys::Date::now() - LOADED.with(Cell::get) > REFRESH_MS;
        if stale && visible() {
            let _ = load();
        }
    }
    /// Abre o cierra el proyecto o la sesión de la fila tocada.
    fn toggle(target: &JsValue) -> Result<(), JsValue> {
        let row = call(
            target,
            "closest",
            &["[data-ub-proj],[data-ub-sess],[data-ub-all]".into()],
        )?;
        if !truthy(&row) {
            return Ok(());
        }
        if call(&row, "hasAttribute", &["data-ub-all".into()]).is_ok_and(|v| truthy(&v)) {
            VIEW.with(|v| {
                let mut v = v.borrow_mut();
                v.all = !v.all;
            });
            return render();
        }
        let data = get(&row, "dataset");
        let (proj, sess) = (get(&data, "ubProj"), get(&data, "ubSess"));
        VIEW.with(|v| {
            let mut v = v.borrow_mut();
            if let Some(name) = proj.as_string() {
                if !v.open_projects.remove(&name) {
                    v.open_projects.insert(name);
                }
            } else if let Some(id) = sess.as_string() {
                v.open_session = (v.open_session.as_deref() != Some(id.as_str())).then_some(id);
            }
        });
        render()
    }
    pub fn mount() -> Result<(), JsValue> {
        set(
            &js_sys::global(),
            "loadUsageBoard",
            &function(|_| Ok(load())),
        )
    }
    pub fn attach() -> Result<(), JsValue> {
        let h = host();
        if !truthy(&h) {
            return Ok(());
        }
        listen(
            &h,
            "click",
            function(|a| {
                toggle(&get(&a.get(0), "target"))?;
                Ok(JsValue::UNDEFINED)
            }),
        );
        listen(
            &h,
            "keydown",
            function(|a| {
                let e = a.get(0);
                let key = string(&get(&e, "key"));
                if key == "Enter" || key == " " {
                    call(&e, "preventDefault", &[])?;
                    toggle(&get(&e, "target"))?;
                }
                Ok(JsValue::UNDEFINED)
            }),
        );
        call(
            &js_sys::global(),
            "setInterval",
            &[
                function(|_| {
                    watch();
                    Ok(JsValue::UNDEFINED)
                }),
                WATCH_MS.into(),
            ],
        )?;
        Ok(())
    }
}
