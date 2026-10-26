//! Chat view of a session: the last turns of its Claude/Codex transcript. Read-only,
//! bounded tail, and cheap to poll: an unchanged transcript answers `unchanged`.
use super::{Answer, Entry, Key, Native, NativeRoute, Verb, reply};
use crate::Request;
use comandos_runtime::{
    pane_snapshot::{PaneInspector, PaneRef},
    session_configuration,
};
use http::StatusCode;
use comandos_core::chat_markdown::{escape, to_html};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

pub const ROUTES: &[Entry] = &[Entry {
    verb: Verb::Post,
    key: Key::Path("/chat/transcript"),
    route: NativeRoute::Chat,
}];
const TAIL: u64 = 1536 * 1024;
const MAX_MESSAGES: usize = 80;
const MAX_TEXT: usize = 6000;
/// Inspecting processes is the expensive part; a pane keeps its transcript while its pid does.
const PATH_TTL: Duration = Duration::from_secs(20);

type PathCache = Mutex<HashMap<String, (Instant, String, String, String)>>;
fn paths() -> &'static PathCache {
    static CACHE: OnceLock<PathCache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}
fn gate() -> Arc<tokio::sync::Semaphore> {
    static GATE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
}
fn error(status: StatusCode, message: &str) -> Answer {
    reply(status, &json!({"error":message}))
}

