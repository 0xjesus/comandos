//! `observe_pane` (`bin/cc-dash` 6912) sin tmux: la evidencia recogida y el
//! resultado en el orden del Python, más los lectores de conversación que usa
//! (rollout de Codex, transcript de Claude, `GROK_HOME`, `acp-panes.json`).
//!
//! Vivía en `comandos-server` (`states/observe.rs` y `states/gather.rs` de la
//! 2d); se movió aquí sin cambiar su lógica (plan 2f-2, Tarea 2) para que el
//! adaptador síncrono de operaciones de sesión observe el pane igual que GET
//! `/state`. El servidor reexporta estas piezas y convierte `ObserveFault` en
//! su `StateFault`.
use crate::{
    Unsure, agent_procs,
    hooks::py::{float_repr, float_value, int_text},
    providers::engine_for_model,
    tui_state::{Obs, StateTracker, TranscriptCache},
};
use comandos_core::json::{MAX_WORKSPACE_JSON_DEPTH, python_eq, truthy, workspace_loads};
use serde_json::{Map, Number, Value};
use std::{
    ffi::OsStr,
    fs, io,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

/// Lo que el Python haría distinto o no se sabe con certeza.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserveFault {
    /// Incierto: quien llama declina (o, tras efectos, falla cerrado).
    Decline,
    /// Una excepción del Python que su llamador no captura.
    Failure,
}

impl From<Unsure> for ObserveFault {
    fn from(_: Unsure) -> Self {
        ObserveFault::Decline
    }
}

/// Lo que se recoge para `observe_pane`, en su orden.
#[derive(Debug, Clone)]
pub struct ObserveEvidence {
    pub agent: String,
    pub pid: i64,
    /// `_identity_key(identity)`.
    pub identity: String,
    /// `inspector({...})` de `PaneInspector`.
    pub snap: Obs,
    /// `account_for_pid(pid, agent).get('account') or 'unknown'`.
    pub account: String,
    /// `capabilities.accounts` del arnés en el registro.
    pub harness_has_accounts: bool,
    pub cmdline: Vec<String>,
    /// codex: contexto del rollout raíz (`{}` si ninguno trae modelo); claude:
    /// transcript único (`{}` si no); grok/opencode/agy: `project_metadata`.
    /// Solo cuenta para esos arneses (y codex/claude con `conversationId`).
    pub conversation: Obs,
    /// acp: `acp_state_for_pane(pane)`; el filtro de pid y sesión se aplica aquí.
    pub acp: Obs,
    /// `pane_visible_config` (solo codex, claude y opencode lo usan).
    pub visible: Obs,
    /// `_process_start(pid)`.
    pub process_start: String,
    /// `ANTHROPIC_BASE_URL` del `environ` decodificado con `replace` (`""` si falta).
    pub base_url: String,
}

fn integer_number(n: &Number) -> bool {
    let raw = n.as_str();
    !raw.contains(['.', 'e', 'E']) && !matches!(raw, "NaN" | "Infinity" | "-Infinity")
}

/// `str(x)` de un escalar de JSON; contenedores (su `repr`) → declinar.
pub fn py_str(value: &Value) -> Result<String, ObserveFault> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Null => Ok("None".into()),
        Value::Bool(true) => Ok("True".into()),
        Value::Bool(false) => Ok("False".into()),
        Value::Number(n) => match n.as_str() {
            "NaN" => Ok("nan".into()),
            "Infinity" => Ok("inf".into()),
            "-Infinity" => Ok("-inf".into()),
            // `str(int)` con más de 4300 dígitos: `ValueError` en CPython.
            raw if integer_number(n) && raw.len() > 4300 => Err(ObserveFault::Decline),
            _ if integer_number(n) => Ok(int_text(n)),
            raw => raw
                .parse::<f64>()
                .map(float_repr)
                .map_err(|_| ObserveFault::Decline),
        },
        Value::Array(_) | Value::Object(_) => Err(ObserveFault::Decline),
    }
}

