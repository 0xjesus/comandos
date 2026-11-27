//! Entrada del hook (JSON de Claude Code por stdin o modo adaptador `--agent X
//! --event Y`, que usan codex/gemini/agy) e identidad del pane: proyecto, sesión
//! de tmux, pid del pane y clave del archivo de estado, como en el bash.
use super::jq::jq_r;
use super::text::{any_line, basename, head_c, jq_lossy, strip_nl, tr};
use super::transcript::{alt, field};
use super::which;
use serde_json::Value;
use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn env_bytes(name: &str) -> Vec<u8> {
    std::env::var_os(name)
        .map(|v| v.into_encoded_bytes())
        .unwrap_or_default()
}

pub fn path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(OsStr::from_bytes(bytes))
}

/// Lo que el bash saca de la entrada (stdin o modo adaptador `--agent/--event`).
#[derive(Default)]
pub struct Input {
    pub agent: Vec<u8>,
    pub event: Vec<u8>,
    pub cwd: Vec<u8>,
    pub msg: Vec<u8>,
    pub transcript: Vec<u8>,
    pub full_arg: Vec<u8>,
    pub options_arg: Vec<u8>,
    pub prompt_id: Vec<u8>,
    pub agent_session_id: Vec<u8>,
    pub turn_id: Vec<u8>,
    pub request_id: Vec<u8>,
    pub notification_type: Vec<u8>,
    pub hook_event_arg: Vec<u8>,
    /// Evento de Grok ya normalizado (`grok-hooks.py`): lo que se registra con
    /// `--accept` en `.<clave>.grok`.
    pub grok: Option<Value>,
    /// `export COMANDOS_USAGE_MODEL`/`…_REASONING_EFFORT` del camino de Grok: pisan
    /// las del entorno en la contabilidad de uso.
    pub usage_model: Option<String>,
    pub usage_effort: Option<String>,
}

pub fn adapter_input(args: &[String]) -> Input {
    let mut input = Input {
        agent: b"claude".to_vec(),
        ..Input::default()
    };
    let mut event = Vec::new();
    let mut i = 0;
    while i < args.len() {
        // El bash se cuelga con una bandera sin valor (`shift 2` falla); aquí vale vacío.
        let value = args
            .get(i + 1)
            .map(|v| v.as_bytes().to_vec())
            .unwrap_or_default();
        let slot = match args[i].as_str() {
            "--agent" => &mut input.agent,
            "--event" => &mut event,
            "--cwd" => &mut input.cwd,
            "--msg" => &mut input.msg,
            "--full" => &mut input.full_arg,
            "--options" => &mut input.options_arg,
            "--session-id" => &mut input.agent_session_id,
            "--turn-id" => &mut input.turn_id,
            "--request-id" => &mut input.request_id,
            "--hook-event" => &mut input.hook_event_arg,
            "--notification-type" => &mut input.notification_type,
            _ => {
                i += 1;
                continue;
            }
        };
        *slot = value;
        i += 2;
    }
    input.event = match event.as_slice() {
        b"working" => b"UserPromptSubmit".to_vec(),
        b"waiting" => b"Notification".to_vec(),
        b"end" => b"SessionEnd".to_vec(),
        _ => b"Stop".to_vec(),
    };
    input
}

/// El camino de Grok (`has("hookEventName")`): `grok-hooks.py` normaliza el
/// payload y su `status` elige el evento; lo demás sale `exit 0` (`None`).
fn grok_input(payload: &Value) -> Option<Input> {
    // `python3 grok-hooks.py 2>/dev/null || true`: un `TypeError` deja la salida vacía.
    let normalized = super::grok::normalize(payload).ok().flatten()?;
    let text = |key: &str| {
        normalized
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let event: &[u8] = match text("status").as_str() {
        "working" => b"UserPromptSubmit",
        "done" => b"Stop",
        "waiting" => b"Notification",
        "error" => b"GrokError",
        "idle" => b"GrokIdle",
        "end" => b"SessionEnd",
        _ => return None,
    };
    let cwd = jq_r(alt(
        field(payload, "cwd").unwrap_or(&Value::Null),
        &Value::String(String::new()),
    ));
    let bytes = |key: &str| strip_nl(text(key).as_bytes()).to_vec();
    Some(Input {
        agent: b"grok".to_vec(),
        event: event.to_vec(),
        cwd,
        msg: bytes("detail"),
        prompt_id: bytes("promptId"),
        agent_session_id: bytes("sessionId"),
        // Grok normaliza la cancelación a `idle`: el evento N1 la conserva.
        hook_event_arg: if text("event") == "StopCancelled" {
            b"GrokCancelled".to_vec()
        } else {
            Vec::new()
        },
        usage_model: Some(String::from_utf8_lossy(&bytes("model")).into_owned()),
        usage_effort: Some(String::from_utf8_lossy(&bytes("effort")).into_owned()),
        grok: Some(Value::Object(normalized)),
        ..Input::default()
    })
}

/// `None`: no hay nada que hacer (JSON inválido, no objeto o evento de Grok que no
/// se reenvía).
pub fn stdin_input() -> Option<Input> {
    let mut raw = Vec::new();
    std::io::stdin().read_to_end(&mut raw).ok()?;
    // Desviación deliberada: el bash trata un JSON truncado como `Stop` sin datos y
    // escribe estado de `$PWD`; aquí un payload que no es objeto no deja rastro.
    let payload: Value = serde_json::from_str(&jq_lossy(&raw)).ok()?;
    if !payload.is_object() {
        return None;
    }
    if payload.get("hookEventName").is_some() {
        return grok_input(&payload);
    }
    let empty = Value::String(String::new());
    let get = |keys: &[&str], default: &Value| -> Vec<u8> {
        let mut value = &Value::Null;
        for key in keys {
            value = alt(value, field(&payload, key).unwrap_or(&Value::Null));
        }
        jq_r(alt(value, default))
    };
    let string = |keys: &[&str]| -> Vec<u8> {
        let mut value = &Value::Null;
        for key in keys {
            value = alt(value, field(&payload, key).unwrap_or(&Value::Null));
        }
        value
            .as_str()
            .map(|s| strip_nl(s.as_bytes()).to_vec())
            .unwrap_or_default()
    };
    Some(Input {
        agent: b"claude".to_vec(),
        event: get(&["hook_event_name"], &Value::String("Stop".into())),
        cwd: get(&["cwd"], &empty),
        msg: get(&["message"], &empty),
        transcript: get(&["transcript_path"], &empty),
        prompt_id: get(&["prompt_id", "promptId"], &empty),
        agent_session_id: get(&["session_id", "sessionId"], &empty),
        turn_id: string(&["turn_id", "turnId"]),
        notification_type: string(&["notification_type"]),
        ..Input::default()
    })
}

pub fn agent_name(agent: &[u8]) -> Vec<u8> {
    match agent {
        b"claude" => b"Claude".to_vec(),
        b"codex" => b"Codex".to_vec(),
        b"grok" => b"Grok".to_vec(),
        b"opencode" => b"OpenCode".to_vec(),
        b"gemini" => b"Gemini".to_vec(),
        b"agy" => b"Antigravity".to_vec(),
        b"omp" => b"Oh My Pi".to_vec(),
        other => other.to_vec(),
    }
}

/// `$(tmux display-message -p -t PANE FORMATO 2>/dev/null || true)`.
fn tmux_query(tmux: &Path, pane: &str, format: &str) -> Vec<u8> {
    Command::new(tmux)
        .args(["display-message", "-p", "-t", pane, format])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| strip_nl(&o.stdout).to_vec())
        .unwrap_or_default()
}

