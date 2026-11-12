//! Iframe pane selection (`selectPaneInFrame`). Selection never changes focus.
pub fn valid_pane(value: &str) -> bool {
    value
        .strip_prefix('%')
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn pane_identity_requires_exact_percent_ascii_digits() {
        for s in ["%0", "%12", "%0009"] {
            assert!(super::valid_pane(s));
        }
        for s in ["", "%", "12", "%12\n", "%١", "%12x", " %2"] {
            assert!(!super::valid_pane(s));
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use web::mount;
#[cfg(target_arch = "wasm32")]
mod web {
    use crate::components::web_support::*;
    use comandos_web_dom::port::*;
    use js_sys::Reflect;
    use wasm_bindgen::JsValue;
    fn tab(session: &JsValue) -> Result<JsValue, JsValue> {
        call(&global("openTerms"), "get", std::slice::from_ref(session))
    }
    fn select_frame(session: JsValue, pane: JsValue) -> Result<JsValue, JsValue> {
        let entry = tab(&session)?;
        let frame = if truthy(&entry) {
            get(&entry, "frame")
        } else {
            JsValue::UNDEFINED
        };
        if !truthy(&frame) {
            return Ok(JsValue::UNDEFINED);
        }
        let captured = frame.clone();
        let post = function(move |_| {
            let window = get(&captured, "contentWindow");
            if !window.is_null() && !window.is_undefined() {
                let message = object();
                set(&message, "source", &"comandos".into())?;
                set(&message, "type", &"select-pane".into())?;
                set(&message, "pane", &pane)?;
                call(
                    &window,
                    "postMessage",
                    &[message, get(&global("location"), "origin")],
                )?;
            }
            Ok(JsValue::UNDEFINED)
        });
        let ready = (|| -> Result<bool, JsValue> {
            let document = Reflect::get(&frame, &"contentDocument".into())?;
            if document.is_null()
                || document.is_undefined()
                || Reflect::get(&document, &"readyState".into())? != JsValue::from_str("complete")
            {
                return Ok(false);
            }
            let window = Reflect::get(&frame, &"contentWindow".into())?;
            let location = Reflect::get(&window, &"location".into())?;
            Ok(Reflect::get(&location, &"href".into())? != JsValue::from_str("about:blank"))
        })()
        .unwrap_or(false);
        if ready {
            invoke(&post, &[])?;
        } else {
            let options = object();
            set(&options, "once", &true.into())?;
            call(&frame, "addEventListener", &["load".into(), post, options])?;
        }
        Ok(JsValue::UNDEFINED)
    }
    pub fn mount() -> Result<(), JsValue> {
        set(
            &js_sys::global(),
            "selectPaneInFrame",
            &function(|args| select_frame(args.get(0), args.get(1))),
        )
    }
}