/// `str(x or "")`.
fn str_or_empty(value: Option<&Value>) -> Result<String, ObserveFault> {
    match value.filter(|v| truthy(v)) {
        Some(v) => py_str(v),
        None => Ok(String::new()),
    }
}

/// `x or default` sobre una clave de un `dict`.
fn or_default(value: Option<&Value>, default: &str) -> Value {
    value
        .filter(|v| truthy(v))
        .cloned()
        .unwrap_or_else(|| Value::from(default))
}

/// `x == "texto"` de Python.
fn is_text(value: Option<&Value>, text: &str) -> bool {
    value.and_then(Value::as_str) == Some(text)
}

/// `snap.get('resume_id') or (snap.get('acp') or {}).get('sessionId') or ''`.
/// Un `acp` verdadero que no es objeto es el `AttributeError` del Python, que
/// `reconcile_card_config` no captura.
pub fn conversation_id(snap: &Obs) -> Result<Value, ObserveFault> {
    if let Some(id) = snap.get("resume_id").filter(|v| truthy(v)) {
        return Ok(id.clone());
    }
    let session = match snap.get("acp").filter(|v| truthy(v)) {
        None => None,
        Some(Value::Object(acp)) => acp.get("sessionId"),
        Some(_) => return Err(ObserveFault::Failure),
    };
    Ok(or_default(session, ""))
}

/// `dict(model=…, effort=…, revision=metadata.get(revision) or '')` si el
/// `sessionId` de los metadatos es la conversación del pane; si no, `{}`.
pub fn project_metadata(metadata: &Obs, conversation_id: &Value, revision: &str) -> Obs {
    let mut out = Map::new();
    if python_eq(
        metadata.get("sessionId").unwrap_or(&Value::Null),
        conversation_id,
    ) {
        out.insert("model".into(), or_default(metadata.get("model"), ""));
        out.insert("effort".into(), or_default(metadata.get("effort"), ""));
        out.insert("revision".into(), or_default(metadata.get(revision), ""));
    }
    out
}

/// `re.fullmatch(r'model_reasoning_effort=["\']?([a-z]+)["\']?', value)`.
fn reasoning_effort(value: &str) -> Option<&str> {
    let rest = value.strip_prefix("model_reasoning_effort=")?;
    let rest = rest.strip_prefix(['"', '\'']).unwrap_or(rest);
    let end = rest
        .bytes()
        .position(|b| !b.is_ascii_lowercase())
        .unwrap_or(rest.len());
    let (word, tail) = rest.split_at(end);
    (!word.is_empty() && matches!(tail, "" | "\"" | "'")).then_some(word)
}

/// `(model, effort)` pedidos en la línea de órdenes del proceso.
fn launch_args(args: &[String]) -> (String, String) {
    let (mut model, mut effort) = (String::new(), String::new());
    for pair in args.windows(2) {
        let [arg, value] = pair else { continue };
        if value.starts_with('-') {
            continue;
        }
        match arg.as_str() {
            "--model" | "-m" => model.clone_from(value),
            "--effort" => effort.clone_from(value),
            "-c" | "--config" => {
                if let Some(word) = reasoning_effort(value) {
                    effort = word.to_owned();
                }
            }
            _ => {}
        }
    }
    (model, effort)
}

fn text(s: &str) -> Value {
    Value::from(s)
}

