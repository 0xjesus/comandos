//! The choice an agent waits on, read from its screen, and the answer to it.
//!
//! [`parse`] reads every terminal menu the agents draw: Claude Code permissions and
//! plans, its question menus (tabs, descriptions under each option, `[ ]` / `[✔]`
//! checkboxes, «Type something», an unnumbered «Submit» row, a rule and «Chat about
//! this»), and Codex approvals (`›`). POST `/chat/answer` answers it the way a person
//! does: it reads the screen again, checks it is the same menu ([`signature`]), moves
//! the cursor with ↑/↓ to the chosen row (typing the text first for «Type something»)
//! and presses Enter. Every client (web, desktop, phone) answers through it.
use super::{Answer, Entry, Key, Native, NativeRoute, Verb, reply, target};
use crate::Request;
use http::StatusCode;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub const ROUTES: &[Entry] = &[Entry {
    verb: Verb::Post,
    key: Key::Path("/chat/answer"),
    route: NativeRoute::ChatAnswer,
}];

/// Lines of context above the question.
const MAX_DETAIL: usize = 8;
/// Longest text typed into «Type something».
const MAX_TEXT: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A numbered option.
    Option,
    /// «Type something»: the answer is free text typed on it.
    Free,
    /// The unnumbered row that ends a multiple choice («Submit»).
    Submit,
    /// «Chat about this»: leave the menu and talk instead.
    Chat,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Option => "option",
            Kind::Free => "free",
            Kind::Submit => "submit",
            Kind::Chat => "chat",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The digits («1»…); empty for an unnumbered row.
    pub key: String,
    pub label: String,
    /// The lines under it.
    pub desc: Vec<String>,
    pub cursor: bool,
    /// Multiple choice: ticked or not.
    pub checked: Option<bool>,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub question: String,
    pub detail: Vec<String>,
    /// The question tabs (label, answered).
    pub tabs: Vec<(String, bool)>,
    pub rows: Vec<Row>,
}

impl Menu {
    pub fn cursor(&self) -> Option<usize> {
        self.rows.iter().position(|r| r.cursor)
    }
    pub fn multi(&self) -> bool {
        self.rows.iter().any(|r| r.checked.is_some())
    }
}

/// Box drawing and the cursor marks a terminal dialog uses, trimmed off a screen line.
pub fn screen_line(line: &str) -> &str {
    line.trim_matches(|c: char| c.is_whitespace() || matches!(c, '│' | '┃' | '║' | '|' | '▌'))
}

const CURSORS: [char; 5] = ['❯', '›', '>', '▶', '→'];

/// (cursor here, the rest of the line without it).
fn cursor_of(line: &str) -> (bool, &str) {
    let t = screen_line(line);
    match t.chars().next() {
        Some(c) if CURSORS.contains(&c) => (true, t[c.len_utf8()..].trim_start()),
        _ => (false, t),
    }
}

/// `❯ 1. Yes` → (cursor, "1", "Yes"); `2) No` → (false, "2", "No").
fn option_line(line: &str) -> Option<(bool, String, String)> {
    let (cursor, rest) = cursor_of(line);
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 2 {
        return None;
    }
    let after = &rest[digits.len()..];
    let label = after.strip_prefix('.').or_else(|| after.strip_prefix(')'))?;
    if !label.starts_with(' ') || label.trim().is_empty() {
        return None;
    }
    Some((cursor, digits, label.trim().to_owned()))
}

/// The unnumbered row that closes a multiple choice, with or without the cursor.
fn submit_line(line: &str) -> Option<(bool, String)> {
    let (cursor, rest) = cursor_of(line);
    let word = rest.trim();
    let known = ["Submit", "Next", "Enviar", "Siguiente", "Continue", "Continuar"]
        .iter()
        .any(|w| word.eq_ignore_ascii_case(w));
    known.then(|| (cursor, word.to_owned()))
}

fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.chars().any(|c| matches!(c, '─' | '━' | '═' | '-'))
        && t.chars().all(|c| matches!(c, '─' | '━' | '═' | '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘' | '-' | '│' | ' '))
}

