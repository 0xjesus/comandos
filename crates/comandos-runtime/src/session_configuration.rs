//! `SessionConfiguration` y `PaneExtensionConfiguration` (`bin/cc-dash`
//! 2848–3298): el adaptador que cambia la cuenta, el modelo, el motor o las
//! extensiones de un pane vivo, sobre `session_operations::run_operation`.
//!
//! Síncrono: corre en un hilo de sistema propio (O1 del plan 2f-2), con tmux
//! por un `TmuxSync` que el llamador ata a su servidor (en producción, el
//! `Tmux` del frente; en pruebas, uno privado con `-S`). Los efectos son los
//! del Python, en su orden, y solo sobre el pane pedido: espera a que el
//! agente quede libre, copia de solo lectura del layout y del historial
//! (`_copy_conversation` solo escribe en la cuenta destino tras validar),
//! `session-handoffs/<id>.md` 0600, salida del agente original (`SIGTERM`
//! solo al pid anotado con el mismo inicio), `stty sane`, confianza de
//! carpeta en la cuenta destino, el comando `env COMANDOS_OPERATION_ID=…`
//! tecleado en el pane, verificación y, si falla, la vuelta al origen.
//!
//! Incertidumbre (`Fail::Unsure`): antes del `claim` quien llama declina
//! (`SessionConfiguration::probe`); después, nunca «seguir como si nada»:
//! antes de la primera tecla la operación falla por el camino de error del
//! Python con el agente original abierto; después, se sigue esperando o se
//! da por no confirmado (notas del controlador del brief de la Tarea 2).
use crate::{
    Unsure,
    accounts::{self, Paths},
    agent_procs::{
        self, AccountCache, AgentInfo, PANE_FORMAT, agent_pane_maps, parse_pane_inventory,
        process_owners,
    },
    claude_trust, dialogs, extension_launch,
    launch_command::{self, ConfigError, Ctx},
    pane_exit,
    pane_observe::{self, ObserveEvidence, ObserveFault},
    pane_snapshot::{PaneInspector, PaneRef},
    pane_typing::TmuxResult,
    providers,
    session_operations::{self as ops, Adapter, Journal, OperationStore, open_journal},
    tmux_snapshot::{self, SnapshotError},
    tui_state::{GrokMetadataCache, Obs, StateTracker, TranscriptCache, screen_state},
};
use comandos_core::{
    json::{python_eq, response_dumps, truthy},
    text::{shlex_quote, strip},
};
use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    ffi::OsString,
    fs, io,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

/// `tmux(*args)` del Python: el resultado, o el texto de la excepción que
/// lanzaría `subprocess.run` (plazo vencido, tmux ausente). El llamador ata
/// el socket: en pruebas siempre un `-S` privado.
pub type TmuxSync = Arc<dyn Fn(&[&str]) -> Result<TmuxResult, String> + Send + Sync>;
/// `grok_pane_busy(sess, pane)`: alguna tarjeta de `read_states_cached` de
/// ese pane está `working`. `Err(Unsure)` = no se pudo saber.
pub type WorkingProbe = Arc<dyn Fn(&str, &str) -> Result<bool, Unsure> + Send + Sync>;
/// `cc_usage.record_change(... "trust_inherited" ...)` con su nota; nunca rompe.
pub type TrustLedger = Arc<dyn Fn(&str) + Send + Sync>;
/// `time.time()`.
pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

/// Las cachés del Python que usa `observe_pane` (`_TUI_TRANSCRIPTS`,
/// `_GROK_METADATA`, `_ACCOUNT_PID_CACHE`, `_TUI_STATES`).
pub struct ObserveCaches {
    pub transcripts: TranscriptCache,
    pub grok: GrokMetadataCache,
    pub accounts: AccountCache,
    pub tracker: StateTracker,
}

impl Default for ObserveCaches {
    /// Los tamaños por defecto de `lib/tui_state.py`.
    fn default() -> Self {
        Self {
            transcripts: TranscriptCache::new(128, 2_097_152),
            grok: GrokMetadataCache::new(128),
            accounts: AccountCache::default(),
            tracker: StateTracker::new(128),
        }
    }
}

/// Lo que el adaptador lee del proceso. Todo se fija al construirlo: el
/// registro y la matriz de capacidades son los de antes del `claim`
/// (`load_provider_registry()` y `capability_matrix()`), así que una operación
/// no cambia de criterio a mitad de camino.
#[derive(Clone)]
pub struct Env {
    pub tmux: TmuxSync,
    /// `os.path.expanduser("~")`.
    pub home: PathBuf,
    /// `HOOKS` (`~/.claude/hooks`).
    pub hooks: PathBuf,
    /// Raíz de `/proc` (`/proc` en producción).
    pub proc_root: PathBuf,
    /// `load_provider_registry()`.
    pub registry: Value,
    /// `capability_matrix()`.
    pub matrix: Vec<Value>,
    /// `REPO_ROOT` (diálogos, `cc-extension-session`).
    pub repo_root: PathBuf,
    /// Directorio de trabajo del proceso (rutas relativas de cuentas).
    pub cwd: PathBuf,
    /// `PATH` del proceso (`provider_registry.which`).
    pub search_path: Option<OsString>,
    /// `int(load_proxy_cfg().get("port") or 18765)`.
    pub proxy_port: u16,
    /// `os.environ` (resolución del comando de `wrap_command`).
    pub environ: HashMap<String, String>,
    /// `H/session-operations.sqlite3`: el adaptador abre su conexión.
    pub journal: PathBuf,
    /// `os.getpid()` del dueño de las filas.
    pub owner: i64,
    /// `time.sleep`; las pruebas acortan como el `monkeypatch` del Python.
    pub sleep: fn(Duration),
    pub clock: Clock,
    pub working: WorkingProbe,
    pub trust_ledger: TrustLedger,
    pub caches: Arc<Mutex<ObserveCaches>>,
    pub dialogs: Arc<dialogs::DialogCache>,
}

impl Env {
    fn tmux(&self, args: &[&str]) -> Result<TmuxResult, Fail> {
        (self.tmux)(args).map_err(Fail::Py)
    }

    fn sleep_s(&self, seconds: f64) {
        (self.sleep)(Duration::from_secs_f64(seconds));
    }

    fn ctx(&self) -> Ctx {
        Ctx {
            registry: self.registry.clone(),
            home: self.home.clone(),
            cwd: self.cwd.clone(),
            search_path: self.search_path.clone(),
            proxy_port: self.proxy_port,
            repo_root: self.repo_root.clone(),
        }
    }

    fn paths(&self) -> Paths {
        Paths::new(&self.home, &self.cwd)
    }

    fn home_text(&self) -> Result<&str, Fail> {
        self.home.to_str().ok_or(Fail::Unsure)
    }

    fn caches(&self) -> std::sync::MutexGuard<'_, ObserveCaches> {
        self.caches.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Lo que puede salir mal en un paso del adaptador.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fail {
    /// Una excepción del Python (`ValueError`, `RuntimeError`, …) con su texto.
    Py(String),
    /// El port no reproduce con certeza lo que haría el Python.
    Unsure,
}

impl From<Unsure> for Fail {
    fn from(_: Unsure) -> Self {
        Fail::Unsure
    }
}

impl From<ObserveFault> for Fail {
    fn from(_: ObserveFault) -> Self {
        Fail::Unsure
    }
}

impl From<ConfigError> for Fail {
    fn from(e: ConfigError) -> Self {
        match e {
            ConfigError::Value(text) => Fail::Py(text),
            ConfigError::Unsure => Fail::Unsure,
        }
    }
}

impl From<ops::Error> for Fail {
    fn from(e: ops::Error) -> Self {
        Fail::Py(e.to_string())
    }
}

impl From<SnapshotError> for Fail {
    fn from(e: SnapshotError) -> Self {
        match e {
            SnapshotError::Value(text) | SnapshotError::Runtime(text) => Fail::Py(text),
            SnapshotError::Io(error) => Fail::Py(io_text(&error)),
            SnapshotError::Unsure => Fail::Unsure,
        }
    }
}

impl From<extension_launch::LaunchError> for Fail {
    fn from(e: extension_launch::LaunchError) -> Self {
        match e {
            extension_launch::LaunchError::Value(text) => Fail::Py(text),
            extension_launch::LaunchError::Other | extension_launch::LaunchError::Unsure => {
                Fail::Unsure
            }
        }
    }
}

/// Texto del error que sale de un paso incierto antes de tocar el pane: el
/// agente original sigue abierto. (No existe en el Python, que no duda.)
pub const UNCERTAIN: &str =
    "no se pudo comprobar el estado del panel con certeza; el agente sigue abierto";
/// Lo mismo cuando el agente original ya se cerró: no se teclea el destino.
pub const UNCERTAIN_APPLY: &str =
    "no se pudo comprobar el panel con certeza; no se envía el comando destino";
/// Lo mismo al verificar el destino ya tecleado: no se confirma (sigue la
/// vuelta al origen, como tras cualquier error del Python en `verify`).
pub const UNCERTAIN_VERIFY: &str = "no se pudo confirmar el destino con certeza";
/// Lo mismo durante la vuelta al origen: no se interrumpe nada.
pub const UNCERTAIN_ROLLBACK: &str = "no se pudo comprobar el panel con certeza; no se interrumpe";

fn to_journal(fail: Fail, uncertain: &str) -> ops::Error {
    match fail {
        Fail::Py(text) => ops::Error::Callback(text),
        Fail::Unsure => ops::Error::Callback(uncertain.to_owned()),
    }
}

/// `str(exc)` de un `OSError` del Python, lo más cerca posible.
pub(crate) fn io_text(error: &io::Error) -> String {
    match error.raw_os_error() {
        Some(code) => format!("[Errno {code}] {}", errno_text(code)),
        None => error.to_string(),
    }
}

/// `str(exc)` de un `OSError` con su ruta (`[Errno 17] File exists: '…'`).
pub(crate) fn io_text_path(error: &io::Error, path: &str) -> String {
    match error.raw_os_error() {
        Some(code) => format!(
            "[Errno {code}] {}: {}",
            errno_text(code),
            pane_exit::py_repr(path)
        ),
        None => error.to_string(),
    }
}

fn errno_text(code: i32) -> String {
    nix::errno::Errno::from_raw(code).desc().to_owned()
}

// --------------------------------------------------------------- utilidades

static NULL: Value = Value::Null;

/// `d.get(k)` (ausente = `None`).
fn get<'a>(map: &'a Map<String, Value>, key: &str) -> &'a Value {
    map.get(key).unwrap_or(&NULL)
}

/// `(x or {})` de un valor que se usará con `.get`: un no-`dict` verdadero
/// es el `AttributeError` del Python (incierto aquí).
fn obj(value: &Value) -> Result<Option<&Map<String, Value>>, Fail> {
    match value {
        Value::Object(map) => Ok(Some(map)),
        v if truthy(v) => Err(Fail::Unsure),
        _ => Ok(None),
    }
}

fn obj_get<'a>(value: &'a Value, key: &str) -> Result<&'a Value, Fail> {
    Ok(obj(value)?.map_or(&NULL, |m| get(m, key)))
}

/// `str(x)` de un escalar de JSON; contenedores → incierto.
fn py_text(value: &Value) -> Result<String, Fail> {
    pane_observe::py_str(value).map_err(|_| Fail::Unsure)
}

/// `str(x or default)`.
fn text_or(value: &Value, default: &str) -> Result<String, Fail> {
    if truthy(value) {
        py_text(value)
    } else {
        Ok(default.to_owned())
    }
}

/// `x or ''` cuando el valor se usa como texto (no se formatea): un valor
/// verdadero que no es texto se declina.
fn as_text(value: &Value) -> Result<String, Fail> {
    match value {
        Value::String(s) => Ok(s.clone()),
        v if truthy(v) => Err(Fail::Unsure),
        _ => Ok(String::new()),
    }
}

fn s(text: &str) -> Value {
    Value::from(text)
}

fn py(text: impl Into<String>) -> Fail {
    Fail::Py(text.into())
}

const SHELLS: [&str; 5] = ["bash", "zsh", "sh", "fish", "dash"];

/// `PANE_RE = ^%\d{1,7}\Z`: `Some` con certeza; `None` si decidiría el `\d`
/// Unicode de Python.
pub fn pane_re(pane: &str) -> Option<bool> {
    let Some(digits) = pane.strip_prefix('%') else {
        return Some(false);
    };
    if digits.is_ascii() {
        return Some((1..=7).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()));
    }
    let count = digits.chars().count();
    if (1..=7).contains(&count) && digits.chars().all(char::is_numeric) {
        return None;
    }
    Some(false)
}

// ------------------------------------------------------- pane y proceso

/// El agente principal de un pane (`agent_info_for_pane` o el `original` que
/// rearma `session_recover`). `pid` 0 = `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub agent: String,
    pub pid: i64,
}

impl From<&AgentInfo> for Proc {
    fn from(info: &AgentInfo) -> Self {
        Self {
            agent: info.agent.clone(),
            pid: info.pid,
        }
    }
}

/// `agent_info_for_pane(pane)` (1591): el agente de `agent_pane_maps` en ese
/// pane, o `None`. Lee `/proc` y `tmux list-panes -a`.
pub fn agent_info_for_pane(env: &Env, pane: &str) -> Result<Option<Proc>, Fail> {
    match pane_re(pane) {
        Some(true) => {}
        Some(false) => return Ok(None),
        None => return Err(Fail::Unsure),
    }
    let conf = providers::read_conf(&env.hooks.join("cc-notify.conf"))?;
    let conf_agents = conf
        .iter()
        .find(|(k, _)| k == "AGENTS")
        .map(|(_, v)| v.as_str());
    let agents = providers::agent_set(conf_agents, &env.registry);
    let aliases = providers::process_aliases(&agents, &env.registry);
    let procs = agent_procs::agent_procs_for_agents(&env.proc_root, &aliases, &agents)?;
    let listed = env.tmux(&["list-panes", "-a", "-F", PANE_FORMAT])?;
    let mut panes = if listed.returncode == 0 {
        parse_pane_inventory(&listed.stdout)
    } else {
        Vec::new()
    };
    agent_procs::resolve_foreground(env.proc_root.as_path(), &mut panes);
    let proc_root = env.proc_root.as_path();
    let mut parents: HashMap<i64, i64> = HashMap::new();
    let mut parent = |pid: i64| {
        *parents
            .entry(pid)
            .or_insert_with(|| agent_procs::parent_pid(proc_root, pid))
    };
    let owners = process_owners(&procs, &panes, &mut parent);
    let mut cmdline = |pid: i64| agent_procs::proc_cmdline(proc_root, pid);
    let maps = agent_pane_maps(&procs, &panes, &owners, &mut cmdline, &mut parent);
    Ok(maps
        .by_cwd
        .iter()
        .flat_map(|(_, infos)| infos)
        .find(|info| info.pane == pane)
        .map(Proc::from))
}

/// A parked operation (`recovery_required`/`awaiting_confirmation`) holds its pane
/// until someone recovers it. When the pane already runs an agent this operation did
/// not launch (no `COMANDOS_OPERATION_ID` of its own) and that is not the original,
/// the user moved on by hand: neither confirming nor recovering applies, and keeping
/// the row only rejected every later account or model switch on that pane (13-oct).
pub fn operation_left_behind(env: &Env, pane: &str, row: &Value) -> bool {
    let state = row.get("state").and_then(Value::as_str).unwrap_or("");
    let Some(id) = row.get("id").and_then(Value::as_str) else {
        return false;
    };
    if !matches!(state, "recovery_required" | "awaiting_confirmation") {
        return false;
    }
    let Ok(Some(info)) = agent_info_for_pane(env, pane) else {
        return false;
    };
    let marker = agent_procs::read_environ(&env.proc_root, info.pid)
        .get(b"COMANDOS_OPERATION_ID".as_slice())
        .cloned()
        .unwrap_or_default();
    if marker == id.as_bytes() {
        return false;
    }
    let snapshot = row.get("snapshot").unwrap_or(&Value::Null);
    let pinned = |record: &Value, pid: &str, start: &str| {
        record.get(pid).and_then(Value::as_i64) == Some(info.pid)
            && record.get(start).filter(|v| truthy(v)).is_none_or(|v| {
                let start = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                start == agent_procs::process_start(&env.proc_root, info.pid)
            })
    };
    !pinned(&snapshot["origin"], "agent_pid", "agent_start")
        && !pinned(&snapshot["destinationProcess"], "pid", "start")
}

/// Los campos de `_pane_identity`, en su orden. Única implementación: GET y
/// POST del frente (`dash::native::target`) usan estas mismas piezas.
pub const IDENTITY_FIELDS: [&str; 8] = [
    "socket_path",
    "pid",
    "session_id",
    "session_name",
    "pane_id",
    "pane_pid",
    "pane_current_command",
    "pane_current_path",
];

/// El formato de `display-message -p` de `_pane_identity`.
pub fn identity_format() -> String {
    IDENTITY_FIELDS
        .iter()
        .map(|f| format!("#{{{f}}}"))
        .collect::<Vec<_>>()
        .join("\t")
}

