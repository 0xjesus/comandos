//! La recolección real de `read_states` (7035) y `write_app_tab_models`
//! (6714): tmux por `tokio::process` en el orden del Python, los escaneos de
//! `/proc`, registros, transcripts y cuentas en el hilo propio de `/state`
//! (`serial`), la base de
//! uso por su carril y `app-tab-models.json` escrito solo si nada declinó.
use super::{
    StateFault, States,
    cards::{self, CardEffects, Inputs},
    observe::{self, ObserveEvidence, Observed},
    records::{Record, RecordCache},
    suggest::{self, SuggestContext},
    tab_models,
};
use crate::dash::native::{
    Native, NativeOptions, catalogs,
    files::{self, Strict},
    light, py,
    tmux::{Output, TmuxError},
};
use comandos_core::json::{response_dumps, truthy};
use comandos_runtime::{
    Unsure,
    agent_procs::{
        self, AccountCache, AgentInfo, AgentMaps, PANE_FORMAT, PaneRow, agent_pane_maps,
        agent_procs_for_agents, external_agents, parse_pane_inventory, process_owners,
    },
    hooks::py::float_value,
    model_catalog::catalog_paths,
    pane_observe::{acp_state, claude_conversation, codex_conversation, grok_home_for},
    pane_snapshot::{PaneInspector, PaneRef},
    providers::{self, RegistryCache},
    tui_state::{GrokMetadataCache, StateTracker, TranscriptCache, screen_state},
};
use rusqlite::{Connection, types::ValueRef};
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

/// `ssh -O check` de `ssh_state` (7708): 3 s.
const SSH_TIMEOUT: Duration = Duration::from_secs(3);
/// Los campos de `_pane_identity`, separados por tabuladores.
const IDENTITY_FORMAT: &str = "#{socket_path}\t#{pid}\t#{session_id}\t#{session_name}\t#{pane_id}\t#{pane_pid}\t#{pane_current_command}\t#{pane_current_path}";

/// Lo que el trabajo bloqueante conserva entre cómputos (D1, D5): las cachés
/// del Python que el frente tiene propias. Se toma sin `await` dentro. El
/// registro de proveedores no vive aquí: es `Native::registry`, la caché
/// única (B9) que comparten `/state`, `/usage/state` y `/providers`, con su
/// propio candado corto (este se sostiene todo el escaneo de `/proc`).
pub struct Blocking {
    records: RecordCache,
    transcripts: TranscriptCache,
    grok: GrokMetadataCache,
    accounts: AccountCache,
}

