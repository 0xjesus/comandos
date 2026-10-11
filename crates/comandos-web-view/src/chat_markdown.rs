//! Agent transcripts as chat HTML: the server renders each message once, so the phone
//! only inserts it. Markdown blocks (paragraphs, headings, lists, quotes, rules, tables,
//! fenced code) come out escaped and wrapping; nothing needs horizontal scrolling.
use crate::escape::text as esc;

/// Inline markdown of one escaped line: `code`, **bold**, *em*, ~~strike~~ and
/// [links](https://…) that open outside.
fn inline(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    let mut rest = line;
    while let Some(i) = rest.find(['`', '*', '_', '~', '[']) {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let (open, close, tag) = if rest.starts_with("**") {
            ("**", "**", "b")
        } else if rest.starts_with("~~") {
            ("~~", "~~", "s")
        } else if rest.starts_with('`') {
            ("`", "`", "code")
        } else if rest.starts_with('*') {
            ("*", "*", "i")
        } else if rest.starts_with('_') && (i == 0 || !out.ends_with(|c: char| c.is_alphanumeric())) {
            ("_", "_", "i")
        } else if rest.starts_with('[') {
            ("[", "](", "a")
        } else {
            out.push_str(&rest[..1]);
            rest = &rest[1..];
            continue;
        };
        let body = &rest[open.len()..];
        let found = body.find(close).filter(|&n| n > 0 && (tag == "code" || !body[..n].starts_with(' ')));
        match found {
            Some(n) if tag == "a" => {
                let after = &body[n + 2..];
                match after.find(')') {
                    Some(end) if after.starts_with("http://") || after.starts_with("https://") => {
                        out.push_str(&format!("<a href=\"{}\" target=\"_blank\" rel=\"noopener\">{}</a>", &after[..end], inline(&body[..n])));
                        rest = &after[end + 1..];
                    }
                    _ => {
                        out.push('[');
                        rest = body;
                    }
                }
            }
            Some(n) if tag == "code" => {
                out.push_str(&format!("<code>{}</code>", &body[..n]));
                rest = &body[n + close.len()..];
            }
            Some(n) => {
                out.push_str(&format!("<{tag}>{}</{tag}>", inline(&body[..n])));
                rest = &body[n + close.len()..];
            }
            None => {
                out.push_str(open);
                rest = body;
            }
        }
    }
    out.push_str(rest);
    out
}

fn list_item(t: &str) -> Option<(bool, &str)> {
    if let Some(item) = t.strip_prefix("- ").or(t.strip_prefix("* ")).or(t.strip_prefix("+ ")) {
        return Some((false, item));
    }
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0 && digits < 4)
        .then(|| t[digits..].strip_prefix(". ").or(t[digits..].strip_prefix(") ")))
        .flatten()
        .map(|item| (true, item))
}
fn table_cells(t: &str) -> Vec<&str> {
    t.trim().trim_start_matches('|').trim_end_matches('|').split('|').map(str::trim).collect()
}