/// La identidad a partir de la salida de tmux, sin `server_start`.
/// `Fail::Py` es el `ValueError` literal del Python.
pub fn identity_from_output(
    ok: bool,
    stdout: &str,
    sess: &str,
    pane: &str,
) -> Result<Map<String, Value>, Fail> {
    let parts: Vec<&str> = strip(stdout).split('\t').collect();
    if !ok || parts.len() != IDENTITY_FIELDS.len() {
        return Err(py("el panel ya no existe"));
    }
    let mut identity = Map::new();
    for (key, value) in IDENTITY_FIELDS.iter().zip(&parts) {
        identity.insert((*key).into(), s(value));
    }
    if parts.get(4) != Some(&pane) || parts.get(3) != Some(&sess) {
        return Err(py("el panel no pertenece a esa sesión"));
    }
    Ok(identity)
}

/// `_process_start` del servidor tmux de `_pane_identity`: el campo 22 de
/// `/proc/<pid>/stat`, `""` si no se lee (`except OSError`); lo que el Python
/// no captura (`UnicodeDecodeError`, `IndexError`) es incierto. Bloquea.
pub fn server_start(proc_root: &Path, pid: &str) -> Result<String, Fail> {
    #[cfg(target_os = "macos")]
    if proc_root == Path::new("/proc") {
        return Ok(pid
            .parse::<i64>()
            .ok()
            .map(|p| crate::agent_procs::process_start(proc_root, p))
            .unwrap_or_default());
    }
    match fs::read(proc_root.join(pid).join("stat")) {
        Err(_) => Ok(String::new()),
        Ok(bytes) => {
            let text = String::from_utf8(bytes).map_err(|_| Fail::Unsure)?;
            Ok(text
                .rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().nth(19))
                .ok_or(Fail::Unsure)?
                .to_owned())
        }
    }
}

/// `_pane_identity(sess, pane)` (6887).
pub fn pane_identity(env: &Env, sess: &str, pane: &str) -> Result<Map<String, Value>, Fail> {
    match pane_re(pane) {
        Some(true) => {}
        Some(false) => return Err(py("se necesita el panel exacto")),
        None => return Err(Fail::Unsure),
    }
    let out = env.tmux(&["display-message", "-p", "-t", pane, &identity_format()])?;
    let mut identity = identity_from_output(out.returncode == 0, &out.stdout, sess, pane)?;
    let start = server_start(&env.proc_root, ident_text(&identity, "pid"))?;
    identity.insert("server_start".into(), s(&start));
    Ok(identity)
}

/// `_identity_key(identity)` (6907).
pub fn identity_key(identity: &Map<String, Value>) -> String {
    [
        "socket_path",
        "pid",
        "server_start",
        "session_id",
        "pane_id",
        "pane_pid",
    ]
    .iter()
    .map(|k| identity.get(*k).and_then(Value::as_str).unwrap_or(""))
    .collect::<Vec<_>>()
    .join("|")
}

fn ident_text<'a>(identity: &'a Map<String, Value>, key: &str) -> &'a str {
    identity.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `observe_pane(sess, pane, info, inspector)` (6912), síncrono: lo mismo que
/// GET `/state` (`pane_observe`), con tmux y `/proc` de este hilo.
pub fn observe_pane(
    env: &Env,
    sess: &str,
    pane: &str,
    info: Option<&Proc>,
    inspector: Option<&PaneInspector>,
) -> Result<Obs, Fail> {
    let found;
    let info = match info.filter(|i| i.pid != 0 || !i.agent.is_empty()) {
        Some(info) => Some(info),
        None => {
            found = agent_info_for_pane(env, pane)?;
            found.as_ref()
        }
    };
    let Some(info) = info.filter(|i| i.pid != 0) else {
        let mut out = Map::new();
        out.insert("confirmed".into(), Value::Bool(false));
        out.insert("model".into(), s(""));
        out.insert("effort".into(), s(""));
        out.insert("source".into(), s("unconfirmed"));
        return Ok(out);
    };
    let identity = pane_identity(env, sess, pane)?;
    let own;
    let inspector = match inspector {
        Some(i) => i,
        None => {
            own = PaneInspector::new(&env.home, &env.proc_root)?;
            &own
        }
    };
    // `int(identity['pane_pid'])`.
    let pane_pid = ident_text(&identity, "pane_pid");
    if pane_pid.is_empty() || !pane_pid.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Fail::Unsure);
    }
    let pane_pid: i64 = pane_pid.parse().map_err(|_| Fail::Unsure)?;
    let snap = inspector.inspect(&PaneRef {
        id: pane,
        pid: pane_pid,
        command: ident_text(&identity, "pane_current_command"),
    })?;
    let (agent, pid) = (info.agent.as_str(), info.pid);
    let proc_root = env.proc_root.as_path();
    let mut caches = env.caches();
    let account = caches
        .accounts
        .account_for_pid(&env.home, proc_root, pid, agent)?;
    let account = match account.get("account").filter(|v| truthy(v)) {
        Some(value) => py_text(value)?,
        None => "unknown".to_owned(),
    };
    let cmdline = agent_procs::proc_cmdline(proc_root, pid);
    let conversation_id = pane_observe::conversation_id(&snap)?;
    let mut acp = Map::new();
    let conversation = match agent {
        "codex" if truthy(&conversation_id) => {
            let id = conversation_id.as_str().ok_or(Fail::Unsure)?;
            pane_observe::codex_conversation(&mut caches.transcripts, proc_root, pid, id)?
        }
        "grok" => {
            let grok_home = pane_observe::grok_home_for(proc_root, &env.home, pid)?;
            let metadata = caches.grok.read(pid, &grok_home)?;
            pane_observe::project_metadata(&metadata, &conversation_id, "lastActiveAt")
        }
        "opencode" | "agy" => match snap.get("nativeMetadata").filter(|v| truthy(v)) {
            None => Map::new(),
            Some(Value::Object(metadata)) => {
                pane_observe::project_metadata(metadata, &conversation_id, "updatedAt")
            }
            Some(_) => return Err(Fail::Unsure),
        },
        "acp" => {
            acp = pane_observe::acp_state(&env.hooks, pane)?;
            Map::new()
        }
        "claude" if truthy(&conversation_id) => {
            let id = conversation_id.as_str().ok_or(Fail::Unsure)?;
            let root = match snap.get("claude_config_dir").filter(|v| truthy(v)) {
                None => env.home.join(".claude"),
                Some(Value::String(dir)) => PathBuf::from(dir),
                Some(_) => return Err(Fail::Unsure),
            };
            pane_observe::claude_conversation(&mut caches.transcripts, &root, id)?
        }
        _ => Map::new(),
    };
    drop(caches);
    let visible = if matches!(agent, "codex" | "claude" | "opencode") {
        let captured = env.tmux(&["capture-pane", "-p", "-t", pane, "-S", "-45"])?;
        if captured.returncode == 0 {
            screen_state(agent, &captured.stdout)?
        } else {
            Map::new()
        }
    } else {
        Map::new()
    };
    let process_start = agent_procs::process_start(proc_root, pid);
    let base_url = if agent == "claude" {
        agent_procs::read_environ(proc_root, pid)
            .get(b"ANTHROPIC_BASE_URL".as_slice())
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let evidence = ObserveEvidence {
        agent: agent.to_owned(),
        pid,
        identity: identity_key(&identity),
        snap,
        account,
        harness_has_accounts: providers::harness_has_accounts(&env.registry, agent),
        cmdline,
        conversation,
        acp,
        visible,
        process_start,
        base_url,
    };
    let now = (env.clock)();
    let mut caches = env.caches();
    Ok(pane_observe::observe(
        &evidence,
        &mut caches.tracker,
        &env.registry,
        now,
    )?)
}

/// `claude_pane_busy(pane)` (3575): el motivo, o `""` si está en el prompt.
/// `Err(Unsure)` cuando la pantalla tiene U+001C–U+001F (el `\s` de Python
/// los cuenta como espacio y el de `regex` no).
fn claude_pane_busy(env: &Env, pane: &str) -> Result<bool, Fail> {
    let r = env.tmux(&["capture-pane", "-p", "-t", pane, "-S", "-14"])?;
    if r.returncode != 0 {
        return Ok(false);
    }
    let tail = r.stdout.trim_end_matches('\n');
    // `re.I` de Python 3.10 empareja İ/ı con i (minúscula simple); `(?i)`
    // de `regex` no: con esas letras no se sabe si está ocupado.
    if tail.contains(['\u{130}', '\u{131}']) {
        return Err(Fail::Unsure);
    }
    let compile = |p: &str| regex::Regex::new(p).map_err(|_| Fail::Unsure);
    if compile(r"(?i)esc to interrupt|ctrl\+c to interrupt|esc para interrumpir")?.is_match(tail) {
        return Ok(true);
    }
    // `\s` de Python = `\s` de `regex` más U+001C–U+001F.
    if compile(r"(?m)^[\s\x1c-\x1f]*(Do you want|Allow|¿Permitir|Yes, |No, )")?.is_match(tail)
        && compile(r"(?i)❯[\s\x1c-\x1f]*1\.|\(y\)|Enter to confirm")?.is_match(tail)
    {
        return Ok(true);
    }
    Ok(!tail.contains('❯') && !tail.contains('>'))
}

/// `harness_pane_busy(sess, pane, harness)` (1914).
pub fn harness_pane_busy(env: &Env, sess: &str, pane: &str, harness: &str) -> Result<bool, Fail> {
    if matches!(harness, "" | "shell") {
        return Ok(false);
    }
    if matches!(harness, "opencode" | "agy") {
        let info = agent_info_for_pane(env, pane)?;
        let pid = info.map_or(0, |i| i.pid);
        let metadata =
            PaneInspector::new(&env.home, &env.proc_root)?.native_metadata(pid, harness)?;
        if let Some(Value::Bool(busy)) = metadata.get("busy") {
            return Ok(*busy);
        }
    }
    if harness == "claude" {
        return claude_pane_busy(env, pane);
    }
    Ok((env.working)(sess, pane)?)
}

// ---------------------------------------------------------- historiales

/// Los comodines de `glob` que cambiarían el patrón.
fn has_magic(text: &str) -> bool {
    pane_observe::has_magic(text)
}

/// `os.path.expanduser(path)` con el HOME del entorno.
fn expanduser(env: &Env, path: &str) -> Result<String, Fail> {
    Ok(claude_trust::expanduser(path, env.home_text()?)?)
}

fn join(a: &str, b: &str) -> String {
    if a.is_empty() || a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}

fn lexists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

fn hidden(name: &[u8]) -> bool {
    name.first() == Some(&b'.')
}

/// `_listdir(dirname, dironly)` de `glob`: nombres de `scandir` en su orden;
/// con `dironly`, solo los que `is_dir()` (siguiendo enlaces). Un error
/// (directorio ausente, permisos) es la lista vacía.
fn listdir(dir: &Path, dironly: bool) -> Vec<(OsString, bool)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let link = entry.file_type().is_ok_and(|t| t.is_symlink());
        let is_dir = fs::metadata(entry.path()).is_ok_and(|m| m.is_dir());
        if !dironly || is_dir {
            out.push((name, link && is_dir));
        }
    }
    out
}

/// `_rlistdir` de `**`: los subdirectorios (no ocultos) en preorden, relativos.
/// Un subdirectorio enlazado haría que `glob` lo recorriera (y con un bucle,
/// resultados repetidos): incierto.
fn rlistdir(root: &Path, relative: &Path, out: &mut Vec<PathBuf>) -> Result<(), Fail> {
    for (name, linked_dir) in listdir(&root.join(relative), true) {
        if hidden(name.as_bytes()) {
            continue;
        }
        if linked_dir {
            return Err(Fail::Unsure);
        }
        let child = relative.join(&name);
        out.push(child.clone());
        rlistdir(root, &child, out)?;
    }
    Ok(())
}

/// `fnmatch` de un componente con un solo `*` entre `prefix` y `suffix`,
/// sin ocultos (como `glob`).
fn wildcard(name: &[u8], prefix: &[u8], suffix: &[u8]) -> bool {
    !hidden(name)
        && name.len() >= prefix.len() + suffix.len()
        && name.starts_with(prefix)
        && name.ends_with(suffix)
}

/// Los patrones de `_snapshot_transcript`/`_copy_conversation` sobre `root`
/// (sin comodines en `root` ni en `sid`, que ya se validó).
fn glob_history(root: &str, agent: &str, sid: &str) -> Result<Vec<String>, Fail> {
    if has_magic(root) || has_magic(sid) {
        return Err(Fail::Unsure);
    }
    let base = Path::new(root);
    let mut out = Vec::new();
    match agent {
        // `projects/*/<sid>.jsonl`.
        "claude" => {
            let name = format!("{sid}.jsonl");
            for (dir, _) in listdir(&base.join("projects"), true) {
                if hidden(dir.as_bytes()) {
                    continue;
                }
                let candidate = base.join("projects").join(&dir).join(&name);
                if lexists(&candidate) {
                    out.push(candidate);
                }
            }
        }
        // `sessions/**/rollout-*<sid>.jsonl` y `sessions/**/<sid>/summary.json`.
        "codex" | "grok" => {
            let sessions = base.join("sessions");
            if !fs::metadata(&sessions).is_ok_and(|m| m.is_dir()) {
                return Ok(Vec::new());
            }
            let mut dirs = vec![PathBuf::new()];
            rlistdir(&sessions, Path::new(""), &mut dirs)?;
            for dir in dirs {
                let here = sessions.join(&dir);
                if agent == "codex" {
                    let suffix = format!("{sid}.jsonl");
                    for (name, _) in listdir(&here, false) {
                        if wildcard(name.as_bytes(), b"rollout-", suffix.as_bytes()) {
                            out.push(here.join(name));
                        }
                    }
                } else {
                    let candidate = here.join(sid).join("summary.json");
                    if lexists(&candidate) {
                        out.push(candidate);
                    }
                }
            }
        }
        _ => return Err(Fail::Unsure),
    }
    out.into_iter()
        .map(|p| p.into_os_string().into_string().map_err(|_| Fail::Unsure))
        .collect()
}

/// `re.fullmatch(r'[A-Za-z0-9_-]+', sid)`.
fn sid_ok(sid: &str) -> bool {
    !sid.is_empty()
        && sid
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `(root_key, default)` de cada agente con historial propio.
fn history_root(agent: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match agent {
        "claude" => Some(("claude_config_dir", "~/.claude", "CLAUDE_CONFIG_DIR")),
        "codex" => Some(("codex_home", "~/.codex", "CODEX_HOME")),
        "grok" => Some(("grok_home", "~/.grok", "GROK_HOME")),
        _ => None,
    }
}

/// `_snapshot_transcript(origin)` (2366): la ruta del historial exacto.
pub fn snapshot_transcript(env: &Env, origin: &Map<String, Value>) -> Result<String, Fail> {
    snapshot_transcript_at(&env.home, origin)
}

/// Resolve the exact live agent transcript without constructing a migration environment.
pub fn snapshot_transcript_at(home: &Path, origin: &Map<String, Value>) -> Result<String, Fail> {
    let agent = get(origin, "agent");
    if is(agent, "opencode") || is(agent, "agy") {
        let path = as_text(get(origin, "transcriptPath"))?;
        if path.is_empty() || !fs::metadata(&path).is_ok_and(|m| m.is_file()) {
            return Err(py("no se encontró el historial exacto del proceso"));
        }
        return Ok(path);
    }
    let Some((root_key, default, _)) = agent.as_str().and_then(history_root) else {
        return Ok(String::new());
    };
    let agent = agent.as_str().unwrap_or("");
    let sid = as_text(get(origin, "resume_id"))?;
    if !sid_ok(&sid) {
        return Err(py("identificador de conversación inválido"));
    }
    let root = match get(origin, root_key) {
        v if truthy(v) => as_text(v)?,
        _ => claude_trust::expanduser(default, home.to_str().ok_or(Fail::Unsure)?)?,
    };
    let matches = glob_history(&root, agent, &sid)?;
    match matches.as_slice() {
        [only] if fs::metadata(only).is_ok_and(|m| m.is_file()) => Ok(only.clone()),
        _ => Err(py(
            "no se encontró un único historial para reanudar; el origen sigue abierto",
        )),
    }
}

fn is(value: &Value, text: &str) -> bool {
    value.as_str() == Some(text)
}

/// `os.path.relpath(path, start)` de dos rutas absolutas ya normalizadas por
/// `glob` (la primera bajo la segunda).
fn relpath(path: &str, start: &str) -> Result<String, Fail> {
    let norm = |p: &str| -> Result<String, Fail> {
        if !p.starts_with('/') {
            return Err(Fail::Unsure);
        }
        String::from_utf8(agent_procs::normpath(p.as_bytes())).map_err(|_| Fail::Unsure)
    };
    let (path, start) = (norm(path)?, norm(start)?);
    let a: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    let b: Vec<&str> = start.split('/').filter(|c| !c.is_empty()).collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<&str> = vec![".."; b.len() - common];
    parts.extend(a.iter().skip(common));
    Ok(if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    })
}

fn dirname(path: &str) -> String {
    String::from_utf8_lossy(&agent_procs::dirname(path.as_bytes())).into_owned()
}

