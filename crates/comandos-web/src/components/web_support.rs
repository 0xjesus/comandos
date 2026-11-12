//! Shared browser primitives; component decisions remain in Rust.
use comandos_web_dom::port::*;
use wasm_bindgen::JsValue;
pub fn global(k: &str) -> JsValue {
    get(&js_sys::global(), k)
}
pub fn doc() -> JsValue {
    global("document")
}
pub fn query(root: &JsValue, sel: &str) -> JsValue {
    call(root, "querySelector", &[sel.into()]).unwrap_or(JsValue::NULL)
}
pub fn all(root: &JsValue, sel: &str) -> Vec<JsValue> {
    let list = call(root, "querySelectorAll", &[sel.into()]).unwrap_or(JsValue::NULL);
    let count = number(&get(&list, "length"));
    (0..count as u32)
        .map(|i| get(&list, &i.to_string()))
        .collect()
}
pub fn id(name: &str) -> JsValue {
    call(&doc(), "getElementById", &[name.into()]).unwrap_or(JsValue::NULL)
}
pub fn classes(el: &JsValue, name: &str, on: bool) {
    let _ = call(&get(el, "classList"), "toggle", &[name.into(), on.into()]);
}
pub fn attr(el: &JsValue, key: &str, value: &str) {
    let _ = call(el, "setAttribute", &[key.into(), value.into()]);
}
pub fn style(el: &JsValue, key: &str, value: &str) {
    let _ = call(
        &get(el, "style"),
        "setProperty",
        &[key.into(), value.into()],
    );
}
pub fn listen(el: &JsValue, event: &str, f: JsValue) {
    let _ = call(el, "addEventListener", &[event.into(), f]);
}
pub fn stop(event: &JsValue) {
    let _ = call(event, "stopPropagation", &[]);
    let _ = call(event, "preventDefault", &[]);
}
pub fn later(f: JsValue, ms: f64) -> JsValue {
    call(&js_sys::global(), "setTimeout", &[f, ms.into()]).unwrap_or(JsValue::NULL)
}
pub fn cancel(timer: JsValue) {
    let _ = call(&js_sys::global(), "clearTimeout", &[timer]);
}
pub fn english() -> bool {
    invoke(&global("__comandosTranslate"), &["es".into(), "en".into()])
        .ok()
        .and_then(|v| v.as_string())
        .as_deref()
        == Some("en")
}
pub fn t(es: &str, en: &str) -> String {
    if english() { en.into() } else { es.into() }
}
pub fn state() -> JsValue {
    invoke(&global("__comandosState"), &[]).unwrap_or(JsValue::UNDEFINED)
}
pub fn icon(name: &str, size: f64) -> String {
    invoke(&global("__comandosIcon"), &[name.into(), size.into()])
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}
pub async fn request(method: &str, path: &str, body: JsValue) -> Result<JsValue, JsValue> {
    let headers = object();
    if let Ok(token) = invoke(&global("authToken"), &[])
        && truthy(&token)
    {
        set(&headers, "X-Comandos-Token", &token)?;
    }
    let options = object();
    set(&options, "method", &method.into())?;
    set(&options, "headers", &headers)?;
    if truthy(&body) {
        set(&headers, "Content-Type", &"application/json".into())?;
        set(&options, "body", &js_sys::JSON::stringify(&body)?.into())?;
    }
    let r = wait(call(&js_sys::global(), "fetch", &[path.into(), options])).await?;
    let out = object();
    set(&out, "status", &get(&r, "status"))?;
    let parsed = wait(call(&r, "json", &[]))
        .await
        .unwrap_or_else(|_| object());
    set(&out, "body", &parsed)?;
    Ok(out)
}
pub fn toast(message: &str) {
    let _ = invoke(&global("toast"), &[message.into(), true.into()]);
}
// Explicitly selected by B6/B7: these arguments are internally encoded text.
#[allow(dead_code)] // shared with comandos-web-content
pub fn utf16_query(root: &JsValue, sel: &str) -> JsValue {
    call(root, "querySelector", &[utf16_value(sel)]).unwrap_or(JsValue::NULL)
}
pub fn utf16_attr(el: &JsValue, key: &str, value: &str) {
    let _ = call(el, "setAttribute", &[utf16_value(key), utf16_value(value)]);
}
#[allow(dead_code)] // shared with comandos-web-content
pub fn utf16_escape(value: &str) -> String {
    invoke(&global("mdEsc"), &[utf16_value(value)])
        .map(|v| utf16_string(&v))
        .unwrap_or_else(|_| comandos_web_view::escape::text(value))
}
pub fn utf16_toast(message: &str) {
    let _ = invoke(&global("toast"), &[utf16_value(message), true.into()]);
}
