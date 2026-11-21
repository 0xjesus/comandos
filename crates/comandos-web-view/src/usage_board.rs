//! Ajustes › Consumos: el tablero que dibuja la respuesta de `/usage/board`.
//!
//! A la izquierda el Explorador (proyecto → sesiones); a la derecha el riel de
//! cuentas, cada una con su semana como curva y la proyección punteada hasta
//! el reinicio, con días y horas. Nunca se muestra dinero.
use crate::escape::{attr_esc, md_esc};
use serde_json::Value;
use std::collections::BTreeSet;

/// Lo que la persona abrió en la tabla.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct View {
    pub open_projects: BTreeSet<String>,
    pub open_session: Option<String>,
    /// Ver todos los proyectos, no sólo los que más pesan.
    pub all: bool,
}

/// Proyectos a la vista antes de «Ver todos».
pub const TOP: usize = 25;

fn tr<'a>(en: bool, es: &'a str, en_text: &'a str) -> &'a str {
    if en { en_text } else { es }
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn f(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn lab(v: &Value, en: bool) -> String {
    md_esc(s(v, if en { "en" } else { "es" }))
}

/// 6 M, 740 k, 312.
pub fn tokens(n: f64) -> String {
    if n >= 1e9 {
        format!("{:.1} G", n / 1e9)
    } else if n >= 1e7 {
        format!("{:.0} M", n / 1e6)
    } else if n >= 1e6 {
        format!("{:.1} M", n / 1e6)
    } else if n >= 1e4 {
        format!("{:.0} k", n / 1e3)
    } else if n >= 1e3 {
        format!("{:.1} k", n / 1e3)
    } else {
        format!("{n:.0}")
    }
}

fn hours(h: f64, en: bool) -> String {
    if h <= 0.0 {
        "—".into()
    } else if h < 1.0 {
        format!("{:.0} min", h * 60.0)
    } else {
        format!("{h:.1} {}", tr(en, "h", "h"))
    }
}

fn color_of(board: &Value, acc: &str) -> String {
    board
        .get("accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|a| s(a, "id") == acc)
        .map(|a| s(a, "color"))
        .filter(|c| c.starts_with('#') && c.len() == 7)
        .unwrap_or("#8A93A6")
        .to_string()
}

fn name_of(board: &Value, acc: &str) -> String {
    board
        .get("accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|a| s(a, "id") == acc)
        .map(|a| s(a, "alias").to_string())
        .unwrap_or_else(|| acc.split(':').nth(1).unwrap_or(acc).to_string())
}

/// Barras de la parte de la semana de cada cuenta.
fn shares(board: &Value, share: Option<&Value>, en: bool) -> String {
    let Some(m) = share.and_then(Value::as_object).filter(|m| !m.is_empty()) else {
        return "<span class=\"ub-none\">—</span>".into();
    };
    let mut items: Vec<(&String, f64)> = m
        .iter()
        .map(|(k, v)| (k, v.as_f64().unwrap_or(0.0)))
        .filter(|(_, pct)| *pct >= 0.5)
        .collect();
    if items.is_empty() {
        return "<span class=\"ub-none\">&lt; 1 %</span>".into();
    }
    items.sort_by(|a, b| b.1.total_cmp(&a.1));
    items
        .iter()
        .map(|(acc, pct)| {
            let name = name_of(board, acc);
            format!(
                "<span class=\"ub-share\" title=\"{t}\"><i style=\"background:{c}\"></i><b>{pct:.0} %</b></span>",
                t = attr_esc(&format!(
                    "{name}: ≈ {pct:.1} % {}",
                    tr(en, "de su semana", "of its week")
                )),
                c = color_of(board, acc),
            )
        })
        .collect()
}

/// Tendencia de 14 días: barras finas con el último día marcado.
fn spark(values: Option<&Value>, days: &Value, en: bool) -> String {
    let v: Vec<f64> = values
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|x| x.as_f64().unwrap_or(0.0))
        .collect();
    let max = v.iter().copied().fold(0.0, f64::max);
    if max <= 0.0 {
        return "<span class=\"ub-none\">—</span>".into();
    }
    let n = v.len().max(1) as f64;
    let (w, h) = (84.0, 20.0);
    let bw = w / n;
    let bars: String = v
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let bh = if *x > 0.0 { (x / max * h).max(1.5) } else { 0.0 };
            let day = days.get(i).map(|d| lab(d, en)).unwrap_or_default();
            format!(
                "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{bh:.1}\" rx=\"1\" class=\"{}\"><title>{day} · {}</title></rect>",
                i as f64 * bw + 0.5,
                h - bh,
                (bw - 1.0).max(1.0),
                if i + 1 == v.len() { "last" } else { "" },
                tokens(*x),
            )
        })
        .collect();
    format!("<svg class=\"ub-spark\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\" role=\"img\" aria-label=\"{}\">{bars}</svg>", tr(en, "tokens nuevos por día, 14 días", "new tokens per day, 14 days"))
}

fn session_label(row: &Value, board: &Value, en: bool) -> String {
    let acc = s(row, "account");
    let model = s(row, "model")
        .trim_start_matches("claude-")
        .split(['-', '.'])
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "<i class=\"ub-dot\" style=\"background:{}\"></i>{} <small>{}{}</small>",
        color_of(board, acc),
        row.get("lastLabel").map(|l| lab(l, en)).unwrap_or_default(),
        md_esc(&name_of(board, acc)),
        if model.is_empty() { String::new() } else { format!(" · {}", md_esc(&model)) },
    )
}