/// `desktop-$(uname -n | tr -c 'A-Za-z0-9_.-' '-' | cut -c1-60)`.
pub fn hostname_device() -> String {
    let name = nix::sys::utsname::uname()
        .map(|u| u.nodename().as_bytes().to_vec())
        .unwrap_or_default();
    let clean: Vec<u8> = strip_nl(&name)
        .iter()
        .map(|&b| {
            if b.is_ascii_alphanumeric() || b"_.-".contains(&b) {
                b
            } else {
                b'-'
            }
        })
        .collect();
    format!("desktop-{}", jq_lossy(strip_nl(head_c(&clean, 60))))
}

/// `$PWD` de bash: el heredado si apunta al directorio actual; si no, `getcwd`.
fn pwd() -> Vec<u8> {
    let current = std::env::current_dir().unwrap_or_default();
    let inherited = path(&env_bytes("PWD"));
    if inherited.is_absolute() && inherited.canonicalize().ok() == current.canonicalize().ok() {
        inherited.into_os_string().into_encoded_bytes()
    } else {
        current.into_os_string().into_encoded_bytes()
    }
}

/// Identidad del pane: lo que el bash calcula antes del `case`.
pub struct Place {
    pub proj: Vec<u8>,
    pub pane: String,
    pub session: Vec<u8>,
    pub pane_pid: Vec<u8>,
    pub state_key: Vec<u8>,
}

/// `date +%s` y `date +%s%N / 1000000`, de una sola lectura del reloj.
pub fn clock() -> (i64, i64) {
    let clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (clock.as_secs() as i64, clock.as_millis() as i64)
}

pub fn locate(input: &Input) -> Place {
    let cwd_or_pwd = if input.cwd.is_empty() {
        pwd()
    } else {
        input.cwd.clone()
    };
    let proj = basename(&cwd_or_pwd);
    let proj_file: Vec<u8> = head_c(
        &proj
            .iter()
            .map(|&b| {
                if b.is_ascii_alphanumeric() || b"._-".contains(&b) {
                    b
                } else {
                    b'-'
                }
            })
            .collect::<Vec<u8>>(),
        80,
    )
    .to_vec();
    let tmux_pane = env_bytes("TMUX_PANE");
    let pane_ok = |l: &[u8]| l.len() > 1 && l[0] == b'%' && l[1..].iter().all(u8::is_ascii_digit);
    let (mut pane, mut session, mut pane_pid) = (String::new(), Vec::new(), env_bytes("PANE_PID"));
    if any_line(&tmux_pane, pane_ok)
        && let Some(tmux) = which("tmux")
    {
        pane = String::from_utf8_lossy(&tmux_pane).into_owned();
        session = tmux_query(&tmux, &pane, "#S");
        pane_pid = tmux_query(&tmux, &pane, "#{pane_pid}");
    }
    let session_ok = |l: &[u8]| {
        (1..=80).contains(&l.len())
            && l.iter()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(b))
    };
    if !any_line(&session, session_ok) {
        session = head_c(&tr(&tr(&proj, b'.', b'-'), b':', b'-'), 60).to_vec();
    }
    let mut state_key = proj_file;
    if !session.is_empty() && !pane.is_empty() {
        state_key.extend_from_slice(b"--");
        state_key.extend_from_slice(&session);
        state_key.extend_from_slice(b"--");
        // `${PANE_HINT#%}`: quita exactamente un `%` inicial.
        state_key.extend_from_slice(pane.strip_prefix('%').unwrap_or(&pane).as_bytes());
    }
    Place {
        proj,
        pane,
        session,
        pane_pid,
        state_key,
    }
}