impl Default for Blocking {
    /// Los tamaños por defecto de `lib/tui_state.py` (79, 193).
    fn default() -> Self {
        Self {
            records: RecordCache::default(),
            transcripts: TranscriptCache::new(128, 2_097_152),
            grok: GrokMetadataCache::new(128),
            accounts: AccountCache::default(),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

fn unsure(_: Unsure) -> StateFault {
    StateFault::Decline
}

/// Lo que el escaneo bloqueante entrega al bucle de tarjetas.
struct Scan {
    panes: Vec<PaneRow>,
    registry: Value,
    maps: AgentMaps,
    external: HashSet<(String, String)>,
    records: Vec<Record>,
    motor: Map<String, Value>,
    tiers: Value,
    tabs: Vec<(String, String)>,
    history: Vec<Map<String, Value>>,
    inspector: PaneInspector,
}

/// `MOTOR_RESULT` desde su espejo `H/motor-results.json` (D1): ausente → `{}`;
/// ilegible, incierto, no objeto o con un valor que no es objeto → declinar
/// (la memoria del Python es desconocida).
pub(crate) fn motor_results(home: &Path, hooks: &Path) -> Result<Map<String, Value>, StateFault> {
    match files::DomainDocument::new(home, hooks, "motor-results.json")
        .map_or(Strict::Unsure, |d| d.strict_in_worker())
    {
        Strict::Missing => Ok(Map::new()),
        Strict::Value(Value::Object(map)) if map.values().all(Value::is_object) => Ok(map),
        _ => Err(StateFault::Decline),
    }
}

/// `load_provider_registry()` con la caché única del frente (`Native::registry`,
/// B9 de la 2e). Su candado solo cubre la carga (lectura, validación e
/// hidratación): nadie lo sostiene mientras escanea `/proc`, así que un
/// `/usage/state` o un `/providers` no esperan al escaneo de `/state`.
pub(crate) fn load_registry(
    opts: &NativeOptions,
    cache: &Mutex<RegistryCache>,
) -> Result<Value, StateFault> {
    let repo = opts.repo_root.as_ref().ok_or(StateFault::Decline)?;
    let catalog = catalog_paths(
        &opts.home,
        &opts.cwd,
        opts.codex_home.as_deref(),
        opts.grok_home.as_deref(),
    );
    lock(cache)
        .load(&repo.join("config/providers.json"), &catalog)
        .map_err(unsure)
}

/// Agentes de `/proc` emparejados con los panes: `agent_pane_maps(procs,
/// panes, ownership)` (6103) tras `agent_procs()` (5992), y los agentes
/// externos de `read_states`. Lee `AGENTS` de `cc-notify.conf`.
pub(crate) fn agent_maps(
    opts: &NativeOptions,
    registry: &Value,
    panes: &[PaneRow],
) -> Result<(AgentMaps, HashSet<(String, String)>), StateFault> {
    let conf = providers::read_conf(&opts.hooks.join("cc-notify.conf")).map_err(unsure)?;
    let conf_agents = conf
        .iter()
        .find(|(k, _)| k == "AGENTS")
        .map(|(_, v)| v.as_str());
    let agents = providers::agent_set(conf_agents, registry);
    let aliases = providers::process_aliases(&agents, registry);
    let proc_root = opts.proc_root.as_path();
    let procs = agent_procs_for_agents(proc_root, &aliases, &agents).map_err(unsure)?;
    // Padres y `cmdline` de `/proc` con cachés locales de este cómputo.
    let mut parents: HashMap<i64, i64> = HashMap::new();
    let mut parent = |pid: i64| {
        *parents
            .entry(pid)
            .or_insert_with(|| agent_procs::parent_pid(proc_root, pid))
    };
    let owners = process_owners(&procs, panes, &mut parent);
    let mut cmdline = |pid: i64| agent_procs::proc_cmdline(proc_root, pid);
    let maps = agent_pane_maps(&procs, panes, &owners, &mut cmdline, &mut parent);
    let external = external_agents(&procs, &owners, &mut parent);
    Ok((maps, external))
}

/// Paso 2: registro, agentes, `/proc`, inspector, registros de estado,
/// resultados del motor, tiers y pestañas.
fn scan(
    opts: &NativeOptions,
    panes: Vec<PaneRow>,
    shared: &Mutex<Blocking>,
    registry: &Mutex<RegistryCache>,
) -> Result<Scan, StateFault> {
    // El registro primero, con su candado corto y antes del de las cachés.
    let registry = load_registry(opts, registry)?;
    let mut cache = lock(shared);
    // `PaneInspector()` se crea al empezar `read_states`.
    let inspector = PaneInspector::new(&opts.home, opts.proc_root.as_path()).map_err(unsure)?;
    let (maps, external) = agent_maps(opts, &registry, &panes)?;
    let tabs = light::tab_labels_in_worker(&opts.home, &opts.hooks)?;
    let history = light::tab_history_in_worker(&opts.home, &opts.hooks)?;
    let records = cache
        .records
        .scan_domain(&opts.home, &opts.hooks.join("state"))?;
    drop(cache);
    let motor = motor_results(&opts.home, &opts.hooks)?;
    let tiers = catalogs::read_model_tiers(opts)?;
    Ok(Scan {
        panes,
        registry,
        maps,
        external,
        records,
        motor,
        tiers,
        tabs,
        history,
        inspector,
    })
}

/// `session_labels` (6700): etiquetas de pestañas más el historial de las
/// sesiones vivas que no tienen etiqueta propia o la tienen igual al nombre.
pub(crate) fn session_labels(
    tabs: Vec<(String, String)>,
    live: &HashSet<String>,
    history: &[Map<String, Value>],
) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = tabs.into_iter().collect();
    for item in history {
        let text = |key: &str| item.get(key).and_then(Value::as_str).unwrap_or("");
        let session = text("session");
        if !live.contains(session) {
            continue;
        }
        let current = out.get(session).map(String::as_str).unwrap_or("");
        if current.is_empty() || current == session {
            let label = text("label");
            let label = if label.is_empty() { session } else { label };
            out.insert(session.to_owned(), label.to_owned());
        }
    }
    out
}

/// GET `/state` sin caché: los ocho pasos de la tarea, en orden. Nada se
/// escribe si algo declinó o falló antes del final. Un `Decline` deja una
/// línea en el journal con la fase en que ocurrió (como mucho una por minuto).
pub async fn compute(native: &Native) -> Result<States, StateFault> {
    let mut phase = "inicio";
    let result = steps(native, &mut phase).await;
    if matches!(result, Err(StateFault::Decline)) {
        native
            .states
            .declines
            .note((native.options().clock)(), phase);
    }
    result
}

/// Los pasos de `compute`; `phase` nombra el que está en curso (solo para la
/// línea del journal: no lleva datos de panes, rutas ni cuentas).
async fn steps(native: &Native, phase: &mut &'static str) -> Result<States, StateFault> {
    let opts = native.options();
    *phase = "sin carril de uso o sin raíz del checkout";
    // Sin carril de uso, cada tarjeta con agente acabaría declinando: se
    // reenvía antes de tocar tmux.
    if !native.usage.enabled() || opts.repo_root.is_none() {
        return Err(StateFault::Decline);
    }
    // Con un fallo del contexto recordado (R3) el cómputo acabaría declinando
    // si alguna tarjeta lo necesita: se reenvía antes del trabajo de tmux y
    // `/proc`, para que una guardia que el frente no reproduce (o la primera
    // lectura de límites aún pendiente) no lo repita en cada sondeo.
    *phase = "fallo del contexto de sugerencias recordado";
    if !opts.standalone && native.states.context.failing((opts.clock)()) {
        return Err(StateFault::Decline);
    }
    let now = (opts.clock)() as f64 / 1000.0;
    // 1. `tmux_pane_inventory()`.
    *phase = "inventario de panes";
    let listed = opts
        .tmux
        .run(&["list-panes", "-a", "-F", PANE_FORMAT])
        .await
        .map_err(|e| StateFault::from_tmux(&e))?;
    let mut panes = if listed.ok {
        parse_pane_inventory(&listed.stdout)
    } else {
        Vec::new()
    };
    agent_procs::resolve_foreground(opts.proc_root.as_path(), &mut panes);
    // 2. Escaneo bloqueante.
    *phase = "escaneo (registro, /proc, registros de estado, motor-results, tiers, pestañas)";
    let shared = native.states.shared.clone();
    let registry_cache = native.registry.clone();
    let scan_opts = opts.clone();
    let scanned = native
        .states
        .serial
        .run(move || scan(&scan_opts, panes, &shared, &registry_cache))
        .await?;
    // 3. `session_labels()`: `tmux_sessions()` sin capturar sus excepciones.
    *phase = "sesiones de tmux";
    let live = light::tmux_sessions(&opts.tmux).await?;
    let labels = session_labels(scanned.tabs, &live, &scanned.history);
    // 4. Tarjetas con los efectos reales.
    let registry = Arc::new(scanned.registry);
    // El rastreador trabaja sobre una copia que solo se confirma si el
    // cómputo emite (paso 8): un cómputo que declina o falla no deja rastro,
    // así lo que ve el siguiente es lo mismo que vio el Python que respondió.
    let effects = RealEffects {
        native,
        inspector: Arc::new(scanned.inspector),
        registry: registry.clone(),
        tracker: Mutex::new(lock(&native.states.tracker).clone()),
    };
    let inputs = Inputs {
        now,
        panes: scanned.panes,
        maps: scanned.maps,
        external: scanned.external,
        labels,
        records: scanned.records,
    };
    *phase = "tarjetas (pantalla, transcripts, cuentas, observación)";
    let mut items = cards::build(&inputs, &effects).await?;
    // 5. Sugerencias: el contexto solo si alguna tarjeta lo consultaría.
    let motor = scanned.motor;
    *phase = "contexto de sugerencias";
    let context = if suggest::needs_context(&items, &motor)? {
        match native
            .states
            .context
            .get(native, &registry, (opts.clock)())
            .await
        {
            Ok(context) => Some(context),
            // El panel completo sigue mostrando sesiones cuando falta el
            // contexto opcional; no inventar sugerencias con límites vacíos.
            Err(_) if opts.standalone => None,
            Err(error) => return Err(error),
        }
    } else {
        Some(Arc::new(SuggestContext::default()))
    };
    *phase = "sugerencias";
    if let Some(context) = context {
        suggest::annotate_all(&mut items, &context, &motor, now)?;
    }
    // 6. Orden final y proyección de modelos.
    *phase = "orden y modelos de pestaña";
    cards::sort_items(&mut items)?;
    let models = tab_models::tab_models(&items, &registry, &motor, &scanned.tiers, now)?;
    // 7. Serializar antes de escribir: nada se escribe si algo declinó.
    *phase = "serialización";
    let items = Value::Array(items);
    let body = response_dumps(&items).map_err(|_| StateFault::Decline)?;
    response_dumps(&models).map_err(|_| StateFault::Decline)?;
    // Ya no se declina: el rastreador de este cómputo pasa a ser el del frente.
    // El vuelo único (`StatesCache`) garantiza un solo cómputo a la vez.
    *lock(&native.states.tracker) = effects
        .tracker
        .into_inner()
        .unwrap_or_else(|p| p.into_inner());
    // 8. `write_app_tab_models`: un fallo se ignora, como su `except`.
    let doc = files::DomainDocument::new(&opts.home, &opts.hooks, "app-tab-models.json");
    let now_ms = (opts.clock)();
    // En el pool y no en `serial`: un trabajo de `serial` en cola se descarta si
    // el cómputo se abandona, y esta escritura, como la del Python, se completa
    // aunque el cliente se vaya.
    let _ = tokio::task::spawn_blocking(move || {
        let doc = doc?;
        let body = response_dumps(&models).map_err(std::io::Error::other)?;
        doc.write_bytes(body.as_bytes(), now_ms)
    })
    .await;
    let Value::Array(items) = items else {
        return Err(StateFault::Failure);
    };
    Ok(States {
        items: Arc::new(items),
        body: bytes::Bytes::from(body),
    })
}

/// Los efectos de `read_states` por pane, hechos de verdad.
struct RealEffects<'a> {
    native: &'a Native,
    inspector: Arc<PaneInspector>,
    registry: Arc<Value>,
    /// Copia del rastreador del frente para este cómputo.
    tracker: Mutex<StateTracker>,
}

impl RealEffects<'_> {
    fn opts(&self) -> &NativeOptions {
        self.native.options()
    }