fn by_hour(row: &Value, en: bool) -> String {
    let items: Vec<(f64, f64)> = row
        .get("byHour")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
        .collect();
    if items.is_empty() {
        return format!(
            "<p class=\"ub-note\">{}</p>",
            tr(en, "Sin turnos medidos en esta sesión.", "No timed turns in this session.")
        );
    }
    let max = items.iter().map(|p| p.1).fold(1.0, f64::max);
    let bars: String = items
        .iter()
        .map(|(_, n)| format!("<i style=\"height:{:.0}%\" title=\"{n:.0}\"></i>", (n / max * 100.0).max(6.0)))
        .collect();
    format!(
        "<div class=\"ub-hours\"><span class=\"ub-note\">{}</span><div class=\"ub-hbars\">{bars}</div></div>",
        tr(en, "Turnos por hora con actividad (hasta 48 h)", "Turns per active hour (up to 48 h)")
    )
}

fn table(board: &Value, view: &View, en: bool) -> String {
    let days = board.get("days").cloned().unwrap_or(Value::Null);
    let head = format!(
        "<tr><th scope=\"col\">{}</th><th scope=\"col\">{}</th><th scope=\"col\" class=\"n\">{}</th><th scope=\"col\" class=\"n\">{}</th><th scope=\"col\" class=\"n\">{}</th><th scope=\"col\" class=\"n\">{}</th><th scope=\"col\">{}</th></tr>",
        tr(en, "Proyecto", "Project"),
        tr(en, "≈ % de la semana", "≈ % of the week"),
        tr(en, "Tokens", "Tokens"),
        tr(en, "Turnos", "Turns"),
        tr(en, "Horas", "Hours"),
        tr(en, "Ses.", "Sess."),
        tr(en, "14 días", "14 days"),
    );
    let mut body = String::new();
    let projects = board.get("projects").and_then(Value::as_array);
    let total = projects.map_or(0, Vec::len);
    let shown = if view.all { total } else { TOP };
    for p in projects.into_iter().flatten().take(shown) {
        let name = s(p, "name");
        let open = view.open_projects.contains(name);
        body.push_str(&format!(
            "<tr class=\"ub-proj{}\" data-ub-proj=\"{}\" tabindex=\"0\" aria-expanded=\"{open}\"><th scope=\"row\"><span class=\"ub-caret\">{}</span>{}</th><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td>{}</td></tr>",
            if open { " open" } else { "" },
            attr_esc(name),
            if open { "▾" } else { "▸" },
            md_esc(name),
            shares(board, p.get("share"), en),
            tokens(f(p, "newTokens")),
            f(p, "turns"),
            hours(f(p, "hours"), en),
            f(p, "sessions"),
            spark(p.get("spark"), &days, en),
        ));
        if !open {
            continue;
        }
        for r in p.get("rows").and_then(Value::as_array).into_iter().flatten() {
            let id = s(r, "id");
            let sel = view.open_session.as_deref() == Some(id);
            body.push_str(&format!(
                "<tr class=\"ub-sess{}\" data-ub-sess=\"{}\" tabindex=\"0\" aria-expanded=\"{sel}\"><th scope=\"row\">{}</th><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\"></td><td>{}</td></tr>",
                if sel { " open" } else { "" },
                attr_esc(id),
                session_label(r, board, en),
                shares(board, r.get("share"), en),
                tokens(f(r, "newTokens")),
                f(r, "turns"),
                hours(f(r, "hours"), en),
                spark(r.get("spark"), &days, en),
            ));
            if sel {
                body.push_str(&format!(
                    "<tr class=\"ub-detail\"><td colspan=\"7\">{}</td></tr>",
                    by_hour(r, en)
                ));
            }
        }
    }
    if total > TOP {
        body.push_str(&format!(
            "<tr class=\"ub-more\" data-ub-all tabindex=\"0\"><td colspan=\"7\">{}</td></tr>",
            if view.all {
                tr(en, "Ver sólo los 25 que más pesan", "Show only the top 25").to_string()
            } else if en {
                format!("Show the other {} projects", total - TOP)
            } else {
                format!("Ver los otros {} proyectos", total - TOP)
            }
        ));
    }
    if body.is_empty() {
        body = format!(
            "<tr><td colspan=\"7\" class=\"ub-empty\">{}</td></tr>",
            tr(
                en,
                "Todavía no hay turnos en los últimos 14 días. Aparecen aquí en cuanto una sesión responde.",
                "No turns in the last 14 days yet. They show up here as soon as a session answers."
            )
        );
    }
    format!(
        "<div class=\"ub-tablewrap\"><table class=\"ub-table\"><thead>{head}</thead><tbody>{body}</tbody></table></div><p class=\"ub-note\">{}</p>",
        tr(
            en,
            "≈ % de la semana: la parte del cupo semanal de cada cuenta que se llevó el proyecto, estimada con sus tokens. Tokens nuevos no cuentan lo que se leyó de la caché.",
            "≈ % of the week: the part of each account's weekly limit the project took, estimated from its tokens. New tokens leave out cache reads."
        )
    )
}