/// Lines under a menu that are not its hints (a status line, a prompt) before it
/// stops counting as open.
const MAX_BELOW: usize = 2;

/// `Enter to select · Tab/Arrow keys to navigate · Esc to cancel` and the like.
fn hint_line(line: &str) -> bool {
    let t = screen_line(line).to_lowercase();
    ["enter to", "esc to", "to navigate", "to cancel", "to confirm", "tab to", "shift+tab", "ctrl+"].iter().any(|h| t.contains(h))
}

/// `←  ☐ Imágenes  ☒ Extras  ✔ Submit  →`: the question tabs (label, answered);
/// «Submit» is where the answers are sent, not a question.
fn tabs_line(line: &str) -> Option<Vec<(String, bool)>> {
    let t = screen_line(line);
    if !(t.starts_with('←') && t.ends_with('→')) {
        return None;
    }
    let inner = t.trim_start_matches('←').trim_end_matches('→');
    let mut tabs = Vec::new();
    let mut cur: Option<(String, bool)> = None;
    for c in inner.chars() {
        match c {
            '☐' | '☒' | '✔' | '✓' | '■' | '□' => {
                if let Some((l, d)) = cur.take() {
                    tabs.push((l.trim().to_owned(), d));
                }
                cur = Some((String::new(), c != '☐' && c != '□'));
            }
            c => {
                if let Some((l, _)) = cur.as_mut() {
                    l.push(c);
                }
            }
        }
    }
    if let Some((l, d)) = cur {
        tabs.push((l.trim().to_owned(), d));
    }
    tabs.retain(|(l, _)| !l.eq_ignore_ascii_case("submit"));
    (!tabs.is_empty()).then_some(tabs)
}

/// A line of Claude's quoted question block: `│ text`, open on the right (a boxed
/// dialog closes its lines with `│` too).
fn quoted(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('│') && !t.ends_with('│') && !screen_line(line).is_empty()
}

/// `[ ] Label` / `[✔] Label` → (Some(ticked), "Label").
fn checkbox(label: &str) -> (Option<bool>, String) {
    for (mark, on) in [("[ ]", false), ("[✔]", true), ("[x]", true), ("[X]", true), ("[✓]", true), ("☐", false), ("☒", true)] {
        if let Some(rest) = label.strip_prefix(mark) {
            return (Some(on), rest.trim().to_owned());
        }
    }
    (None, label.to_owned())
}

fn kind_of(label: &str) -> Kind {
    let l = label.trim().trim_end_matches('.').to_lowercase();
    if l == "type something" || l == "escribe algo" {
        Kind::Free
    } else if l == "chat about this" || l == "hablar de esto" {
        Kind::Chat
    } else {
        Kind::Option
    }
}

/// A line that belongs to the menu but is not a row: a description or a wrapped label
/// (indented), or a rule between rows.
fn inner_line(line: &str) -> bool {
    let indented = line.starts_with("  ") || line.starts_with('\t') || line.starts_with("│  ");
    !screen_line(line).is_empty() && (indented || is_rule(line))
}

fn row_line(line: &str) -> bool {
    option_line(line).is_some() || submit_line(line).is_some()
}