/// `shutil.copy2` + `fsync` + `os.replace` de `atomic_copy`: el temporal
/// `.comandos-copy-*` en el directorio destino, con el modo y las fechas del
/// origen. (Los atributos extendidos que también copia `copy2` no se
/// reproducen: los historiales no los llevan.)
fn atomic_copy(source: &Path, target: &Path) -> Result<(), Fail> {
    let dir = target.parent().ok_or(Fail::Unsure)?;
    let name = crate::fresh_id(".comandos-copy").map_err(|_| Fail::Unsure)?;
    let temporary = dir.join(name);
    let result = (|| -> io::Result<()> {
        let meta = fs::metadata(source)?;
        let mut from = fs::File::open(source)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        io::copy(&mut from, &mut file)?;
        let times = fs::FileTimes::new()
            .set_accessed(meta.accessed()?)
            .set_modified(meta.modified()?);
        file.set_times(times)?;
        file.set_permissions(fs::Permissions::from_mode(meta.mode() & 0o7777))?;
        file.sync_all()?;
        fs::rename(&temporary, target)
    })();
    if lexists(&temporary) {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| Fail::Py(io_text(&e)))
}

/// `shutil.copytree(source, target, dirs_exist_ok=True,
/// copy_function=atomic_copy)`: directorios (y su `copystat` al final), sin
/// enlaces (ya validado).
fn copy_tree(source: &Path, target: &Path) -> Result<(), Fail> {
    let entries: Vec<_> = fs::read_dir(source)
        .map_err(|e| Fail::Py(io_text(&e)))?
        .collect::<io::Result<Vec<_>>>()
        .map_err(|e| Fail::Py(io_text(&e)))?;
    fs::create_dir_all(target).map_err(|e| Fail::Py(io_text(&e)))?;
    for entry in entries {
        let from = entry.path();
        let to = target.join(entry.file_name());
        if fs::metadata(&from).is_ok_and(|m| m.is_dir()) {
            copy_tree(&from, &to)?;
        } else {
            atomic_copy(&from, &to)?;
        }
    }
    let meta = fs::metadata(source).map_err(|e| Fail::Py(io_text(&e)))?;
    let applied = (|| -> io::Result<()> {
        let dir = fs::File::open(target)?;
        let times = fs::FileTimes::new()
            .set_accessed(meta.accessed()?)
            .set_modified(meta.modified()?);
        dir.set_times(times)?;
        fs::set_permissions(target, fs::Permissions::from_mode(meta.mode() & 0o7777))
    })();
    applied.map_err(|e| Fail::Py(io_text(&e)))
}

/// `glob(join(target_item, '**', '*'), recursive=True)` en el orden de
/// `glob`: las entradas del directorio y después las de cada subdirectorio
/// en preorden. Un subdirectorio enlazado aparece como entrada antes de
/// recorrerse y ya hace fallar la validación. Desviación: también los
/// ocultos, que `glob` omite pero `copytree` sí sobrescribe; así ningún
/// archivo divergente del destino se pisa sin comparar su prefijo.
fn existing_tree(target: &Path) -> Result<Vec<PathBuf>, Fail> {
    if !fs::metadata(target).is_ok_and(|m| m.is_dir()) {
        return Ok(Vec::new());
    }
    let mut dirs = vec![PathBuf::new()];
    let mut stack = Vec::new();
    // Preorden de `_rlistdir` sin seguir enlaces (un enlace ya es error).
    fn walk(root: &Path, rel: &Path, out: &mut Vec<PathBuf>) {
        for (name, linked) in listdir(&root.join(rel), true) {
            if linked {
                continue;
            }
            let child = rel.join(&name);
            out.push(child.clone());
            walk(root, &child, out);
        }
    }
    walk(target, Path::new(""), &mut stack);
    dirs.extend(stack);
    let mut out = Vec::new();
    for dir in dirs {
        for (name, _) in listdir(&target.join(&dir), false) {
            out.push(target.join(&dir).join(name));
        }
    }
    Ok(out)
}

fn walk_has_link(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_link(&path) {
            return true;
        }
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) && walk_has_link(&path) {
            return true;
        }
    }
    false
}

/// `SessionConfiguration._compatible_copy(source, target)`: el destino es un
/// prefijo del origen (nada propio que se perdería al sobrescribir).
pub fn compatible_copy(source: &Path, target: &Path) -> io::Result<bool> {
    use std::io::Read;
    let mut src = fs::File::open(source)?;
    let mut dst = fs::File::open(target)?;
    let mut chunk = vec![0u8; 65536];
    let mut other = vec![0u8; 65536];
    loop {
        let n = read_full(&mut dst, &mut chunk)?;
        if n == 0 {
            return Ok(true);
        }
        let m = read_full(&mut src, other.get_mut(..n).unwrap_or_default())?;
        if m != n || chunk.get(..n) != other.get(..n) {
            return Ok(false);
        }
    }
    fn read_full(file: &mut fs::File, buf: &mut [u8]) -> io::Result<usize> {
        let mut total = 0;
        while total < buf.len() {
            let n = file.read(buf.get_mut(total..).unwrap_or_default())?;
            if n == 0 {
                break;
            }
            total += n;
        }
        Ok(total)
    }
}

// ---------------------------------------------------------------- rutas

/// `provider_registry.route_for(registry, route_id)`: la primera ruta con ese id.
fn route_for(registry: &Value, route_id: &str) -> Result<Option<Map<String, Value>>, Fail> {
    let routes = match registry.get("routes") {
        Some(Value::Array(r)) => r.as_slice(),
        Some(v) if truthy(v) => return Err(Fail::Unsure),
        _ => &[],
    };
    for route in routes {
        let route = route.as_object().ok_or(Fail::Unsure)?;
        if is(get(route, "id"), route_id) {
            return Ok(Some(route.clone()));
        }
    }
    Ok(None)
}

/// `(registry.get('acpAgents') or {}).get(motor) or {}).get('resume')`.
fn acp_resume(registry: &Value, motor: &Value) -> Result<bool, Fail> {
    let agents = obj(registry.get("acpAgents").unwrap_or(&NULL))?;
    let Some(agents) = agents else {
        return Ok(false);
    };
    let agent = match motor {
        Value::String(m) => get(agents, m),
        Value::Null | Value::Bool(_) | Value::Number(_) => &NULL,
        _ => return Err(Fail::Unsure),
    };
    Ok(truthy(obj_get(agent, "resume")?))
}

/// `session_change_support(registry, observed)` (2671) de una ruta: la versión
/// sin `observed` de `providers`, más la sesión ACP que sí publica su esfuerzo.
fn change_support(
    registry: &Value,
    observed: &Map<String, Value>,
    route_id: &str,
) -> Result<Option<Value>, Fail> {
    let support = providers::session_change_support(registry)?;
    let Some(item) = support.get(route_id).cloned() else {
        return Ok(None);
    };
    let code = item.get("reason").and_then(|r| r.get("code")).cloned();
    if code.as_ref().and_then(Value::as_str) == Some("acp_effort_unobserved")
        && let Some(route) = route_for(registry, route_id)?
        && is(get(observed, "harness"), "acp")
        && python_eq(get(observed, "motor"), get(&route, "motor"))
        && is(get(observed, "effortSource"), "acp-config-options")
    {
        return Ok(Some(json!({"selectable": true, "reason": null})));
    }
    Ok(Some(item))
}

/// `resolve_route_selection(data, scope)` (1661) sobre la matriz fijada.
/// `Fail::Py(código)` es el `ProviderRegistryError` del Python.
fn resolve_route_selection(
    env: &Env,
    data: &Map<String, Value>,
    scope: &str,
) -> Result<(Obs, Obs), Fail> {
    resolve_profile_route(&env.registry, &env.matrix, &env.paths(), data, scope)
}

/// Resolve a launch draft using the same route and account checks as pane configuration.
pub fn resolve_profile_route(
    registry: &Value,
    matrix: &[Value],
    paths: &accounts::Paths,
    data: &Map<String, Value>,
    scope: &str,
) -> Result<(Obs, Obs), Fail> {
    let mut route_id = text_or(get(data, "routeId"), "")?;
    if route_id.is_empty() {
        route_id = match text_or(get(data, "agent"), "")?.as_str() {
            "claude" => "claude:claude",
            "claude-codex" => "claude:codex",
            "claude-grok" => "claude:grok",
            "codex" => "codex:codex",
            "grok" => "grok:grok",
            _ => "",
        }
        .to_owned();
    }
    let mut model = text_or(get(data, "model"), "")?;
    let mut effort = text_or(get(data, "effort"), "")?;
    let mut route = None;
    for cell in matrix {
        let cell = cell.as_object().ok_or(Fail::Unsure)?;
        if is(get(cell, "id"), &route_id) {
            route = Some(cell.clone());
            break;
        }
    }
    let Some(route) = route else {
        return Err(py("route_unknown"));
    };
    let motor = get(&route, "motor").clone();
    let motors = obj(registry.get("motors").unwrap_or(&NULL))?;
    let spec = match (&motor, motors) {
        (Value::String(m), Some(motors)) => get(motors, m).clone(),
        (Value::Array(_) | Value::Object(_), _) => return Err(Fail::Unsure),
        _ => Value::Null,
    };
    let models: Vec<Value> = match obj_get(&spec, "models")? {
        Value::Array(models) => models.clone(),
        v if truthy(v) => return Err(Fail::Unsure),
        _ => Vec::new(),
    };
    if model.is_empty()
        && let Some(first) = models.first()
    {
        model = as_text(obj_get(first, "id")?)?;
    }
    let mut chosen = None;
    for m in &models {
        let m = m.as_object().ok_or(Fail::Unsure)?;
        if is(get(m, "id"), &model) {
            chosen = Some(m);
            break;
        }
    }
    if effort.is_empty()
        && let Some(chosen) = chosen.filter(|c| !c.is_empty())
    {
        let default = get(chosen, "defaultEffort");
        effort = if truthy(default) {
            as_text(default)?
        } else {
            match get(chosen, "efforts") {
                Value::Array(list) if !list.is_empty() => as_text(list.first().unwrap_or(&NULL))?,
                v if truthy(v) => return Err(Fail::Unsure),
                _ => String::new(),
            }
        };
    }
    let selection = json!({"routeId": route_id, "model": model, "effort": effort});
    match providers::validate_selection(registry, matrix, &selection, scope)? {
        Ok(_) => {}
        Err(code) if code.contains(" object has no attribute ") => return Err(Fail::Unsure),
        Err(code) => return Err(Fail::Py(code)),
    }
    let harness = get(&route, "harness").clone();
    let mut harness_alias = text_or(
        if truthy(get(data, "harnessAccount")) {
            get(data, "harnessAccount")
        } else {
            get(data, "account")
        },
        "main",
    )?;
    let same = python_eq(&motor, &harness);
    let mut motor_alias = if same {
        text_or(get(data, "motorAccount"), &harness_alias)?
    } else {
        text_or(get(data, "motorAccount"), "main")?
    };
    let caps = |name: &Value| -> Result<bool, Fail> {
        let harnesses = obj(registry.get("harnesses").unwrap_or(&NULL))?;
        let item = match (name, harnesses) {
            (Value::String(n), Some(h)) => get(h, n),
            _ => &NULL,
        };
        Ok(truthy(obj_get(obj_get(item, "capabilities")?, "accounts")?))
    };
    let selectable = |provider: &Value, alias: &str| -> Result<bool, Fail> {
        let provider = provider.as_str().ok_or(Fail::Unsure)?;
        match accounts::list_accounts(registry, provider, paths) {
            Ok(list) => Ok(list
                .iter()
                .find(|a| is(&a["alias"], alias))
                .is_some_and(|a| truthy(&a["selectable"]))),
            Err(e) if accounts::is_account_error(&e) => Err(py("account_invalid")),
            Err(_) => Err(Fail::Unsure),
        }
    };
    if caps(&harness)? {
        if !selectable(&harness, &harness_alias)? {
            return Err(py("harness_account_login_required"));
        }
    } else {
        harness_alias = "main".into();
    }
    let scopes = match get(&route, "accountScopes") {
        Value::Array(list) => list.clone(),
        v if truthy(v) => return Err(Fail::Unsure),
        _ => Vec::new(),
    };
    if same {
        motor_alias.clone_from(&harness_alias);
    } else if scopes.iter().any(|s| is(s, "motor")) && caps(&motor)? {
        if !selectable(&motor, &motor_alias)? {
            return Err(py("motor_account_login_required"));
        }
        if motor_alias != "main" && !is(get(&route, "driver"), "acp") {
            return Err(py("motor_account_gateway_not_ready"));
        }
    } else {
        motor_alias = "main".into();
    }
    let mut selected = Map::new();
    selected.insert("routeId".into(), s(&route_id));
    selected.insert("model".into(), s(&model));
    selected.insert("effort".into(), s(&effort));
    selected.insert("harnessAccount".into(), s(&harness_alias));
    selected.insert("motorAccount".into(), s(&motor_alias));
    Ok((route, selected))
}

/// `_claude_config_dir(alias)` (2389).
fn claude_config_dir(env: &Env, alias: &Value) -> Result<Option<String>, Fail> {
    let alias = as_text(alias)?;
    if alias.is_empty() || alias == "main" {
        return Ok(None);
    }
    let spec = obj_get(obj_get(&env.registry, "harnesses")?, "claude")?;
    let root = obj_get(spec, "accountsRoot")?;
    let root = if truthy(root) {
        as_text(root)?
    } else {
        "~/.claude-accounts".to_owned()
    };
    let root = expanduser(env, &root)?;
    if alias.starts_with('/') {
        return Ok(Some(alias));
    }
    Ok(Some(join(&root, &alias)))
}

// ------------------------------------------------------------ adaptador

/// Qué clase del Python representa: `SessionConfiguration` o su subclase
/// `PaneExtensionConfiguration` (`prepare`, `snapshot`, `_verify` y
/// `rollback` propios).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Session,
    Extensions,
}

/// El adaptador. `store` del Python es la conexión propia al journal.
#[derive(Clone)]
pub struct SessionConfiguration {
    kind: Kind,
    env: Env,
    data: Map<String, Value>,
    identity: Map<String, Value>,
    sess: String,
    pane: String,
    request_id: String,
    original: Option<Proc>,
    original_start: String,
    frm: String,
    plan: Option<Map<String, Value>>,
    /// El tipo de diálogo que `pending_confirmation` vio en pantalla.
    pub screen_dialog_kind: String,
    snapshot_record: Option<Map<String, Value>>,
    in_recovery: bool,
    refreshing: bool,
    allow_pending_recovery: bool,
    /// `verify` con otro número de intentos (el `monkeypatch` de las pruebas).
    attempts_override: Option<i64>,
    probing: bool,
}

fn key_text(data: &Map<String, Value>, key: &str) -> Result<String, Fail> {
    match data.get(key) {
        Some(Value::String(v)) => Ok(v.clone()),
        Some(_) => Err(Fail::Unsure),
        // `data['session']`: `KeyError` (quien llama ya lo validó).
        None => Err(Fail::Py(format!("'{key}'"))),
    }
}

impl SessionConfiguration {
    /// `__init__(data, identity)`: el agente del pane y su inicio.
    pub fn new(
        kind: Kind,
        data: Value,
        identity: Map<String, Value>,
        env: Env,
    ) -> Result<Self, Fail> {
        let Value::Object(data) = data else {
            return Err(Fail::Unsure);
        };
        let sess = key_text(&data, "session")?;
        let pane = key_text(&data, "pane")?;
        let request_id = match data.get("requestId") {
            Some(Value::String(id)) => id.clone(),
            _ => String::new(),
        };
        let original = agent_info_for_pane(&env, &pane)?;
        let original_start = original
            .as_ref()
            .filter(|o| o.pid != 0)
            .map(|o| agent_procs::process_start(&env.proc_root, o.pid))
            .unwrap_or_default();
        let pane_cmd = ident_text(&identity, "pane_current_command");
        let frm = match &original {
            Some(o) if !o.agent.is_empty() => o.agent.clone(),
            _ if SHELLS.contains(&pane_cmd) => "shell".to_owned(),
            _ => String::new(),
        };
        Ok(Self {
            kind,
            env,
            data,
            identity,
            sess,
            pane,
            request_id,
            original,
            original_start,
            frm,
            plan: None,
            screen_dialog_kind: String::new(),
            snapshot_record: None,
            in_recovery: false,
            refreshing: false,
            allow_pending_recovery: false,
            attempts_override: None,
            probing: false,
        })
    }

    /// El adaptador que arma `session_recover` (3387): el agente y el pid del
    /// snapshot, el plan guardado y si la fila esperaba confirmación.
    pub fn for_recovery(
        kind: Kind,
        request: Value,
        identity: Map<String, Value>,
        env: Env,
        snapshot: &Value,
        allow_pending_recovery: bool,
    ) -> Result<Self, Fail> {
        let mut adapter = Self::new(kind, request, identity, env)?;
        let origin = obj(snapshot.get("origin").unwrap_or(&NULL))?.ok_or(Fail::Unsure)?;
        let agent = text_or(get(origin, "agent"), "shell")?;
        let pid = match get(origin, "agent_pid") {
            Value::Number(n) => n.as_i64().ok_or(Fail::Unsure)?,
            Value::Null => 0,
            _ => return Err(Fail::Unsure),
        };
        adapter.original = Some(Proc {
            agent: agent.clone(),
            pid,
        });
        adapter.frm = agent;
        adapter.plan = Some(
            obj(snapshot.get("destination").unwrap_or(&NULL))?
                .ok_or(Fail::Unsure)?
                .clone(),
        );
        adapter.allow_pending_recovery = allow_pending_recovery;
        Ok(adapter)
    }