/// La semana de una cuenta: curva, proyección punteada, días y el 100 %.
pub fn curve_svg(account: &Value, en: bool) -> String {
    let Some(w) = account.get("week") else {
        return String::new();
    };
    let (start, reset) = (f(w, "start"), f(w, "reset"));
    if reset <= start {
        return String::new();
    }
    let color = {
        let c = s(account, "color");
        if c.starts_with('#') && c.len() == 7 { c.to_string() } else { "#8A93A6".into() }
    };
    let (vw, vh, l, r, t, b) = (300.0, 132.0, 6.0, 30.0, 10.0, 20.0);
    let pw = vw - l - r;
    let ph = vh - t - b;
    let top = f(w, "atReset").max(100.0).min(150.0);
    let x = |at: f64| l + (at - start) / (reset - start) * pw;
    let y = |pct: f64| t + ph - pct.min(top) / top * ph;
    let pts: Vec<(f64, f64)> = w
        .get("curve")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
        .collect();
    let line: String = pts
        .iter()
        .map(|(a, p)| format!("{:.1},{:.1}", x(*a), y(*p)))
        .collect::<Vec<_>>()
        .join(" ");
    let (nx, np) = pts.last().copied().unwrap_or((start, 0.0));
    let area = format!(
        "{:.1},{:.1} {line} {:.1},{:.1}",
        x(start),
        y(0.0),
        x(nx),
        y(0.0)
    );
    // Proyección: hasta el 100 % si se agota antes, si no hasta el reinicio.
    let out = w.get("out").and_then(Value::as_f64);
    let (px, pp) = match out {
        Some(o) if o > nx => (o, 100.0),
        Some(_) => (nx, np),
        None => (reset, f(w, "atReset")),
    };
    let axis: String = w
        .get("axis")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|d| {
            let ax = x(f(d, "at"));
            format!(
                "<line x1=\"{ax:.1}\" x2=\"{ax:.1}\" y1=\"{t}\" y2=\"{:.1}\" class=\"grid\"/><text x=\"{ax:.1}\" y=\"{:.1}\" class=\"day\">{}</text>",
                t + ph,
                vh - 6.0,
                lab(d, en),
            )
        })
        .collect();
    let y100 = y(100.0);
    format!(
        "<svg class=\"ub-curve\" viewBox=\"0 0 {vw} {vh}\" role=\"img\" aria-label=\"{aria}\" style=\"--acc:{color}\">\
<line x1=\"{l}\" x2=\"{:.1}\" y1=\"{y100:.1}\" y2=\"{y100:.1}\" class=\"limit\"/><text x=\"{:.1}\" y=\"{:.1}\" class=\"lbl\">100 %</text>\
{axis}<polygon points=\"{area}\" class=\"area\"/><polyline points=\"{line}\" class=\"line\"/>\
<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" class=\"proj\"/>\
<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"3.5\" class=\"now\"/></svg>",
        l + pw,
        l + pw + 3.0,
        y100 + 3.0,
        x(nx),
        y(np),
        x(px),
        y(pp),
        x(nx),
        y(np),
        aria = attr_esc(&format!(
            "{} {:.0} %",
            tr(en, "Semana usada:", "Week used:"),
            f(w, "percent")
        )),
    )
}

