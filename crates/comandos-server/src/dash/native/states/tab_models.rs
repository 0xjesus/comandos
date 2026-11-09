//! `write_app_tab_models` (6714) sin escribir: la proyección de las tarjetas
//! vivas que lee la barra de modelo de cc-app. La Tarea 5 la escribe solo
//! después de saber que `/state` no declina.
use super::{PyFloat, StateFault, is_text, py_float, py_str, str_or_empty};
use comandos_core::json::truthy;
use comandos_runtime::providers::{engine_for_model, model_tier, tier_symbol};
use serde_json::{Map, Value};
use std::collections::HashMap;

const DETECTING: &str = "detectando…";
/// Arneses con detalle por pane.
const DETAIL_HARNESSES: [&str; 7] = [
    "claude", "codex", "grok", "opencode", "gemini", "acp", "agy",
];

/// `$` de Python sin `re.M`: al final o antes de un `\n` final.
fn split_final_newline(s: &str) -> (&str, &str) {
    match s.strip_suffix('\n') {
        Some(body) => (body, "\n"),
        None => (s, ""),
    }
}

/// `re.sub(r"\[.*$", "", s)`: `.` no cruza `\n`, así que casa el primer `[`
/// cuya línea llega al final (o al `\n` final).
fn strip_context(s: &str) -> String {
    for (i, _) in s.match_indices('[') {
        let tail = s.get(i..).unwrap_or("");
        match tail.find('\n') {
            None => return s.get(..i).unwrap_or("").to_owned(),
            Some(p) if i + p + 1 == s.len() => {
                return format!("{}\n", s.get(..i).unwrap_or(""));
            }
            Some(_) => {}
        }
    }
    s.to_owned()
}

/// `re.sub(r"-(?:202[0-9]{5}|5)$", "", s)`.
fn strip_date(s: &str) -> String {
    let (body, newline) = split_final_newline(s);
    let bytes = body.as_bytes();
    let dated = bytes.len() >= 9
        && bytes
            .get(bytes.len() - 9..)
            .is_some_and(|t| t.starts_with(b"-202") && t.iter().skip(4).all(u8::is_ascii_digit));
    let cut = if dated {
        Some(bytes.len() - 9)
    } else if body.ends_with("-5") {
        Some(bytes.len() - 2)
    } else {
        None
    };
    match cut {
        Some(at) => format!("{}{newline}", body.get(..at).unwrap_or("")),
        None => s.to_owned(),
    }
}

/// El modelo corto que muestra la pestaña (`claude-opus-5-5` → `opus-5`):
/// la misma limpieza que `tab_models` aplica antes de la etiqueta.
pub(crate) fn short_model(model: &str) -> String {
    strip_date(&strip_context(model).replace("claude-", ""))
}

/// `str.capitalize()` para ASCII; con no-ASCII (título Unicode) se declina.
fn capitalize(s: &str) -> Result<String, StateFault> {
    if !s.is_ascii() {
        return Err(StateFault::Decline);
    }
    let mut out = s.to_ascii_lowercase();
    if let Some(first) = out.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    Ok(out)
}

fn text(s: &str) -> Value {
    Value::from(s)
}

