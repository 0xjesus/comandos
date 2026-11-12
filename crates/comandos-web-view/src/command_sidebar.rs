//! Left sidebar shell: «Sesiones» feed, «Consumos» quota cards and the Servers sheet.
use crate::escape::text as esc;
use serde_json::{Value, json};
pub fn at<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&Value::Null)
}
pub fn txt(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    }
}
pub fn yes(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}
pub fn pick(v: &Value, keys: &[&str], default: &str) -> String {
    keys.iter()
        .map(|k| at(v, k))
        .find(|v| yes(v))
        .map(txt)
        .unwrap_or_else(|| default.into())
}
pub fn arr(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

/// Toolbar: views (Sesiones, Consumos) and the Servers sheet button.
pub fn tools_html() -> String {
    r#"<div class="cs-tools" role="toolbar" aria-label="Barra lateral"><button type="button" data-flat class="tb" data-sidebar-view="sessions" role="tab" aria-selected="true"><span class="lb">Sesiones</span><b class="fs-n" hidden></b></button><button type="button" data-flat class="tb" data-sidebar-view="usage" role="tab" aria-selected="false"><span data-icon="chart" data-size="15"></span><span class="lb">Consumos</span></button><button type="button" data-flat class="tb cs-server-button" data-sheet="srv" aria-label="Servidores" title="Servidores"><span data-icon="server" data-size="15"></span></button></div>"#.into()
}

/// Sheet header: title and close button.
pub fn head_html() -> String {
    "<div class=\"cs-head\"><div class=\"sh-h\"><span class=\"sh-t\"></span><button type=\"button\" data-flat class=\"cls\" data-sheet-close title=\"Cerrar (Esc)\">Cerrar <kbd>Esc</kbd></button></div></div>".into()
}

/// Whole sidebar: toolbar, Servers sheet and the view area (Sessions feed or
/// account usage), which takes all the remaining height.
pub fn shell_html() -> String {
    format!(
        "{}<div class=\"sec-cmds cs-sheet\" role=\"dialog\" aria-label=\"Servidores\" hidden>{}<div class=\"cs-srv\"><div class=\"cs-srv-slot\"></div></div></div><div class=\"cs-empty-terms\"><div class=\"cs-feed\" role=\"region\" aria-label=\"Sesiones\"></div><div class=\"et-lim\" hidden><div class=\"cs-credits\"></div><div class=\"et-h\"><b>Uso de tus cuentas</b><small>% usado</small></div><div class=\"cs-limits\"></div></div></div>",
        tools_html(),
        head_html()
    )
}

/// Views that exist; anything else falls back to the Sessions feed.
pub fn view_name(name: &str) -> &'static str {
    match name {
        "usage" => "usage",
        _ => "sessions",
    }
}

/// Only the Servers sheet remains.
pub fn sheet_name(name: &str) -> &'static str {
    match name {
        "srv" => "srv",
        _ => "",
    }
}