fn account_card(a: &Value, en: bool) -> String {
    let w = a.get("week").cloned().unwrap_or(Value::Null);
    let pct = f(&w, "percent");
    let state = if pct >= 100.0 {
        "crit"
    } else if w.get("out").is_some_and(|o| !o.is_null()) {
        "serious"
    } else {
        "good"
    };
    let reset = w.get("resetLabel").map(|l| lab(l, en)).unwrap_or_default();
    let forecast = if pct >= 100.0 {
        format!("{} <b>{reset}</b>", tr(en, "Agotada hasta el", "Used up until"))
    } else if let Some(out) = w.get("outLabel").filter(|o| !o.is_null()) {
        format!("{} <b>{}</b>", tr(en, "Se agota el", "Runs out"), lab(out, en))
    } else {
        format!(
            "{} <b>≈ {:.0} %</b> {}",
            tr(en, "Llega a", "Reaches"),
            f(&w, "atReset").max(pct),
            tr(en, "al reinicio", "by the reset")
        )
    };
    let block = match a.get("h5").filter(|h| !h.is_null()) {
        Some(h) => format!(
            "<p class=\"ub-block\">{} <b>{:.0} %</b> · {} {}</p>",
            tr(en, "Bloque de 5 h:", "5-hour block:"),
            f(h, "percent"),
            tr(en, "reinicia", "resets"),
            h.get("resetLabel").map(|l| lab(l, en)).unwrap_or_default()
        ),
        None => String::new(),
    };
    let estimated = if w.get("estimated").and_then(Value::as_bool) == Some(true) {
        ""
    } else {
        tr(en, "<p class=\"ub-note\">Sin turnos locales esta semana: sólo el punto de ahora.</p>", "<p class=\"ub-note\">No local turns this week: only the current point.</p>")
    };
    let stale = if w.get("stale").and_then(Value::as_bool) == Some(true) {
        tr(en, "<p class=\"ub-note\">El proveedor no respondió ahora: es la última lectura guardada.</p>", "<p class=\"ub-note\">The provider did not answer just now: this is the last stored reading.</p>")
    } else {
        ""
    };
    let plan = s(a, "plan");
    format!(
        "<article class=\"ub-acc {state}\"><header><i class=\"ub-dot\" style=\"background:{c}\"></i><b>{name}</b><small>{cli}{plan}</small><span class=\"ub-pct\">{pct:.0} %</span></header>\
{svg}<p class=\"ub-fore\">{forecast}</p><p class=\"ub-reset\">{} <b>{reset}</b></p>{block}{estimated}{stale}</article>",
        tr(en, "Reinicio", "Reset"),
        c = attr_esc(s(a, "color")),
        name = md_esc(s(a, "alias")),
        cli = md_esc(s(a, "cli")),
        plan = if plan.is_empty() { String::new() } else { format!(" · {}", md_esc(plan)) },
        svg = curve_svg(a, en),
    )
}