/// The menu on an agent's screen: the last block of rows with a cursor, numbered from
/// 1. `None` when nothing waits (a plain numbered list has no cursor).
pub fn parse(screen: &str) -> Option<Menu> {
    let lines: Vec<&str> = screen.lines().collect();
    let last = lines.iter().rposition(|l| row_line(l))?;
    // Walk up over rows, their descriptions and inner rules to the first row.
    let mut first = last;
    let mut i = last;
    while i > 0 {
        i -= 1;
        if row_line(lines[i]) {
            first = i;
        } else if inner_line(lines[i]) {
            // A rule above the first row is the menu's top border.
            if is_rule(lines[i]) && !lines[..i].iter().rev().find(|l| !screen_line(l).is_empty()).is_some_and(|l| row_line(l) || inner_line(l)) {
                break;
            }
        } else {
            break;
        }
    }
    // The last row's own description: indented lines down to a gap or a border.
    let mut end = last;
    while end + 1 < lines.len() && inner_line(lines[end + 1]) && !is_rule(lines[end + 1]) && end - last < 4 {
        end += 1;
    }
    // Output under the menu means it was answered and the screen moved on: nothing
    // waits there any more (only its key hints and borders may follow).
    let below = lines[end + 1..].iter().filter(|l| !screen_line(l).is_empty() && !is_rule(l) && !hint_line(l)).count();
    if below > MAX_BELOW {
        return None;
    }
    let mut rows: Vec<Row> = Vec::new();
    for line in &lines[first..=end] {
        if let Some((cursor, key, label)) = option_line(line) {
            let (checked, label) = checkbox(&label);
            let kind = kind_of(&label);
            rows.push(Row { key, label, desc: Vec::new(), cursor, checked, kind });
        } else if let Some((cursor, label)) = submit_line(line) {
            rows.push(Row { key: String::new(), label, desc: Vec::new(), cursor, checked: None, kind: Kind::Submit });
        } else if !is_rule(line)
            && let Some(prev) = rows.last_mut()
        {
            let more = screen_line(line);
            if !more.is_empty() {
                prev.desc.push(more.to_owned());
            }
        }
    }
    // Claude's question menus put a description under each option; everywhere else
    // (permissions, plans, Codex) an indented line is the label wrapping.
    let footer = lines[end + 1..].iter().take(4).any(|l| l.contains("Enter to select") || l.contains("to navigate"));
    let question_menu = footer || rows.iter().any(|r| r.checked.is_some() || r.kind != Kind::Option);
    if !question_menu {
        for r in &mut rows {
            for more in r.desc.drain(..) {
                r.label.push(' ');
                r.label.push_str(&more);
            }
        }
    }
    let numbered: Vec<&Row> = rows.iter().filter(|r| !r.key.is_empty()).collect();
    let in_order = numbered.iter().enumerate().all(|(n, r)| r.key == (n + 1).to_string());
    if numbered.len() < 2 || !in_order || !rows.iter().any(|r| r.cursor) {
        return None;
    }
    // Above the rows: the question (skipping blanks), then its context up to a rule.
    let mut tabs = Vec::new();
    let mut question = String::new();
    let mut at = first;
    while at > 0 {
        at -= 1;
        let t = screen_line(lines[at]);
        if t.is_empty() {
            continue;
        }
        if let Some(found) = tabs_line(lines[at]) {
            tabs = found;
            break;
        }
        if !is_rule(lines[at]) {
            question = t.to_owned();
            // Claude quotes a long question in a `│ ` block that wraps over
            // several lines: all of it is the question.
            while quoted(lines[at]) && at > 0 && quoted(lines[at - 1]) {
                at -= 1;
                question = format!("{} {question}", screen_line(lines[at]));
            }
        }
        break;
    }
    let mut detail: Vec<String> = Vec::new();
    if !question.is_empty() {
        for line in lines[..at].iter().rev() {
            if is_rule(line) || detail.len() >= MAX_DETAIL {
                break;
            }
            if let Some(found) = tabs_line(line) {
                tabs = found;
                break;
            }
            let text = screen_line(line);
            if text.is_empty() && detail.is_empty() {
                continue;
            }
            detail.push(text.to_owned());
        }
        while detail.last().is_some_and(String::is_empty) {
            detail.pop();
        }
        detail.reverse();
    }
    Some(Menu { question, detail, tabs, rows })
}