pub fn acc_limits(a: &Value) -> Vec<Value> {
    let mut ls = Vec::new();
    if !at(a, "week").is_null() {
        ls.push(json!({"n":"Semana","u":at(a,"week"),"left":at(a,"left"),"reset":at(a,"reset")}));
    }
    let model = at(a, "model");
    if yes(model) {
        ls.push(json!({"n":at(model,"n"),"u":at(model,"v"),"left":model.get("left").filter(|v|!v.is_null()).unwrap_or(at(a,"left")),"reset":model.get("reset").filter(|v|!v.is_null()).unwrap_or(at(a,"reset"))}));
    }
    if !at(a, "h5").is_null() {
        ls.push(
            json!({"n":"5 horas","u":at(a,"h5"),"left":at(a,"h5Left"),"reset":at(a,"h5Reset")}),
        );
    }
    ls.into_iter()
        .filter_map(|mut l| {
            let n = at(&l, "u").as_f64().filter(|n| n.is_finite())?;
            if let Some(o) = l.as_object_mut() {
                o.insert("u".into(), json!((n + 0.5).floor().clamp(0., 100.)));
            }
            Some(l)
        })
        .collect()
}
pub fn can_switch(a: &Value, cur: &str) -> bool {
    !cur.is_empty()
        && at(a, "id") != cur
        && cur.split(':').next() == at(a, "provider").as_str()
        && ["claude", "codex", "grok"].contains(&txt(at(a, "provider")).as_str())
        && !acc_limits(a).is_empty()
        && acc_limits(a)
            .iter()
            .all(|l| at(l, "u").as_f64().is_some_and(|n| n < 100.))
}
fn measured(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite() && *n >= 0.)
}
fn num(n: f64) -> String {
    if n == 0. { "0".into() } else { n.to_string() }
}
pub fn token_text(v: &Value) -> String {
    let Some(n) = measured(v) else {
        return "—".into();
    };
    for (scale, unit) in [(1e9, "B"), (1e6, "M"), (1000., "K")] {
        if n >= scale {
            return format!("{}{unit}", fixed(n / scale, 1));
        }
    }
    num(n)
}
fn hours(d: &Value) -> String {
    at(d, "h")
        .as_f64()
        .filter(|n| n.is_finite())
        .map(|n| {
            format!(
                "{} h · {} ses",
                fixed(n, 1),
                d.get("ses")
                    .filter(|v| !v.is_null())
                    .map(txt)
                    .unwrap_or_else(|| "0".into())
            )
        })
        .unwrap_or_else(|| "—".into())
}
fn tone(u: f64) -> &'static str {
    if u >= 90. {
        "bad"
    } else if u >= 70. {
        "warn"
    } else {
        ""
    }
}
fn color(a: &Value) -> String {
    let value = pick(a, &["color"], "");
    if value.starts_with('#')
        && [4, 5, 7, 9].contains(&value.len())
        && value
            .get(1..)
            .is_some_and(|s| s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        value
    } else {
        "var(--brand)".into()
    }
}
fn limit_detail(a: &Value, limits: &[Value], cur: &str, pending: bool) -> String {
    let pair = |k: &str, v: &str| format!("<span>{}</span><b>{}</b>", esc(k), esc(v));
    let mut html = String::new();
    for l in limits {
        let reset = if yes(at(l, "reset")) {
            format!("reinicia {}", txt(at(l, "reset")))
        } else if yes(at(l, "left")) {
            format!("se reinicia en {}", txt(at(l, "left")))
        } else {
            "sin reinicio reportado".into()
        };
        html.push_str(&pair(
            &txt(at(l, "n")),
            &format!("{}% · {reset}", num(at(l, "u").as_f64().unwrap_or(0.))),
        ));
    }
    if limits.is_empty() {
        let m = at(a, "measured");
        html.push_str(&pair("Modelo", &{
            let models = arr(at(m, "models"))
                .iter()
                .map(txt)
                .collect::<Vec<_>>()
                .join(" · ");
            if models.is_empty() {
                "—".into()
            } else {
                models
            }
        }));
        html.push_str(&pair(
            "Costo semana",
            &measured(at(m, "costUsd"))
                .map(|n| format!("${}", fixed(n, 2)))
                .unwrap_or_else(|| "—".into()),
        ));
    } else {
        html.push_str(&pair("Hoy", &hours(at(a, "hoy"))));
        html.push_str(&pair("Semana", &hours(at(a, "sem"))));
    }
    if yes(at(a, "plan")) {
        html.push_str(&pair("Plan", &txt(at(a, "plan"))));
    }
    let switch = if can_switch(a, cur) {
        format!(
            "<button type=\"button\" data-flat data-limit-action=\"switch\" class=\"pri\"{}>{}</button>",
            if pending { " disabled" } else { "" },
            esc(&format!("Seguir este pane en {} →", txt(at(a, "alias"))))
        )
    } else {
        String::new()
    };
    format!(
        "<div class=\"det\"><div class=\"kv\">{html}</div><div class=\"acts\">{switch}</div></div>"
    )
}
pub fn limits_html(accounts: &Value, cur: &str, opts: &Value) -> String {
    let accounts = arr(accounts)
        .into_iter()
        .filter(|a| at(a, "id").is_string())
        .collect::<Vec<_>>();
    if accounts.is_empty() {
        return "<div class=\"wait\">Sin límites que mostrar</div>".into();
    }
    let mut html = String::new();
    for a in accounts {
        let limits = acc_limits(&a);
        let peak = limits.iter().max_by(|a, b| {
            at(a, "u")
                .as_f64()
                .unwrap_or(0.)
                .total_cmp(&at(b, "u").as_f64().unwrap_or(0.))
        });
        let color = color(&a);
        let current = !cur.is_empty() && at(&a, "id") == cur;
        let open = at(opts, "openId") == at(&a, "id");
        let logo = logo(&txt(at(&a, "provider")));
        let logo = if logo.is_empty() {
            esc(&pick(&a, &["cli"], "?")
                .chars()
                .next()
                .unwrap_or('?')
                .to_string())
        } else {
            logo.into()
        };
        let name = format!(
            "<span class=\"logo\">{logo}</span><span class=\"nm\"><b>{}</b><small>{}</small></span>{}",
            esc(&txt(at(&a, "cli"))),
            esc(&txt(at(&a, "alias"))),
            if current {
                "<span class=\"pane\">este pane</span>"
            } else {
                ""
            }
        );
        let content = if let Some(peak) = peak {
            let u = at(peak, "u").as_f64().unwrap_or(0.);
            let mut meters = String::new();
            for l in &limits {
                let u = at(l, "u").as_f64().unwrap_or(0.);
                let c = if u >= 90. {
                    "var(--bad)"
                } else if u >= 70. {
                    "var(--warn)"
                } else {
                    &color
                };
                let segments = (0..20)
                    .map(|i| {
                        format!(
                            "<i class=\"{}\"></i>",
                            if (i as f64) < (u / 5. + 0.5).floor() {
                                "on"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect::<String>();
                meters.push_str(&format!("<div class=\"meter\" style=\"--lc:{c}\"><span class=\"meter-name\">{}</span><span class=\"seg\" aria-hidden=\"true\">{segments}</span><b class=\"meter-value {}\">{}%</b><span class=\"r\">{}</span></div>",esc(&txt(at(l,"n"))),tone(u),num(u),if yes(at(l,"left")){format!("se reinicia en {}",esc(&txt(at(l,"left"))))}else{"sin reinicio reportado".into()}));
            }
            format!(
                "<div class=\"row1\">{name}<b class=\"big {}\">{}%</b></div>{meters}",
                tone(u),
                num(u)
            )
        } else {
            let m = at(&a, "measured");
            format!(
                "<div class=\"free-head\">{name}<span class=\"tag\">sin cuota</span></div><div class=\"fr\"><span><b>{}</b>sesiones · semana</span><span><b>{}</b>tokens · semana</span><span><b>{}</b>costo · semana</span></div>",
                measured(at(m, "sessions"))
                    .map(num)
                    .unwrap_or_else(|| "—".into()),
                token_text(at(m, "tokens")),
                measured(at(m, "costUsd"))
                    .map(|n| format!("${}", fixed(n, 2)))
                    .unwrap_or_else(|| "—".into())
            )
        };
        let used = peak
            .map(|p| format!("{}% usado", num(at(p, "u").as_f64().unwrap_or(0.))))
            .unwrap_or_else(|| "consumo medido".into());
        html.push_str(&format!("<div class=\"card{}{}{}\" data-account=\"{}\" style=\"--ac:{color}\"><button type=\"button\" data-flat class=\"usage-toggle\" aria-expanded=\"{open}\" aria-label=\"{}: {used}\">{content}</button>{}</div>",if peak.is_none(){" free"}else{""},if current{" cur"}else{""},if open{" open"}else{""},esc(&txt(at(&a,"id"))),esc(&format!("{} {}",txt(at(&a,"cli")),txt(at(&a,"alias")))),limit_detail(&a,&limits,cur,yes(at(opts,"pending")))));
    }
    format!("<div class=\"cs-lim\">{html}</div>")
}

pub fn logo(id: &str) -> &'static str {
    match id {
        "claude" => {
            r####"<svg class="logo" aria-hidden="true" focusable="false" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path d="M4.709 15.955l4.72-2.647.08-.23-.08-.128H9.2l-.79-.048-2.698-.073-2.339-.097-2.266-.122-.571-.121L0 11.784l.055-.352.48-.321.686.06 1.52.103 2.278.158 1.652.097 2.449.255h.389l.055-.157-.134-.098-.103-.097-2.358-1.596-2.552-1.688-1.336-.972-.724-.491-.364-.462-.158-1.008.656-.722.881.06.225.061.893.686 1.908 1.476 2.491 1.833.365.304.145-.103.019-.073-.164-.274-1.355-2.446-1.446-2.49-.644-1.032-.17-.619a2.97 2.97 0 01-.104-.729L6.283.134 6.696 0l.996.134.42.364.62 1.414 1.002 2.229 1.555 3.03.456.898.243.832.091.255h.158V9.01l.128-1.706.237-2.095.23-2.695.08-.76.376-.91.747-.492.584.28.48.685-.067.444-.286 1.851-.559 2.903-.364 1.942h.212l.243-.242.985-1.306 1.652-2.064.73-.82.85-.904.547-.431h1.033l.76 1.129-.34 1.166-1.064 1.347-.881 1.142-1.264 1.7-.79 1.36.073.11.188-.02 2.856-.606 1.543-.28 1.841-.315.833.388.091.395-.328.807-1.969.486-2.309.462-3.439.813-.042.03.049.061 1.549.146.662.036h1.622l3.02.225.79.522.474.638-.079.485-1.215.62-1.64-.389-3.829-.91-1.312-.329h-.182v.11l1.093 1.068 2.006 1.81 2.509 2.33.127.578-.322.455-.34-.049-2.205-1.657-.851-.747-1.926-1.62h-.128v.17l.444.649 2.345 3.521.122 1.08-.17.353-.608.213-.668-.122-1.374-1.925-1.415-2.167-1.143-1.943-.14.08-.674 7.254-.316.37-.729.28-.607-.461-.322-.747.322-1.476.389-1.924.315-1.53.286-1.9.17-.632-.012-.042-.14.018-1.434 1.967-2.18 2.945-1.726 1.845-.414.164-.717-.37.067-.662.401-.589 2.388-3.036 1.44-1.882.93-1.086-.006-.158h-.055L4.132 18.56l-1.13.146-.487-.456.061-.746.231-.243 1.908-1.312-.006.006z" fill="#D97757" fill-rule="nonzero"></path></svg>"####
        }
        "codex" => {
            r####"<svg class="logo" aria-hidden="true" focusable="false" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path d="M19.503 0H4.496A4.496 4.496 0 000 4.496v15.007A4.496 4.496 0 004.496 24h15.007A4.496 4.496 0 0024 19.503V4.496A4.496 4.496 0 0019.503 0z" fill="#fff"></path><path d="M9.064 3.344a4.578 4.578 0 012.285-.312c1 .115 1.891.54 2.673 1.275.01.01.024.017.037.021a.09.09 0 00.043 0 4.55 4.55 0 013.046.275l.047.022.116.057a4.581 4.581 0 012.188 2.399c.209.51.313 1.041.315 1.595a4.24 4.24 0 01-.134 1.223.123.123 0 00.03.115c.594.607.988 1.33 1.183 2.17.289 1.425-.007 2.71-.887 3.854l-.136.166a4.548 4.548 0 01-2.201 1.388.123.123 0 00-.081.076c-.191.551-.383 1.023-.74 1.494-.9 1.187-2.222 1.846-3.711 1.838-1.187-.006-2.239-.44-3.157-1.302a.107.107 0 00-.105-.024c-.388.125-.78.143-1.204.138a4.441 4.441 0 01-1.945-.466 4.544 4.544 0 01-1.61-1.335c-.152-.202-.303-.392-.414-.617a5.81 5.81 0 01-.37-.961 4.582 4.582 0 01-.014-2.298.124.124 0 00.006-.056.085.085 0 00-.027-.048 4.467 4.467 0 01-1.034-1.651 3.896 3.896 0 01-.251-1.192 5.189 5.189 0 01.141-1.6c.337-1.112.982-1.985 1.933-2.618.212-.141.413-.251.601-.33.215-.089.43-.164.646-.227a.098.098 0 00.065-.066 4.51 4.51 0 01.829-1.615 4.535 4.535 0 011.837-1.388zm3.482 10.565a.637.637 0 000 1.272h3.636a.637.637 0 100-1.272h-3.636zM8.462 9.23a.637.637 0 00-1.106.631l1.272 2.224-1.266 2.136a.636.636 0 101.095.649l1.454-2.455a.636.636 0 00.005-.64L8.462 9.23z" fill="url(#cs-logo-codex-_R_0_)"></path><defs><linearGradient gradientUnits="userSpaceOnUse" id="cs-logo-codex-_R_0_" x1="12" x2="12" y1="3" y2="21"><stop stop-color="#B1A7FF"></stop><stop offset=".5" stop-color="#7A9DFF"></stop><stop offset="1" stop-color="#3941FF"></stop></linearGradient></defs></svg>"####
        }
        "grok" => {
            r####"<svg class="logo" aria-hidden="true" focusable="false" fill="currentColor" fill-rule="evenodd" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path d="M9.27 15.29l7.978-5.897c.391-.29.95-.177 1.137.272.98 2.369.542 5.215-1.41 7.169-1.951 1.954-4.667 2.382-7.149 1.406l-2.711 1.257c3.889 2.661 8.611 2.003 11.562-.953 2.341-2.344 3.066-5.539 2.388-8.42l.006.007c-.983-4.232.242-5.924 2.75-9.383.06-.082.12-.164.179-.248l-3.301 3.305v-.01L9.267 15.292M7.623 16.723c-2.792-2.67-2.31-6.801.071-9.184 1.761-1.763 4.647-2.483 7.166-1.425l2.705-1.25a7.808 7.808 0 00-1.829-1A8.975 8.975 0 005.984 5.83c-2.533 2.536-3.33 6.436-1.962 9.764 1.022 2.487-.653 4.246-2.34 6.022-.599.63-1.199 1.259-1.682 1.925l7.62-6.815"></path></svg>"####
        }
        "opencode" => {
            r####"<svg class="logo" aria-hidden="true" focusable="false" viewBox="0 0 512 512" fill="none" xmlns="http://www.w3.org/2000/svg"><rect width="512" height="512" rx="96" fill="#131010"/><path d="M320 224V352H192V224H320Z" fill="#5A5858"/><path fill-rule="evenodd" clip-rule="evenodd" d="M384 416H128V96H384V416ZM320 160H192V352H320V160Z" fill="white"/></svg>"####
        }
        "agy" => {
            r####"<svg class="logo" aria-hidden="true" focusable="false" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><mask height="23" id="cs-logo-antigravity-0-_R_0_" maskUnits="userSpaceOnUse" width="24" x="0" y="1"><path d="M21.751 22.607c1.34 1.005 3.35.335 1.508-1.508C17.73 15.74 18.904 1 12.037 1 5.17 1 6.342 15.74.815 21.1c-2.01 2.009.167 2.511 1.507 1.506 5.192-3.517 4.857-9.714 9.715-9.714 4.857 0 4.522 6.197 9.714 9.715z" fill="#fff"></path></mask><g mask="url(#cs-logo-antigravity-0-_R_0_)"><g filter="url(#cs-logo-antigravity-1-_R_0_)"><path d="M-1.018-3.992c-.408 3.591 2.686 6.89 6.91 7.37 4.225.48 7.98-2.043 8.387-5.633.408-3.59-2.686-6.89-6.91-7.37-4.225-.479-7.98 2.043-8.387 5.633z" fill="#FFE432"></path></g><g filter="url(#cs-logo-antigravity-2-_R_0_)"><path d="M15.269 7.747c1.058 4.557 5.691 7.374 10.348 6.293 4.657-1.082 7.575-5.653 6.516-10.21-1.058-4.556-5.691-7.374-10.348-6.292-4.657 1.082-7.575 5.653-6.516 10.21z" fill="#FC413D"></path></g><g filter="url(#cs-logo-antigravity-3-_R_0_)"><path d="M-12.443 10.804c1.338 4.703 7.36 7.11 13.453 5.378 6.092-1.733 9.947-6.95 8.61-11.652C8.282-.173 2.26-2.58-3.833-.848-9.925.884-13.78 6.1-12.443 10.804z" fill="#00B95C"></path></g><g filter="url(#cs-logo-antigravity-4-_R_0_)"><path d="M-12.443 10.804c1.338 4.703 7.36 7.11 13.453 5.378 6.092-1.733 9.947-6.95 8.61-11.652C8.282-.173 2.26-2.58-3.833-.848-9.925.884-13.78 6.1-12.443 10.804z" fill="#00B95C"></path></g><g filter="url(#cs-logo-antigravity-5-_R_0_)"><path d="M-7.608 14.703c3.352 3.424 9.126 3.208 12.896-.483 3.77-3.69 4.108-9.459.756-12.883C2.69-2.087-3.083-1.871-6.853 1.82c-3.77 3.69-4.108 9.458-.755 12.883z" fill="#00B95C"></path></g><g filter="url(#cs-logo-antigravity-6-_R_0_)"><path d="M9.932 27.617c1.04 4.482 5.384 7.303 9.7 6.3 4.316-1.002 6.971-5.448 5.93-9.93-1.04-4.483-5.384-7.304-9.7-6.301-4.316 1.002-6.971 5.448-5.93 9.93z" fill="#3186FF"></path></g><g filter="url(#cs-logo-antigravity-7-_R_0_)"><path d="M2.572-8.185C.392-3.329 2.778 2.472 7.9 4.771c5.122 2.3 11.042.227 13.222-4.63 2.18-4.855-.205-10.656-5.327-12.955-5.122-2.3-11.042-.227-13.222 4.63z" fill="#FBBC04"></path></g><g filter="url(#cs-logo-antigravity-8-_R_0_)"><path d="M-3.267 38.686c-5.277-2.072 3.742-19.117 5.984-24.83 2.243-5.712 8.34-8.664 13.616-6.592 5.278 2.071 11.533 13.482 9.29 19.195-2.242 5.713-23.613 14.298-28.89 12.227z" fill="#3186FF"></path></g><g filter="url(#cs-logo-antigravity-9-_R_0_)"><path d="M28.71 17.471c-1.413 1.649-5.1.808-8.236-1.878-3.135-2.687-4.531-6.201-3.118-7.85 1.412-1.649 5.1-.808 8.235 1.878s4.532 6.2 3.119 7.85z" fill="#749BFF"></path></g><g filter="url(#cs-logo-antigravity-10-_R_0_)"><path d="M18.163 9.077c5.81 3.93 12.502 4.19 14.946.577 2.443-3.612-.287-9.727-6.098-13.658-5.81-3.931-12.502-4.19-14.946-.577-2.443 3.612.287 9.727 6.098 13.658z" fill="#FC413D"></path></g><g filter="url(#cs-logo-antigravity-11-_R_0_)"><path d="M-.915 2.684c-1.44 3.473-.97 6.967 1.05 7.804 2.02.837 4.824-1.3 6.264-4.772 1.44-3.473.97-6.967-1.05-7.804-2.02-.837-4.824 1.3-6.264 4.772z" fill="#FFEE48"></path></g></g><defs><filter color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="17.587" id="cs-logo-antigravity-1-_R_0_" width="19.838" x="-3.288" y="-11.917"><feFlood flood-opacity="0" result="BackgroundImageFix"></feFlood><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"></feBlend><feGaussianBlur result="effect1_foregroundBlur_977_115" stdDeviation="1.117"></feGaussianBlur></filter><filter color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="38.565" id="cs-logo-antigravity-2-_R_0_" width="38.9" x="4.251" y="-13.493"><feFlood flood-opacity="0" result="BackgroundImageFix"></feFlood><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"></feBlend><feGaussianBlur result="effect1_foregroundBlur_977_115" stdDeviation="5.4"></feGaussianBlur></filter><filter color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="36.517" id="cs-logo-antigravity-3-_R_0_" width="40.955" x="-21.889" y="-10.592"><feFlood flood-opacity="0" result="BackgroundImageFix"></feFlood><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"></feBlend><feGaussianBlur result="effect1_foregroundBlur_977_115" stdDeviation="4.591"></feGaussianBlur></filter><filter color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="36.517" id="cs-logo-antigravity-4-_R_0_" width="40.955" x="-21.889" y="-10.592"><feFlood flood-opacity="0" result="BackgroundImageFix"></feFlood><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"></feBlend><feGaussianBlur result="effect1_foregroundBlur_977_115" stdDeviation="4.591"></feGaussianBlur></filter><filter color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="36.595" id="cs-logo-antigravity-5-_R_0_" width="36.632" x="-19.099" y="-10.278"><feFlood flood-opacity="0" result="BackgroundImageFix"></feFlood><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"></feBlend><feGaussianBlur result="effect1_foregroundBlur_977_115" stdDeviation="4.591"></feGaussianBlur></filter><filter color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="34.087" id="cs-logo-antigravity-6-_R_0_" width="33.533" x=".981" y="8.758"><feFlood flood-opacity="0" result="BackgroundImageFix"></feFlood><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"></feBlend><feGaussianBlur result="effect1_foregroundBlur_977_115" stdDeviation="4.363"></feGaussianBlur></filter><filter color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="35.276" id="cs-logo-antigravity-7-_R_0_" width="35.978" x="-6.143" y="-21.659"><feFlood flood-opacity="0" result="BackgroundImageFix"></feFlood><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"></feBlend><feGaussianBlur result="effect1_foregroundBlur_977_115" stdDeviation="3.954"></feGaussianBlur></filter><filter color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="46.523" id="cs-logo-antigravity-8-_R_0_" width="45.114" x="-11.96" y="-.46"><feFlood flood-opacity="0" result="BackgroundImageFix"></feFlood><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"></feBlend><feGaussianBlur result="effect1_foregroundBlur_977_115" stdDeviation="3.531"></feGaussianBlur></filter><filter color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="24.054" id="cs-logo-antigravity-9-_R_0_" width="25.094" x="10.485" y=".58"><feFlood flood-opacity="0" result="BackgroundImageFix"></feFlood><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"></feBlend><feGaussianBlur result="effect1_foregroundBlur_977_115" stdDeviation="3.159"></feGaussianBlur></filter><filter color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="30.007" id="cs-logo-antigravity-10-_R_0_" width="33.508" x="5.833" y="-12.467"><feFlood flood-opacity="0" result="BackgroundImageFix"></feFlood><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"></feBlend><feGaussianBlur result="effect1_foregroundBlur_977_115" stdDeviation="2.669"></feGaussianBlur></filter><filter color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="26.151" id="cs-logo-antigravity-11-_R_0_" width="22.194" x="-8.355" y="-8.876"><feFlood flood-opacity="0" result="BackgroundImageFix"></feFlood><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"></feBlend><feGaussianBlur result="effect1_foregroundBlur_977_115" stdDeviation="3.303"></feGaussianBlur></filter></defs></svg>"####
        }
        _ => "",
    }
}

fn fixed(n: f64, places: usize) -> String {
    // Number.toFixed rounds the exact binary value, with ties away from zero.
    // Multiplying in f64 first loses that distinction (1.15 becomes 1.2).
    // The original renderer uses only zero and one fractional digit.
    if !n.is_finite() || n.abs() >= 1e21 {
        return num(n);
    }
    let bits = n.abs().to_bits();
    let raw_exponent = ((bits >> 52) & 0x7ff) as i32;
    let mantissa =
        u128::from(bits & ((1_u64 << 52) - 1)) | if raw_exponent == 0 { 0 } else { 1_u128 << 52 };
    let exponent = if raw_exponent == 0 {
        -1074
    } else {
        raw_exponent - 1075
    };
    let scale = 10_u128.pow(places as u32);
    let numerator = mantissa * scale;
    let rounded = if exponent >= 0 {
        numerator.checked_shl(exponent as u32).unwrap_or(0)
    } else {
        let shift = (-exponent) as u32;
        if shift >= 128 {
            0
        } else {
            let denominator = 1_u128 << shift;
            (numerator >> shift) + u128::from(numerator & (denominator - 1) >= denominator / 2)
        }
    };
    let sign = if n < 0. { "-" } else { "" };
    if places == 0 {
        format!("{sign}{rounded}")
    } else {
        let whole = rounded / scale;
        let fraction = rounded % scale;
        format!("{sign}{whole}.{fraction:0places$}")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn shell_keeps_sessions_usage_and_servers_only() {
        let html = shell_html();
        for kept in [
            "data-sidebar-view=\"sessions\"",
            "data-sidebar-view=\"usage\"",
            "Consumos",
            "data-sheet=\"srv\"",
            "cs-srv-slot",
            "cs-feed",
            "cs-limits",
            "cs-credits",
            "Uso de tus cuentas",
        ] {
            assert!(html.contains(kept), "{kept}");
        }
        for gone in [
            "data-sidebar-view=\"files\"",
            "Archivos",
            "cs-explorer",
            "cs-more",
            "data-sheet=\"cmds\"",
            "data-sheet=\"chains\"",
            "cs-footer",
            "sec-terms",
            "cs-terms",
            "class=\"mini\"",
            "et-foot",
            "cs-search",
        ] {
            assert!(!html.contains(gone), "{gone}");
        }
    }
    #[test]
    fn removed_views_and_sheets_fall_back() {
        assert_eq!(view_name("usage"), "usage");
        assert_eq!(view_name("files"), "sessions");
        assert_eq!(view_name(""), "sessions");
        assert_eq!(sheet_name("srv"), "srv");
        assert_eq!(sheet_name("cmds"), "");
        assert_eq!(sheet_name("chains"), "");
    }
    #[test]
    fn quota_cards_never_link_to_the_retired_analytics_modal() {
        let html = limits_html(
            &json!([{"id":"claude:main","provider":"claude","cli":"Claude","alias":"main","week":20}]),
            "claude:other",
            &json!({"openId":"claude:main"}),
        );
        assert!(html.contains("data-limit-action=\"switch\""));
        assert!(!html.contains("analytics"));
        assert!(!html.contains("Analytics"));
    }
    #[test]
    fn led_measurements_do_not_coerce_strings_or_invent_quotas() {
        assert!(acc_limits(&json!({"week":"90","h5":null})).is_empty());
        assert_eq!(
            acc_limits(&json!({"week":999}))
                .first()
                .unwrap()
                .get("u")
                .unwrap()
                .as_f64(),
            Some(100.)
        );
        let html = limits_html(
            &json!([{"id":"custom","provider":"custom","cli":"X","alias":"main","measured":{"tokens":-1,"costUsd":-1}}]),
            "",
            &json!({}),
        );
        assert!(!html.contains("NaN"));
        assert!(!html.contains("class=\"seg\""));
        assert!(html.contains("sin cuota"));
        assert_eq!(token_text(&json!(21600000)), "21.6M");
        assert_eq!(token_text(&json!(1000000000)), "1.0B");
        assert_eq!(token_text(&json!(1000)), "1.0K");
        assert_eq!(token_text(&json!(0)), "0");
    }
    #[test]
    fn switch_only_between_same_supported_provider_below_every_limit() {
        let account = json!({"id":"claude:main","provider":"claude","week":20,"model":{"v":99}});
        assert!(can_switch(&account, "claude:other"));
        assert!(!can_switch(&account, "claude:main"));
        assert!(!can_switch(&account, "codex:other"));
        assert!(!can_switch(
            &json!({"id":"claude:main","provider":"claude","week":20,"model":{"v":100}}),
            "claude:other"
        ));
        assert!(!can_switch(
            &json!({"id":"opencode:main","provider":"opencode","week":20}),
            "opencode:other"
        ));
        assert!(!can_switch(
            &json!({"id":"claude:main","provider":"claude","measured":{"tokens":200}}),
            "claude:other"
        ));
    }
}