/// «main · Claude»: dos herramientas pueden tener una cuenta con el mismo nombre.
fn full_name(board: &Value, acc: &str) -> String {
    let cli = board
        .get("accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|a| s(a, "id") == acc)
        .map(|a| s(a, "cli"))
        .unwrap_or_default();
    if cli.is_empty() {
        name_of(board, acc)
    } else {
        format!("{} · {cli}", name_of(board, acc))
    }
}

fn free_tile(board: &Value, en: bool) -> String {
    let free: Vec<&Value> = board
        .get("free")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let Some(best) = free.first() else {
        return String::new();
    };
    let acc = s(best, "account");
    let mut lines = vec![format!(
        "<p><i class=\"ub-dot\" style=\"background:{}\"></i>{} <b>{}</b>: {} <b>{:.0} %</b> {} {}</p>",
        color_of(board, acc),
        tr(en, "Usa", "Use"),
        md_esc(&full_name(board, acc)),
        tr(en, "le queda", "it has"),
        f(best, "left"),
        tr(en, "hasta el", "left until"),
        best.get("resetLabel").map(|l| lab(l, en)).unwrap_or_default(),
    )];
    for a in &free {
        let Some(out) = a.get("outLabel").filter(|o| !o.is_null()) else {
            continue;
        };
        if f(a, "left") <= 0.0 {
            continue;
        }
        let acc = s(a, "account");
        lines.push(format!(
            "<p class=\"warn\"><i class=\"ub-dot\" style=\"background:{}\"></i><b>{}</b> {} <b>{}</b>, {} {}.</p>",
            color_of(board, acc),
            md_esc(&full_name(board, acc)),
            tr(en, "se agota el", "runs out"),
            lab(out, en),
            tr(en, "antes del reinicio del", "before its reset on"),
            a.get("resetLabel").map(|l| lab(l, en)).unwrap_or_default(),
        ));
    }
    for a in free.iter().filter(|a| a.get("blocked").and_then(Value::as_bool) == Some(true)) {
        let acc = s(a, "account");
        lines.push(format!(
            "<p class=\"warn\"><i class=\"ub-dot\" style=\"background:{}\"></i><b>{}</b> {}</p>",
            color_of(board, acc),
            md_esc(&full_name(board, acc)),
            tr(en, "tiene el bloque de 5 h agotado.", "has its 5-hour block used up."),
        ));
    }
    format!(
        "<section class=\"ub-free\"><h3>{}</h3>{}</section>",
        tr(en, "Dónde hay cupo", "Where there is room"),
        lines.join("")
    )
}

/// El tablero entero.
pub fn board(board: &Value, view: &View, en: bool) -> String {
    let accounts: String = board
        .get("accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|a| account_card(a, en))
        .collect();
    let rail = if accounts.is_empty() {
        format!(
            "<p class=\"ub-note\">{}</p>",
            tr(en, "Ninguna cuenta informa todavía su cupo semanal.", "No account reports its weekly limit yet.")
        )
    } else {
        accounts
    };
    format!(
        "<div class=\"ub\"><div class=\"ub-main\"><h3>{}</h3>{}{}</div><aside class=\"ub-rail\" aria-label=\"{}\">{rail}</aside></div>",
        tr(en, "¿En qué se va? · 14 días", "Where does it go? · 14 days"),
        table(board, view, en),
        free_tile(board, en),
        tr(en, "Cuentas", "Accounts"),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "now": 1000.0,
            "days": [{"es":"lun","en":"Mon"},{"es":"mar","en":"Tue"}],
            "accounts": [{
                "id":"claude:main","alias":"main","cli":"Claude","color":"#8B7CFF","plan":null,
                "week":{"percent":81.0,"start":0.0,"reset":2000.0,"resetLabel":{"es":"sáb 02:00","en":"Sat 02:00"},
                    "curve":[[0.0,0.0],[500.0,40.0],[1000.0,81.0]],"estimated":true,"atReset":120.0,
                    "out":1200.0,"outLabel":{"es":"vie 20:00","en":"Fri 20:00"},
                    "axis":[{"at":864.0,"es":"vie","en":"Fri"}]},
                "h5":{"percent":9.0,"reset":1500.0,"resetLabel":{"es":"sáb 00:29","en":"Sat 00:29"}}
            }],
            "projects":[{"name":"Comand<OS>","newTokens":6_100_000,"turns":42,"hours":3.4,"sessions":2,
                "share":{"claude:main":12.4},"spark":[0,5],
                "rows":[{"id":"s-1","account":"claude:main","model":"claude-opus-5-5","lastLabel":{"es":"vie 12:40","en":"Fri 12:40"},
                    "newTokens":100,"turns":3,"hours":0.2,"share":{},"spark":[0,1],"byHour":[[0.0,2],[3600.0,1]]}]}],
            "free":[{"account":"claude:main","left":19.0,"blocked":false,"out":1200.0,"outLabel":{"es":"vie 20:00","en":"Fri 20:00"},"resetLabel":{"es":"sáb 02:00","en":"Sat 02:00"}}]
        })
    }

    #[test]
    fn closed_projects_hide_sessions_and_escape_names() {
        let html = board(&sample(), &View::default(), false);
        assert!(html.contains("Comand&lt;OS&gt;"));
        assert!(html.contains("<b>12 %</b>"));
        assert!(html.contains("6.1 M"));
        assert!(!html.contains("data-ub-sess"));
        assert!(html.contains("Se agota el <b>vie 20:00</b>"));
        assert!(html.contains("Bloque de 5 h:"));
        assert!(html.contains("Dónde hay cupo"));
        assert!(!html.contains('$'));
    }

    #[test]
    fn opening_a_project_and_a_session() {
        let mut view = View::default();
        view.open_projects.insert("Comand<OS>".into());
        view.open_session = Some("s-1".into());
        let html = board(&sample(), &view, true);
        assert!(html.contains("data-ub-sess=\"s-1\""));
        assert!(html.contains("opus 5"));
        assert!(html.contains("Turns per active hour"));
        assert!(html.contains("Runs out <b>Fri 20:00</b>"));
    }

    #[test]
    fn curve_projects_to_the_limit_when_it_runs_out() {
        let svg = curve_svg(&sample()["accounts"][0], false);
        assert!(svg.contains("class=\"proj\""));
        assert!(svg.contains(">vie</text>"));
        assert!(svg.contains("100 %"));
    }

    #[test]
    fn token_units() {
        assert_eq!(tokens(312.0), "312");
        assert_eq!(tokens(1_500.0), "1.5 k");
        assert_eq!(tokens(740_000.0), "740 k");
        assert_eq!(tokens(6_100_000.0), "6.1 M");
        assert_eq!(tokens(298_000_000.0), "298 M");
    }
}