/// Markdown as escaped block HTML: paragraphs, headings, lists, quotes, rules, tables
/// and fenced code. Every block wraps; nothing needs horizontal scrolling.
pub fn render_text(raw: &str) -> String {
    let mut out = String::new();
    let mut para: Vec<String> = Vec::new();
    let mut list: Option<bool> = None;
    let mut table = false;
    let flush = |out: &mut String, para: &mut Vec<String>, list: &mut Option<bool>, table: &mut bool| {
        if !para.is_empty() {
            out.push_str(&format!("<p>{}</p>", para.join("<br>")));
            para.clear();
        }
        if let Some(ordered) = list.take() {
            out.push_str(if ordered { "</ol>" } else { "</ul>" });
        }
        if std::mem::take(table) {
            out.push_str("</table>");
        }
    };
    for (i, part) in raw.split("```").enumerate() {
        if i % 2 == 1 {
            flush(&mut out, &mut para, &mut list, &mut table);
            let body = part.split_once('\n').map_or(part, |(_, b)| b);
            out.push_str(&format!("<pre>{}</pre>", esc(body.trim_end())));
            continue;
        }
        let escaped = esc(part);
        for line in escaped.split('\n') {
            let t = line.trim_start();
            if t.is_empty() {
                flush(&mut out, &mut para, &mut list, &mut table);
                continue;
            }
            if t.starts_with('|') {
                let cells = table_cells(t);
                if cells.iter().all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' '))) {
                    continue;
                }
                let head = !table;
                if head {
                    flush(&mut out, &mut para, &mut list, &mut table);
                    out.push_str("<table>");
                    table = true;
                }
                let cell = if head { "th" } else { "td" };
                out.push_str("<tr>");
                for c in cells {
                    out.push_str(&format!("<{cell}>{}</{cell}>", inline(c)));
                }
                out.push_str("</tr>");
                continue;
            }
            if let Some((ordered, item)) = list_item(t) {
                if list != Some(ordered) {
                    flush(&mut out, &mut para, &mut list, &mut table);
                    out.push_str(if ordered {
                        let n: String = t.chars().take_while(char::is_ascii_digit).collect();
                        if n == "1" { "<ol>".to_owned() } else { format!("<ol start=\"{n}\">") }
                    } else {
                        "<ul>".to_owned()
                    }
                    .as_str());
                    list = Some(ordered);
                }
                let nested = line.len() - t.len() >= 2;
                out.push_str(&format!("<li{}>{}</li>", if nested { " class=\"n\"" } else { "" }, inline(item)));
                continue;
            }
            let heading = t.strip_prefix("### ").map(|h| (3, h)).or(t.strip_prefix("## ").map(|h| (2, h))).or(t.strip_prefix("# ").map(|h| (1, h)));
            if let Some((level, h)) = heading {
                flush(&mut out, &mut para, &mut list, &mut table);
                out.push_str(&format!("<h{} class=\"cv-h\">{}</h{0}>", level + 2, inline(h)));
            } else if t.len() >= 3 && t.chars().all(|c| c == '-' || c == '*' || c == '_') {
                flush(&mut out, &mut para, &mut list, &mut table);
                out.push_str("<hr>");
            } else if let Some(q) = t.strip_prefix("&gt; ").or(t.strip_prefix("&gt;")) {
                flush(&mut out, &mut para, &mut list, &mut table);
                out.push_str(&format!("<blockquote>{}</blockquote>", inline(q)));
            } else {
                if list.is_some() || table {
                    flush(&mut out, &mut para, &mut list, &mut table);
                }
                para.push(inline(line.trim_end()));
            }
        }
    }
    flush(&mut out, &mut para, &mut list, &mut table);
    out
}

pub fn render_message(role: &str, t: &str) -> String {
    match role {
        "u" => format!("<div class=\"cv-me\">{}</div>", render_text(t)),
        "a" => format!("<div class=\"cv-ai\">{}</div>", render_text(t)),
        _ => {
            let mut out = String::from("<div class=\"cv-tool\">");
            for line in t.lines() {
                out.push_str(&format!("<span>{}</span>", esc(line)));
            }
            out.push_str("</div>");
            out
        }
    }
}
pub fn render_messages(messages: &[(String, String)]) -> String {
    messages.iter().map(|(r, t)| render_message(r, t)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn m(r: &str, t: &str) -> (String, String) {
        (r.into(), t.into())
    }
    #[test]
    fn text_escapes_and_formats_markdown_blocks() {
        assert_eq!(
            render_text("<b>x</b> **ok** `a<b`\nfin"),
            "<p>&lt;b&gt;x&lt;/b&gt; <b>ok</b> <code>a&lt;b</code><br>fin</p>"
        );
        assert_eq!(render_text("ver:\n```rust\nlet a = 1;\n```"), "<p>ver:</p><pre>let a = 1;</pre>");
        assert_eq!(
            render_text("## Plan\n- uno\n- **dos**\n\n1. a\n2. b"),
            "<h4 class=\"cv-h\">Plan</h4><ul><li>uno</li><li><b>dos</b></li></ul><ol><li>a</li><li>b</li></ol>"
        );
        assert_eq!(
            render_text("| a | b |\n|---|:-:|\n| 1 | `x` |"),
            "<table><tr><th>a</th><th>b</th></tr><tr><td>1</td><td><code>x</code></td></tr></table>"
        );
        assert_eq!(render_text("> ojo\n---"), "<blockquote>ojo</blockquote><hr>");
        assert_eq!(
            render_text("[doc](https://x.io/?a=1&b=2) snake_case_name 2 * 3"),
            "<p><a href=\"https://x.io/?a=1&amp;b=2\" target=\"_blank\" rel=\"noopener\">doc</a> snake_case_name 2 * 3</p>"
        );
        assert_eq!(render_text("[x](javascript:alert(1))"), "<p>[x](javascript:alert(1))</p>");
    }
    #[test]
    fn roles_render_as_bubbles_and_tool_lines() {
        let html = render_messages(&[m("u", "hola"), m("t", "Bash · ls\nRead · a.rs"), m("a", "listo")]);
        assert_eq!(html, "<div class=\"cv-me\"><p>hola</p></div><div class=\"cv-tool\"><span>Bash · ls</span><span>Read · a.rs</span></div><div class=\"cv-ai\"><p>listo</p></div>");
    }
}
