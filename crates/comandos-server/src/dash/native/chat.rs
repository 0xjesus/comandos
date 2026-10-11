//! Chat view of a session: the last turns of its Claude/Codex transcript. Read-only,
//! bounded tail, and cheap to poll: an unchanged transcript answers `unchanged`.
use super::{Answer, Entry, Key, Native, NativeRoute, Verb, reply};
use crate::Request;
use comandos_runtime::{
    pane_snapshot::{PaneInspector, PaneRef},
    session_configuration,
};
use http::StatusCode;
use comandos_web_view::chat_markdown::render_message;
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

pub async fn answer(native: &Arc<Native>, request: &Request) -> Answer {
    let Some(body) = request.data.as_ref().and_then(Value::as_object) else {
        return error(StatusCode::BAD_REQUEST, "Expected chat request");
    };
    let session = body.get("session").and_then(Value::as_str).unwrap_or("");
    if session.is_empty() || session.len() > 200 || session.chars().any(char::is_control) {
        return error(StatusCode::BAD_REQUEST, "Invalid session");
    }
    let since = body.get("since").and_then(Value::as_str).unwrap_or("").to_owned();
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
    let home = native.options().home.clone();
    let proc_root = native.options().proc_root.clone();
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
            return json!({"ok":true,"agent":null,"messages":[]});
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
            .map(|(r, t)| json!({"r":r,"h":render_message(r, &t),"t":t}))
            .collect();
        json!({"ok":true,"agent":agent,"pane":pane,"token":token,"age":age,"messages":messages})
    })
    .await
    .map_err(|_| super::Fault::Error(crate::HandlerError::Failure))?;
    reply(StatusCode::OK, &result)
}

#[cfg(test)]
mod tests {
    use super::parse;
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