/// `observe_pane` desde `result = {...}` hasta el final.
pub fn observe(
    evidence: &ObserveEvidence,
    tracker: &mut StateTracker,
    registry: &Value,
    now: f64,
) -> Result<Obs, ObserveFault> {
    let agent = evidence.agent.as_str();
    let conversation_id = conversation_id(&evidence.snap)?;
    let mut result = Map::new();
    result.insert("harness".into(), text(agent));
    result.insert("pid".into(), Value::from(evidence.pid));
    result.insert("conversationId".into(), conversation_id.clone());
    result.insert("model".into(), text(""));
    result.insert("effort".into(), text(""));
    result.insert("source".into(), text("unconfirmed"));
    result.insert("observedAt".into(), float_value(now));
    result.insert("identity".into(), text(&evidence.identity));
    let account = if evidence.harness_has_accounts {
        evidence.account.clone()
    } else {
        "main".to_owned()
    };
    result.insert("harnessAccount".into(), text(&account));
    let (mut launch_model, mut launch_effort) = launch_args(&evidence.cmdline);
    let has_conversation = truthy(&conversation_id);
    let conversation = match agent {
        "codex" | "claude" if has_conversation => evidence.conversation.clone(),
        "grok" | "opencode" | "agy" | "omp" => evidence.conversation.clone(),
        "acp" => {
            // Los argumentos son peticiones: solo el protocolo del mismo proceso confirma.
            launch_model.clear();
            launch_effort.clear();
            let empty = Map::new();
            let same = python_eq(
                evidence.acp.get("pid").unwrap_or(&Value::Null),
                &Value::from(evidence.pid),
            ) && python_eq(
                evidence.acp.get("sessionId").unwrap_or(&Value::Null),
                &conversation_id,
            );
            let metadata = if same { &evidence.acp } else { &empty };
            let mut conversation = Map::new();
            conversation.insert(
                "model".into(),
                or_default(metadata.get("observedModel"), ""),
            );
            conversation.insert(
                "effort".into(),
                or_default(metadata.get("observedEffort"), ""),
            );
            let requested = |primary: &str, fallback: &str| {
                metadata
                    .get(primary)
                    .filter(|v| truthy(v))
                    .or_else(|| metadata.get(fallback).filter(|v| truthy(v)))
                    .cloned()
                    .unwrap_or_else(|| text(""))
            };
            result.insert("motor".into(), or_default(metadata.get("agent"), ""));
            result.insert(
                "motorAccount".into(),
                or_default(metadata.get("account"), "unknown"),
            );
            result.insert("harnessAccount".into(), text("main"));
            result.insert(
                "effortSource".into(),
                or_default(metadata.get("effortSource"), "unconfirmed"),
            );
            result.insert(
                "requestedModel".into(),
                requested("requestedModel", "model"),
            );
            result.insert(
                "requestedEffort".into(),
                requested("requestedEffort", "effort"),
            );
            conversation
        }
        _ => Map::new(),
    };
    let visible = if matches!(agent, "codex" | "claude" | "opencode") {
        evidence.visible.clone()
    } else {
        Map::new()
    };
    let mut launch = Map::new();
    launch.insert("model".into(), text(&launch_model));
    launch.insert("effort".into(), text(&launch_effort));
    // La clave del Python es la tupla (identidad, agente, pid, inicio, conversación).
    let key = format!(
        "{}\u{1f}{agent}\u{1f}{}\u{1f}{}\u{1f}{conversation_id}",
        evidence.identity, evidence.pid, evidence.process_start
    );
    for (field, value) in tracker.observe(&key, &launch, &conversation, &visible, now) {
        result.insert(field, value);
    }
    let motor = match result.get("motor").filter(|v| truthy(v)) {
        Some(motor) => motor.clone(),
        None if matches!(agent, "opencode" | "agy" | "omp" | "gemini") => text(agent),
        None => {
            let model = str_or_empty(result.get("model"))?;
            let engine = engine_for_model(registry, &model)?;
            text(if engine.is_empty() { agent } else { &engine })
        }
    };
    result.insert("motor".into(), motor.clone());
    let motor_account = match result.get("motorAccount").filter(|v| truthy(v)) {
        Some(existing) => existing.clone(),
        None if python_eq(&motor, &text(agent)) => text(&account),
        None => text("unknown"),
    };
    result.insert("motorAccount".into(), motor_account);
    if agent == "claude"
        && (is_text(Some(&motor), "codex") || is_text(Some(&motor), "grok"))
        && (evidence.base_url.starts_with("http://127.0.0.1:")
            || evidence.base_url.starts_with("http://localhost:"))
    {
        result.insert("motorAccount".into(), text("main"));
    }
    let model_set = result.get("model").is_some_and(truthy);
    let source = result.get("source");
    let confirmed = model_set
        && !["process", "unconfirmed", "status-script"]
            .iter()
            .any(|s| is_text(source, s));
    result.insert("confirmed".into(), Value::Bool(confirmed));
    result.insert(
        "accountSource".into(),
        text(if agent == "acp" {
            "acp-state"
        } else {
            "process-environment"
        }),
    );
    let mut limitations = Vec::new();
    if matches!(agent, "opencode" | "agy" | "omp" | "gemini") && visible.is_empty() {
        limitations.push(text("native-runtime-state-unavailable"));
    }
    if !result.get("effort").is_some_and(truthy) {
        limitations.push(text("effort-unobserved"));
    }
    result.insert("limitations".into(), Value::Array(limitations));
    Ok(result)
}