    fn shared(&self) -> Arc<Mutex<Blocking>> {
        self.native.states.shared.clone()
    }
}

impl CardEffects for RealEffects<'_> {
    fn tmux(
        &self,
        args: &[&str],
        timeout: Duration,
    ) -> impl Future<Output = Result<Output, TmuxError>> + Send {
        let mut tmux = self.opts().tmux.clone();
        tmux.timeout = timeout;
        async move { tmux.run(args).await }
    }

    fn account(
        &self,
        pid: i64,
        agent: &str,
    ) -> impl Future<Output = Result<Map<String, Value>, StateFault>> + Send {
        let (shared, home, proc_root) = (
            self.shared(),
            self.opts().home.clone(),
            self.opts().proc_root.clone(),
        );
        let agent = agent.to_owned();
        self.native.states.serial.run(move || {
            lock(&shared)
                .accounts
                .account_for_pid(&home, &proc_root, pid, &agent)
                .map_err(unsure)
        })
    }

    fn grok_metadata(
        &self,
        pid: i64,
    ) -> impl Future<Output = Result<Map<String, Value>, StateFault>> + Send {
        let (shared, home, proc_root) = (
            self.shared(),
            self.opts().home.clone(),
            self.opts().proc_root.clone(),
        );
        self.native.states.serial.run(move || {
            let grok_home = grok_home_for(&proc_root, &home, pid)?;
            lock(&shared).grok.read(pid, &grok_home).map_err(unsure)
        })
    }

    fn observe(
        &self,
        session: &str,
        pane: &str,
        info: &AgentInfo,
    ) -> impl Future<Output = Result<Observed, StateFault>> + Send {
        let session = session.to_owned();
        let pane = pane.to_owned();
        let info = info.clone();
        async move { self.observe_pane(&session, &pane, &info).await }
    }

    fn session_config(
        &self,
        session: &str,
        pane: &str,
    ) -> impl Future<Output = Result<Option<Map<String, Value>>, StateFault>> + Send {
        let (session, pane) = (session.to_owned(), pane.to_owned());
        async move {
            self.native
                .usage
                .with(move |b| latest_session_config(&b.conn, &session, &pane))
                .await?
        }
    }

    fn ssh_check(&self, host: &str) -> impl Future<Output = bool> + Send {
        let program = self.opts().ssh.clone();
        let host = host.to_owned();
        async move {
            let mut cmd = program.command();
            cmd.args(["-O", "check", host.as_str()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            // `subprocess.run(..., timeout=3)` dentro de `try/except Exception`.
            let Ok(mut child) = cmd.spawn() else {
                return false;
            };
            matches!(
                tokio::time::timeout(SSH_TIMEOUT, child.wait()).await,
                Ok(Ok(status)) if status.success()
            )
        }
    }
}

/// `cc_usage.latest_session_config` (`bin/cc_usage.py:2370`): la última fila
/// con las columnas en el orden de `SELECT *`. Un error de SQL es la
/// excepción sin capturar (500); un BLOB o texto no UTF-8 no se reproduce.
fn latest_session_config(
    conn: &Connection,
    session: &str,
    pane: &str,
) -> Result<Option<Map<String, Value>>, StateFault> {
    let failure = |_| StateFault::Failure;
    let mut stmt = conn
        .prepare(
            "SELECT * FROM usage_session_configs WHERE tmux_session=? \
             AND (tmux_pane=? OR (?='' AND tmux_pane='')) \
             ORDER BY effective_at DESC LIMIT 1",
        )
        .map_err(failure)?;
    let names: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|n| (*n).to_owned())
        .collect();
    let mut rows = stmt.query((session, pane, pane)).map_err(failure)?;
    let Some(row) = rows.next().map_err(failure)? else {
        return Ok(None);
    };
    let mut out = Map::new();
    for (index, name) in names.into_iter().enumerate() {
        let value = match row.get_ref(index).map_err(failure)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(n) => Value::from(n),
            ValueRef::Real(f) if f.is_finite() => float_value(f),
            ValueRef::Real(_) | ValueRef::Blob(_) => return Err(StateFault::Decline),
            ValueRef::Text(raw) => {
                Value::from(std::str::from_utf8(raw).map_err(|_| StateFault::Decline)?)
            }
        };
        out.insert(name, value);
    }
    Ok(Some(out))
}

