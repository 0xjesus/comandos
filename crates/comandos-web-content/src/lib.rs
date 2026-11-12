//! The news reader component in a separate Rust/WASM artifact.
//! The core registry owns selection; public APIs stay synchronous after transport initialization.
#![cfg(target_arch = "wasm32")]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::panic,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]
use comandos_web_dom::bridge::global_set;
use comandos_web_dom::port::*;
use wasm_bindgen::{JsValue, prelude::wasm_bindgen};
mod news_reader;
#[allow(dead_code)]
#[path = "../../comandos-web/src/components/web_support.rs"]
mod web_support;
/// Registers private native transport hooks; component globals still mount in registry order.
#[wasm_bindgen]
pub fn register_content() -> Result<(), JsValue> {
    for (key, mount) in [
        (
            "__comandosMountNewsReader",
            news_reader::mount as fn() -> Result<(), JsValue>,
        ),
        (
            "__comandosAttachNewsReader",
            news_reader::attach as fn() -> Result<(), JsValue>,
        ),
        (
            "__comandosRetireNewsVendor",
            news_reader::retire_vendor as fn() -> Result<(), JsValue>,
        ),
    ] {
        global_set(
            key,
            &function(move |_| {
                mount()?;
                Ok(JsValue::UNDEFINED)
            }),
        )?;
    }
    Ok(())
}