// ---------------------------------------------------------------- lectores

/// `grok_metadata_for_pid` (1495): `GROK_HOME` del `environ` (o `~/.grok`),
/// `expanduser` con el `HOME` del proceso y `realpath`.
pub fn grok_home_for(proc_root: &Path, home: &Path, pid: i64) -> Result<PathBuf, ObserveFault> {
    let env = agent_procs::read_environ(proc_root, pid);
    let raw = env
        .get(b"GROK_HOME".as_slice())
        .map(|v| String::from_utf8_lossy(v).into_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "~/.grok".to_owned());
    let expanded = expanduser(&raw, home)?;
    Ok(PathBuf::from(OsStr::from_bytes(&agent_procs::realpath(
        expanded.as_bytes(),
    ))))
}

/// `os.path.expanduser` con `HOME` = `home`; `~usuario` no se reproduce.
pub fn expanduser(raw: &str, home: &Path) -> Result<String, ObserveFault> {
    if raw != "~" && !raw.starts_with("~/") {
        if raw.starts_with('~') {
            return Err(ObserveFault::Decline);
        }
        return Ok(raw.to_owned());
    }
    let home = home.to_str().ok_or(ObserveFault::Decline)?;
    let tail = raw.get(1..).unwrap_or("");
    let joined = format!("{}{tail}", home.trim_end_matches('/'));
    Ok(if joined.is_empty() {
        "/".into()
    } else {
        joined
    })
}

/// `glob.has_magic`.
pub fn has_magic(text: &str) -> bool {
    text.contains(['*', '?', '['])
}

/// `json.load(open(path))` clasificado como lo ve el Python (lo mismo que
/// `files::read_json_strict` del servidor).
enum Strict {
    Missing,
    Unreadable,
    Unsure,
    Value(Value),
}

fn read_json_strict(path: &Path) -> Strict {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Strict::Missing,
        Err(_) => return Strict::Unsure,
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Strict::Unsure;
    };
    if text.starts_with('\u{feff}') {
        return Strict::Unreadable;
    }
    match workspace_loads(text) {
        Ok(value) => Strict::Value(value),
        Err(_) if surrogate_escape(text) || deep(text) => Strict::Unsure,
        Err(_) => Strict::Unreadable,
    }
}

fn surrogate_escape(text: &str) -> bool {
    text.match_indices("\\u").any(|(i, _)| {
        let hex = text.as_bytes().get(i + 2..i + 4).unwrap_or_default();
        matches!(hex, [b'd' | b'D', b'8'..=b'9' | b'a'..=b'f' | b'A'..=b'F'])
    })
}

fn deep(text: &str) -> bool {
    text.bytes().filter(|b| matches!(b, b'[' | b'{')).count() >= MAX_WORKSPACE_JSON_DEPTH
}

/// `acp_state_for_pane` (1904): `{}` ante cualquier excepción; un valor
/// verdadero que no es objeto llega a `observe_pane` y su `.get` lanza.
pub fn acp_state(hooks: &Path, pane: &str) -> Result<Obs, ObserveFault> {
    match read_json_strict(&hooks.join("acp-panes.json")) {
        Strict::Unsure => Err(ObserveFault::Decline),
        Strict::Missing | Strict::Unreadable => Ok(Map::new()),
        Strict::Value(Value::Object(data)) => match data.get(pane).filter(|v| truthy(v)) {
            None => Ok(Map::new()),
            Some(Value::Object(state)) => Ok(state.clone()),
            Some(_) => Err(ObserveFault::Failure),
        },
        Strict::Value(_) => Ok(Map::new()),
    }
}