/// Lo que `_pane_identity` sacó de tmux.
struct Identity {
    fields: Vec<String>,
}

impl Identity {
    fn field(&self, index: usize) -> &str {
        self.fields.get(index).map_or("", String::as_str)
    }
}

/// `observe_pane` después de `_pane_identity`, hasta antes de la pantalla.
enum Gathered {
    Seen(Box<ObserveEvidence>),
    /// `OSError`/`ValueError`/`RuntimeError` dentro del `try` (A7).
    Unconfirmed,
}

/// Lo que el trabajo bloqueante necesita de la observación.
struct Probe {
    home: PathBuf,
    hooks: PathBuf,
    proc_root: PathBuf,
    pane: String,
    info: AgentInfo,
    identity: Identity,
}

/// `observe_pane` (6788) de `_pane_identity` al final de la conversación,
/// más `_process_start` y `ANTHROPIC_BASE_URL`: solo lecturas de archivos.
fn gather_evidence(
    probe: &Probe,
    inspector: &PaneInspector,
    shared: &Mutex<Blocking>,
    registry: &Value,
) -> Result<Gathered, StateFault> {
    let identity = &probe.identity;
    // `server_start`: `open('/proc/' + pid + '/stat')`; `OSError` → `''`.
    let tmux_pid = identity.field(1);
    if tmux_pid.is_empty() || !tmux_pid.bytes().all(|b| b.is_ascii_digit()) {
        return Err(StateFault::Decline);
    }
    #[cfg(target_os = "macos")]
    let darwin_start = (probe.proc_root == Path::new("/proc")).then(|| {
        tmux_pid
            .parse::<i64>()
            .ok()
            .map(|p| agent_procs::process_start(&probe.proc_root, p))
            .unwrap_or_default()
    });
    #[cfg(not(target_os = "macos"))]
    let darwin_start: Option<String> = None;
    let server_start = match darwin_start {
        Some(start) => start,
        None => match fs::read(probe.proc_root.join(tmux_pid).join("stat")) {
            Err(_) => String::new(),
            Ok(raw) => {
                // Modo texto: `UnicodeDecodeError` es `ValueError`.
                let Ok(text) = String::from_utf8(raw) else {
                    return Ok(Gathered::Unconfirmed);
                };
                // `rsplit(')', 1)[1].split()[19]`: el `IndexError` no se captura.
                let (_, after) = text.rsplit_once(')').ok_or(StateFault::Failure)?;
                after
                    .split(py::is_space)
                    .filter(|t| !t.is_empty())
                    .nth(19)
                    .ok_or(StateFault::Failure)?
                    .to_owned()
            }
        },
    };
    let key = [
        identity.field(0),
        tmux_pid,
        &server_start,
        identity.field(2),
        identity.field(4),
        identity.field(5),
    ]
    .join("|");
    // `int(identity['pane_pid'])`.
    let pane_pid = match py::int(identity.field(5)) {
        Ok(pid) => pid,
        Err(py::NumError::Invalid) => return Ok(Gathered::Unconfirmed),
        Err(py::NumError::Exotic) => return Err(StateFault::Decline),
    };
    let snap = inspector
        .inspect(&PaneRef {
            id: &probe.pane,
            pid: pane_pid,
            command: identity.field(6),
        })
        .map_err(unsure)?;
    let (agent, pid) = (probe.info.agent.as_str(), probe.info.pid);
    let proc_root = probe.proc_root.as_path();
    let mut cache = lock(shared);
    let account = cache
        .accounts
        .account_for_pid(&probe.home, proc_root, pid, agent)
        .map_err(unsure)?;
    let account = match account.get("account").filter(|v| truthy(v)) {
        Some(value) => super::py_str(value)?,
        None => "unknown".to_owned(),
    };
    let cmdline = agent_procs::proc_cmdline(proc_root, pid);
    let conversation_id = observe::conversation_id(&snap)?;
    let mut acp = Map::new();
    let conversation = match agent {
        "codex" if truthy(&conversation_id) => {
            let id = conversation_id.as_str().ok_or(StateFault::Failure)?;
            codex_conversation(&mut cache.transcripts, proc_root, pid, id)?
        }
        "grok" => {
            let grok_home = grok_home_for(proc_root, &probe.home, pid)?;
            let metadata = cache.grok.read(pid, &grok_home).map_err(unsure)?;
            observe::project_metadata(&metadata, &conversation_id, "lastActiveAt")
        }
        "opencode" | "agy" => match snap.get("nativeMetadata").filter(|v| truthy(v)) {
            None => Map::new(),
            Some(Value::Object(metadata)) => {
                observe::project_metadata(metadata, &conversation_id, "updatedAt")
            }
            // `.get` de un no-`dict`: `AttributeError`, que no se captura.
            Some(_) => return Err(StateFault::Failure),
        },
        "acp" => {
            acp = acp_state(&probe.hooks, &probe.pane)?;
            Map::new()
        }
        "claude" if truthy(&conversation_id) => {
            let id = conversation_id.as_str().ok_or(StateFault::Failure)?;
            let root = match snap.get("claude_config_dir").filter(|v| truthy(v)) {
                None => probe.home.join(".claude"),
                Some(Value::String(dir)) => PathBuf::from(dir),
                Some(_) => return Err(StateFault::Failure),
            };
            claude_conversation(&mut cache.transcripts, &root, id)?
        }
        _ => Map::new(),
    };
    drop(cache);
    let process_start = agent_procs::process_start(proc_root, pid);
    let base_url = if agent == "claude" {
        agent_procs::read_environ(proc_root, pid)
            .get(b"ANTHROPIC_BASE_URL".as_slice())
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };
    Ok(Gathered::Seen(Box::new(ObserveEvidence {
        agent: agent.to_owned(),
        pid,
        identity: key,
        snap,
        account,
        harness_has_accounts: providers::harness_has_accounts(registry, agent),
        cmdline,
        conversation,
        acp,
        visible: Map::new(),
        process_start,
        base_url,
    })))
}

