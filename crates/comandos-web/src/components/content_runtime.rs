//! Native feature loading is separate from synchronous component APIs.
use comandos_web_dom::{bridge::global_set, port::*};
use js_sys::Promise;
use std::cell::RefCell;
use wasm_bindgen::{JsValue, prelude::wasm_bindgen};
thread_local! {static LOAD:RefCell<Option<Promise>>=const{RefCell::new(None)};}
fn status(feature: &str, state: &str, error: Option<&JsValue>) -> Result<(), JsValue> {
    let reports = get(&js_sys::global(), "__comandosFeatureReports");
    let reports = if reports.is_object() {
        reports
    } else {
        object()
    };
    let entry = object();
    set(&entry, "state", &state.into())?;
    if let Some(error) = error {
        set(&entry, "error", &string(error).into())?;
    }
    set(&reports, feature, &entry)?;
    global_set("__comandosFeatureReports", &reports)
}
pub fn declare(feature: &str) -> Result<(), JsValue> {
    status(feature, "deferred", None)
}
pub fn ready() -> bool {
    get(&js_sys::global(), "__comandosContentReady") == JsValue::TRUE
}
#[wasm_bindgen]
pub fn install_content_loader(loader: js_sys::Function) -> Result<(), JsValue> {
    global_set("__comandosImportContent", &loader.into())
}
fn initialize() -> Result<(), JsValue> {
    for (key, public) in [("__comandosMountNewsReader", "NewsReader")] {
        if get(&js_sys::global(), &format!("__comandosDeferred{public}")) != JsValue::TRUE {
            continue;
        }
        let old = get(&js_sys::global(), public);
        invoke(&get(&js_sys::global(), key), &[])?;
        let api = get(&js_sys::global(), public);
        global_set(&format!("__comandosImplementation{public}"), &api)?;
        if old.is_object() {
            js_sys::Object::assign(&old.clone().into(), &api.into());
            global_set(public, &old)?;
        }
    }
    if get(&js_sys::global(), "__comandosDeferredNewsReader") == JsValue::TRUE {
        invoke(&get(&js_sys::global(), "__comandosRetireNewsVendor"), &[])?;
        super::news_reader::remove_cold_listener()?;
        if get(&js_sys::global(), "__comandosNewsDisposed") != JsValue::TRUE {
            invoke(&get(&js_sys::global(), "__comandosAttachNewsReader"), &[])?;
        }
    }
    global_set("__comandosContentReady", &JsValue::TRUE)
}
/// Explicit async preparation; factories themselves continue to return their original types.
#[wasm_bindgen]
pub fn prepare_content(feature: &str) -> Promise {
    let feature = feature.to_owned();
    let existing = LOAD
        .try_with(|slot| slot.try_borrow().ok().and_then(|v| v.clone()))
        .ok()
        .flatten();
    let loaded = if let Some(p) = existing {
        p
    } else {
        let p = wasm_bindgen_futures::future_to_promise(async move {
            let imported = invoke(&get(&js_sys::global(), "__comandosImportContent"), &[])?;
            wait(Ok(imported)).await?;
            initialize()?;
            Ok(JsValue::UNDEFINED)
        });
        let _ = LOAD.try_with(|slot| {
            if let Ok(mut slot) = slot.try_borrow_mut() {
                *slot = Some(p.clone());
            }
        });
        p
    };
    let _ = status(&feature, "loading", None);
    wasm_bindgen_futures::future_to_promise(async move {
        match wait(Ok(loaded.into())).await {
            Ok(value) => {
                status(&feature, "ready", None)?;
                Ok(value)
            }
            Err(error) => {
                let _ = status(&feature, "failed", Some(&error));
                let _ = invoke(
                    &get(&js_sys::global(), "toast"),
                    &[
                        format!("{feature}: {}", string(&error)).into(),
                        JsValue::TRUE,
                    ],
                );
                Err(error)
            }
        }
    })
}
pub fn factory(public: &str, names: &[&'static str]) -> Result<JsValue, JsValue> {
    let api = object();
    let public = public.to_owned();
    for &name in names {
        let public = public.clone();
        method(&api, name, move |args| {
            let implementation = get(
                &js_sys::global(),
                &format!("__comandosImplementation{public}"),
            );
            if !implementation.is_object() {
                return Err(js_sys::Error::new("Native feature is not initialized; await prepare_content before synchronous factory use").into());
            }
            call(&implementation, name, &args.iter().collect::<Vec<_>>())
        })?;
    }
    global_set(&public, &api)?;
    Ok(api)
}