fn clip(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(MAX_TEXT) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_owned(),
    }
}
/// Injected context, slash-command plumbing and reminders are not conversation.
fn noise(text: &str) -> bool {
    let t = text.trim_start();
    t.is_empty()
        || t.starts_with('<')
        || t.starts_with("Caveat:")
        || t.starts_with("[Request interrupted")
}
fn tool_line(name: &str, input: &Value) -> String {
    let input = match input {
        Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
        v => v.clone(),
    };
    let pick = ["command", "file_path", "path", "pattern", "query", "url", "description", "cmd"]
        .iter()
        .find_map(|k| match &input[*k] {
            Value::String(s) => Some(s.clone()),
            Value::Array(a) => Some(
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        })
        .unwrap_or_default();
    let pick: String = pick.lines().next().unwrap_or("").chars().take(160).collect();
    if pick.is_empty() { name.to_owned() } else { format!("{name} · {pick}") }
}

/// Turns transcript records (Claude Code or Codex rollout) into `(role, text)` pairs.
pub fn parse(records: &str) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let mut push = |role: &'static str, text: String| {
        if role == "t"
            && let Some(last) = out.last_mut()
            && last.0 == "t"
        {
            last.1.push('\n');
            last.1.push_str(&text);
            return;
        }
        out.push((role, text));
    };
    for row in records.lines() {
        let Ok(record) = serde_json::from_str::<Value>(row) else {
            continue;
        };
        // Compaction summaries are context for the model, not something the person said.
        if record["isSidechain"] == true || record["isMeta"] == true || record["isCompactSummary"] == true {
            continue;
        }
        if record["type"] == "response_item" {
            let p = &record["payload"];
            match p["type"].as_str() {
                Some("message") => {
                    let role = match p["role"].as_str() {
                        Some("user") => "u",
                        Some("assistant") => "a",
                        _ => continue,
                    };
                    let text: String = p["content"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|c| c["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !noise(&text) {
                        push(role, clip(&text));
                    }
                }
                Some("function_call" | "custom_tool_call" | "local_shell_call") => {
                    let name = p["name"].as_str().unwrap_or("tool");
                    let args = if p["arguments"].is_null() { &p["input"] } else { &p["arguments"] };
                    push("t", tool_line(name, args));
                }
                _ => {}
            }
            continue;
        }
        let role = match record["type"].as_str() {
            Some("user") => "u",
            Some("assistant") => "a",
            _ => continue,
        };
        match &record["message"]["content"] {
            Value::String(text) if !noise(text) => push(role, clip(text)),
            Value::Array(parts) => {
                for part in parts {
                    match part["type"].as_str() {
                        Some("text") => {
                            if let Some(text) = part["text"].as_str().filter(|t| !noise(t)) {
                                push(role, clip(text));
                            }
                        }
                        Some("tool_use") => push(
                            "t",
                            tool_line(part["name"].as_str().unwrap_or("tool"), &part["input"]),
                        ),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let skip = out.len().saturating_sub(MAX_MESSAGES);
    out.into_iter().skip(skip).collect()
}

fn read_tail(path: &str) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(TAIL).read_to_end(&mut bytes).ok()?;
    let slice = if start > 0 {
        &bytes[bytes.iter().position(|b| *b == b'\n')? + 1..]
    } else {
        &bytes
    };
    Some(String::from_utf8_lossy(slice).into_owned())
}

/// One message as the phone draws it. Agent answers and prompts carry their markdown as
/// HTML in `.cv-body` and big buttons below (labels come from the CSS, so copying the
/// bubble's text never picks them up); tool calls are slim lines.
pub fn bubble(role: &str, text: &str) -> String {
    const COPY: &str = "<button type=\"button\" class=\"cv-act\" data-cv-act=\"copy\" aria-label=\"Copiar\"></button>";
    match role {
        "a" => format!(
            "<div class=\"cv-ai\"><div class=\"cv-body\">{}</div><div class=\"cv-acts\" contenteditable=\"false\">{COPY}<button type=\"button\" class=\"cv-act\" data-cv-act=\"edit\" aria-label=\"Editar para copiar\"></button><button type=\"button\" class=\"cv-act\" data-cv-act=\"done\" aria-label=\"Listo\"></button><button type=\"button\" class=\"cv-act\" data-cv-act=\"undo\" aria-label=\"Deshacer cambios\"></button></div></div>",
            to_html(text)
        ),
        "u" => format!("<div class=\"cv-me\"><div class=\"cv-body\">{}</div><div class=\"cv-acts\">{COPY}</div></div>", to_html(text)),
        _ => {
            let lines: String = text.lines().map(|l| format!("<span>{}</span>", escape(l))).collect();
            format!("<div class=\"cv-tool\">{lines}</div>")
        }
    }
}

/// Box drawing and the cursor marks a terminal dialog uses, trimmed off a screen line.
fn screen_line(line: &str) -> &str {
    line.trim_matches(|c: char| c.is_whitespace() || matches!(c, '│' | '┃' | '║' | '|'))
}

/// `❯ 1. Yes` → (cursor here, "1", "Yes"); `2) No` → (false, "2", "No").
fn option_line(line: &str) -> Option<(bool, String, String)> {
    let mut rest = screen_line(line);
    let cursor = ['❯', '›', '>', '▶', '→'].iter().any(|m| rest.starts_with(*m));
    if cursor {
        rest = rest[rest.chars().next()?.len_utf8()..].trim_start();
    }
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

fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.chars().any(|c| matches!(c, '─' | '━' | '═' | '-'))
        && t.chars().all(|c| matches!(c, '─' | '━' | '═' | '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘' | '-' | '│' | ' '))
}

/// The choice an agent is waiting on (a permission, a plan, a question), read from its
/// screen: the last numbered list that has the `❯` cursor, numbered from 1, the line
/// asking above it and up to eight lines of detail above that. `None` when nothing waits.
pub fn ask(screen: &str) -> Option<Value> {
    let lines: Vec<&str> = screen.lines().collect();
    let last = lines.iter().rposition(|l| option_line(l).is_some())?;
    // Walk up to the first option; wrapped labels (indented, not options) belong to the one above.
    let mut first = last;
    let mut i = last;
    while i > 0 {
        i -= 1;
        let line = screen_line(lines[i]);
        if option_line(lines[i]).is_some() {
            first = i;
        } else if line.is_empty() || is_rule(lines[i]) || first - i > 2 {
            break;
        }
    }
    // The last label may wrap too: take its indented lines down to a gap or a border.
    let mut end = last;
    while end + 1 < lines.len() && !screen_line(lines[end + 1]).is_empty() && !is_rule(lines[end + 1]) && end - last < 3 {
        end += 1;
    }
    let mut options: Vec<(bool, String, String)> = Vec::new();
    for line in &lines[first..=end] {
        match option_line(line) {
            Some(o) => options.push(o),
            None => {
                if let Some(prev) = options.last_mut() {
                    let more = screen_line(line);
                    if !more.is_empty() {
                        prev.2.push(' ');
                        prev.2.push_str(more);
                    }
                }
            }
        }
    }
    let numbered = options.iter().enumerate().all(|(n, o)| o.1 == (n + 1).to_string());
    if options.len() < 2 || !numbered || !options.iter().any(|o| o.0) {
        return None;
    }
    let mut above = lines[..first].iter().rev().map(|l| screen_line(l)).skip_while(|l| l.is_empty());
    let question = above.next().filter(|q| !is_rule(q)).unwrap_or("").to_owned();
    let mut detail: Vec<String> = Vec::new();
    let start = lines[..first].iter().rposition(|l| screen_line(l) == question).unwrap_or(first);
    for line in lines[..start].iter().rev() {
        if is_rule(line) || detail.len() >= 8 {
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
    Some(json!({
        "question": question,
        "detail": detail,
        "options": options.iter().map(|(on, key, label)| json!({"key": key, "label": label, "on": on})).collect::<Vec<_>>(),
    }))
}

pub async fn answer(native: &Arc<Native>, request: &Request) -> Answer {
    let Some(body) = request.data.as_ref().and_then(Value::as_object) else {
        return error(StatusCode::BAD_REQUEST, "Expected chat request");
    };
    let session = body.get("session").and_then(Value::as_str).unwrap_or("");
    if session.is_empty() || session.len() > 200 || session.chars().any(char::is_control) {
        return error(StatusCode::BAD_REQUEST, "Invalid session");
    }
    let since = body.get("since").and_then(Value::as_str).unwrap_or("").to_owned();
    // The pane the phone chose among a split; empty or gone falls back to the old order.
    let wanted = body.get("pane").and_then(Value::as_str).unwrap_or("").to_owned();
    let Ok(_permit) = gate().try_acquire_owned() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "Chat busy");
    };
    // Every pane is listed so tmux's fuzzy target matching cannot pick another session.
    let rows = match native
        .options()
        .tmux
        .run(&["list-panes", "-a", "-F", "#{session_name}\t#{pane_id}\t#{window_active}\t#{pane_active}\t#{pane_pid}\t#{pane_current_command}"])
        .await
    {
        Ok(v) if v.ok => v.stdout,
        _ => return error(StatusCode::BAD_REQUEST, "Unable to list panes"),
    };
    let mut panes: Vec<(bool, String, String, String)> = rows
        .lines()
        .filter_map(|row| {
            let c: Vec<_> = row.splitn(6, '\t').collect();
            (c.len() == 6 && c[0] == session)
                .then(|| (c[2] == "1" && c[3] == "1", c[1].into(), c[4].into(), c[5].into()))
        })
        .collect();
    if panes.is_empty() {
        return error(StatusCode::NOT_FOUND, "Session not found");
    }
    panes.sort_by_key(|p| !p.0);
    let proc_root = native.options().proc_root.clone();
    // Restored panes run `sh -lc "claude …"`: name what really runs there.
    for pane in &mut panes {
        if let Ok(pid) = pane.2.parse() {
            pane.3 = comandos_runtime::agent_procs::foreground_command(&proc_root, pid, &pane.3);
        }
    }
    if panes.iter().any(|p| p.1 == wanted) {
        panes.retain(|p| p.1 == wanted);
    }
    let home = native.options().home.clone();
    let result = tokio::task::spawn_blocking(move || -> Value {
        let now = Instant::now();
        let mut found = None;
        {
            let cache = paths().lock().unwrap_or_else(|e| e.into_inner());
            for (_, pane, pid, _) in &panes {
                if let Some((at, cpid, path, agent)) = cache.get(pane)
                    && cpid == pid
                    && at.elapsed() < PATH_TTL
                {
                    found = Some((pane.clone(), path.clone(), agent.clone()));
                    break;
                }
            }
        }
        if found.is_none()
            && let Ok(inspector) = PaneInspector::new(&home, &proc_root)
        {
            for (_, pane, pid, command) in &panes {
                let Ok(pid_n) = pid.parse() else { continue };
                let Ok(snapshot) = inspector.inspect(&PaneRef { id: pane, pid: pid_n, command })
                else {
                    continue;
                };
                let agent = snapshot.get("agent").and_then(Value::as_str).unwrap_or("");
                if !matches!(agent, "claude" | "codex") {
                    continue;
                }
                let Ok(path) = session_configuration::snapshot_transcript_at(&home, &snapshot)
                else {
                    continue;
                };
                if path.is_empty() {
                    continue;
                }
                paths()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(pane.clone(), (now, pid.clone(), path.clone(), agent.to_owned()));
                found = Some((pane.clone(), path, agent.to_owned()));
                break;
            }
        }
        let Some((pane, path, agent)) = found else {
            let pane = (panes.len() == 1).then(|| panes[0].1.clone());
            return json!({"ok":true,"agent":null,"pane":pane,"messages":[]});
        };
        let Ok(meta) = std::fs::metadata(&path) else {
            return json!({"ok":true,"agent":null,"messages":[]});
        };
        let token = format!("{}:{}:{}", meta.ino(), meta.len(), meta.mtime_nsec());
        // Seconds since the transcript last moved; clients show «working» while it is fresh.
        let age = meta.modified().ok().and_then(|m| m.elapsed().ok()).map_or(0, |d| d.as_secs());
        if token == since {
            return json!({"ok":true,"unchanged":true,"token":token,"pane":pane,"age":age});
        }
        let messages: Vec<Value> = read_tail(&path)
            .map(|records| parse(&records))
            .unwrap_or_default()
            .into_iter()
            .map(|(r, t)| json!({"r":r,"h":bubble(r, &t),"t":t}))
            .collect();
        json!({"ok":true,"agent":agent,"pane":pane,"token":token,"age":age,"messages":messages})
    })
    .await
    .map_err(|_| super::Fault::Error(crate::HandlerError::Failure))?;
    let mut result = result;
    // What the agent waits on, from its visible screen (read-only, no time formats).
    if let Some(pane) = result["pane"].as_str().filter(|p| p.starts_with('%')).map(str::to_owned)
        && let Ok(screen) = native.options().tmux.run(&["capture-pane", "-p", "-t", &pane]).await
        && screen.ok
    {
        result["ask"] = ask(&screen.stdout).unwrap_or(Value::Null);
    }
    reply(StatusCode::OK, &result)
}

#[cfg(test)]
mod tests {
    use super::{bubble, parse};
    #[test]
    fn bubbles_carry_markdown_and_their_buttons() {
        let a = bubble("a", "**hola** <b>x</b>");
        assert!(a.starts_with("<div class=\"cv-ai\"><div class=\"cv-body\"><p><strong>hola</strong> &lt;b&gt;x&lt;/b&gt;</p></div><div class=\"cv-acts\""), "{a}");
        for act in ["copy", "edit", "done", "undo"] {
            assert!(a.contains(&format!("data-cv-act=\"{act}\"")), "{act}: {a}");
        }
        assert_eq!(
            bubble("u", "hola"),
            "<div class=\"cv-me\"><div class=\"cv-body\"><p>hola</p></div><div class=\"cv-acts\"><button type=\"button\" class=\"cv-act\" data-cv-act=\"copy\" aria-label=\"Copiar\"></button></div></div>"
        );
        assert_eq!(bubble("t", "Bash · ls <x>\nRead · a.rs"), "<div class=\"cv-tool\"><span>Bash · ls &lt;x&gt;</span><span>Read · a.rs</span></div>");
    }
    #[test]
    fn a_permission_dialog_becomes_a_question_with_its_options() {
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
│      commands in /home/x/repo                │
│   3. No, and tell Claude what to do          │
│   differently (esc)                          │
╰──────────────────────────────────────────────╯
";
        let a = super::ask(screen).expect("ask");
        assert_eq!(a["question"], "Do you want to proceed?");
        assert_eq!(a["detail"], serde_json::json!(["Bash command", "", "git push origin main", "Push the fix"]));
        let options = a["options"].as_array().unwrap();
        assert_eq!(options.len(), 3);
        assert_eq!(options[0], serde_json::json!({"key":"1","label":"Yes","on":true}));
        assert_eq!(options[1]["label"], "Yes, and don't ask again for git push commands in /home/x/repo");
        assert_eq!(options[2]["label"], "No, and tell Claude what to do differently (esc)");
    }

    #[test]
    fn plain_lists_and_prompts_are_not_questions() {
        assert!(super::ask("Pasos:\n1. Compilar\n2. Probar\n\n> escribe aquí").is_none(), "no cursor");
        assert!(super::ask("❯ 2. algo\n  3. otro").is_none(), "not numbered from 1");
        assert!(super::ask("╭────╮\n│ > hola │\n╰────╯").is_none());
        let codex = super::ask("Allow command?\n› 1. Yes\n  2. No").unwrap();
        assert_eq!(codex["question"], "Allow command?");
        assert_eq!(codex["options"][1]["key"], "2");
    }

    #[test]
    fn claude_turns_tools_and_noise() {
        let rows = [
            r#"{"type":"user","message":{"role":"user","content":"revisa las reglas"}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"<command-name>/x</command-name>"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Voy."},{"type":"tool_use","name":"Bash","input":{"command":"ls -la\nmás"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/a.rs"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"sub"}]}}"#,
            r#"{"type":"user","isCompactSummary":true,"message":{"content":"This session is being continued"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Listo."}]}}"#,
        ]
        .join("\n");
        let got = parse(&rows);
        assert_eq!(
            got,
            vec![
                ("u", "revisa las reglas".into()),
                ("a", "Voy.".into()),
                ("t", "Bash · ls -la\nRead · /a.rs".into()),
                ("a", "Listo.".into()),
            ]
        );
    }
    #[test]
    fn codex_rollout_messages_and_calls() {
        let rows = [
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>x</environment_context>"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hola"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"cargo test\"]}"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hecho"}]}}"#,
        ]
        .join("\n");
        assert_eq!(
            parse(&rows),
            vec![
                ("u", "hola".into()),
                ("t", "shell · bash -lc cargo test".into()),
                ("a", "Hecho".into()),
            ]
        );
    }
}
