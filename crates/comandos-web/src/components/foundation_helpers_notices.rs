//! Sovereignty inventory modal and the new-model notice on load.
use super::*;
fn storage(k: &str) -> JsValue {
    call(&global("localStorage"), "getItem", &[k.into()]).unwrap_or(JsValue::NULL)
}
fn locale(v: &Value) -> String {
    call(&js_sys::Number::from(num(v)).into(), "toLocaleString", &[])
        .map(|v| utf16_string(&v))
        .unwrap_or_else(|_| s(v))
}
async fn sovereignty() -> Result<JsValue, JsValue> {
    let box_ = id("sov-body");
    if box_.is_null() {
        return Ok(JsValue::UNDEFINED);
    }
    let d = match api("/sovereignty", None).await {
        Ok(d) => to_utf16_json(&d),
        Err(_) => {
            html(
                &box_,
                format!(
                    "<div class=\"desc\">{}</div>",
                    tf(
                        "No se pudo leer el inventario",
                        "Could not read the inventory"
                    )
                ),
            );
            return Ok(JsValue::UNDEFINED);
        }
    };
    let stores = list(at(&d, "stores"));
    let total = stores.iter().map(|x| num(at(x, "bytes"))).sum::<f64>();
    let mut out = format!(
        "<div class=\"sov-h\">{}</div><div class=\"sov-flow\"><div class=\"sov-node\"><b>{}</b><small>Claude Code · Codex · OpenCode · Gemini<br>{}</small></div><div class=\"sov-arrow\">→</div><div class=\"sov-node here\"><b>{}</b><small>~/.claude/hooks · sqlite · jsonl · json<br>{} {} {} {}</small></div><div class=\"sov-arrow\">→</div><div class=\"sov-node\"><b>{}</b><small>127.0.0.1:4777 · cc-app (WebKitGTK)<br>{}</small></div></div><div class=\"sov-h\">{}</div><div class=\"sov-out\">",
        tf("Flujo", "Flow"),
        tf("Agentes", "Agents"),
        tf("hooks locales → eventos", "local hooks → events"),
        tf("Esta máquina", "This machine"),
        fmt_bytes(total),
        tf("en", "in"),
        stores.len(),
        tf("almacenes", "stores"),
        tf("Tablero y app", "Dashboard & app"),
        tf("popups con tema", "themed popups"),
        tf("Canales de salida", "Outbound channels")
    );
    for c in list(at(&d, "outbound")) {
        let on = truth(at(&c, "on"));
        out += &format!(
            "<div class=\"sov-ch {}\"><span class=\"st\">{}</span><span>{}</span><small>{}</small></div>",
            if on { "on" } else { "off" },
            if on {
                tf("ACTIVO", "ON")
            } else {
                tf("APAGADO", "OFF")
            },
            md_esc(&s(at(&c, "label"))),
            md_esc(&s(at(&c, "how")))
        );
    }
    out += &format!(
        "</div><div class=\"sov-h\">{}</div><table class=\"sov-table\"><thead><tr><th>{}</th><th>{}</th><th style=\"text-align:right\">{}</th><th>{}</th></tr></thead><tbody>",
        tf("Almacenes locales", "Local stores"),
        tf("Qué", "What"),
        tf("Dónde", "Where"),
        tf("Tamaño / filas", "Size / rows"),
        tf("Privado", "Private")
    );
    for x in stores {
        let tables = if x.get("tables").is_some() {
            format!(
                "<div class=\"sov-tables\">{}</div>",
                list(at(&x, "tables"))
                    .iter()
                    .map(|t| format!("{} {}", md_esc(&s(at(t, "name"))), locale(at(t, "rows"))))
                    .collect::<Vec<_>>()
                    .join(" · ")
            )
        } else {
            String::new()
        };
        let mut size = if x.get("bytes").is_some() {
            fmt_bytes(num(at(&x, "bytes")))
        } else {
            String::new()
        };
        if x.get("count").is_some() {
            if x.get("bytes").is_some() {
                size += " · ";
            }
            size += &locale(at(&x, "count"));
        }
        let secret = s(at(&x, "kind")) == "secret";
        let private = secret && truth(at(&x, "private"));
        out += &format!(
            "<tr><td>{}{tables}</td><td class=\"path\">{}</td><td class=\"num\">{size}</td><td><span class=\"sov-priv {}\">{}</span></td></tr>",
            md_esc(&s(at(&x, "label"))),
            md_esc(&s(at(&x, "path"))),
            if private { "" } else { "no" },
            if private {
                "0600 ✓".into()
            } else if truth(at(&x, "mode")) {
                s(at(&x, "mode"))
            } else {
                "—".into()
            }
        );
    }
    for b in list(at(&d, "browser")) {
        let v = storage(&s(at(&b, "key")));
        out += &format!(
            "<tr><td>{}</td><td class=\"path\">localStorage · {}</td><td class=\"num\">{}</td><td><span class=\"sov-priv no\">{}</span></td></tr>",
            md_esc(&s(at(&b, "label"))),
            s(at(&b, "key")),
            if v.is_null() {
                "—".into()
            } else {
                fmt_bytes(utf16_string(&v).encode_utf16().count() as f64)
            },
            tf("navegador", "browser")
        );
    }
    out += "</tbody></table>";
    html(&box_, out);
    Ok(JsValue::UNDEFINED)
}
pub fn mount() -> Result<(), JsValue> {
    let scope: JsValue = js_sys::global().into();
    method(&scope, "sovRender", |_| Ok(promise(sovereignty())))?;
    Ok(())
}
pub fn attach() -> Result<(), JsValue> {
    listen(
        &id("btn-sov"),
        "click",
        event(|_| {
            classes(&id("sovereignty"), "open", true);
            wasm_bindgen_futures::spawn_local(async {
                let _ = sovereignty().await;
            });
            Ok(())
        }),
    );
    wasm_bindgen_futures::spawn_local(async {
        if let Ok(v) = api("/models/latest", None).await {
            let _ = global_set("MODEL_NEWS", &v);
            let ns = get(&v, "newSince");
            let ns = if ns.is_falsy() { object() } else { ns };
            let entries = js_sys::Object::entries(&js_sys::Object::from(ns));
            let mut signature = serde_json::Map::new();
            let mut lines = Vec::new();
            for entry in entries.iter() {
                let pair = js_sys::Array::from(&entry);
                let provider = utf16_string(&pair.get(0));
                let models = get(&pair.get(1), "models");
                signature.insert(provider.clone(), to_utf16_json(&models));
                lines.push(format!(
                    "{provider}: {}",
                    js_rows(&models)
                        .iter()
                        .take(3)
                        .map(utf16_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            let signature = Value::Object(signature).to_string();
            if !lines.is_empty()
                && storage("cc-model-news").as_string().as_deref() != Some(&signature)
            {
                let _ = call(
                    &global("localStorage"),
                    "setItem",
                    &["cc-model-news".into(), signature.into()],
                );
                let line = lines.join(" · ");
                toast(
                    tf(
                        &format!(
                            "Modelos nuevos detectados — {line}. Pídeme agregarlos al registry."
                        ),
                        &format!(
                            "New models detected — {line}. Ask me to add them to the registry."
                        ),
                    ),
                    false,
                );
            }
        }
    });
    Ok(())
}