/// El documento de `H/app-tab-models.json`: por sesión, las dos primeras
/// etiquetas, si hay mezcla y hasta seis panes con su estado de modelo.
pub fn tab_models(
    items: &[Value],
    registry: &Value,
    motor_results: &Map<String, Value>,
    tiers: &Value,
    now: f64,
) -> Result<Value, StateFault> {
    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    let mut detail: HashMap<String, Vec<Value>> = HashMap::new();
    for item in items {
        let Some(item) = item.as_object() else {
            continue;
        };
        if !item.get("alive").is_some_and(truthy) || !item.get("session").is_some_and(truthy) {
            continue;
        }
        // Clave de `dict` y de `json.dumps`: solo texto se reproduce.
        let session = item
            .get("session")
            .and_then(Value::as_str)
            .ok_or(StateFault::Decline)?
            .to_owned();
        let model = str_or_empty(item.get("model"))?;
        let motor = match item.get("motor").filter(|v| truthy(v)) {
            Some(motor) => py_str(motor)?,
            None => {
                let engine = engine_for_model(registry, &model)?;
                if engine.is_empty() {
                    str_or_empty(item.get("agent"))?
                } else {
                    engine
                }
            }
        };
        let mut short = strip_date(&strip_context(&model).replace("claude-", ""));
        let agent = item.get("agent");
        let main_agent = ["claude", "codex", "grok"]
            .iter()
            .any(|a| is_text(agent, a));
        if !motor.is_empty() && short.is_empty() && main_agent {
            short = DETECTING.to_owned();
        }
        let label = if motor.is_empty() {
            short.clone()
        } else if short.is_empty() {
            capitalize(&motor)?
        } else {
            format!("{} · {short}", capitalize(&motor)?)
        };
        if !label.is_empty() {
            let index = match grouped.iter().position(|(s, _)| *s == session) {
                Some(index) => index,
                None => {
                    grouped.push((session.clone(), Vec::new()));
                    grouped.len() - 1
                }
            };
            if let Some((_, labels)) = grouped.get_mut(index)
                && !labels.contains(&label)
            {
                labels.push(label);
            }
        }
        let pane_id = str_or_empty(item.get("pane"))?;
        if !DETAIL_HARNESSES.iter().any(|a| is_text(agent, a)) {
            continue;
        }
        let pending = match motor_results
            .get(&format!("{session}|{pane_id}"))
            .filter(|v| truthy(v))
        {
            None => None,
            Some(Value::Object(pending)) => Some(pending),
            Some(_) => return Err(StateFault::Decline),
        };
        let switching = match pending {
            Some(p) if p.contains_key("stage") && !p.contains_key("ok") => {
                let ts = match p.get("ts").filter(|v| truthy(v)) {
                    None => 0.0,
                    Some(ts) => match py_float(ts) {
                        PyFloat::Value(f) => f,
                        // `float()` imposible: excepción sin capturar (500).
                        PyFloat::Raises => return Err(StateFault::Failure),
                        PyFloat::Unsure => return Err(StateFault::Decline),
                    },
                };
                now - ts < 300.0
            }
            _ => false,
        };
        let target = if switching {
            let requested =
                str_or_empty(pending.and_then(|p| p.get("model")))?.replace("claude-", "");
            let last = requested.rsplit('/').next().unwrap_or("");
            if last.is_empty() {
                "…".to_owned()
            } else {
                last.to_owned()
            }
        } else {
            String::new()
        };
        let verified = !short.is_empty() && short != DETECTING;
        let tier = if verified {
            model_tier(tiers, &short)?
        } else {
            String::new()
        };
        let account = match item.get("harnessAccount").filter(|v| truthy(v)) {
            Some(account) => py_str(account)?,
            None => str_or_empty(item.get("account"))?,
        };
        let state = if switching {
            "changing"
        } else if verified {
            "verified"
        } else {
            "detecting"
        };
        let mut entry = Map::new();
        entry.insert("pane".into(), text(&pane_id));
        entry.insert("harness".into(), text(&str_or_empty(agent)?));
        entry.insert("motor".into(), text(&motor));
        entry.insert(
            "model".into(),
            text(if short == DETECTING { "" } else { &short }),
        );
        entry.insert("effort".into(), text(&str_or_empty(item.get("effort"))?));
        entry.insert("hAcct".into(), text(&account));
        entry.insert(
            "mAcct".into(),
            text(&str_or_empty(item.get("motorAccount"))?),
        );
        entry.insert("state".into(), text(state));
        entry.insert("target".into(), text(&target));
        entry.insert(
            "tierSym".into(),
            if tier.is_empty() {
                text("")
            } else {
                tier_symbol(tiers, &tier)?
            },
        );
        detail
            .entry(session)
            .or_default()
            .push(Value::Object(entry));
    }
    let mut public = Map::new();
    for (session, labels) in grouped {
        let panes: Vec<Value> = detail
            .remove(&session)
            .unwrap_or_default()
            .into_iter()
            .take(6)
            .collect();
        let shown: Vec<&str> = labels.iter().take(2).map(String::as_str).collect();
        let mut doc = Map::new();
        doc.insert("label".into(), text(&shown.join(" / ")));
        doc.insert("mixed".into(), Value::Bool(labels.len() > 1));
        doc.insert("panes".into(), Value::Array(panes));
        public.insert(session, Value::Object(doc));
    }
    Ok(Value::Object(public))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutions_match_python_re_sub() {
        // `re.sub` de CPython 3 con los mismos patrones.
        assert_eq!(strip_context("claude-fable-5[1m]"), "claude-fable-5");
        assert_eq!(strip_context("a[x\nb[y"), "a[x\nb");
        assert_eq!(strip_context("a[x\n"), "a\n");
        assert_eq!(strip_context("a[x\nb\n"), "a[x\nb\n");
        assert_eq!(strip_context("plain"), "plain");
        assert_eq!(strip_date("opus-5-20251101"), "opus-5");
        assert_eq!(strip_date("fable-5"), "fable");
        assert_eq!(strip_date("fable-5\n"), "fable\n");
        assert_eq!(strip_date("x-2019123"), "x-2019123");
        assert_eq!(strip_date("x-20251101-5"), "x-20251101");
        assert_eq!(strip_date("-5"), "");
        assert_eq!(capitalize("codex").unwrap(), "Codex");
        assert_eq!(capitalize("gROK").unwrap(), "Grok");
        assert_eq!(capitalize("").unwrap(), "");
        assert!(capitalize("ñu").is_err());
    }
}
