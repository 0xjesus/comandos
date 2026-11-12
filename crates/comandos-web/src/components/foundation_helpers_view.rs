//! Pure formatting and decisions shared by the complete helpers browser port.
use serde_json::{Value, json};
pub fn at<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&Value::Null)
}
pub fn s(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    }
}
pub fn truth(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}
pub fn num(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => {
            if s.trim().is_empty() {
                0.0
            } else {
                s.trim().parse().unwrap_or(f64::NAN)
            }
        }
        _ => f64::NAN,
    }
}
pub fn list(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}
pub fn md_esc(v: &str) -> String {
    comandos_web_view::escape::md_esc(v)
}
pub fn attr_esc(v: &str) -> String {
    comandos_web_view::escape::attr_esc(v)
}
pub fn short_path(v: &str) -> String {
    if let Some(tail) = v.strip_prefix("/home/")
        && let Some((_, path)) = tail.split_once('/')
    {
        return path
            .strip_prefix("codebase/")
            .map(str::to_string)
            .unwrap_or_else(|| format!("~/{path}"));
    }
    if let Some(tail) = v.strip_prefix("/home/")
        && !tail.is_empty()
    {
        return "~".into();
    }
    v.into()
}
pub fn ago_txt(ts: f64, now: f64, en: bool) -> String {
    if ts == 0.0 || ts.is_nan() {
        return String::new();
    }
    let seconds = (now - ts).floor().max(0.0);
    let unit = |n: f64, es: &str, short: &str| {
        if en {
            format!("{n}{short} ago")
        } else {
            format!("hace {n} {es}")
        }
    };
    if seconds < 60.0 {
        return unit(seconds, "s", "s");
    }
    if seconds < 3600.0 {
        return unit((seconds / 60.0).floor(), "min", "m");
    }
    if seconds < 86400.0 {
        let h = (seconds / 3600.0).floor();
        let m = ((seconds % 3600.0) / 60.0).floor();
        return if en {
            format!(
                "{h}h{} ago",
                if m != 0.0 {
                    format!(" {m}m")
                } else {
                    String::new()
                }
            )
        } else {
            format!(
                "hace {h} h{}",
                if m != 0.0 {
                    format!(" {m} min")
                } else {
                    String::new()
                }
            )
        };
    }
    unit((seconds / 86400.0).floor(), "d", "d")
}
pub fn fmt_money(n: f64) -> String {
    if n == 0.0 || n.is_nan() {
        "$0.00".into()
    } else if n < 0.01 {
        format!("${n:.4}")
    } else {
        format!("${n:.2}")
    }
}
pub fn fmt_tokens(n: f64) -> String {
    if n >= 1e9 {
        format!("{:.2}B tok", n / 1e9)
    } else if n >= 1e6 {
        format!("{:.1}M tok", n / 1e6)
    } else if n >= 1000.0 {
        format!("{}k tok", (n / 1000.0 + 0.5).floor())
    } else {
        format!("{} tok", (n + 0.5).floor())
    }
}
pub fn fmt_percent(v: &Value) -> String {
    if v.is_null() {
        "--".into()
    } else {
        let n = num(v);
        if n.fract() == 0.0 {
            format!("{n:.0}%")
        } else {
            format!("{n:.1}%")
        }
    }
}
pub fn fmt_bytes(n: f64) -> String {
    if n < 1024.0 {
        format!("{n} B")
    } else if n < 1048576.0 {
        format!("{:.0} KB", n / 1024.0)
    } else {
        format!("{:.1} MB", n / 1048576.0)
    }
}
pub fn short_model(m: &str) -> String {
    m.strip_prefix("claude-").unwrap_or(m).into()
}
pub fn confidence(c: &str) -> &str {
    match c {
        "detected" => "detectado",
        "exact" => "exacto",
        "reconciled" => "reconciliado",
        "unattributed" => "sin asignar",
        "transcript" => "del transcript",
        "measured" => "medido local",
        _ => c,
    }
}
pub fn stage_label(r: &Value) -> String {
    if truth(at(r, "pendingReason")) {
        return s(at(r, "pendingReason"));
    }
    let stage = if truth(at(r, "stage")) {
        s(at(r, "stage"))
    } else {
        s(at(r, "state"))
    };
    match stage.as_str() {
        "validating" => "Validando configuración…",
        "waiting" => "Esperando a que termine el turno…",
        "snapshot" => "Guardando recuperación…",
        "applying" => "Iniciando CLI…",
        "verifying" => "Comprobando configuración…",
        "recovering" => "Recuperando sesión original…",
        "awaiting_confirmation" => "Confirmación pendiente: revisa la terminal",
        "" => "Comprobando…",
        _ => &stage,
    }
    .into()
}
fn paired(input: &str, delimiter: &str, open: &str, close: &str, no_newline: bool) -> String {
    let mut rest = input;
    let mut out = String::new();
    while let Some(start) = rest.find(delimiter) {
        out.push_str(rest.get(..start).unwrap_or_default());
        let tail = rest.get(start + delimiter.len()..).unwrap_or_default();
        if let Some(end) = tail.find(delimiter)
            && end > 0
            && (!no_newline || !tail.get(..end).unwrap_or_default().contains('\n'))
        {
            out.push_str(open);
            out.push_str(tail.get(..end).unwrap_or_default());
            out.push_str(close);
            rest = tail.get(end + delimiter.len()..).unwrap_or_default();
        } else {
            out.push_str(delimiter);
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}
pub fn md_inline(l: &str) -> String {
    let bold = paired(l, "**", "<b>", "</b>", true);
    let mut italics = String::new();
    let mut rest = bold.as_str();
    let mut previous = None;
    while let Some(i) = rest.find('*') {
        let prefix = rest.get(..i).unwrap_or_default();
        italics += prefix;
        let before = prefix.chars().last().or(previous);
        let tail = rest.get(i + 1..).unwrap_or_default();
        if before.is_none_or(|c| c.is_whitespace() || c == '(')
            && let Some(end) = tail.find('*')
            && end > 0
            && !tail.get(..end).unwrap_or_default().contains('\n')
            && tail
                .get(end + 1..)
                .and_then(|s| s.chars().next())
                .is_none_or(|c| c.is_whitespace() || ").,;:!?".contains(c))
        {
            italics += "<i>";
            italics += tail.get(..end).unwrap_or_default();
            italics += "</i>";
            rest = tail.get(end + 1..).unwrap_or_default();
            previous = Some('>');
        } else {
            italics.push('*');
            previous = Some('*');
            rest = tail;
        }
    }
    italics += rest;
    let code = paired(&italics, "`", "<code>", "</code>", false);
    let mut out = String::new();
    let mut rest = code.as_str();
    while let Some(start) = rest.find('[') {
        out += rest.get(..start).unwrap_or_default();
        let tail = rest.get(start + 1..).unwrap_or_default();
        if let Some(end) = tail.find("](")
            && end > 0
            && let Some(close) = tail.get(end + 2..).and_then(|s| s.find(')'))
            && !tail
                .get(end + 2..end + 2 + close)
                .unwrap_or_default()
                .chars()
                .any(char::is_whitespace)
        {
            out += "<u>";
            out += tail.get(..end).unwrap_or_default();
            out += "</u>";
            rest = tail.get(end + 3 + close..).unwrap_or_default();
        } else {
            out.push('[');
            rest = tail;
        }
    }
    out += rest;
    out
}
fn table(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = String::from("<table>");
    let mut i = 0;
    for l in lines {
        let trim = l.trim();
        if trim.chars().all(|c| c.is_whitespace() || "|-:".contains(c)) {
            continue;
        }
        let tag = if i == 0 { "th" } else { "td" };
        i += 1;
        out += "<tr>";
        for c in trim
            .strip_prefix('|')
            .unwrap_or(trim)
            .strip_suffix('|')
            .unwrap_or(trim.strip_prefix('|').unwrap_or(trim))
            .split('|')
        {
            out += &format!("<{tag}>{}</{tag}>", md_inline(c.trim()));
        }
        out += "</tr>";
    }
    out += "</table>";
    out
}
pub fn md_html(input: &str, en: bool) -> String {
    let mut out = String::new();
    let mut code = false;
    let mut tbl = vec![];
    for l in md_esc(input).split('\n') {
        if l.trim().starts_with("```") {
            out += &table(&tbl);
            tbl.clear();
            code = !code;
            out += if code { "<pre>" } else { "</pre>" };
            continue;
        }
        if code {
            out += l;
            out.push('\n');
            continue;
        }
        if l.trim().starts_with('|') && l.trim().ends_with('|') {
            tbl.push(l.to_string());
            continue;
        }
        out += &table(&tbl);
        tbl.clear();
        let hashes = l.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && l.get(hashes..).is_some_and(|l| l.starts_with(' ')) {
            out += &format!(
                "<b class=\"mdh\">{}</b>",
                md_inline(l.get(hashes + 1..).unwrap_or_default())
            );
            continue;
        }
        if l.trim().len() >= 3 && l.trim().chars().all(|c| c == '-') {
            out += "<hr>";
            continue;
        }
        let inline = md_inline(l);
        let trimmed = inline.trim_start();
        if let Some(tail) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            out += inline
                .get(..inline.len() - trimmed.len())
                .unwrap_or_default();
            out += "• ";
            out += tail;
        } else {
            out += &inline
        }
        out += "<br>";
    }
    out += &table(&tbl);
    if code {
        out += "</pre>"
    }
    linkify_paths(&out, en)
}
fn path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_.+@%,=-".contains(c)
}
fn text_paths(input: &str, en: bool) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        let rest = input.get(i..).unwrap_or_default();
        let home = rest.starts_with("~/");
        if home || rest.starts_with('/') {
            let prefix = if home { 2 } else { 1 };
            let mut segments = 0;
            let mut current = false;
            let mut valid_end = 0;
            for (ch_pos, ch) in rest.get(prefix..).unwrap_or_default().char_indices() {
                if path_char(ch) {
                    current = true;
                    if home || segments >= 1 {
                        valid_end = prefix + ch_pos + ch.len_utf8();
                    }
                } else if ch == '/' && current {
                    segments += 1;
                    current = false;
                } else {
                    break;
                }
            }
            if valid_end > prefix {
                let end = valid_end;
                let path = rest.get(..end).unwrap_or_default();
                let mut line_end = end;
                if rest.get(end..).is_some_and(|r| r.starts_with(':')) {
                    let digits = rest
                        .get(end + 1..)
                        .unwrap_or_default()
                        .bytes()
                        .take_while(u8::is_ascii_digit)
                        .count();
                    if digits > 0 {
                        line_end = end + 1 + digits;
                    }
                }
                out += &format!(
                    "<a href=\"#\" class=\"pathlink\" data-path=\"{}\" title=\"{}\">{}</a>",
                    attr_esc(path),
                    if en {
                        "Open with default app"
                    } else {
                        "Abrir con la app default"
                    },
                    rest.get(..line_end).unwrap_or_default()
                );
                i += line_end;
                continue;
            }
        }
        if let Some(c) = rest.chars().next() {
            out.push(c);
            i += c.len_utf8();
        } else {
            break;
        }
    }
    out
}
pub fn linkify_paths(html: &str, en: bool) -> String {
    let mut rest = html;
    let mut out = String::new();
    let mut anchors = 0usize;
    while let Some(start) = rest.find('<') {
        let pre = rest.get(..start).unwrap_or_default();
        out += &if anchors > 0 {
            pre.into()
        } else {
            text_paths(pre, en)
        };
        let tail = rest.get(start..).unwrap_or_default();
        if let Some(end) = tail.find('>') {
            let tag = tail.get(..end + 1).unwrap_or_default();
            let lower = tag.to_ascii_lowercase();
            if lower.starts_with("<a>")
                || lower.starts_with("<a ")
                || lower.starts_with("<a\t")
                || lower.starts_with("<a\n")
            {
                anchors += 1;
            } else if lower == "</a>" {
                anchors = anchors.saturating_sub(1);
            }
            out += tag;
            rest = tail.get(end + 1..).unwrap_or_default();
        } else {
            rest = tail;
            break;
        }
    }
    out += &if anchors > 0 {
        rest.into()
    } else {
        text_paths(rest, en)
    };
    out
}
pub fn md_strip(s: &str) -> String {
    let s = s.replace("**", "").replace('`', "");
    let hashes = s.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && s.get(hashes..).is_some_and(|s| s.starts_with(' ')) {
        s.get(hashes + 1..).unwrap_or_default().into()
    } else {
        s
    }
}
pub fn row_key(it: &Value) -> String {
    let session = s(at(it, "session"));
    let pane = s(at(it, "pane"));
    format!(
        "{session}{}",
        if pane.is_empty() {
            String::new()
        } else {
            format!("|{pane}")
        }
    )
}
pub fn pick_sel(items: &Value, selected: &str, selected_ts: f64, active: &Value) -> Value {
    let items: list_wrapper::Items = list(items)
        .into_iter()
        .filter(|it| truth(at(it, "alive")) && at(it, "operable").as_bool() != Some(false))
        .collect();
    let session = s(at(active, "session"));
    let pane = s(at(active, "pane"));
    if !session.is_empty() {
        let same: Vec<_> = items
            .iter()
            .filter(|it| s(at(it, "session")) == session)
            .collect();
        if selected.split('|').next() == Some(&session)
            && (pane.is_empty() || selected_ts > num(at(active, "ts")) * 1000.0)
            && let Some(it) = same.iter().find(|it| row_key(it) == selected)
        {
            return (*it).clone();
        }
        return if pane.is_empty() {
            same.first().map(|it| (*it).clone()).unwrap_or(Value::Null)
        } else {
            same.into_iter()
                .find(|it| s(at(it, "pane")) == pane)
                .cloned()
                .unwrap_or(Value::Null)
        };
    }
    if !selected.is_empty() {
        return items
            .iter()
            .find(|it| {
                row_key(it) == selected
                    || (!selected.contains('|') && s(at(it, "session")) == selected)
            })
            .cloned()
            .unwrap_or(Value::Null);
    }
    items
        .iter()
        .find(|it| s(at(it, "session")) == "local")
        .or_else(|| items.first())
        .cloned()
        .unwrap_or(Value::Null)
}
mod list_wrapper {
    pub type Items = Vec<serde_json::Value>;
}
pub fn ns_defaults() -> Value {
    json!({"harness":"codex","motor":"codex","routeId":"codex:codex","model":"gpt-6.1-sol","effort":"high","harnessAccount":"main","motorAccount":"main"})
}
