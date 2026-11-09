//! Lo último que el usuario le escribió al agente, leído de la cola de su
//! transcript exacto (Claude: `projects/*/<sesión>.jsonl` del config-dir de su
//! cuenta; Codex: el rollout). Nunca se lee el archivo entero: una ventana del
//! final que crece solo si hace falta, y después solo lo que se añadió.
use comandos_runtime::session_configuration;
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Tope de `prompt`.
pub const MAX_PROMPT: usize = 400;
/// Ventanas sucesivas desde el final del archivo.
const WINDOWS: [u64; 4] = [256 * 1024, 1024 * 1024, 4 * 1024 * 1024, 8 * 1024 * 1024];
/// Lo añadido que se lee incrementalmente; más que esto, se vuelve a la ventana.
const MAX_APPEND: u64 = 4 * 1024 * 1024;
/// Entradas de cada caché antes de vaciarla.
const MAX_ENTRIES: usize = 512;
/// Un transcript que no se encontró se vuelve a buscar pasado este plazo.
const MISS_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Claude,
    Codex,
}

impl Kind {
    pub fn of(agent: &str) -> Option<Kind> {
        match agent {
            "claude" => Some(Kind::Claude),
            "codex" => Some(Kind::Codex),
            _ => None,
        }
    }
}

/// Espacios (y controles, como el `\x1f` de las opciones) colapsados.
pub fn collapse(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Contexto inyectado que no escribió la persona.
const CLAUDE_WRAPPERS: [&str; 8] = [
    "<command-",
    "<local-command-",
    "<system-reminder>",
    "<bash-",
    "<task-notification>",
    "<user-prompt-submit-hook>",
    "Caveat:",
    "[Request interrupted",
];

/// Quita los bloques `<system-reminder>…</system-reminder>` de un texto.
fn without_reminders(text: &str) -> String {
    const OPEN: &str = "<system-reminder>";
    const CLOSE: &str = "</system-reminder>";
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        match rest[start..].find(CLOSE) {
            Some(end) => rest = &rest[start + end + CLOSE.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn texts(parts: &Value, kinds: &[&str]) -> Vec<String> {
    parts
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["type"].as_str().is_some_and(|t| kinds.contains(&t)))
        .filter_map(|p| p["text"].as_str().map(str::to_owned))
        .collect()
}

fn finish(parts: Vec<String>, noise: impl Fn(&str) -> bool) -> Option<String> {
    let kept: Vec<String> = parts
        .into_iter()
        .map(|p| without_reminders(&p))
        .filter(|p| {
            let t = p.trim_start();
            !t.is_empty() && !noise(t)
        })
        .collect();
    let text = collapse(&kept.join("\n"));
    (!text.is_empty()).then_some(text)
}

/// El prompt de un registro de Claude Code, si lo es: `type:"user"` con texto
/// propio, sin `tool_result`, `isMeta`, subagentes ni envoltorios de comandos.
pub fn claude_prompt(record: &Value) -> Option<String> {
    if record["type"] != "user"
        || record["isMeta"] == true
        || record["isSidechain"] == true
        || record["isCompactSummary"] == true
        || record["isVisibleInTranscriptOnly"] == true
        || !record["toolUseResult"].is_null()
    {
        return None;
    }
    let parts = match &record["message"]["content"] {
        Value::String(text) => vec![text.clone()],
        Value::Array(items) => {
            if items.iter().any(|p| p["type"] == "tool_result") {
                return None;
            }
            texts(&record["message"]["content"], &["text"])
        }
        _ => return None,
    };
    finish(parts, |t| CLAUDE_WRAPPERS.iter().any(|w| t.starts_with(w)))
}

/// El prompt de un registro del rollout de Codex: el evento `user_message`, el
/// `UserMessage` de `item_completed` o el mensaje `user` del modelo, sin el
/// contexto que inyecta Codex (`<environment_context>`, `AGENTS.md`…).
pub fn codex_prompt(record: &Value) -> Option<String> {
    let payload = &record["payload"];
    let parts = match (record["type"].as_str(), payload["type"].as_str()) {
        (Some("event_msg"), Some("user_message")) => {
            vec![payload["message"].as_str()?.to_owned()]
        }
        (Some("event_msg"), Some("item_completed")) if payload["item"]["type"] == "UserMessage" => {
            texts(&payload["item"]["content"], &["text", "input_text"])
        }
        (Some("response_item"), Some("message")) if payload["role"] == "user" => {
            texts(&payload["content"], &["input_text", "text"])
        }
        _ => return None,
    };
    finish(parts, |t| {
        t.starts_with('<') || t.starts_with("# AGENTS.md instructions")
    })
}

/// El último prompt de unas líneas JSONL completas.
pub fn last_prompt(kind: Kind, lines: &str) -> Option<String> {
    lines.lines().rev().find_map(|line| {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let record: Value = serde_json::from_str(line).ok()?;
        match kind {
            Kind::Claude => claude_prompt(&record),
            Kind::Codex => codex_prompt(&record),
        }
    })
}

/// Los bytes `[start, end)` y la posición tras su último `\n`, sin la línea
/// parcial del final ni (si `start > 0`) la cortada del principio.
fn read_lines(file: &mut File, start: u64, end: u64, cut_head: bool) -> Option<(String, u64)> {
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(end - start)
        .read_to_end(&mut bytes)
        .ok()?;
    let last = bytes.iter().rposition(|b| *b == b'\n')?;
    let head = if cut_head {
        bytes.iter().position(|b| *b == b'\n')? + 1
    } else {
        0
    };
    let body = bytes.get(head..=last).unwrap_or_default();
    Some((
        String::from_utf8_lossy(body).into_owned(),
        start + last as u64 + 1,
    ))
}

struct Seen {
    ino: u64,
    len: u64,
    mtime: (i64, i64),
    /// Fin de la última línea completa ya leída.
    scanned: u64,
    prompt: Option<String>,
}

/// Cachés de bloqueo del feed: rutas resueltas y prompts por archivo.
#[derive(Default)]
pub struct Prompts {
    paths: HashMap<(Kind, PathBuf, String), (Instant, Option<PathBuf>)>,
    seen: HashMap<PathBuf, Seen>,
}

/// El directorio del historial de una cuenta: `~/.claude` o
/// `~/.claude-accounts/<alias>` (Codex: `~/.codex` o `$CODEX_HOME`, y
/// `~/.codex-accounts/<alias>`). `None` con un alias inválido.
pub fn account_root(
    home: &Path,
    codex_home: Option<&Path>,
    kind: Kind,
    account: &str,
) -> Option<PathBuf> {
    let name = match kind {
        Kind::Claude => "claude",
        Kind::Codex => "codex",
    };
    if account.is_empty() || account == "main" {
        return Some(match (kind, codex_home) {
            (Kind::Codex, Some(dir)) => dir.to_path_buf(),
            _ => home.join(format!(".{name}")),
        });
    }
    let alias = comandos_runtime::accounts::validate_alias(&Value::from(account)).ok()?;
    Some(home.join(format!(".{name}-accounts")).join(alias))
}

impl Prompts {
    /// La ruta única del transcript de `sid` bajo `root`, o `None`.
    fn path(&mut self, home: &Path, kind: Kind, root: &Path, sid: &str) -> Option<PathBuf> {
        let key = (kind, root.to_path_buf(), sid.to_owned());
        if let Some((at, found)) = self.paths.get(&key) {
            match found {
                Some(path) if path.is_file() => return Some(path.clone()),
                None if at.elapsed() < MISS_TTL => return None,
                _ => {}
            }
        }
        let (agent, root_key) = match kind {
            Kind::Claude => ("claude", "claude_config_dir"),
            Kind::Codex => ("codex", "codex_home"),
        };
        let mut origin = Map::new();
        origin.insert("agent".into(), Value::from(agent));
        origin.insert("resume_id".into(), Value::from(sid));
        origin.insert(
            root_key.into(),
            Value::from(root.to_string_lossy().as_ref()),
        );
        let found = session_configuration::snapshot_transcript_at(home, &origin)
            .ok()
            .filter(|p| !p.is_empty())
            .map(PathBuf::from);
        if self.paths.len() >= MAX_ENTRIES {
            self.paths.clear();
        }
        self.paths.insert(key, (Instant::now(), found.clone()));
        found
    }

    /// El último prompt del transcript de `sid` en la cuenta, `""` si no se
    /// puede saber con certeza.
    pub fn prompt(&mut self, home: &Path, kind: Kind, root: &Path, sid: &str) -> String {
        match self.path(home, kind, root, sid) {
            Some(path) => self.prompt_at(&path, kind).unwrap_or_default(),
            None => String::new(),
        }
    }

    /// El último prompt de `path`; relee solo si cambió su tamaño o `mtime`.
    pub fn prompt_at(&mut self, path: &Path, kind: Kind) -> Option<String> {
        let mut file = File::open(path).ok()?;
        let meta = file.metadata().ok()?;
        let (ino, len, mtime) = (meta.ino(), meta.len(), (meta.mtime(), meta.mtime_nsec()));
        if let Some(seen) = self.seen.get_mut(path)
            && seen.ino == ino
        {
            if seen.len == len && seen.mtime == mtime {
                return seen.prompt.clone();
            }
            // Solo se añadió al final: basta leer lo nuevo.
            if len >= seen.scanned && len - seen.scanned <= MAX_APPEND {
                if let Some((lines, end)) = read_lines(&mut file, seen.scanned, len, false) {
                    if let Some(prompt) = last_prompt(kind, &lines) {
                        seen.prompt = Some(prompt);
                    }
                    seen.scanned = end;
                }
                seen.len = len;
                seen.mtime = mtime;
                return seen.prompt.clone();
            }
        }
        let mut prompt = None;
        let mut scanned = 0;
        for window in WINDOWS {
            let start = len.saturating_sub(window);
            if let Some((lines, end)) = read_lines(&mut file, start, len, start > 0) {
                scanned = end;
                prompt = last_prompt(kind, &lines);
            }
            if prompt.is_some() || start == 0 {
                break;
            }
        }
        if self.seen.len() >= MAX_ENTRIES {
            self.seen.clear();
        }
        self.seen.insert(
            path.to_path_buf(),
            Seen {
                ino,
                len,
                mtime,
                scanned,
                prompt: prompt.clone(),
            },
        );
        prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn claude_prompt_skips_tools_meta_and_wrappers() {
        let rows = [
            r#"{"type":"user","message":{"role":"user","content":"revisa las reglas"}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"pushea   todo\n\n"},{"type":"text","text":"<system-reminder>x</system-reminder>"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Voy."}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"Caveat: local commands"}}"#,
            r#"{"type":"user","message":{"content":"<command-name>/model</command-name>\n<command-args></command-args>"}}"#,
            r#"{"type":"user","message":{"content":"<local-command-stdout>Set model</local-command-stdout>"}}"#,
            r#"{"type":"user","isSidechain":true,"message":{"content":"subagente"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
            r#"{"type":"user","toolUseResult":{"x":1},"message":{"content":"salida"}}"#,
        ]
        .join("\n");
        assert_eq!(
            last_prompt(Kind::Claude, &rows).as_deref(),
            Some("pushea todo")
        );
        assert_eq!(
            claude_prompt(&serde_json::json!({"type":"user","message":{"content":"hola <system-reminder>r</system-reminder> qué tal"}})).as_deref(),
            Some("hola qué tal")
        );
    }

    #[test]
    fn codex_prompt_new_and_old_rollouts() {
        let rows = [
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"viejo formato"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>x</environment_context>"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"como que el enlace\n directo"}]}}"#,
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"text","text":"como que el enlace\n directo","text_elements":[]}]}}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Me refiero a…"}]}}"#,
            r##"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for /x\n\n<INSTRUCTIONS>"}]}}"##,
        ]
        .join("\n");
        assert_eq!(
            last_prompt(Kind::Codex, &rows).as_deref(),
            Some("como que el enlace directo")
        );
        let old = r#"{"type":"event_msg","payload":{"type":"user_message","message":"hola\tmundo","images":[]}}"#;
        assert_eq!(last_prompt(Kind::Codex, old).as_deref(), Some("hola mundo"));
    }

    #[test]
    fn tail_is_read_by_windows_then_incrementally() {
        let dir = std::env::temp_dir().join(format!("cmd-feed-prompt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let mut f = File::create(&path).unwrap();
        writeln!(f, r#"{{"type":"user","message":{{"content":"primero"}}}}"#).unwrap();
        // Más de una ventana de relleno: el prompt queda fuera de la primera.
        let filler = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{}"}}]}}}}"#,
            "x".repeat(1000)
        );
        for _ in 0..400 {
            writeln!(f, "{filler}").unwrap();
        }
        f.flush().unwrap();
        let mut prompts = Prompts::default();
        assert_eq!(
            prompts.prompt_at(&path, Kind::Claude).as_deref(),
            Some("primero")
        );
        // Una línea a medio escribir no cuenta hasta que termina.
        write!(f, r#"{{"type":"user","message":{{"content":"segu"#).unwrap();
        f.flush().unwrap();
        assert_eq!(
            prompts.prompt_at(&path, Kind::Claude).as_deref(),
            Some("primero")
        );
        writeln!(f, r#"ndo"}}}}"#).unwrap();
        writeln!(f, "{filler}").unwrap();
        f.flush().unwrap();
        assert_eq!(
            prompts.prompt_at(&path, Kind::Claude).as_deref(),
            Some("segundo")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn account_roots() {
        let home = Path::new("/h");
        assert_eq!(
            account_root(home, None, Kind::Claude, "main"),
            Some(PathBuf::from("/h/.claude"))
        );
        assert_eq!(
            account_root(home, None, Kind::Claude, "jesus-signara"),
            Some(PathBuf::from("/h/.claude-accounts/jesus-signara"))
        );
        assert_eq!(
            account_root(home, Some(Path::new("/c")), Kind::Codex, ""),
            Some(PathBuf::from("/c"))
        );
        assert_eq!(account_root(home, None, Kind::Claude, "../x"), None);
        assert_eq!(Kind::of("grok"), None);
    }
}