/// Identity of a menu: what it asks and offers, not where the cursor is nor what is
/// ticked (toggling a checkbox keeps it the same menu). FNV-1a, hex.
pub fn signature(m: &Menu) -> String {
    let mut text = m.question.clone();
    for r in &m.rows {
        text.push('\u{1}');
        text.push_str(&r.key);
        text.push(':');
        text.push_str(if r.kind == Kind::Free { "free" } else { &r.label });
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The menu as the clients draw it (`ask` of `/chat/transcript`).
pub fn to_json(m: &Menu) -> Value {
    json!({
        "question": m.question,
        "detail": m.detail,
        "tabs": m.tabs.iter().map(|(l, d)| json!({"label": l, "done": d})).collect::<Vec<_>>(),
        "multi": m.multi(),
        "sig": signature(m),
        "options": m.rows.iter().map(|r| json!({
            "key": r.key,
            "label": r.label,
            "desc": r.desc,
            "on": r.cursor,
            "checked": r.checked,
            "kind": r.kind.name(),
        })).collect::<Vec<_>>(),
    })
}

/// The keys that answer [`m`] with row [`row`]: ↑/↓ from the cursor to it, then
/// Enter (the free text is typed in between).
pub fn moves(m: &Menu, row: usize) -> Option<Vec<&'static str>> {
    let cursor = m.cursor()?;
    if row >= m.rows.len() {
        return None;
    }
    let key = if row > cursor { "Down" } else { "Up" };
    Some(std::iter::repeat_n(key, row.abs_diff(cursor)).collect())
}

fn error(status: StatusCode, message: &str) -> Answer {
    reply(status, &json!({"ok": false, "error": message}))
}

/// POST `/chat/answer` `{session, pane, sig, row, text?}`.
pub async fn answer(native: &Arc<Native>, request: &Request) -> Answer {
    let Some(body) = request.data.as_ref().and_then(Value::as_object) else {
        return error(StatusCode::BAD_REQUEST, "Se esperaba {session, pane, sig, row}");
    };
    let session = body.get("session").and_then(Value::as_str).unwrap_or("");
    let pane = body.get("pane").and_then(Value::as_str).unwrap_or("");
    let sig = body.get("sig").and_then(Value::as_str).unwrap_or("");
    let Some(row) = body.get("row").and_then(Value::as_u64).and_then(|r| usize::try_from(r).ok()) else {
        return error(StatusCode::BAD_REQUEST, "Falta la opción");
    };
    let text = body.get("text").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
    if session.is_empty() || sig.is_empty() || super::py::is_pane(pane) != Some(true) {
        return error(StatusCode::BAD_REQUEST, "Sesión o pane inválidos");
    }
    if text.is_some_and(|t| t.chars().count() > MAX_TEXT || t.chars().any(|c| c.is_control() && c != '\n')) {
        return error(StatusCode::BAD_REQUEST, "Texto no válido");
    }
    let pt = match target::post_target(native, "/chat/answer", &json!({"session": session, "pane": pane})).await {
        Ok(pt) => pt,
        Err(Ok(_)) => return error(StatusCode::BAD_REQUEST, "Sesión inválida"),
        Err(Err(fault)) => return Err(fault),
    };
    if pt.pane != pane {
        return error(StatusCode::NOT_FOUND, "Ese pane ya no existe");
    }
    let tmux = &native.options().tmux;
    if !native.typing.locks.acquire(pane) {
        return error(StatusCode::CONFLICT, "Se está escribiendo en ese pane; vuelve a intentarlo");
    }
    let result = async {
        // A phone swipe may have left tmux's history open: keys would scroll it.
        if let Ok(m) = tmux.run(&["display-message", "-p", "-t", pane, "#{pane_in_mode}"]).await
            && m.ok
            && m.stdout.trim() == "1"
        {
            let _ = tmux.run(&["send-keys", "-X", "-t", pane, "cancel"]).await;
        }
        let screen = tmux.run(&["capture-pane", "-p", "-t", pane]).await.map_err(|_| "No pude leer el pane")?;
        let Some(menu) = parse(&screen.stdout) else {
            return Err("Ya no hay una pregunta abierta; no respondí");
        };
        if signature(&menu) != sig {
            return Err("La pregunta cambió; no respondí");
        }
        let Some(keys) = moves(&menu, row) else {
            return Err("Esa opción ya no está; no respondí");
        };
        if text.is_some() != (menu.rows[row].kind == Kind::Free) {
            return Err("Esa opción no lleva texto");
        }
        if !keys.is_empty() {
            let mut args = vec!["send-keys", "-t", pane];
            args.extend(keys);
            if !tmux.run(&args).await.is_ok_and(|o| o.ok) {
                return Err("tmux no pudo mover la selección");
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
        if let Some(t) = text {
            if !tmux.run(&["send-keys", "-t", pane, "-l", t]).await.is_ok_and(|o| o.ok) {
                return Err("tmux no pudo escribir el texto");
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
        if !tmux.run(&["send-keys", "-t", pane, "Enter"]).await.is_ok_and(|o| o.ok) {
            return Err("tmux no pudo confirmar");
        }
        Ok(())
    }
    .await;
    native.typing.locks.release(pane);
    match result {
        Ok(()) => reply(StatusCode::OK, &json!({"ok": true})),
        Err(why) => error(StatusCode::CONFLICT, why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claude Code's question menu, as tmux captured it (two questions, the first).
    const QUESTION: &str = "\
● Bash(echo listo)
  ⎿  listo
────────────────────────────────────────────────────────────
←  ☐ Imágenes  ☐ Extras  ✔ Submit  →

│ ¿Qué imágenes debe mostrar el chat?

❯ 1. Solo dentro del proyecto
     Archivos de imagen dentro de la carpeta de la sesión.
  2. Cualquier ruta de imagen
     Cualquier archivo .png que el agente nombre,
     esté donde esté.
  3. Solo imágenes de internet
     Solo enlaces http(s).
  4. Type something.
────────────────────────────────────────────────────────────
  5. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel
";

    /// Its multiple choice, cursor on «Submit».
    const MULTI: &str = "\
←  ☒ Imágenes  ☐ Extras  ✔ Submit  →

¿Qué más quieres que se pinte?

  1. [✔] Diagramas Mermaid
         Bloques mermaid dibujados.
  2. [ ] Videos y GIFs
         Archivos reproducibles.
  3. [ ] Type something
❯    Submit
────────────────────────────────────────────────────────────
  4. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel
";

    #[test]
    fn a_question_menu_with_descriptions_a_rule_and_tabs_is_read_whole() {
        let m = parse(QUESTION).expect("menu");
        assert_eq!(m.question, "¿Qué imágenes debe mostrar el chat?");
        assert_eq!(m.tabs, vec![("Imágenes".to_owned(), false), ("Extras".to_owned(), false)]);
        let labels: Vec<&str> = m.rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Solo dentro del proyecto", "Cualquier ruta de imagen", "Solo imágenes de internet", "Type something.", "Chat about this"]);
        assert_eq!(m.rows[1].desc, ["Cualquier archivo .png que el agente nombre,", "esté donde esté."]);
        assert_eq!(m.rows[3].kind, Kind::Free);
        assert_eq!(m.rows[4].kind, Kind::Chat);
        assert_eq!(m.cursor(), Some(0));
        assert!(!m.multi());
        assert_eq!(moves(&m, 2).unwrap(), ["Down", "Down"]);
        assert_eq!(moves(&m, 0).unwrap(), Vec::<&str>::new());
        assert!(moves(&m, 9).is_none());
    }

    #[test]
    fn a_long_quoted_question_is_read_whole() {
        let screen = "\
❯ borra lo viejo
────────────────────────────────────────
←  ☐ Borrar  ✔ Submit  →

│ ¿Qué borro del proyecto viejo? Lo que se borra no se puede
│ recuperar. Antes de borrar dejo respaldo; el motor se
│ apagaría.

  1. [ ] Despliegues automáticos
         Quito los workflows que fallan.
  2. [ ] Servidor
         El droplet de la web.
❯    Submit
────────────────────────────────────────
  3. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel
";
        let m = parse(screen).expect("menu");
        assert_eq!(
            m.question,
            "¿Qué borro del proyecto viejo? Lo que se borra no se puede recuperar. Antes de borrar dejo respaldo; el motor se apagaría."
        );
        assert!(m.detail.is_empty(), "{:?}", m.detail);
        assert_eq!(m.tabs, vec![("Borrar".to_owned(), false)]);
        // A boxed dialog keeps one line as its question and the rest as context.
        let boxed = "\
╭──────────────────────────────╮
│ Bash command                 │
│   rm -rf build               │
│ Do you want to proceed?      │
│ ❯ 1. Yes                     │
│   2. No                      │
╰──────────────────────────────╯
";
        let m = parse(boxed).expect("boxed");
        assert_eq!(m.question, "Do you want to proceed?");
        assert_eq!(m.detail, vec!["Bash command".to_owned(), "rm -rf build".to_owned()]);
    }

    #[test]
    fn a_menu_the_screen_has_moved_past_is_not_open() {
        let answered = format!("{QUESTION}\n● Listo, solo imágenes de internet.\n  Edité dos archivos.\n  Corrí las pruebas.\n");
        assert_eq!(parse(&answered), None);
        // Its hints, a border and one status line still leave it open.
        let open = format!("{QUESTION}────────\n  ? for shortcuts\n");
        assert!(parse(&open).is_some());
    }

    #[test]
    fn a_multiple_choice_keeps_its_ticks_and_the_submit_row() {
        let m = parse(MULTI).expect("menu");
        assert!(m.multi());
        assert_eq!(m.rows[0].checked, Some(true));
        assert_eq!(m.rows[1].checked, Some(false));
        assert_eq!(m.rows[0].label, "Diagramas Mermaid");
        assert_eq!(m.rows[3].kind, Kind::Submit);
        assert_eq!(m.cursor(), Some(3), "the cursor on the unnumbered row still counts");
        assert_eq!(moves(&m, 1).unwrap(), ["Up", "Up"]);
        // Ticking keeps the same menu: the signature ignores ticks and the cursor.
        let ticked = MULTI.replace("[ ] Videos", "[✔] Videos").replace("❯    Submit", "     Submit").replace("  1. [✔]", "❯ 1. [✔]");
        assert_eq!(signature(&parse(&ticked).unwrap()), signature(&m));
    }

    #[test]
    fn permissions_plans_and_codex_still_read_and_plain_lists_do_not() {
        let perm = "Bash command\n\n  rm -rf build\n\nDo you want to proceed?\n❯ 1. Yes\n  2. Yes, and don't ask again\n  3. No, and tell Claude what to do differently (esc)\n";
        let m = parse(perm).unwrap();
        assert_eq!(m.question, "Do you want to proceed?");
        assert_eq!(m.detail, ["Bash command", "", "rm -rf build"]);
        assert_eq!(m.rows.len(), 3);
        let codex = parse("Allow command?\n› 1. Yes\n  2. No").unwrap();
        assert_eq!(codex.rows[0].label, "Yes");
        assert!(parse("Pasos:\n1. Compilar\n2. Probar\n\n> escribe aquí").is_none(), "no cursor");
        assert!(parse("❯ 2. algo\n  3. otro").is_none(), "not numbered from 1");
        assert!(parse("╭────╮\n│ > hola │\n╰────╯").is_none());
        assert_ne!(signature(&m), signature(&codex));
    }

    #[test]
    fn a_boxed_permission_joins_its_wrapped_labels() {
        let screen = "\
● Voy a subirlo.

╭──────────────────────────────────────────────╮
│ Bash command                                 │
│                                              │
│   git push origin main                       │
│   Push the fix                               │
│                                              │
│ Do you want to proceed?                      │
│ ❯ 1. Yes                                     │
│   2. Yes, and don't ask again for git push   │
│      commands in /srv/demo/repo              │
│   3. No, and tell Claude what to do          │
│   differently (esc)                          │
╰──────────────────────────────────────────────╯
";
        let m = parse(screen).expect("menu");
        assert_eq!(m.question, "Do you want to proceed?");
        assert_eq!(m.detail, ["Bash command", "", "git push origin main", "Push the fix"]);
        let labels: Vec<&str> = m.rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Yes", "Yes, and don't ask again for git push commands in /srv/demo/repo", "No, and tell Claude what to do differently (esc)"]);
        assert!(m.rows.iter().all(|r| r.desc.is_empty()));
    }

    #[test]
    fn the_json_carries_what_every_client_draws() {
        let v = to_json(&parse(QUESTION).unwrap());
        assert_eq!(v["options"][0]["on"], true);
        assert_eq!(v["options"][3]["kind"], "free");
        assert_eq!(v["options"][4]["key"], "5");
        assert_eq!(v["options"][1]["desc"][1], "esté donde esté.");
        assert_eq!(v["multi"], false);
        assert!(v["sig"].as_str().is_some_and(|s| s.len() == 16));
    }
}