    /// `verify` con `attempts` intentos (pruebas: el `monkeypatch` del Python).
    #[must_use]
    pub fn with_verify_attempts(mut self, attempts: i64) -> Self {
        self.attempts_override = Some(attempts);
        self
    }

    pub fn frm(&self) -> &str {
        &self.frm
    }

    pub fn plan(&self) -> Option<&Map<String, Value>> {
        self.plan.as_ref()
    }

    /// Antes del `claim`: ¿podría el port reproducir esta operación? Repite
    /// `prepare` (sin efectos hasta `:2963`) y las sondas de las notas del
    /// controlador: inventario de extensiones, diálogos, confianza de carpeta
    /// y `verify_attempts`. `Err(Unsure)` = declinar. Un error del Python en
    /// `prepare` no declina: la operación se reclama y falla con ese texto.
    pub fn probe(&self) -> Result<(), Unsure> {
        let mut dry = self.clone();
        dry.probing = true;
        let plan = match dry.prepare_plan() {
            Ok(plan) => plan,
            Err(Fail::Py(_)) => return Ok(()),
            Err(Fail::Unsure) => return Err(Unsure),
        };
        let return_launch = plan
            .get("returnOrigin")
            .and_then(|o| o.get("extensionLaunch"))
            .filter(|v| truthy(v));
        let to = plan.get("to").and_then(Value::as_str).ok_or(Unsure)?;
        let need = extension_launch::InventoryNeed {
            extensions_only: self.kind == Kind::Extensions,
            from: &self.frm,
            to,
            same_conversation: plan.get("sameConversation").is_some_and(truthy),
            unchanged: plan.get("unchanged").is_some_and(truthy),
            original_pid: self
                .original
                .as_ref()
                .and_then(|o| u32::try_from(o.pid).ok()),
            return_origin_launch: return_launch,
        };
        if extension_launch::needs_inventory(&need)? {
            let account = plan
                .get("harnessAccount")
                .and_then(Value::as_str)
                .unwrap_or("main");
            let cwd = Path::new(ident_text(&self.identity, "pane_current_path"));
            match extension_launch::inventory(
                &self.env.registry,
                to,
                account,
                cwd,
                &self.extension_paths(),
            ) {
                Ok(_) | Err(extension_launch::LaunchError::Value(_)) => {}
                Err(_) => return Err(Unsure),
            }
        }
        self.env.dialogs.probe().map_err(|_| Unsure)?;
        let home = self.env.home.to_str().ok_or(Unsure)?;
        let cwd = ident_text(&self.identity, "pane_current_path");
        let from_alias = plan
            .get("previous")
            .and_then(|p| p.get("harnessAccount"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let to_alias = plan
            .get("harnessAccount")
            .and_then(Value::as_str)
            .unwrap_or("");
        match to {
            "claude" => {
                launch_command::trust_probe(&self.env.registry, home, from_alias, to_alias)?
            }
            "codex" => launch_command::codex_trust_probe(
                &self.env.registry,
                home,
                cwd,
                from_alias,
                to_alias,
            )?,
            _ => {}
        }
        let config_dir = claude_config_dir(&self.env, plan.get("harnessAccount").unwrap_or(&NULL))
            .map_err(|_| Unsure)?;
        dialogs::verify_attempts(&self.env.repo_root, cwd, config_dir.as_deref(), home)?;
        Ok(())
    }

    fn store_conn(&self) -> Result<Connection, Fail> {
        open_journal(&self.env.journal).map_err(Fail::from)
    }

    fn with_store<T>(
        &self,
        run: impl FnOnce(&OperationStore<'_>) -> ops::Result<T>,
    ) -> Result<T, Fail> {
        let conn = self.store_conn()?;
        let owner = self.env.owner;
        let owner = move || owner;
        let clock = self.env.clock.clone();
        let clock = move || clock();
        let store = OperationStore::new(&conn, &owner, &clock)?;
        Ok(run(&store)?)
    }

    fn plan_ref(&self) -> Result<&Map<String, Value>, Fail> {
        self.plan.as_ref().ok_or(Fail::Unsure)
    }

    fn original_pid(&self) -> i64 {
        self.original.as_ref().map_or(0, |o| o.pid)
    }

    // ------------------------------------------------------------ prepare

    fn prepare_plan(&mut self) -> Result<Map<String, Value>, Fail> {
        match self.kind {
            Kind::Session => self.prepare_session(),
            Kind::Extensions => self.prepare_extensions(),
        }
    }

    fn extension_paths(&self) -> crate::capabilities::Paths {
        let mut paths = crate::capabilities::Paths::new(&self.env.home, &self.env.cwd);
        paths.env = self
            .env
            .environ
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        paths
    }
    fn prepare_extensions(&mut self) -> Result<Map<String, Value>, Fail> {
        if self.data.get("reconnect").is_some_and(|v| !v.is_boolean()) {
            return Err(py("reconnect inválido"));
        }
        let reconnect = self.data.get("reconnect") == Some(&Value::Bool(true));
        let identity = pane_identity(&self.env, &self.sess, &self.pane)?;
        let info = agent_info_for_pane(&self.env, &self.pane)?;
        let observed = if let Some(info) = &info {
            observe_pane(&self.env, &self.sess, &self.pane, Some(info), None)?
        } else {
            Map::new()
        };
        let harness = match &info {
            Some(info) => info.agent.clone(),
            None => as_text(get(&self.data, "harness"))?,
        };
        if !["claude", "codex", "grok", "opencode", "agy"].contains(&harness.as_str()) {
            return Err(py("elige un CLI compatible para este panel"));
        }
        if info.is_some()
            && truthy(get(&self.data, "harness"))
            && !is(get(&self.data, "harness"), &harness)
        {
            return Err(py("el CLI del panel cambió"));
        }
        let operation = self.with_store(|store| store.latest_for_target(&self.sess, &self.pane))?;
        let active = operation.filter(|r| {
            r["pane_key"] == identity_key(&identity)
                && !matches!(
                    r["state"].as_str(),
                    Some("confirmed" | "failed" | "rolled_back")
                )
        });
        if info.is_none()
            && !SHELLS.contains(&ident_text(&identity, "pane_current_command"))
            && active.is_none()
        {
            return Err(py("hay un proceso sin identificar en el panel"));
        }
        let mut account = text_or(
            get(&observed, "harnessAccount"),
            if info.is_none() { "main" } else { "unknown" },
        )?;
        if account == "unknown"
            && let Some(operation) = &active
        {
            account = text_or(
                operation
                    .pointer("/snapshot/destination/harnessAccount")
                    .unwrap_or(&NULL),
                "unknown",
            )?;
        }
        if account == "unknown" {
            return Err(py("no se identificó la cuenta del proceso"));
        }
        let sid = as_text(get(&observed, "conversationId"))?;
        if !is(
            get(&self.data, "expectedIdentity"),
            &identity_key(&identity),
        ) || !self.data.contains_key("expectedConversationId")
            || !is(get(&self.data, "expectedConversationId"), &sid)
        {
            return Err(py(
                "cambió el panel o la conversación; vuelve a cargar el estante",
            ));
        }
        let cwd = PathBuf::from(ident_text(&self.identity, "pane_current_path"));
        let paths = self.extension_paths();
        let inv =
            extension_launch::inventory(&self.env.registry, &harness, &account, &cwd, &paths)?;
        let conn = self.store_conn()?;
        let clock = self.env.clock.clone();
        let clock = move || clock();
        let store = crate::pane_extensions::ExtensionStore::new(&conn, &clock)
            .map_err(|e| py(e.to_string()))?;
        let draft = store
            .require_revision(
                &key_text(&self.data, "extensionDraftKey")?,
                get(&self.data, "revision"),
            )
            .map_err(|e| py(e.to_string()))?;
        if draft["identity"] != identity_key(&self.identity)
            || draft["conversation"] != sid
            || draft["harness"] != harness
        {
            return Err(py("el borrador no pertenece a este panel y conversación"));
        }
        let desired = resolve_extension_draft(&inv, &draft["desired"])?;
        if !["shell", "claude", "codex", "grok", "opencode", "agy"].contains(&self.frm.as_str()) {
            return Err(py("CLI no compatible con extensiones por proceso"));
        }
        if self.frm != "shell" && sid.is_empty() {
            return Err(py(
                "no se encontró la conversación exacta; el agente sigue abierto",
            ));
        }
        let mut origin = Map::new();
        if self.frm != "shell" {
            let inspector = PaneInspector::new(&self.env.home, &self.env.proc_root)?;
            let pid = ident_text(&self.identity, "pane_pid")
                .parse()
                .map_err(|_| Fail::Unsure)?;
            origin = inspector.inspect(&PaneRef {
                id: &self.pane,
                pid,
                command: ident_text(&self.identity, "pane_current_command"),
            })?;
            if !is(get(&origin, "resume_id"), &sid) {
                return Err(py("la conversación cambió durante la preparación"));
            }
            if !self.probing {
                snapshot_transcript(&self.env, &origin)?;
            }
        }
        let bundle = if let Some(pid) = info.as_ref().and_then(|p| u32::try_from(p.pid).ok()) {
            extension_launch::launch_from_pid(pid)?
        } else {
            None
        };
        let loaded = bundle
            .as_ref()
            .filter(|b| b["harness"] == harness)
            .map(|b| b["selection"].clone());
        let launch = if self.probing {
            None
        } else {
            Some(extension_launch::prepare_launch_with_binding(
                &self.env.registry,
                &harness,
                &account,
                &cwd,
                &desired,
                &self.env.hooks.join("extension-launches"),
                &self.request_id,
                &paths,
                Some(&identity_key(&self.identity)),
            )?)
        };
        let environment = if self.frm == "opencode" && !self.probing {
            Some(extension_launch::capture_opencode_environment(
                &agent_procs::read_environ(&self.env.proc_root, self.original_pid()),
                &self.env.hooks.join("extension-launches"),
                &inv,
                bundle.is_some(),
            )?)
        } else {
            None
        };
        let flags = match get(&origin, "flags") {
            Value::Array(items) => items
                .iter()
                .map(|v| v.as_str().map(str::to_owned).ok_or(Fail::Unsure))
                .collect::<Result<Vec<_>, _>>()?,
            v if truthy(v) => return Err(Fail::Unsure),
            _ => vec![],
        };
        let confirmed = truthy(get(&observed, "confirmed"));
        let model = if confirmed {
            as_text(get(&observed, "model"))?
        } else {
            String::new()
        };
        let effort = if confirmed {
            as_text(get(&observed, "effort"))?
        } else {
            String::new()
        };
        let motor = text_or(get(&observed, "motor"), &harness)?;
        let mut command = launch_command::configuration_command(
            &self.env.ctx(),
            &harness,
            &motor,
            &model,
            &effort,
            &account,
            &sid,
            &flags,
            model.is_empty(),
        )?;
        if let Some(launch) = &launch {
            command = extension_launch::wrap_command(
                &command,
                launch,
                &extension_launch::require_helper(&self.env.home)?,
                &self.env.environ,
            )?;
        }
        if let Some(environment) = &environment {
            command = extension_launch::wrap_environment(
                &command,
                environment,
                &extension_launch::require_helper(&self.env.home)?,
            )?;
        }
        let unchanged = !reconnect
            && bundle.is_some()
            && loaded.as_ref().is_some_and(|v| python_eq(v, &desired));
        let mut plan=json!({"opencodeEnvironment":environment,"to":harness,"motor":motor,"model":model,"effort":effort,"harnessAccount":account,"motorAccount":or_text(get(&observed,"motorAccount"),&account),"command":command,"previous":observed,"routeId":format!("{harness}:{motor}"),"expectedSid":sid,"sameConversation":self.frm!="shell","extensionsOnly":true,"extensionLaunch":launch,"unchanged":unchanged,"continuity":{"continuity":if sid.is_empty(){"new-conversation"}else{"resumed"},"handoffRequired":false}}).as_object().cloned().ok_or(Fail::Unsure)?;
        if unchanged {
            plan.insert("extensionLaunch".into(), bundle.unwrap_or(Value::Null));
        }
        self.plan = Some(plan.clone());
        Ok(plan)
    }

    fn prepare_session(&mut self) -> Result<Map<String, Value>, Fail> {
        if self.frm.is_empty() {
            return Err(py("no se reconoce el proceso del panel"));
        }
        if !matches!(
            self.frm.as_str(),
            "shell" | "claude" | "codex" | "grok" | "acp"
        ) {
            return Err(py(format!(
                "{} no ofrece recuperación exacta; se conserva el agente actual",
                self.frm
            )));
        }
        let data = self.data.clone();
        let current = match &self.original {
            Some(original) => {
                observe_pane(&self.env, &self.sess, &self.pane, Some(original), None)?
            }
            None => Map::new(),
        };
        if truthy(get(&data, "profileId")) {
            return Err(py(
                "los perfiles se aplican al crear una sesión; no se cambia el agente actual",
            ));
        }
        let key = identity_key(&self.identity);
        if truthy(get(&data, "expectedIdentity"))
            && !python_eq(get(&data, "expectedIdentity"), &s(&key))
        {
            return Err(py(
                "cambió la identidad del panel desde que se abrió la configuración",
            ));
        }
        if truthy(get(&data, "expectedConversationId"))
            && !python_eq(
                get(&data, "expectedConversationId"),
                get(&current, "conversationId"),
            )
        {
            return Err(py(
                "cambió la conversación desde que se abrió la configuración",
            ));
        }
        if truthy(get(&data, "accountOnly")) {
            return self.prepare_account_only(current);
        }
        let registry = self.env.registry.clone();
        if self.frm == "acp" && !acp_resume(&registry, get(&current, "motor"))? {
            return Err(py(
                "este motor ACP no admite reanudación exacta; el agente sigue abierto",
            ));
        }
        let route_arg = text_or(get(&data, "routeId"), "")?;
        let requested_route = route_for(&registry, &route_arg)?.unwrap_or_default();
        if truthy(get(&data, "routeId")) && requested_route.is_empty() {
            return Err(py("ruta desconocida"));
        }
        // `data.get('toHarness') or requested_route.get('harness') or self.frm`.
        let to = if truthy(get(&data, "toHarness")) {
            get(&data, "toHarness").clone()
        } else if truthy(get(&requested_route, "harness")) {
            get(&requested_route, "harness").clone()
        } else {
            s(&self.frm)
        };
        if !requested_route.is_empty()
            && (!python_eq(&to, get(&requested_route, "harness"))
                || (truthy(get(&data, "motor"))
                    && !python_eq(get(&data, "motor"), get(&requested_route, "motor"))))
        {
            return Err(py("la ruta no coincide con el harness y motor solicitados"));
        }
        let model_motor =
            providers::engine_for_model(&registry, &text_or(get(&data, "model"), "")?)?;
        let same_harness = python_eq(&to, &s(&self.frm));
        let mut motor = if truthy(get(&data, "motor")) {
            get(&data, "motor").clone()
        } else if truthy(get(&requested_route, "motor")) {
            get(&requested_route, "motor").clone()
        } else if same_harness {
            if model_motor.is_empty() {
                get(&current, "motor").clone()
            } else {
                s(&model_motor)
            }
        } else {
            to.clone()
        };
        if is(&to, "acp") && (motor.is_null() || ["acp", "shell", ""].iter().any(|m| is(&motor, m)))
        {
            motor = s("claude");
        }
        // Desde aquí `to` es uno de los seis harnesses (texto).
        let to_text = match &to {
            Value::String(t)
                if matches!(
                    t.as_str(),
                    "claude" | "codex" | "grok" | "acp" | "opencode" | "agy"
                ) =>
            {
                t.clone()
            }
            other => return Err(py(format!("harness desconocido: {}", py_text(other)?))),
        };
        if python_eq(&to, &motor)
            && truthy(get(&data, "motorAccount"))
            && truthy(get(&data, "harnessAccount"))
            && !python_eq(get(&data, "motorAccount"), get(&data, "harnessAccount"))
        {
            return Err(py(
                "un CLI nativo necesita la misma cuenta de harness y motor",
            ));
        }
        let motor_text = py_text(&motor)?;
        let same_motor = python_eq(&motor, get(&current, "motor"));
        let mut selection = data.clone();
        selection.insert("routeId".into(), s(&format!("{to_text}:{motor_text}")));
        if !truthy(get(&selection, "model")) && same_motor {
            selection.insert("model".into(), or_empty(get(&current, "model")));
        }
        if !selection.contains_key("effort") && same_motor {
            selection.insert("effort".into(), or_empty(get(&current, "effort")));
        }
        if !selection.contains_key("harnessAccount") {
            let value = if same_harness {
                get(&current, "harnessAccount").clone()
            } else {
                s("main")
            };
            selection.insert("harnessAccount".into(), value);
        }
        if !selection.contains_key("motorAccount") {
            let value = if same_motor {
                get(&current, "motorAccount").clone()
            } else if python_eq(&motor, &to) {
                get(&selection, "harnessAccount").clone()
            } else {
                s("main")
            };
            selection.insert("motorAccount".into(), value);
        }
        let scope = if !same_harness {
            "new_session"
        } else if same_motor {
            "session_model"
        } else {
            "session_motor"
        };
        let (route, selected) = resolve_route_selection(&self.env, &selection, scope)?;
        for account_key in ["harnessAccount", "motorAccount"] {
            if truthy(get(&data, account_key))
                && !python_eq(get(&selected, account_key), get(&data, account_key))
            {
                return Err(py(format!(
                    "{account_key}: esa cuenta no es compatible con el destino"
                )));
            }
        }
        if matches!(to_text.as_str(), "opencode" | "agy")
            || (to_text == "acp" && !acp_resume(&registry, &motor)?)
        {
            return Err(py(format!(
                "{to_text}:{motor_text} no ofrece recuperación exacta en este adaptador; crea una sesión nueva"
            )));
        }
        let route_id = as_text(get(&route, "id"))?;
        if let Some(support) = change_support(&registry, &current, &route_id)?
            && !truthy(support.get("selectable").unwrap_or(&NULL))
        {
            let message = support
                .get("reason")
                .and_then(|r| r.get("message"))
                .ok_or(Fail::Unsure)?;
            return Err(py(as_text(message)?));
        }
        let model = as_text(get(&selected, "model"))?;
        let effort = as_text(get(&selected, "effort"))?;
        let account = if to_text == "acp" {
            as_text(get(&selected, "motorAccount"))?
        } else {
            as_text(get(&selected, "harnessAccount"))?
        };
        let same_conversation = self.frm == to_text && (to_text != "acp" || same_motor);
        let mut saved_origin = if same_conversation {
            None
        } else {
            self.with_store(|store| store.saved_origin(&key, &to_text))?
        };
        if let Some(saved) = &saved_origin
            && to_text == "acp"
            && !python_eq(
                obj_get(saved.get("observed").unwrap_or(&NULL), "motor")?,
                &motor,
            )
        {
            saved_origin = None;
        }
        if let Some(saved) = &saved_origin {
            snapshot_transcript(&self.env, saved.as_object().ok_or(Fail::Unsure)?)?;
        }
        let sid = match &saved_origin {
            Some(saved) => as_text(obj_get(
                saved.get("observed").unwrap_or(&NULL),
                "conversationId",
            )?)?,
            None => as_text(get(&current, "conversationId"))?,
        };
        let resumes = same_conversation || saved_origin.is_some();
        let flags: Vec<String> = match saved_origin
            .as_ref()
            .map(|o| obj_get(o, "flags"))
            .transpose()?
        {
            Some(Value::Array(list)) => list
                .iter()
                .map(|f| f.as_str().map(str::to_owned).ok_or(Fail::Unsure))
                .collect::<Result<_, _>>()?,
            Some(v) if truthy(v) => return Err(Fail::Unsure),
            _ => Vec::new(),
        };
        // Validate the target command now, before a snapshot or any terminal input.
        let cmd = launch_command::configuration_command(
            &self.env.ctx(),
            &to_text,
            &motor_text,
            &model,
            &effort,
            &account,
            if resumes { &sid } else { "" },
            &flags,
            false,
        )?;
        if !fs::metadata(ident_text(&self.identity, "pane_current_path")).is_ok_and(|m| m.is_dir())
        {
            return Err(py("la carpeta del panel ya no existe"));
        }
        let mut plan = Map::new();
        plan.insert("to".into(), s(&to_text));
        plan.insert("motor".into(), motor.clone());
        plan.insert("model".into(), s(&model));
        plan.insert("effort".into(), s(&effort));
        plan.insert(
            "harnessAccount".into(),
            get(&selected, "harnessAccount").clone(),
        );
        plan.insert(
            "motorAccount".into(),
            get(&selected, "motorAccount").clone(),
        );
        plan.insert("command".into(), s(&cmd));
        plan.insert("previous".into(), Value::Object(current.clone()));
        plan.insert("routeId".into(), s(&route_id));
        plan.insert(
            "returnOrigin".into(),
            saved_origin.clone().unwrap_or(Value::Null),
        );
        plan.insert("expectedSid".into(), s(if resumes { &sid } else { "" }));
        plan.insert("sameConversation".into(), Value::Bool(same_conversation));
        let unchanged = self.frm == to_text
            && ["motor", "model", "effort", "harnessAccount", "motorAccount"]
                .iter()
                .all(|k| python_eq(get(&plan, k), get(&current, k)));
        plan.insert("unchanged".into(), Value::Bool(unchanged));
        let handoff = self.frm != "shell" && !same_conversation && saved_origin.is_none();
        plan.insert(
            "continuity".into(),
            self.continuity(
                if resumes {
                    "resumed"
                } else {
                    "new-conversation"
                },
                handoff,
            )?,
        );
        self.preserve_extension_plan(&mut plan)?;
        self.plan = Some(plan.clone());
        Ok(plan)
    }

    /// `{'continuity': …, 'handoffRequired': …}`. Con traspaso, `handoffPath`
    /// va ya aquí (el Python lo añade en `snapshot` al crear el archivo): la
    /// ruta es fija (`H/session-handoffs/<requestId>.md`) y `run_operation` de
    /// Rust copia la continuidad del plan de `prepare`, no la de `snapshot`.
    /// Solo se lee en resultados posteriores a un `snapshot` que creó ese
    /// archivo (o falló antes): mismo contenido y orden de claves.
    fn continuity(&self, kind: &str, handoff: bool) -> Result<Value, Fail> {
        let mut out = Map::new();
        out.insert("continuity".into(), s(kind));
        out.insert("handoffRequired".into(), Value::Bool(handoff));
        if handoff {
            out.insert("handoffPath".into(), s(&self.handoff_path()?));
        }
        Ok(Value::Object(out))
    }

    fn handoff_path(&self) -> Result<String, Fail> {
        let dir = self.env.hooks.join("session-handoffs");
        let path = dir.join(format!("{}.md", self.request_id));
        path.into_os_string()
            .into_string()
            .map_err(|_| Fail::Unsure)
    }

    /// `_prepare_account_only(current)` (2978): misma conversación, modelo y
    /// esfuerzo tal cual; solo cambia la cuenta. No valida el modelo contra el
    /// registro.
    fn prepare_account_only(
        &mut self,
        current: Map<String, Value>,
    ) -> Result<Map<String, Value>, Fail> {
        if self.frm == "shell" {
            return Err(py(
                "no hay un agente en el panel; abre uno antes de cambiar de cuenta",
            ));
        }
        if self.frm == "acp" {
            return Err(py(
                "cambiar cuenta ACP requiere exportación de conversación; se conserva el origen",
            ));
        }
        let alias = text_or(get(&self.data, "harnessAccount"), "main")?;
        let list = match accounts::list_accounts(&self.env.registry, &self.frm, &self.env.paths()) {
            Ok(list) => list,
            Err(e) if accounts::is_account_error(&e) => return Err(Fail::Py(e.0)),
            Err(_) => return Err(Fail::Unsure),
        };
        let chosen = list.iter().find(|a| is(&a["alias"], &alias));
        if !chosen.is_some_and(|a| truthy(&a["selectable"])) {
            return Err(py(format!(
                "la cuenta {alias} no tiene login en {}",
                self.frm
            )));
        }
        let sid = as_text(get(&current, "conversationId"))?;
        if sid.is_empty() {
            return Err(py(
                "no se encontró la conversación exacta; el agente sigue abierto",
            ));
        }
        let motor = text_or(get(&current, "motor"), &self.frm)?;
        let model = if truthy(get(&current, "confirmed")) {
            launch_command::launch_model_id(
                &self.env.registry,
                &motor,
                &as_text(get(&current, "model"))?,
            )?
        } else {
            String::new()
        };
        let effort = if model.is_empty() {
            String::new()
        } else {
            as_text(get(&current, "effort"))?
        };
        let command = launch_command::configuration_command(
            &self.env.ctx(),
            &self.frm,
            &motor,
            &model,
            &effort,
            &alias,
            &sid,
            &[],
            model.is_empty(),
        )?;
        if !fs::metadata(ident_text(&self.identity, "pane_current_path")).is_ok_and(|m| m.is_dir())
        {
            return Err(py("la carpeta del panel ya no existe"));
        }
        let motor_account = if motor == self.frm {
            s(&alias)
        } else {
            or_text(get(&current, "motorAccount"), "main")
        };
        let mut plan = Map::new();
        plan.insert("to".into(), s(&self.frm));
        plan.insert("motor".into(), s(&motor));
        plan.insert("model".into(), s(&model));
        plan.insert("effort".into(), s(&effort));
        plan.insert("harnessAccount".into(), s(&alias));
        plan.insert("motorAccount".into(), motor_account);
        plan.insert("command".into(), s(&command));
        plan.insert("previous".into(), Value::Object(current.clone()));
        plan.insert("routeId".into(), s(&format!("{}:{motor}", self.frm)));
        plan.insert("returnOrigin".into(), Value::Null);
        plan.insert("expectedSid".into(), s(&sid));
        plan.insert("sameConversation".into(), Value::Bool(true));
        plan.insert("accountOnly".into(), Value::Bool(true));
        plan.insert(
            "unchanged".into(),
            Value::Bool(python_eq(get(&current, "harnessAccount"), &s(&alias))),
        );
        plan.insert("continuity".into(), self.continuity("resumed", false)?);
        self.preserve_extension_plan(&mut plan)?;
        self.plan = Some(plan.clone());
        Ok(plan)
    }

    /// Preserve verified extension selections across account/model changes.
    /// The pre-claim probe validates mapping without producing private files.
    fn preserve_extension_plan(&self, plan: &mut Map<String, Value>) -> Result<(), Fail> {
        let mut prior = obj_get(get(plan, "returnOrigin"), "extensionLaunch")?.clone();
        let pid = self.original_pid();
        if truthy(get(plan, "sameConversation")) && pid != 0 {
            let upid = u32::try_from(pid).map_err(|_| Fail::Unsure)?;
            prior = extension_launch::launch_from_pid(upid)?.unwrap_or(Value::Null);
            let manifest = agent_procs::read_environ(&self.env.proc_root, pid)
                .get(extension_launch::MANIFEST_ENV.as_bytes())
                .is_some_and(|v| !v.is_empty());
            if manifest && !truthy(&prior) {
                return Err(py(
                    "no se pudo verificar el lanzamiento original; el agente sigue abierto",
                ));
            }
        }
        if !truthy(&prior) || !python_eq(obj_get(&prior, "harness")?, get(plan, "to")) {
            return Ok(());
        }
        if truthy(get(plan, "unchanged")) {
            plan.insert("extensionLaunch".into(), prior);
            return Ok(());
        }
        let previous = obj_get(get(plan, "returnOrigin"), "observed")?;
        let previous = if truthy(previous) {
            previous
        } else {
            get(plan, "previous")
        };
        let previous = obj(previous)?.ok_or(Fail::Unsure)?;
        let harness = as_text(get(plan, "to"))?;
        let account = text_or(get(plan, "harnessAccount"), "main")?;
        let before_account = text_or(get(previous, "harnessAccount"), "main")?;
        let cwd = Path::new(ident_text(&self.identity, "pane_current_path"));
        let paths = self.extension_paths();
        let source = extension_launch::inventory(
            &self.env.registry,
            &harness,
            &before_account,
            cwd,
            &paths,
        )?;
        let dest =
            extension_launch::inventory(&self.env.registry, &harness, &account, cwd, &paths)?;
        let mut choices = Map::new();
        for kind in ["mcps", "skills"] {
            let before = source[kind].as_array().ok_or(Fail::Unsure)?;
            let after = dest[kind].as_array().ok_or(Fail::Unsure)?;
            let mut selected = Map::new();
            for row in after {
                if truthy(&row["toggleable"]) {
                    selected.insert(
                        row["id"].as_str().ok_or(Fail::Unsure)?.into(),
                        Value::Bool(false),
                    );
                }
            }
            let prior_selection = prior["selection"][kind].as_object().ok_or(Fail::Unsure)?;
            for (id, enabled) in prior_selection {
                let mut row = after.iter().find(|r| r["id"] == *id);
                if row.is_none()
                    && let Some(original) = before.iter().find(|r| r["id"] == *id)
                {
                    let matches = after
                        .iter()
                        .filter(|r| {
                            ["name", "scope", "plugin"]
                                .iter()
                                .all(|k| python_eq(&r[*k], &original[*k]))
                        })
                        .collect::<Vec<_>>();
                    if matches.len() == 1 {
                        row = matches.first().copied();
                    }
                }
                if let Some(row) = row {
                    selected.insert(
                        row["id"].as_str().ok_or(Fail::Unsure)?.into(),
                        enabled.clone(),
                    );
                } else if truthy(enabled) {
                    return Err(py(
                        "una extensión activa no existe en la cuenta destino; revisa el estante",
                    ));
                }
            }
            choices.insert(kind.into(), Value::Object(selected));
        }
        let choices = Value::Object(choices);
        extension_launch::normalize(&dest, &choices)?;
        if self.probing {
            return Ok(());
        }
        let launch = extension_launch::prepare_launch_with_binding(
            &self.env.registry,
            &harness,
            &account,
            cwd,
            &choices,
            &self.env.hooks.join("extension-launches"),
            &self.request_id,
            &paths,
            Some(&identity_key(&self.identity)),
        )?;
        let command = extension_launch::wrap_command(
            &as_text(get(plan, "command"))?,
            &launch,
            &extension_launch::require_helper(&self.env.home)?,
            &self.env.environ,
        )?;
        plan.insert("extensionLaunch".into(), launch);
        plan.insert("command".into(), s(&command));
        Ok(())
    }

    // ------------------------------------------------------------ espera

    fn check_identity_inner(&mut self) -> Result<(), Fail> {
        let row = self.with_store(|store| store.get(&self.request_id))?;
        if row.as_ref().is_some_and(|r| is(&r["state"], "failed")) {
            return Err(py("operación cancelada"));
        }
        let current = pane_identity(&self.env, &self.sess, &self.pane)?;
        if identity_key(&current) != identity_key(&self.identity) {
            return Err(py("cambió la identidad del panel; no se enviaron comandos"));
        }
        let live = agent_info_for_pane(&self.env, &self.pane)?;
        let live_pid = live.as_ref().map_or(0, |l| l.pid);
        if live_pid != self.original_pid()
            || (live_pid != 0
                && agent_procs::process_start(&self.env.proc_root, live_pid) != self.original_start)
        {
            return Err(py("el proceso original cambió durante la espera"));
        }
        Ok(())
    }

    fn wait_idle_inner(&mut self) -> Result<(), Fail> {
        if truthy(get(&self.data, "interrupt")) || self.frm == "shell" {
            return Ok(());
        }
        for _ in 0..1350 {
            self.check_identity_inner()?;
            // Una duda cuenta como «ocupado»: se sigue esperando dentro del tope
            // de 45 minutos (la fila sigue en `waiting`, cancelable); nunca se
            // cierra un agente que quizá trabaja. Un error del Python se propaga.
            let busy = match harness_pane_busy(&self.env, &self.sess, &self.pane, &self.frm) {
                Ok(busy) => busy,
                Err(Fail::Unsure) => true,
                Err(error) => return Err(error),
            };
            if !busy {
                return Ok(());
            }
            self.env.sleep_s(2.0);
        }
        Err(py("el agente no quedó libre en 45 minutos"))
    }

    // ----------------------------------------------------------- snapshot

    /// `capture_session(tmux, sess, inspector)` y el pane pedido.
    fn layout(&self, inspector: &PaneInspector) -> Result<(Value, Map<String, Value>), Fail> {
        let env = self.env.clone();
        let mut tmux = move |args: &[&str]| -> tmux_snapshot::Result<TmuxResult> {
            (env.tmux)(args).map_err(SnapshotError::Runtime)
        };
        let layout = tmux_snapshot::capture_session(&mut tmux, &self.sess, inspector)?;
        let mut origin = None;
        for window in layout
            .get("windows")
            .and_then(Value::as_array)
            .ok_or(Fail::Unsure)?
        {
            for pane in window
                .get("panes")
                .and_then(Value::as_array)
                .ok_or(Fail::Unsure)?
            {
                if is(pane.get("id").unwrap_or(&NULL), &self.pane) {
                    origin = pane.as_object().cloned();
                    break;
                }
            }
            if origin.is_some() {
                break;
            }
        }
        // `next(...)` sin valor por omisión: `StopIteration` (texto vacío).
        let origin = origin.ok_or_else(|| py(""))?;
        Ok((layout, origin))
    }

    fn snapshot_session(&mut self) -> Result<Value, Fail> {
        let mut plan = self.plan.clone().ok_or(Fail::Unsure)?;
        let inspector = PaneInspector::new(&self.env.home, &self.env.proc_root)?;
        let (layout, mut origin) = self.layout(&inspector)?;
        let observed = match &self.original {
            Some(original) => observe_pane(
                &self.env,
                &self.sess,
                &self.pane,
                Some(original),
                Some(&inspector),
            )?,
            None => Map::new(),
        };
        let account_only = truthy(get(&plan, "accountOnly"));
        let frm = self.frm.clone();
        let previous = obj(get(&plan, "previous"))?.cloned().unwrap_or_default();
        if frm != "shell" {
            let fields: &[&str] = if account_only {
                &["harness", "harnessAccount", "conversationId"]
            } else {
                &[
                    "harness",
                    "motor",
                    "model",
                    "effort",
                    "harnessAccount",
                    "motorAccount",
                    "conversationId",
                ]
            };
            if fields
                .iter()
                .any(|k| !python_eq(get(&observed, k), get(&previous, k)))
            {
                return Err(py(
                    "cambió la conversación o configuración durante la espera; vuelve a revisar el cambio",
                ));
            }
            if account_only {
                let model = if truthy(get(&observed, "confirmed")) {
                    launch_command::launch_model_id(
                        &self.env.registry,
                        &text_or(get(&observed, "motor"), &frm)?,
                        &as_text(get(&observed, "model"))?,
                    )?
                } else {
                    String::new()
                };
                let effort = if model.is_empty() {
                    String::new()
                } else {
                    as_text(get(&observed, "effort"))?
                };
                plan.insert("model".into(), s(&model));
                plan.insert("effort".into(), s(&effort));
            } else {
                if !truthy(get(&observed, "confirmed")) || !truthy(get(&observed, "model")) {
                    return Err(py(
                        "no se observó la configuración original; el agente sigue abierto",
                    ));
                }
                let motor = as_text(get(&observed, "motor"))?;
                let spec = providers::model_spec(
                    &self.env.registry,
                    &motor,
                    &as_text(get(&observed, "model"))?,
                    "motors",
                )?
                .unwrap_or(Value::Null);
                if truthy(obj_get(&spec, "efforts")?) && !truthy(get(&observed, "effort")) {
                    return Err(py(
                        "no se observó el esfuerzo original; no se puede asegurar su recuperación",
                    ));
                }
            }
            if ["harnessAccount", "motorAccount"]
                .iter()
                .any(|k| !truthy(get(&observed, k)) || is(get(&observed, k), "unknown"))
            {
                return Err(py(
                    "no se identificaron las cuentas originales; el agente sigue abierto",
                ));
            }
            if !is(get(&origin, "agent"), &frm) {
                return Err(py("cambió el agente durante el snapshot"));
            }
        }
        let sid = if truthy(get(&origin, "resume_id")) {
            as_text(get(&origin, "resume_id"))?
        } else {
            as_text(obj_get(get(&origin, "acp"), "sessionId")?)?
        };
        if frm != "shell" && sid.is_empty() {
            return Err(py(
                "no se encontró la conversación exacta; el agente sigue abierto",
            ));
        }
        if frm != "shell" && !python_eq(&s(&sid), get(&observed, "conversationId")) {
            return Err(py("la conversación cambió durante el snapshot"));
        }
        let transcript = snapshot_transcript(&self.env, &origin)?;
        origin.insert("transcriptPath".into(), s(&transcript));
        origin.insert("observed".into(), Value::Object(observed.clone()));
        let pid = self.original_pid();
        origin.insert(
            "agent_pid".into(),
            if pid == 0 {
                Value::Null
            } else {
                Value::from(pid)
            },
        );
        origin.insert("agent_start".into(), s(&self.original_start));
        origin.insert("identity".into(), Value::Object(self.identity.clone()));
        let launch = if pid != 0 {
            let upid = u32::try_from(pid).map_err(|_| Fail::Unsure)?;
            // Una duda aquí no es `None`: sin ella la recuperación relanzaría el
            // agente sin sus extensiones (nota del controlador).
            extension_launch::launch_from_pid(upid)?.unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        origin.insert("extensionLaunch".into(), launch);
        let helper = if frm == "opencode"
            || truthy(&get(&plan, "returnOrigin")["opencodeEnvironment"])
            || truthy(get(&origin, "extensionLaunch"))
            || truthy(get(&plan, "extensionLaunch"))
            || truthy(get(&plan, "opencodeEnvironment"))
            || truthy(get(&origin, "opencodeEnvironment"))
        {
            extension_launch::require_helper(&self.env.home)?
        } else {
            extension_launch::helper_for_home(&self.env.home)
        };
        let return_origin = get(&plan, "returnOrigin").clone();
        if truthy(&return_origin) {
            let to_acp = is(get(&plan, "to"), "acp");
            let account_key = if to_acp {
                "motorAccount"
            } else {
                "harnessAccount"
            };
            let account = get(&plan, account_key).clone();
            let prior_account = obj_get(obj_get(&return_origin, "observed")?, account_key)?;
            if !python_eq(&account, prior_account) {
                let prior = return_origin.as_object().ok_or(Fail::Unsure)?;
                self.copy_conversation(prior, &as_text(&account)?)?;
            }
        }
        if frm != "shell" {
            let account_key = if frm == "acp" {
                "motorAccount"
            } else {
                "harnessAccount"
            };
            let account = as_text(get(&observed, account_key))?;
            if account.is_empty() || account == "unknown" {
                return Err(py("no se pudo identificar la cuenta original"));
            }
            let (origin_model, origin_effort) = if account_only {
                (
                    as_text(get(&plan, "model"))?,
                    as_text(get(&plan, "effort"))?,
                )
            } else {
                (
                    as_text(get(&observed, "model"))?,
                    as_text(get(&observed, "effort"))?,
                )
            };
            let flags: Vec<String> = match get(&origin, "flags") {
                Value::Array(list) => list
                    .iter()
                    .map(|f| f.as_str().map(str::to_owned).ok_or(Fail::Unsure))
                    .collect::<Result<_, _>>()?,
                v if truthy(v) => return Err(Fail::Unsure),
                _ => Vec::new(),
            };
            let ctx = self.env.ctx();
            let resume = launch_command::configuration_command(
                &ctx,
                &frm,
                &text_or(get(&observed, "motor"), &frm)?,
                &origin_model,
                &origin_effort,
                &account,
                &sid,
                &flags,
                account_only && origin_model.is_empty(),
            )?;
            origin.insert("resume_command".into(), s(&resume));
            if truthy(get(&plan, "sameConversation")) {
                let target_key = if frm == "acp" {
                    "motorAccount"
                } else {
                    "harnessAccount"
                };
                let target_account = as_text(get(&plan, target_key))?;
                if target_account != account {
                    self.copy_conversation(&origin, &target_account)?;
                }
                let model = as_text(get(&plan, "model"))?;
                let command = launch_command::configuration_command(
                    &ctx,
                    &as_text(get(&plan, "to"))?,
                    &py_text(get(&plan, "motor"))?,
                    &model,
                    &as_text(get(&plan, "effort"))?,
                    &target_account,
                    &sid,
                    &flags,
                    account_only && model.is_empty(),
                )?;
                plan.insert("command".into(), s(&command));
                plan.insert("expectedSid".into(), s(&sid));
            }
            if truthy(get(&origin, "extensionLaunch")) {
                let wrapped = extension_launch::wrap_command(
                    &as_text(get(&origin, "resume_command"))?,
                    get(&origin, "extensionLaunch"),
                    &helper,
                    &self.env.environ,
                )?;
                origin.insert("resume_command".into(), s(&wrapped));
            }
            if truthy(get(&plan, "extensionLaunch")) && truthy(get(&plan, "sameConversation")) {
                let wrapped = extension_launch::wrap_command(
                    &as_text(get(&plan, "command"))?,
                    get(&plan, "extensionLaunch"),
                    &helper,
                    &self.env.environ,
                )?;
                plan.insert("command".into(), s(&wrapped));
            }
            if frm == "opencode" {
                let account = as_text(get(&observed, "harnessAccount"))?;
                let cwd = Path::new(ident_text(&self.identity, "pane_current_path"));
                let inv = extension_launch::inventory(
                    &self.env.registry,
                    "opencode",
                    &account,
                    cwd,
                    &self.extension_paths(),
                )?;
                let environment = extension_launch::capture_opencode_environment(
                    &agent_procs::read_environ(&self.env.proc_root, self.original_pid()),
                    &self.env.hooks.join("extension-launches"),
                    &inv,
                    truthy(get(&origin, "extensionLaunch")),
                )?;
                origin.insert("opencodeEnvironment".into(), environment.clone());
                let resume = extension_launch::wrap_environment(
                    &as_text(get(&origin, "resume_command"))?,
                    &environment,
                    &helper,
                )?;
                origin.insert("resume_command".into(), s(&resume));
                if truthy(get(&plan, "sameConversation")) {
                    plan.insert("opencodeEnvironment".into(), environment.clone());
                    let command = extension_launch::wrap_environment(
                        &as_text(get(&plan, "command"))?,
                        &environment,
                        &helper,
                    )?;
                    plan.insert("command".into(), s(&command));
                }
            }
            let return_environment = obj_get(&return_origin, "opencodeEnvironment")?;
            if truthy(return_environment) && is(get(&plan, "to"), "opencode") {
                let wrapped = extension_launch::wrap_environment(
                    &as_text(get(&plan, "command"))?,
                    return_environment,
                    &helper,
                )?;
                plan.insert("command".into(), s(&wrapped));
            }
            let path = self.handoff_path()?;
            let dir = self.env.hooks.join("session-handoffs");
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&dir)
                .map_err(|e| Fail::Py(io_text_path(&e, &dir.to_string_lossy())))?;
            // `open(path, 'x')` y `chmod 0600`, sin la ventana del Python: el
            // archivo nace 0600 (`O_CREAT|O_EXCL` no sigue un enlace ni pisa
            // nada) y el `chmod` va por descriptor (`fchmod`), no por ruta.
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(|e| Fail::Py(io_text_path(&e, &path)))?;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|e| Fail::Py(io_text_path(&e, &path)))?;
            let claude = frm == "claude";
            let text = pane_exit::capture_handoff(
                &self.env,
                &self.pane,
                &frm,
                &as_text(get(&plan, "to"))?,
                &as_text(get(&origin, "cwd"))?,
                if claude { &sid } else { "" },
                if claude { &transcript } else { "" },
            )?;
            use std::io::Write;
            file.write_all(text.as_bytes())
                .map_err(|e| Fail::Py(io_text(&e)))?;
            origin.insert("handoffPath".into(), s(&path));
        }
        self.plan = Some(plan.clone());
        let mut snapshot = Map::new();
        snapshot.insert("session".into(), s(&self.sess));
        snapshot.insert("layout".into(), layout);
        snapshot.insert("origin".into(), Value::Object(origin));
        snapshot.insert("destination".into(), Value::Object(plan));
        Ok(Value::Object(snapshot))
    }

    /// `PaneExtensionConfiguration.snapshot` (3253).
    fn snapshot_extensions(&mut self) -> Result<Value, Fail> {
        let plan = self.plan.clone().ok_or(Fail::Unsure)?;
        let inspector = PaneInspector::new(&self.env.home, &self.env.proc_root)?;
        let (layout, mut origin) = self.layout(&inspector)?;
        let observed = match &self.original {
            Some(original) => observe_pane(
                &self.env,
                &self.sess,
                &self.pane,
                Some(original),
                Some(&inspector),
            )?,
            None => Map::new(),
        };
        let sid = as_text(get(&origin, "resume_id"))?;
        let previous = obj(get(&plan, "previous"))?.cloned().unwrap_or_default();
        if self.frm != "shell" {
            if !is(get(&origin, "agent"), &self.frm)
                || sid.is_empty()
                || !python_eq(&s(&sid), get(&plan, "expectedSid"))
                || !python_eq(get(&observed, "conversationId"), &s(&sid))
            {
                return Err(py(
                    "la conversación cambió durante la espera; el origen sigue abierto",
                ));
            }
            if ["model", "effort", "motor", "confirmed"]
                .iter()
                .any(|k| !python_eq(get(&observed, k), get(&previous, k)))
            {
                return Err(py(
                    "la configuración cambió durante la espera; el origen sigue abierto",
                ));
            }
            if truthy(get(&plan, "opencodeEnvironment")) {
                let cwd = Path::new(ident_text(&self.identity, "pane_current_path"));
                let account = as_text(get(&plan, "harnessAccount"))?;
                let inv = extension_launch::inventory(
                    &self.env.registry,
                    "opencode",
                    &account,
                    cwd,
                    &self.extension_paths(),
                )?;
                let pid = u32::try_from(self.original_pid()).map_err(|_| Fail::Unsure)?;
                let current = extension_launch::capture_opencode_environment(
                    &agent_procs::read_environ(&self.env.proc_root, self.original_pid()),
                    &self.env.hooks.join("extension-launches"),
                    &inv,
                    extension_launch::launch_from_pid(pid)?.is_some(),
                )?;
                if current["sha256"] != get(&plan, "opencodeEnvironment")["sha256"] {
                    return Err(py(
                        "la configuración OpenCode cambió durante la espera; el origen sigue abierto",
                    ));
                }
            }
            if !python_eq(
                get(&observed, "harnessAccount"),
                get(&plan, "harnessAccount"),
            ) {
                return Err(py("la cuenta original cambió durante la espera"));
            }
        }
        origin.insert("observed".into(), Value::Object(observed.clone()));
        let pid = self.original_pid();
        origin.insert(
            "agent_pid".into(),
            if pid == 0 {
                Value::Null
            } else {
                Value::from(pid)
            },
        );
        origin.insert("agent_start".into(), s(&self.original_start));
        origin.insert("identity".into(), Value::Object(self.identity.clone()));
        if self.frm != "shell" {
            let transcript = snapshot_transcript(&self.env, &origin)?;
            origin.insert("transcriptPath".into(), s(&transcript));
            let upid = u32::try_from(pid).map_err(|_| Fail::Unsure)?;
            let bundle = extension_launch::launch_from_pid(upid)?.unwrap_or(Value::Null);
            let manifest = agent_procs::read_environ(&self.env.proc_root, pid)
                .get(extension_launch::MANIFEST_ENV.as_bytes())
                .is_some_and(|v| !v.is_empty());
            if manifest && !truthy(&bundle) {
                return Err(py(
                    "el lanzamiento original no se pudo verificar; no se detiene",
                ));
            }
            origin.insert("extensionLaunch".into(), bundle.clone());
            let confirmed = truthy(get(&observed, "confirmed"));
            let model = if confirmed {
                as_text(get(&observed, "model"))?
            } else {
                String::new()
            };
            let effort = if confirmed {
                as_text(get(&observed, "effort"))?
            } else {
                String::new()
            };
            let flags: Vec<String> = match get(&origin, "flags") {
                Value::Array(list) => list
                    .iter()
                    .map(|f| f.as_str().map(str::to_owned).ok_or(Fail::Unsure))
                    .collect::<Result<_, _>>()?,
                v if truthy(v) => return Err(Fail::Unsure),
                _ => Vec::new(),
            };
            let account = match observed.get("harnessAccount") {
                Some(Value::String(a)) => a.clone(),
                Some(_) => return Err(Fail::Unsure),
                None => return Err(py("'harnessAccount'")),
            };
            let command = launch_command::configuration_command(
                &self.env.ctx(),
                &self.frm,
                &text_or(get(&observed, "motor"), &self.frm)?,
                &model,
                &effort,
                &account,
                &sid,
                &flags,
                model.is_empty(),
            )?;
            let resume = if truthy(&bundle) {
                extension_launch::wrap_command(
                    &command,
                    &bundle,
                    &extension_launch::require_helper(&self.env.home)?,
                    &self.env.environ,
                )?
            } else {
                command
            };
            origin.insert("resume_command".into(), s(&resume));
        }
        let mut snapshot = Map::new();
        snapshot.insert("session".into(), s(&self.sess));
        snapshot.insert("layout".into(), layout);
        snapshot.insert("origin".into(), Value::Object(origin));
        snapshot.insert("destination".into(), Value::Object(plan));
        self.snapshot_record = Some(snapshot.clone());
        Ok(Value::Object(snapshot))
    }

    /// `_copy_conversation(origin, account)` (3094): copia el historial exacto
    /// (y su carpeta de subagentes de Claude) a la cuenta destino, solo si
    /// nada que ya esté allí diverge del origen.
    fn copy_conversation(&self, origin: &Map<String, Value>, account: &str) -> Result<(), Fail> {
        let agent = text_or(get(origin, "agent"), &self.frm)?;
        if agent == "acp" {
            return Err(py(
                "cambiar cuenta ACP requiere exportación de conversación; se conserva el origen",
            ));
        }
        let sid = as_text(get(origin, "resume_id"))?;
        // El `KeyError` del `dict` de raíces del Python.
        let (root_key, default, env_key) =
            history_root(&agent).ok_or_else(|| Fail::Py(format!("'{agent}'")))?;
        let source_root = match get(origin, root_key) {
            v if truthy(v) => as_text(v)?,
            _ => expanduser(&self.env, default)?,
        };
        let environment = match accounts::account_environment(
            &self.env.registry,
            &agent,
            &s(account),
            &self.env.paths(),
        ) {
            Ok(Value::Object(map)) => map,
            Ok(_) => return Err(Fail::Unsure),
            Err(e) if accounts::is_account_error(&e) => return Err(Fail::Py(e.0)),
            Err(_) => return Err(Fail::Unsure),
        };
        let target_root = match environment.get(env_key) {
            Some(v) if truthy(v) => as_text(v)?,
            _ => expanduser(&self.env, default)?,
        };
        // `sid` vacío o con caracteres raros cambiaría el patrón de `glob`.
        if !sid.is_empty() && !sid_ok(&sid) {
            return Err(Fail::Unsure);
        }
        let paths = glob_history(&source_root, &agent, &sid)?;
        let [source] = paths.as_slice() else {
            return Err(py("no se encontró un único historial de origen"));
        };
        let source = if agent == "grok" {
            dirname(source)
        } else {
            source.clone()
        };
        let target = join(&target_root, &relpath(&source, &source_root)?);
        let mut copies = vec![(source.clone(), target.clone())];
        let extra = join(&dirname(&source), &sid);
        if agent == "claude" && fs::metadata(&extra).is_ok_and(|m| m.is_dir()) {
            copies.push((extra, join(&dirname(&target), &sid)));
        }
        for (source_item, target_item) in &copies {
            for (item, root) in [(source_item, &source_root), (target_item, &target_root)] {
                let relative = relpath(item, root)?;
                let mut current = root.clone();
                for part in relative.split('/') {
                    current = join(&current, part);
                    if part == ".." || is_link(Path::new(&current)) {
                        return Err(py(
                            "el historial contiene un enlace o sale de la cuenta; no se copia",
                        ));
                    }
                }
                if fs::metadata(item).is_ok_and(|m| m.is_dir()) && walk_has_link(Path::new(item)) {
                    return Err(py("el historial contiene un enlace; no se copia"));
                }
            }
            let source_dir = fs::metadata(source_item).is_ok_and(|m| m.is_dir());
            let existing = if source_dir {
                existing_tree(Path::new(target_item))?
            } else {
                vec![PathBuf::from(target_item)]
            };
            for found in existing {
                if !found.exists() {
                    continue;
                }
                if is_link(&found) {
                    return Err(py(
                        "el historial destino contiene un enlace; no se sobrescribe",
                    ));
                }
                if found.is_dir() {
                    continue;
                }
                let original = if source_dir {
                    let found_text = found.to_str().ok_or(Fail::Unsure)?;
                    PathBuf::from(join(source_item, &relpath(found_text, target_item)?))
                } else {
                    PathBuf::from(source_item)
                };
                let compatible = original.is_file()
                    && compatible_copy(&original, &found).map_err(|e| Fail::Py(io_text(&e)))?;
                if !compatible {
                    return Err(py(
                        "el historial destino contiene cambios propios; no se sobrescribe",
                    ));
                }
            }
        }
        for (source_item, target_item) in &copies {
            let parent = dirname(target_item);
            fs::create_dir_all(&parent).map_err(|e| Fail::Py(io_text(&e)))?;
            if fs::metadata(source_item).is_ok_and(|m| m.is_dir()) {
                copy_tree(Path::new(source_item), Path::new(target_item))?;
            } else {
                atomic_copy(Path::new(source_item), Path::new(target_item))?;
            }
        }
        Ok(())
    }

    // -------------------------------------------------------------- apply

    fn apply_inner(&mut self, snapshot: &Value) -> Result<(), Fail> {
        let plan = self.plan_ref()?.clone();
        if self.frm != "shell" {
            let pid = self.original_pid();
            let closed = pane_exit::exit_current(
                &self.env,
                &self.pane,
                pid,
                &self.frm,
                Some(&self.original_start),
            )
            .map_err(Fail::Py)?;
            if !closed {
                return Err(py("el agente original no cerró"));
            }
        }
        let current = pane_identity(&self.env, &self.sess, &self.pane)?;
        if identity_key(&current) != identity_key(&self.identity) {
            return Err(py("el panel cambió después de cerrar el origen"));
        }
        let command = ident_text(&current, "pane_current_command");
        if !SHELLS.contains(&command) || agent_info_for_pane(&self.env, &self.pane)?.is_some() {
            return Err(py(
                "hay otro proceso en el panel; no se envía el comando destino",
            ));
        }
        let snapshot_map = snapshot.as_object().ok_or(Fail::Unsure)?;
        let origin = obj(get(snapshot_map, "origin"))?
            .cloned()
            .unwrap_or_default();
        if is(get(&plan, "to"), &self.frm) && self.frm != "shell" {
            let account_key = if self.frm == "acp" {
                "motorAccount"
            } else {
                "harnessAccount"
            };
            let previous = obj_get(get(&origin, "observed"), account_key)?;
            if !python_eq(get(&plan, account_key), previous) {
                self.copy_conversation(&origin, &as_text(get(&plan, account_key))?)?;
            }
        }
        self.snapshot_record = Some(snapshot_map.clone());
        pane_exit::restore_shell_tty(&self.env, &self.pane).map_err(Fail::Py)?;
        let cwd = ident_text(&current, "pane_current_path").to_owned();
        let from_alias = as_text(obj_get(get(&origin, "observed"), "harnessAccount")?)?;
        let to_alias = as_text(get(&plan, "harnessAccount"))?;
        let harness = text_or(get(&plan, "to"), &self.frm)?;
        let home = self.env.home_text()?.to_owned();
        // Tras los efectos, una duda es el `except` que el Python traga: `false`.
        let inherited = launch_command::inherit_trust_for_switch(
            &self.env.registry,
            &home,
            &cwd,
            &from_alias,
            &to_alias,
            &harness,
        )
        .unwrap_or(false);
        if inherited {
            (self.env.trust_ledger)(&launch_command::trust_note(&cwd, &from_alias, &to_alias));
        }
        let line = format!(
            "env {} {}",
            shlex_quote(&format!("COMANDOS_OPERATION_ID={}", self.request_id)),
            as_text(get(&plan, "command"))?
        );
        pane_exit::send_configuration_command(&self.env, &self.pane, &line).map_err(Fail::Py)
    }

    // ------------------------------------------------------------- verify

    /// `_verify(expected, sid, attempts)` según la clase.
    fn verify_loop(
        &mut self,
        expected: &Map<String, Value>,
        sid: &str,
        attempts: i64,
    ) -> Result<Option<Obs>, Fail> {
        match self.kind {
            Kind::Session => self.verify_session(expected, sid, attempts),
            Kind::Extensions => self.verify_extensions(expected, sid, attempts),
        }
    }

    /// El marcador `COMANDOS_OPERATION_ID` del proceso, decodificado con
    /// `replace` (o estricto: `None` si no es UTF-8, que el Python lanzaría).
    fn marker(&self, pid: i64, strict: bool) -> Option<String> {
        let raw = agent_procs::read_environ(&self.env.proc_root, pid)
            .get(b"COMANDOS_OPERATION_ID".as_slice())
            .cloned()
            .unwrap_or_default();
        if strict {
            String::from_utf8(raw).ok()
        } else {
            Some(String::from_utf8_lossy(&raw).into_owned())
        }
    }

    /// `snapshot_record['destinationProcess'] = pin` y, salvo al refrescar,
    /// `store.stage(id, 'verifying', snapshot=…)`.
    fn pin(&mut self, pin: Map<String, Value>) -> Result<(), Fail> {
        let Some(record) = self.snapshot_record.as_mut() else {
            return Ok(());
        };
        record.insert("destinationProcess".into(), Value::Object(pin));
        if !self.refreshing {
            let saved = Value::Object(record.clone());
            let id = self.request_id.clone();
            self.with_store(|store| store.stage(&id, "verifying", Some(&saved), None))?;
        }
        Ok(())
    }

    fn screen(&self) -> Result<String, Fail> {
        Ok(self
            .env
            .tmux(&["capture-pane", "-p", "-t", &self.pane])?
            .stdout)
    }

    fn verify_session(
        &mut self,
        expected: &Map<String, Value>,
        sid: &str,
        attempts: i64,
    ) -> Result<Option<Obs>, Fail> {
        let interrupt = regex::Regex::new(r"(?i)esc to interrupt|ctrl\+c to interrupt")
            .map_err(|_| Fail::Unsure)?;
        for _ in 0..attempts.max(0) {
            if attempts > 1 {
                self.env.sleep_s(0.5);
            }
            let identity = pane_identity(&self.env, &self.sess, &self.pane)?;
            if identity_key(&identity) != identity_key(&self.identity) {
                return Err(py("el panel cambió durante la verificación"));
            }
            // Una duda cuenta como «aún no»; un error del Python se propaga.
            let info = match agent_info_for_pane(&self.env, &self.pane) {
                Ok(Some(info)) => info,
                Ok(None) | Err(Fail::Unsure) => continue,
                Err(error) => return Err(error),
            };
            if !is(get(expected, "to"), &info.agent) {
                continue;
            }
            if !self.in_recovery
                && !truthy(get(expected, "unchanged"))
                && self.marker(info.pid, false).as_deref() != Some(self.request_id.as_str())
            {
                continue;
            }
            let launch = get(expected, "extensionLaunch");
            if truthy(launch) {
                let Ok(pid) = u32::try_from(info.pid) else {
                    continue;
                };
                // Una duda cuenta como no verificado (se repite el intento).
                if !extension_launch::verify_launch(pid, launch).unwrap_or(false) {
                    continue;
                }
            }
            let observed = match observe_pane(&self.env, &self.sess, &self.pane, Some(&info), None)
            {
                Ok(observed) => observed,
                Err(Fail::Unsure) => continue,
                Err(other) => return Err(other),
            };
            if self.snapshot_record.is_some() && !self.in_recovery {
                let mut pin = Map::new();
                pin.insert("pid".into(), Value::from(info.pid));
                pin.insert(
                    "start".into(),
                    s(&agent_procs::process_start(&self.env.proc_root, info.pid)),
                );
                pin.insert(
                    "conversationId".into(),
                    or_text(get(&observed, "conversationId"), ""),
                );
                let previous = self
                    .snapshot_record
                    .as_ref()
                    .and_then(|r| r.get("destinationProcess"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let pin_value = Value::Object(pin.clone());
                if truthy(&previous)
                    && truthy(obj_get(&previous, "conversationId")?)
                    && !python_eq(&pin_value, &previous)
                {
                    return Err(py("cambió la conversación destino durante la verificación"));
                }
                if !python_eq(&pin_value, &previous) {
                    self.pin(pin)?;
                }
            }
            if !sid.is_empty() && !is(get(&observed, "conversationId"), sid) {
                continue;
            }
            if !truthy(get(&observed, "confirmed"))
                || !python_eq(get(&observed, "motor"), get(expected, "motor"))
            {
                continue;
            }
            if ["claude", "codex", "grok", "acp"]
                .iter()
                .any(|h| is(get(expected, "to"), h))
                && !truthy(get(&observed, "conversationId"))
            {
                continue;
            }
            // Una duda al comparar modelos cuenta como «no es el mismo».
            let same = (|| -> Result<bool, Fail> {
                Ok(launch_command::same_model(
                    &self.env.registry,
                    &text_or(get(expected, "motor"), "")?,
                    &text_or(get(expected, "model"), "")?,
                    &text_or(get(&observed, "model"), "")?,
                )?)
            })();
            if !same.unwrap_or(false) {
                continue;
            }
            if truthy(get(expected, "effort"))
                && !python_eq(get(&observed, "effort"), get(expected, "effort"))
            {
                continue;
            }
            if truthy(get(expected, "harnessAccount"))
                && !python_eq(
                    get(&observed, "harnessAccount"),
                    get(expected, "harnessAccount"),
                )
            {
                continue;
            }
            if truthy(get(expected, "motorAccount"))
                && !python_eq(
                    get(&observed, "motorAccount"),
                    get(expected, "motorAccount"),
                )
            {
                continue;
            }
            let screen = self.screen()?;
            // Cualquier duda sobre la pantalla cuenta como diálogo presente.
            if !self
                .env
                .dialogs
                .screen_dialog_fail_closed(&screen)
                .is_empty()
            {
                continue;
            }
            if !prompt_ready(&screen) && !interrupt.is_match(&screen) {
                continue;
            }
            return Ok(Some(observed));
        }
        Ok(None)
    }

    /// `PaneExtensionConfiguration._verify` (3290).
    fn verify_extensions(
        &mut self,
        expected: &Map<String, Value>,
        sid: &str,
        attempts: i64,
    ) -> Result<Option<Obs>, Fail> {
        for _ in 0..attempts.max(0) {
            if attempts > 1 {
                self.env.sleep_s(0.5);
            }
            let identity = pane_identity(&self.env, &self.sess, &self.pane)?;
            if identity_key(&identity) != identity_key(&self.identity) {
                return Err(py("el panel cambió durante la verificación"));
            }
            // Una duda cuenta como «aún no»; un error del Python se propaga.
            let info = match agent_info_for_pane(&self.env, &self.pane) {
                Ok(Some(info)) => info,
                Ok(None) | Err(Fail::Unsure) => continue,
                Err(error) => return Err(error),
            };
            if !is(get(expected, "to"), &info.agent) {
                continue;
            }
            let recovering = self.in_recovery;
            let launch = if recovering {
                self.snapshot_record
                    .as_ref()
                    .map(|r| obj_get(get(r, "origin"), "extensionLaunch").cloned())
                    .transpose()?
                    .unwrap_or(Value::Null)
            } else {
                get(expected, "extensionLaunch").clone()
            };
            if truthy(&launch) {
                let Ok(pid) = u32::try_from(info.pid) else {
                    continue;
                };
                if !extension_launch::verify_launch(pid, &launch).unwrap_or(false) {
                    continue;
                }
            }
            if !recovering
                && !truthy(get(expected, "unchanged"))
                && self.marker(info.pid, true).as_deref() != Some(self.request_id.as_str())
            {
                continue;
            }
            let observed = match observe_pane(&self.env, &self.sess, &self.pane, Some(&info), None)
            {
                Ok(observed) => observed,
                Err(Fail::Unsure) => continue,
                Err(other) => return Err(other),
            };
            if self.snapshot_record.is_some() && !recovering {
                let conversation = observed
                    .get("conversationId")
                    .cloned()
                    .ok_or_else(|| py("'conversationId'"))?;
                let mut pin = Map::new();
                pin.insert("pid".into(), Value::from(info.pid));
                pin.insert(
                    "start".into(),
                    s(&agent_procs::process_start(&self.env.proc_root, info.pid)),
                );
                pin.insert("conversationId".into(), conversation.clone());
                let old = self
                    .snapshot_record
                    .as_ref()
                    .and_then(|r| r.get("destinationProcess"))
                    .cloned()
                    .unwrap_or(Value::Null);
                if truthy(&old)
                    && (!python_eq(obj_get(&old, "pid")?, &Value::from(info.pid))
                        || !python_eq(obj_get(&old, "start")?, get(&pin, "start"))
                        || (truthy(obj_get(&old, "conversationId")?)
                            && !python_eq(obj_get(&old, "conversationId")?, &conversation)))
                {
                    return Err(py("el proceso destino cambió durante la verificación"));
                }
                if !python_eq(&old, &Value::Object(pin.clone())) {
                    self.pin(pin)?;
                }
            }
            let conversation = get(&observed, "conversationId");
            if !truthy(conversation) || (!sid.is_empty() && !is(conversation, sid)) {
                continue;
            }
            if truthy(get(expected, "harnessAccount"))
                && !python_eq(
                    get(&observed, "harnessAccount"),
                    get(expected, "harnessAccount"),
                )
            {
                continue;
            }
            let screen = self.screen()?;
            if !self
                .env
                .dialogs
                .screen_dialog_fail_closed(&screen)
                .is_empty()
            {
                continue;
            }
            let mut out = observed;
            out.insert("extensionsConfirmed".into(), Value::Bool(truthy(&launch)));
            return Ok(Some(out));
        }
        Ok(None)
    }

    fn verify_inner(&mut self) -> Result<Option<Obs>, Fail> {
        let plan = self.plan_ref()?.clone();
        let identity = pane_identity(&self.env, &self.sess, &self.pane)?;
        let cwd = ident_text(&identity, "pane_current_path").to_owned();
        let attempts = match self.attempts_override {
            Some(n) => n,
            None => {
                // Una duda solo alarga la espera (180, el valor con MCP por
                // omisión): esperar más nunca teclea nada.
                let decided = (|| -> Result<i64, Fail> {
                    let dir = claude_config_dir(&self.env, get(&plan, "harnessAccount"))?;
                    Ok(dialogs::verify_attempts(
                        &self.env.repo_root,
                        &cwd,
                        dir.as_deref(),
                        self.env.home_text()?,
                    )?)
                })();
                decided.unwrap_or(180)
            }
        };
        let sid = as_text(get(&plan, "expectedSid"))?;
        self.verify_loop(&plan, &sid, attempts)
    }

    /// `pending_confirmation(plan, snapshot)` (3141). Toda duda → `None`.
    fn pending_inner(&mut self, snapshot: &Map<String, Value>) -> Result<Option<Obs>, Fail> {
        let plan = self.plan_ref()?.clone();
        let identity = pane_identity(&self.env, &self.sess, &self.pane)?;
        if identity_key(&identity) != identity_key(&self.identity) {
            return Ok(None);
        }
        let info = match agent_info_for_pane(&self.env, &self.pane) {
            Ok(info) => info,
            Err(Fail::Unsure) => return Ok(None),
            Err(error) => return Err(error),
        };
        let pin = obj(get(snapshot, "destinationProcess"))
            .ok()
            .flatten()
            .cloned()
            .unwrap_or_default();
        let Some(info) = info else {
            return Ok(None);
        };
        if !is(get(&plan, "to"), &info.agent)
            || pin.is_empty()
            || !python_eq(&Value::from(info.pid), get(&pin, "pid"))
            || !python_eq(
                &s(&agent_procs::process_start(&self.env.proc_root, info.pid)),
                get(&pin, "start"),
            )
            || self.marker(info.pid, true).as_deref() != Some(self.request_id.as_str())
        {
            return Ok(None);
        }
        let screen = self.screen()?;
        let kind = self.env.dialogs.screen_dialog_fail_closed(&screen);
        self.screen_dialog_kind = kind.to_owned();
        if kind == "error" {
            return Ok(None);
        }
        // Un diálogo de arranque no invalida la observación.
        match observe_pane(&self.env, &self.sess, &self.pane, Some(&info), None) {
            Ok(observed) => Ok(Some(observed)),
            Err(Fail::Unsure) => Ok(None),
            Err(other) => Err(other),
        }
    }

    // ----------------------------------------------------------- rollback

    fn rollback_inner(&mut self, snapshot: &Map<String, Value>) -> Result<Option<Obs>, Fail> {
        if self.kind == Kind::Extensions {
            self.snapshot_record = Some(snapshot.clone());
        }
        self.in_recovery = true;
        let origin = obj(get(snapshot, "origin"))?.cloned().ok_or(Fail::Unsure)?;
        let current = pane_identity(&self.env, &self.sess, &self.pane)?;
        if identity_key(&current) != identity_key(&self.identity) {
            return Err(py(
                "el panel cambió; el snapshot conserva la recuperación exacta",
            ));
        }
        let live = agent_info_for_pane(&self.env, &self.pane)?;
        let original_observed = obj(get(&origin, "observed"))?.cloned().unwrap_or_default();
        let mut expected = original_observed.clone();
        expected.insert("to".into(), s(&self.frm));
        expected.insert(
            "extensionLaunch".into(),
            get(&origin, "extensionLaunch").clone(),
        );
        let sid = as_text(get(&original_observed, "conversationId"))?;
        let live_pid = live.as_ref().map_or(0, |l| l.pid);
        let agent_start = match origin.get("agent_start") {
            Some(v) => as_text(v)?,
            None => String::new(),
        };
        if live_pid == self.original_pid() && self.frm != "shell" {
            let Some(live) = &live else {
                // `_process_start(live['pid'])` sobre `{}`: `KeyError`.
                return Err(py("'pid'"));
            };
            if agent_procs::process_start(&self.env.proc_root, live.pid) == agent_start {
                if let Some(observed) = self.verify_loop(&expected, &sid, 60)? {
                    return Ok(Some(observed));
                }
                return Err(py(
                    "el proceso de origen sigue vivo pero no confirmó su estado",
                ));
            }
        }
        let plan = self.plan_ref()?.clone();
        if let Some(live) = &live {
            let marker = self.marker(live.pid, false).unwrap_or_default();
            if !is(get(&plan, "to"), &live.agent) || marker != self.request_id {
                return Err(py("hay otro proceso en el panel; no se interrumpe"));
            }
            let pin = obj(get(snapshot, "destinationProcess"))?
                .cloned()
                .unwrap_or_default();
            let live_start = agent_procs::process_start(&self.env.proc_root, live.pid);
            if !pin.is_empty()
                && (!python_eq(&Value::from(live.pid), get(&pin, "pid"))
                    || !python_eq(&s(&live_start), get(&pin, "start")))
            {
                return Err(py("el proceso destino fue sustituido; no se interrumpe"));
            }
            let expected_sid = if truthy(get(&pin, "conversationId")) {
                as_text(get(&pin, "conversationId"))?
            } else {
                as_text(get(&plan, "expectedSid"))?
            };
            if expected_sid.is_empty() && (!self.allow_pending_recovery || pin.is_empty()) {
                return Err(py(
                    "no se guardó la conversación destino; cierra ese CLI manualmente antes de recuperar el origen",
                ));
            }
            if !expected_sid.is_empty() {
                let observed = observe_pane(&self.env, &self.sess, &self.pane, Some(live), None)?;
                if !is(get(&observed, "conversationId"), &expected_sid) {
                    return Err(py("la conversación destino cambió; no se interrumpe"));
                }
            }
            let started = if truthy(get(&pin, "start")) {
                as_text(get(&pin, "start"))?
            } else {
                live_start
            };
            let stopped = if self.allow_pending_recovery {
                pane_exit::stop_owned_startup(&self.env, live.pid, &started).map_err(Fail::Py)?
            } else {
                pane_exit::exit_current(
                    &self.env,
                    &self.pane,
                    live.pid,
                    &live.agent,
                    Some(&started),
                )
                .map_err(Fail::Py)?
            };
            if !stopped {
                return Err(py("no se pudo cerrar el destino fallido"));
            }
        }
        if live.is_none() && !SHELLS.contains(&ident_text(&current, "pane_current_command")) {
            return Err(py(
                "hay un proceso sin identificar en el panel; no se envía la recuperación",
            ));
        }
        if self.frm == "shell" {
            let mut out = Map::new();
            out.insert("harness".into(), s("shell"));
            out.insert("motor".into(), s("shell"));
            out.insert("source".into(), s("pane-process"));
            out.insert("confirmed".into(), Value::Bool(true));
            return Ok(Some(out));
        }
        // Desviación (bug del Python): tras cerrar el destino, el pane debe
        // volver a ser un shell sin agente antes de teclear `stty sane` y el
        // comando de vuelta; un envoltorio que siga en primer plano lo leería.
        self.wait_for_shell()?;
        pane_exit::restore_shell_tty(&self.env, &self.pane).map_err(Fail::Py)?;
        let resume = as_text(get(&origin, "resume_command"))?;
        pane_exit::send_configuration_command(&self.env, &self.pane, &resume).map_err(Fail::Py)?;
        match self.verify_loop(&expected, &sid, 60)? {
            Some(observed) => Ok(Some(observed)),
            None => Err(py(
                "no se confirmó la reanudación de la conversación original",
            )),
        }
    }

    /// Antes de teclear la recuperación: misma identidad, ningún agente y un
    /// shell en primer plano, sondeado hasta 25 × 0,2 s. Lo que no se cumple
    /// a tiempo (o no se puede leer) no teclea nada.
    fn wait_for_shell(&self) -> Result<(), Fail> {
        const UNKNOWN: &str =
            "hay un proceso sin identificar en el panel; no se envía la recuperación";
        for attempt in 0..25 {
            if attempt > 0 {
                self.env.sleep_s(0.2);
            }
            let Ok(current) = pane_identity(&self.env, &self.sess, &self.pane) else {
                continue;
            };
            if identity_key(&current) != identity_key(&self.identity) {
                return Err(py(UNKNOWN));
            }
            let shell = SHELLS.contains(&ident_text(&current, "pane_current_command"));
            if shell && matches!(agent_info_for_pane(&self.env, &self.pane), Ok(None)) {
                return Ok(());
            }
        }
        Err(py(UNKNOWN))
    }

    /// `adapter.rollback(snapshot)` de `session_recover`.
    pub fn recover(&mut self, snapshot: &Value) -> Result<Option<Obs>, Fail> {
        let map = snapshot.as_object().ok_or(Fail::Unsure)?.clone();
        self.rollback_inner(&map)
    }
}

/// `re.search(r'(^|\n)\s*[❯›>]', screen)` con el `\s` de Python (que,
/// a diferencia del de `regex`, incluye U+001C–U+001F y cruza saltos).
fn prompt_ready(screen: &str) -> bool {
    let starts = std::iter::once(0).chain(screen.match_indices('\n').map(|(i, _)| i + 1));
    starts.into_iter().any(|start| {
        screen
            .get(start..)
            .unwrap_or("")
            .trim_start_matches(comandos_core::text::is_space)
            .starts_with(['❯', '›', '>'])
    })
}

/// `x or ''` como valor.
fn or_empty(value: &Value) -> Value {
    if truthy(value) { value.clone() } else { s("") }
}

/// `x or default` como valor.
fn or_text(value: &Value, default: &str) -> Value {
    if truthy(value) {
        value.clone()
    } else {
        s(default)
    }
}

impl Adapter for SessionConfiguration {
    fn prepare(&mut self) -> ops::Result<Value> {
        self.prepare_plan()
            .map(Value::Object)
            .map_err(|e| to_journal(e, UNCERTAIN))
    }

    fn wait_idle(&mut self) -> ops::Result<()> {
        self.wait_idle_inner().map_err(|e| to_journal(e, UNCERTAIN))
    }

    fn check_identity(&mut self) -> ops::Result<()> {
        self.check_identity_inner()
            .map_err(|e| to_journal(e, UNCERTAIN))
    }

    fn snapshot(&mut self, _plan: &Value) -> ops::Result<Value> {
        let taken = match self.kind {
            Kind::Session => self.snapshot_session(),
            Kind::Extensions => self.snapshot_extensions(),
        };
        taken.map_err(|e| to_journal(e, UNCERTAIN))
    }

    fn apply(&mut self, _plan: &Value, snapshot: &Value) -> ops::Result<()> {
        self.apply_inner(snapshot)
            .map_err(|e| to_journal(e, UNCERTAIN_APPLY))
    }

    fn verify(&mut self, _plan: &Value, _snapshot: Option<&Value>) -> ops::Result<Option<Value>> {
        self.verify_inner()
            .map(|o| o.map(Value::Object))
            .map_err(|e| to_journal(e, UNCERTAIN_VERIFY))
    }

    fn rollback(&mut self, snapshot: &Value) -> ops::Result<Option<Value>> {
        // El Python recibe el mismo `dict` que `apply` anotó (con su
        // `destinationProcess`): aquí es `snapshot_record` si ya existe.
        let map = match &self.snapshot_record {
            Some(record) => record.clone(),
            None => snapshot.as_object().cloned().unwrap_or_default(),
        };
        self.rollback_inner(&map)
            .map(|o| o.map(Value::Object))
            .map_err(|e| to_journal(e, UNCERTAIN_ROLLBACK))
    }

    fn pending_confirmation(
        &mut self,
        _plan: &Value,
        snapshot: &Value,
    ) -> ops::Result<Option<Value>> {
        let map = match &self.snapshot_record {
            Some(record) => record.clone(),
            None => snapshot.as_object().cloned().unwrap_or_default(),
        };
        match self.pending_inner(&map) {
            Ok(found) => Ok(found.map(Value::Object)),
            Err(Fail::Unsure) => Ok(None),
            Err(Fail::Py(text)) => Err(ops::Error::Callback(text)),
        }
    }

    fn snapshot_record(&self) -> Option<Value> {
        self.snapshot_record.clone().map(Value::Object)
    }
}

// ------------------------------------------------ refresh_session_confirmation

/// Lo que dejó `refresh_confirmation`: la fila (la misma si nada cambió) y,
/// si esta lectura confirmó el destino, su observación y su ruta (para
/// `record_runtime_config`, que es del frente).
pub struct Refreshed {
    pub row: Value,
    pub confirmed: Option<(Map<String, Value>, String, String, String)>,
}

/// `refresh_session_confirmation(store, row)` (3300) sin el registro de uso:
/// una lectura del destino fijado de una fila `awaiting_confirmation`; nunca
/// teclea nada. Toda duda o error deja la fila como estaba.
pub fn refresh_confirmation(env: &Env, row: &Value) -> Refreshed {
    let unchanged = || Refreshed {
        row: row.clone(),
        confirmed: None,
    };
    if !is(row.get("state").unwrap_or(&NULL), "awaiting_confirmation")
        || !row.get("snapshot").is_some_and(truthy)
    {
        return unchanged();
    }
    match refresh_inner(env, row) {
        Ok(Some(done)) => done,
        _ => unchanged(),
    }
}

fn refresh_inner(env: &Env, row: &Value) -> Result<Option<Refreshed>, Fail> {
    let snapshot = row
        .get("snapshot")
        .and_then(Value::as_object)
        .ok_or(Fail::Unsure)?
        .clone();
    let request = row.get("request").cloned().ok_or(Fail::Unsure)?;
    let kind = if truthy(obj_get(&request, "extensionsOnly")?) {
        Kind::Extensions
    } else {
        Kind::Session
    };
    let identity = obj(obj_get(get(&snapshot, "origin"), "identity")?)?
        .cloned()
        .ok_or(Fail::Unsure)?;
    let mut adapter = SessionConfiguration::new(kind, request, identity, env.clone())?;
    let plan = obj(get(&snapshot, "destination"))?
        .cloned()
        .ok_or(Fail::Unsure)?;
    adapter.plan = Some(plan.clone());
    adapter.snapshot_record = Some(snapshot.clone());
    adapter.refreshing = true;
    let before = comandos_core::json::dumps(&Value::Object(snapshot.clone()), true, false)
        .map_err(|_| Fail::Unsure)?;
    if adapter.pending_inner(&snapshot)?.is_none() {
        return Ok(None);
    }
    let sid = as_text(get(&plan, "expectedSid"))?;
    let observed = adapter.verify_loop(&plan, &sid, 1)?;
    let record = adapter.snapshot_record.clone().unwrap_or(snapshot);
    let after = comandos_core::json::dumps(&Value::Object(record.clone()), true, false)
        .map_err(|_| Fail::Unsure)?;
    if observed.is_none() && after == before {
        return Ok(None);
    }
    // `row['result'].get('observed', {})`: la clave presente, aunque sea `null`.
    let previous_observed = match row.get("result") {
        Some(Value::Object(result)) => result
            .get("observed")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new())),
        _ => return Err(Fail::Unsure),
    };
    let confirmed = observed.is_some();
    let mut result = Map::new();
    result.insert("ok".into(), Value::Bool(true));
    result.insert("pending".into(), Value::Bool(!confirmed));
    result.insert("confirmed".into(), Value::Bool(confirmed));
    result.insert("recoveryAllowed".into(), Value::Bool(!confirmed));
    result.insert(
        "observed".into(),
        match &observed {
            Some(o) => Value::Object(o.clone()),
            None => previous_observed,
        },
    );
    if let Some(continuity) = obj(get(&plan, "continuity"))? {
        for (k, v) in continuity {
            result.insert(k.clone(), v.clone());
        }
    }
    let id = row
        .get("id")
        .and_then(Value::as_str)
        .ok_or(Fail::Unsure)?
        .to_owned();
    let conn = open_journal(&env.journal)?;
    let snapshot_raw = response_dumps(&Value::Object(record)).map_err(|_| Fail::Unsure)?;
    let result_raw = response_dumps(&Value::Object(result)).map_err(|_| Fail::Unsure)?;
    let changed = conn
        .execute(
            "UPDATE session_operations SET state=?,snapshot=?,result=?,updated=? \
             WHERE id=? AND state='awaiting_confirmation'",
            params![
                if confirmed {
                    "confirmed"
                } else {
                    "awaiting_confirmation"
                },
                snapshot_raw,
                result_raw,
                (env.clock)(),
                id
            ],
        )
        .map_err(|e| Fail::Py(e.to_string()))?;
    let owner = env.owner;
    let owner = move || owner;
    let clock = env.clock.clone();
    let clock = move || clock();
    let store = OperationStore::new(&conn, &owner, &clock)?;
    let fresh = store.get(&id)?;
    let row = match fresh {
        Some(fresh) => fresh,
        None if changed == 0 => row.clone(),
        None => Value::Null,
    };
    let confirmed = match (changed, observed) {
        (n, Some(observed)) if n > 0 => {
            let route = as_text(get(&plan, "routeId"))?;
            Some((observed, route, adapter.sess.clone(), adapter.pane.clone()))
        }
        _ => None,
    };
    Ok(Some(Refreshed { row, confirmed }))
}

/// Apply the current catalog defaults before validating an older saved draft.
fn resolve_extension_draft(inv: &Value, desired: &Value) -> Result<Value, Fail> {
    let desired = crate::pane_extensions::canonical_selection(inv, desired);
    Ok(extension_launch::normalize(inv, &desired)?)
}

#[cfg(test)]
mod extension_draft_tests {
    use super::*;

    #[test]
    fn applying_old_drafts_inherits_new_defaults_and_keeps_explicit_exclusions() {
        let inventory = json!({
            "mcps":[
                {"id":"new-mcp","name":"new","enabled":true,"toggleable":true},
                {"id":"excluded-mcp","name":"excluded","enabled":true,"toggleable":true},
                {"id":"globally-off","name":"off","enabled":false,"toggleable":false}
            ],
            "skills":[
                {"id":"shared:new","name":"new","enabled":true,"toggleable":true},
                {"id":"shared:excluded","name":"excluded","enabled":true,"toggleable":true},
                {"id":"shared:off","name":"off","enabled":false,"toggleable":true}
            ]
        });
        let draft = json!({"mcps":{"excluded-mcp":false,"globally-off":true},"skills":{"shared:excluded":false}});
        let original = draft.clone();
        let selected = resolve_extension_draft(&inventory, &draft).unwrap();
        assert_eq!(
            selected,
            json!({
                "mcps":{"new-mcp":true,"excluded-mcp":false,"globally-off":false},
                "skills":{"shared:new":true,"shared:excluded":false,"shared:off":false}
            })
        );
        assert_eq!(
            selected,
            crate::pane_extensions::canonical_selection(&inventory, &draft)
        );
        assert_eq!(
            draft, original,
            "resolving defaults must not rewrite saved choices"
        );
    }
}