impl RealEffects<'_> {
    /// `observe_pane` dentro del `try` de `reconcile_card_config` (6884):
    /// identidad por tmux, evidencia en un hilo de bloqueo, pantalla y el
    /// rastreador del motor.
    async fn observe_pane(
        &self,
        session: &str,
        pane: &str,
        info: &AgentInfo,
    ) -> Result<Observed, StateFault> {
        let opts = self.opts();
        // (a) `_pane_identity`: tmux fuera de un `try` propio; el plazo es 504.
        let out = match opts
            .tmux
            .run(&["display-message", "-p", "-t", pane, IDENTITY_FORMAT])
            .await
        {
            Ok(out) => out,
            Err(TmuxError::Timeout { .. }) => return Err(StateFault::Timeout),
            Err(_) => return Ok(Observed::Unconfirmed),
        };
        // `result.stdout.strip().split('\t')`.
        let fields: Vec<String> = py::strip(&out.stdout)
            .split('\t')
            .map(str::to_owned)
            .collect();
        if !out.ok || fields.len() != 8 {
            return Ok(Observed::Unconfirmed);
        }
        let identity = Identity { fields };
        if identity.field(4) != pane || identity.field(3) != session {
            return Ok(Observed::Unconfirmed);
        }
        // (b) Lecturas de `/proc`, cuentas y conversación.
        let probe = Probe {
            home: opts.home.clone(),
            hooks: opts.hooks.clone(),
            proc_root: opts.proc_root.clone(),
            pane: pane.to_owned(),
            info: info.clone(),
            identity,
        };
        let (inspector, shared, registry) =
            (self.inspector.clone(), self.shared(), self.registry.clone());
        let gathered = self
            .native
            .states
            .serial
            .run(move || gather_evidence(&probe, &inspector, &shared, &registry))
            .await?;
        let mut evidence = match gathered {
            Gathered::Seen(evidence) => evidence,
            Gathered::Unconfirmed => return Ok(Observed::Unconfirmed),
        };
        // (c) `pane_visible_config` para codex, claude y opencode.
        if matches!(info.agent.as_str(), "codex" | "claude" | "opencode") {
            let captured = match opts
                .tmux
                .run(&["capture-pane", "-p", "-t", pane, "-S", "-45"])
                .await
            {
                Ok(out) => out,
                Err(TmuxError::Timeout { .. }) => return Err(StateFault::Timeout),
                Err(_) => return Ok(Observed::Unconfirmed),
            };
            if captured.ok {
                evidence.visible = screen_state(&info.agent, &captured.stdout).map_err(unsure)?;
            }
        }
        // (d) El rastreador del motor, sin `await` mientras se tiene.
        let now = (opts.clock)() as f64 / 1000.0;
        let mut tracker = lock(&self.tracker);
        let seen = observe::observe(&evidence, &mut tracker, &self.registry, now)?;
        Ok(Observed::Seen(seen))
    }
}

/// El rastreador de configuración del frente (`_TUI_STATES`).
pub fn new_tracker() -> Mutex<StateTracker> {
    Mutex::new(StateTracker::new(128))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_labels_fill_from_live_history() {
        let tabs = vec![("a".into(), "Alfa".into()), ("b".into(), "b".into())];
        let live: HashSet<String> = ["a", "b", "c"].iter().map(|s| (*s).to_owned()).collect();
        let row = |s: &str, l: &str| {
            let mut m = Map::new();
            m.insert("session".into(), Value::from(s));
            m.insert("label".into(), Value::from(l));
            m
        };
        let history = vec![
            row("a", "Otro"),
            row("b", "Beta"),
            row("c", ""),
            row("d", "Delta"),
        ];
        let out = session_labels(tabs, &live, &history);
        assert_eq!(out.get("a").map(String::as_str), Some("Alfa"));
        assert_eq!(out.get("b").map(String::as_str), Some("Beta"));
        assert_eq!(out.get("c").map(String::as_str), Some("c"));
        assert!(!out.contains_key("d"));
    }
}