/// El rollout raíz que tiene abierto ESTE proceso: `fd` de `/proc/<pid>/fd/*`
/// que acaban en `<id>.jsonl`, en el orden de `read_dir`, hasta uno con modelo.
pub fn codex_conversation(
    transcripts: &mut TranscriptCache,
    proc_root: &Path,
    pid: i64,
    id: &str,
) -> Result<Obs, ObserveFault> {
    let mut conversation = Map::new();
    let suffix = format!("{id}.jsonl");
    let Ok(listing) = fs::read_dir(proc_root.join(pid.to_string()).join("fd")) else {
        return Ok(conversation);
    };
    for entry in listing.flatten() {
        if entry.file_name().as_bytes().first() == Some(&b'.') {
            continue;
        }
        let fd = entry.path();
        let Ok(target) = fs::read_link(&fd) else {
            continue;
        };
        if !target.as_os_str().as_bytes().ends_with(suffix.as_bytes()) {
            continue;
        }
        let context = transcripts.read("codex", id, &fd)?;
        if context.get("model").is_some_and(truthy) {
            conversation = context;
        }
        if conversation.get("model").is_some_and(truthy) {
            break;
        }
    }
    Ok(conversation)
}

/// `glob(<config>/projects/*/<id>.jsonl)`: exactamente una → su transcript.
pub fn claude_conversation(
    transcripts: &mut TranscriptCache,
    root: &Path,
    id: &str,
) -> Result<Obs, ObserveFault> {
    let root_text = root.to_str().ok_or(ObserveFault::Decline)?;
    // Comodines en la ruta o el id harían otro patrón; un `/` en el id, otra ruta.
    if has_magic(root_text) || has_magic(id) || id.contains('/') {
        return Err(ObserveFault::Decline);
    }
    let projects = root.join("projects");
    let name = format!("{id}.jsonl");
    let mut found = Vec::new();
    if let Ok(listing) = fs::read_dir(&projects) {
        for entry in listing.flatten() {
            if entry.file_name().as_bytes().first() == Some(&b'.') {
                continue;
            }
            // `_iterdir(..., dironly=True)`: `entry.is_dir()` sigue enlaces.
            let dir = entry.path();
            if !fs::metadata(&dir).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            let candidate = dir.join(&name);
            // `_glob0`: `os.path.lexists`.
            if fs::symlink_metadata(&candidate).is_ok() {
                found.push(candidate);
            }
        }
    }
    match found.as_slice() {
        [only] => Ok(transcripts.read("claude", id, only)?),
        _ => Ok(Map::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_effort_fullmatch() {
        assert_eq!(
            reasoning_effort("model_reasoning_effort=high"),
            Some("high")
        );
        assert_eq!(
            reasoning_effort("model_reasoning_effort=\"low\""),
            Some("low")
        );
        assert_eq!(
            reasoning_effort("model_reasoning_effort='max\""),
            Some("max")
        );
        assert_eq!(reasoning_effort("model_reasoning_effort=x'"), Some("x"));
        assert_eq!(reasoning_effort("model_reasoning_effort=\""), None);
        assert_eq!(reasoning_effort("model_reasoning_effort=High"), None);
        assert_eq!(reasoning_effort("model_reasoning_effort=a''"), None);
    }

    #[test]
    fn expanduser_like_python() {
        let home = Path::new("/home/u/");
        assert_eq!(expanduser("~/.grok", home).unwrap(), "/home/u/.grok");
        assert_eq!(expanduser("~", home).unwrap(), "/home/u");
        assert_eq!(expanduser("/x/y", home).unwrap(), "/x/y");
        assert_eq!(expanduser("rel", home).unwrap(), "rel");
        assert_eq!(expanduser("~otro/x", home), Err(ObserveFault::Decline));
        assert_eq!(expanduser("~/x", Path::new("/")).unwrap(), "/x");
    }
}
