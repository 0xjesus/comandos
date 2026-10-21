//! Motor de GET `/usage/state` (bin/cc-dash:8441-8462): el cálculo completo de
//! la respuesta con `record_pane` y el memo de `cached_usage_state` (269). La
//! ruta (`usage::answer`) añade la importación y los bordes de pane (Tarea 8).
//!
//! Orden: el primer trabajo del carril de uso (`usage_settings`) es el único
//! punto de declinar (D1); desde ahí un trabajo del carril que declina se
//! reintenta una vez (la puerta pudo dar `SQLITE_BUSY`) y, si vuelve a
//! declinar, es un 500 (el carril se apagó a mitad y el Python habría leído).
//! `record_pane` va en un trabajo del carril de importación que la respuesta
//! no espera (R3). Saltos de
//! bloqueo por cómputo: uno (lecturas de `/proc`, registro, pestañas y
//! archivos de entorno) y otro más solo cuando el memo no sirve (el ensamblado
//! de `build_usage_state`); el resto son procesos async (`tmux`, `git`) y
//! trabajos del carril, que tiene hilo propio.
use super::super::{
    Fault, Native, NativeOptions,
    lanes::{Lane, UsageImportBackend},
    light,
    states::{StateFault, gather},
    tmux::{self, Program},
};
use crate::{HandlerError, Reply, ReplyBody};
use comandos_core::{
    allocation,
    json::response_dumps_entry_chunks,
    text,
    usage_state::{self, UsageError},
};
use comandos_runtime::{
    agent_procs::{self, AgentMaps, PANE_FORMAT, PaneRow, parse_pane_inventory},
    providers::{self, RegistryCache},
};
use comandos_store::usage_read::{self, ReadError};
use futures_util::{StreamExt, stream};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

type Row = Map<String, Value>;

/// `git rev-parse` de `git_root_for_path` (cc_usage.py:423): 3 s.
const GIT_TIMEOUT: Duration = Duration::from_secs(3);
/// Cuántos `git rev-parse` corren a la vez (el Python los hace uno a uno).
const GIT_CONCURRENCY: usize = 8;
/// `USAGE_STATE_DAYS * 86400` (cc_usage.py:181).
const STATE_WINDOW_S: i64 = 14 * 86_400;
/// `list_alerts(db_path, limit=12)` (cc_usage.py:992).
const ALERTS_LIMIT: i64 = 12;

fn failure() -> Fault {
    Fault::Error(HandlerError::Failure)
}

/// D1: pasado el punto de declinar, un `Decline` (el carril se apagó a mitad,
/// o la puerta falló dos veces seguidas en `with_retry`) es el 500 que el
/// Python no daría pero el frente tampoco puede reenviar.
fn no_decline(fault: Fault) -> Fault {
    match fault {
        Fault::Decline => failure(),
        other => other,
    }
}

/// La respuesta calculada: el cuerpo serializado, el estado del memo (lo que
/// usa el escritor de bordes de la Tarea 7), los paneles vivos con agente y
/// todos los panes vivos de tmux (`None` si `list-panes` falló), con los que
/// se acota la memoria de avisos de nivel. `live_declined`: la recolección de
/// la 2d declinó y `live_panes` va vacía sin serlo: esa vuelta no escribe
/// bordes (D1; con la lista vacía los quitaría de todos los panes).
pub struct UsageStateReply {
    pub body: UsageBody,
    pub state: Arc<UsageMemo>,
    pub live_panes: Vec<Row>,
    pub live_declined: bool,
    pub tmux_panes: Option<BTreeSet<String>>,
}

/// Lo que devuelve el paso 2.
struct Live {
    /// `None` si la recolección de la 2d declinó.
    panes: Option<Vec<Row>>,
    /// Los ids de `list-panes -a` (sesiones válidas: las únicas con filas).
    tmux_panes: Option<BTreeSet<String>>,
    conf: ConfFile,
    usage_file: ConfFile,
}

/// Memo de `build_usage_state` (`cached_usage_state` 269): una entrada.
#[derive(Default)]
pub struct UsageEngine {
    generation: AtomicU64,
    /// Vuelo único (`with _usage_state_lock`): quien lo sostiene es el único
    /// que encola una reconstrucción; los demás esperan y reutilizan.
    gate: tokio::sync::Mutex<()>,
    /// El memo. Lo escribe el propio trabajo del carril al terminar, así que una
    /// reconstrucción abandonada por quien la pidió se guarda igual (M3).
    slot: Arc<Mutex<MemoSlot>>,
    panes: std::sync::Mutex<PendingPanes>,
}

#[derive(Default)]
struct MemoSlot {
    memo: Option<(MemoKey, Arc<UsageMemo>)>,
    /// Reconstrucciones empezadas (para las pruebas).
    builds: u64,
}

fn slot(slot: &Mutex<MemoSlot>) -> std::sync::MutexGuard<'_, MemoSlot> {
    slot.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl MemoSlot {
    fn hit(&self, key: &MemoKey) -> Option<Arc<UsageMemo>> {
        match &self.memo {
            Some((k, state)) if k == key => Some(state.clone()),
            _ => None,
        }
    }
}

/// `record_pane` pendiente: solo el último juego de paneles vivos. Mientras el
/// carril de escritura está ocupado (una importación), las peticiones que
/// llegan reemplazan el juego en vez de encolar un trabajo cada una.
#[derive(Default)]
struct PendingPanes {
    rows: Option<Vec<Row>>,
    queued: bool,
}

/// El estado de `build_usage_state` ya escrito: cada clave de nivel superior
/// como su entrada `"clave": valor` de `response_dumps`, en su orden. Como
/// árbol `Value` ocupaba ≈ 11 MiB (≈ 1 MB de JSON) y cada petición lo volvía a
/// escribir entero; ahora la respuesta solo escribe las claves que pone encima.
/// `panes` (las filas de los bordes) y `windows` (las cuentas de tokens de los
/// límites) se guardan también como valores: son las únicas que se leen.
pub struct UsageMemo {
    entries: Vec<(String, Vec<bytes::Bytes>)>,
    pub panes: Value,
    windows: Value,
}

impl UsageMemo {
    /// Escribe el estado. Lo que `response_dumps` no puede escribir (no es un
    /// objeto o pasa la profundidad) es el error que el Python daría al
    /// responder. `panes` y `windows` se copian en vez de moverse: lo que queda
    /// del memo se asigna después de soltar las filas, en sus huecos, y no entre
    /// los nodos del árbol, que se suelta entero y deja la cima de la arena
    /// libre para que glibc la recorte.
    fn encode(state: &Value) -> Result<Self, String> {
        let Value::Object(map) = state else {
            return Err("el estado no es un objeto".into());
        };
        let mut entries = Vec::with_capacity(map.len());
        for (key, value) in map {
            let chunks = response_dumps_entry_chunks(key, value, BODY_CHUNK)?;
            entries.push((
                key.clone(),
                chunks.into_iter().map(bytes::Bytes::from).collect(),
            ));
        }
        let field = |key: &str| map.get(key).cloned().unwrap_or(Value::Null);
        Ok(Self {
            entries,
            panes: field("panes"),
            windows: field("windows"),
        })
    }

    /// El cuerpo de `dict(memo)` con `over` asignadas encima: las claves del
    /// memo en su sitio y las nuevas al final, en el orden en que se pusieron.
    fn body(&self, over: &[(&str, Value)]) -> Result<UsageBody, String> {
        let mut fresh: Vec<Vec<bytes::Bytes>> = Vec::with_capacity(over.len());
        for (key, value) in over {
            let chunks = response_dumps_entry_chunks(key, value, BODY_CHUNK)?;
            fresh.push(chunks.into_iter().map(bytes::Bytes::from).collect());
        }
        let pick = |key: &str| over.iter().position(|(k, _)| *k == key);
        let mut body = UsageBody::default();
        body.push(bytes::Bytes::from_static(b"{"));
        let mut first = true;
        let mut entry = |body: &mut UsageBody, parts: &[bytes::Bytes]| {
            if !std::mem::take(&mut first) {
                body.push(bytes::Bytes::from_static(b", "));
            }
            for part in parts {
                body.push(part.clone());
            }
        };
        for (key, parts) in &self.entries {
            match pick(key).and_then(|i| fresh.get(i)) {
                Some(new) => entry(&mut body, new),
                None => entry(&mut body, parts),
            }
        }
        for ((key, _), new) in over.iter().zip(&fresh) {
            if !self.entries.iter().any(|(k, _)| k == key) {
                entry(&mut body, new);
            }
        }
        body.push(bytes::Bytes::from_static(b"}"));
        Ok(body)
    }
}

/// Trozos del memo y del cuerpo: 32 KiB, bajo el umbral de `mmap` de glibc
/// (128 KiB). Un cuerpo de 1 MB en una sola asignación lo sirve glibc con
/// `mmap` y, al soltarlo, sube el umbral de `mmap` y el de recorte; desde
/// entonces las arenas no devuelven memoria.
const BODY_CHUNK: usize = 32 * 1024;

/// El cuerpo de GET `/usage/state` en trozos compartidos con el memo (sin
/// copiarlos): se envía con su longitud, como un cuerpo de un solo bloque.
#[derive(Default)]
pub struct UsageBody {
    parts: Vec<bytes::Bytes>,
    len: usize,
}

impl UsageBody {
    fn push(&mut self, part: bytes::Bytes) {
        self.len += part.len();
        self.parts.push(part);
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// El cuerpo entero en un bloque (para las pruebas).
    pub fn to_vec(&self) -> Vec<u8> {
        self.parts.concat()
    }

    /// La respuesta `200 application/json` con `Content-Length`.
    pub fn into_reply(self) -> Reply {
        let mut reply = Reply::bytes(
            http::StatusCode::OK,
            "application/json",
            bytes::Bytes::new(),
        );
        let (tx, receiver) = tokio::sync::mpsc::channel(self.parts.len().max(1));
        for part in self.parts {
            // Hay sitio para todos: el canal se creó con su número.
            let _ = tx.try_send(Ok(part));
        }
        reply.body = ReplyBody::SizedStream {
            length: self.len as u64,
            receiver,
        };
        reply
    }
}

#[derive(Clone, PartialEq, Eq)]
struct MemoKey {
    generation: u64,
    /// `sorted((tmux_session, tmux_pane, pane_pwd, agent))`.
    panes: Vec<(String, String, String, String)>,
}

impl MemoKey {
    fn new(generation: u64, live: &[Row]) -> Self {
        let field = |row: &Row, key: &str| {
            row.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let mut panes: Vec<_> = live
            .iter()
            .map(|p| {
                (
                    field(p, "tmux_session"),
                    field(p, "tmux_pane"),
                    field(p, "pane_pwd"),
                    field(p, "agent"),
                )
            })
            .collect();
        panes.sort();
        Self { generation, panes }
    }
}

impl UsageEngine {
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Cuántas reconstrucciones del memo empezaron (para las pruebas).
    pub fn builds(&self) -> u64 {
        slot(&self.slot).builds
    }

    /// Al terminar una importación completa (`_usage_state_generation += 1`).
    pub fn bump(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// El estado del memo, o uno nuevo si la clave cambió. El vuelo único se
    /// sostiene durante el cálculo: los que llegan esperan y reutilizan (el
    /// `with _usage_state_lock` del Python). Si quien pidió se desconecta, el
    /// trabajo que ya corre en el carril termina y guarda el memo él mismo (el
    /// hilo del Python también lo guardaba); uno que aún esperaba en la cola no
    /// llega a correr. El siguiente encola el suyo, que al empezar mira si el
    /// anterior lo dejó hecho (D4 de la 2d, M3 de la revisión final).
    async fn state(
        &self,
        native: &Native,
        live: &[Row],
        settings: Row,
    ) -> Result<Arc<UsageMemo>, Fault> {
        let _gate = self.gate.lock().await;
        // La generación se lee con el vuelo único tomado: una importación que
        // termina mientras otro calcula invalida lo que este guarde después.
        let key = MemoKey::new(self.generation(), live);
        {
            let mut held = slot(&self.slot);
            if let Some(state) = held.hit(&key) {
                return Ok(state);
            }
            // El memo viejo se suelta antes de calcular el nuevo: así el nuevo
            // no queda por encima de los temporales del cálculo en la arena (las
            // respuestas en curso guardan su propio `Arc`).
            held.memo = None;
        }
        // `int(time.time())` de `build_usage_state`.
        let now = (native.options().clock)().div_euclid(1000);
        let live = live.to_vec();
        let shared = self.slot.clone();
        // Lectura, ensamblado y escritura en el hilo del carril de uso: el pico
        // (≈ 20 MiB con la base real) se queda en una sola arena de glibc, que
        // cada reconstrucción (una por importación, cada 60 s) reutiliza.
        native
            .usage
            .with_retry(move |u| -> Result<Arc<UsageMemo>, ()> {
                // Una reconstrucción anterior, abandonada por quien la pidió,
                // pudo dejarlo hecho mientras este trabajo esperaba en la cola.
                {
                    let mut held = slot(&shared);
                    if let Some(state) = held.hit(&key) {
                        return Ok(state);
                    }
                    held.builds += 1;
                }
                let rows = usage_read::state_rows(&u.conn, now - STATE_WINDOW_S).map_err(|_| ())?;
                let panes = if live.is_empty() {
                    usage_read::list_panes(&u.conn).map_err(|_| ())?
                } else {
                    live
                };
                let state = usage_state::build_state(
                    now,
                    panes,
                    &rows.turns,
                    &rows.provider_usage,
                    &rows.provider_costs,
                    &settings,
                )
                .map_err(|_| ())?;
                drop(rows);
                let memo = UsageMemo::encode(&state).map_err(|_| ());
                drop(state);
                let memo = Arc::new(memo?);
                slot(&shared).memo = Some((key, memo.clone()));
                Ok(memo)
            })
            .await
            .map_err(no_decline)?
            .map_err(|_| failure())
    }
}

/// Deja `live` como el juego pendiente de `record_pane` y, si no hay ya un
/// trabajo en la cola del carril de escritura, encola uno que toma el último
/// juego al correr.
fn record_latest_panes(
    engine: &Arc<UsageEngine>,
    lane: &Arc<Lane<UsageImportBackend>>,
    live: Vec<Row>,
) {
    fn lock(e: &UsageEngine) -> std::sync::MutexGuard<'_, PendingPanes> {
        e.panes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    {
        let mut pending = lock(engine);
        pending.rows = Some(live);
        if pending.queued {
            return;
        }
        pending.queued = true;
    }
    let (engine, lane) = (engine.clone(), lane.clone());
    tokio::spawn(async move {
        let job = {
            let engine = engine.clone();
            move |u: &mut UsageImportBackend| {
                let rows = {
                    let mut pending = lock(&engine);
                    pending.queued = false;
                    pending.rows.take()
                };
                if let Some(rows) = rows {
                    let _ = usage_read::record_panes(&u.conn, &rows);
                }
            }
        };
        // El carril declinó o se apagó: el trabajo no corrió.
        if lane.with(job).await.is_err() {
            lock(&engine).queued = false;
        }
    });
}

/// Un archivo de configuración leído una vez: `cc-notify.conf` lo usan el
/// entorno (`_parse_env_file`, que ignora cualquier `OSError`) y `ui_lang`
/// (`read_conf`, que solo ignora `FileNotFoundError`).
pub(crate) enum ConfFile {
    Missing,
    Unreadable,
    Bytes(Vec<u8>),
}

impl ConfFile {
    pub(crate) fn read(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => Self::Bytes(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Self::Missing,
            Err(_) => Self::Unreadable,
        }
    }

    /// `_parse_env_file` (cc_usage.py:2157): un `OSError` es `{}`; texto que no
    /// es UTF-8 es el `UnicodeDecodeError` no capturado (500).
    fn env_pairs(&self) -> Result<Vec<(String, String)>, Fault> {
        match self {
            Self::Missing | Self::Unreadable => Ok(Vec::new()),
            Self::Bytes(bytes) => {
                let text = std::str::from_utf8(bytes).map_err(|_| failure())?;
                Ok(usage_state::parse_env_text(text))
            }
        }
    }

    /// `read_conf().get("CC_LANG")`: ausente → nada; otro `OSError` o texto
    /// que no es UTF-8 → la excepción no capturada (500).
    fn cc_lang(&self) -> Result<Option<String>, Fault> {
        let text = match self {
            Self::Missing => return Ok(None),
            Self::Unreadable => return Err(failure()),
            Self::Bytes(bytes) => std::str::from_utf8(bytes).map_err(|_| failure())?,
        };
        let conf = providers::parse_conf(text).map_err(|_| failure())?;
        Ok(conf
            .into_iter()
            .find(|(k, _)| k == "CC_LANG")
            .map(|(_, v)| v))
    }
}

fn lang_of(cc_lang: Option<&str>, usage_env: &BTreeMap<String, String>) -> &'static str {
    let lang = usage_env.get("LANG").map(String::as_str);
    if super::pane_models::ui_lang_es(cc_lang, lang) {
        "es"
    } else {
        "en"
    }
}

/// `ui_lang()` (cc-dash:7272) desde un hilo de bloqueo: `CC_LANG` de
/// `cc-notify.conf` si es `es`/`en`; si no, `es` cuando el `LANG` capturado
/// (D7) empieza por `es`. `Err`: `read_conf` lanzaría (500).
pub fn ui_lang(conf: &Path, usage_env: &BTreeMap<String, String>) -> Result<&'static str, Fault> {
    let cc_lang = ConfFile::read(conf).cc_lang()?;
    Ok(lang_of(cc_lang.as_deref(), usage_env))
}

/// `git_root_for_path` (cc_usage.py:423): `git rev-parse --show-toplevel` en
/// `path` con plazo de 3 s; cualquier fallo (plazo, directorio inexistente,
/// salida que no es UTF-8, salida vacía) devuelve `path`.
pub async fn git_root_for_path(path: &Path) -> String {
    let fallback = path.to_string_lossy().into_owned();
    if fallback.is_empty() {
        return fallback;
    }
    let git = Program::named("git");
    match tmux::run_program_in(&git, &["rev-parse", "--show-toplevel"], path, GIT_TIMEOUT).await {
        Ok(out) if out.ok => {
            let root = text::strip(&out.stdout);
            if root.is_empty() {
                fallback
            } else {
                root.to_owned()
            }
        }
        _ => fallback,
    }
}

/// Lo que el salto de bloqueo devuelve.
struct Scanned {
    conf: ConfFile,
    usage_file: ConfFile,
    /// Etiquetas y agentes; `Err` de la recolección de la 2d.
    agents: Result<(HashMap<String, String>, AgentMaps), StateFault>,
}

/// El único salto de bloqueo de todo cómputo: pestañas, registro (la caché
/// única del frente, B9), agentes de `/proc` y los dos archivos de entorno.
/// `live` son las sesiones de `tmux_sessions()`. Ningún candado que sostenga
/// el escaneo de `/state`: el del registro solo cubre su carga, así que este
/// hilo del pool no queda aparcado detrás de `/state` (I2 de la revisión).
fn scan(
    opts: &NativeOptions,
    panes: &[PaneRow],
    live: &HashSet<String>,
    registry: &Mutex<RegistryCache>,
) -> Scanned {
    let agents = (|| -> Result<_, StateFault> {
        // `session_labels()`: `tab_labels()` y `read_tab_history()`.
        let tabs = light::tab_labels_domain(&opts.home, &opts.hooks)?;
        let history = light::read_tab_history_domain(&opts.home, &opts.hooks)?;
        let labels = gather::session_labels(tabs, live, &history);
        // `agent_pane_maps(agent_procs())`.
        let registry = gather::load_registry(opts, registry)?;
        let (maps, _external) = gather::agent_maps(opts, &registry, panes)?;
        Ok((labels, maps))
    })();
    Scanned {
        conf: ConfFile::read(&opts.hooks.join("cc-notify.conf")),
        usage_file: ConfFile::read(&opts.hooks.join("usage.env")),
        agents,
    }
}

/// Paso 2 sin `record_pane` (`usage_live_panes` 7230): `None` si la
/// recolección de la 2d declinó (lista vacía esta vuelta, sin `record_pane`).
async fn live_panes(native: &Native, now: i64) -> Result<Live, Fault> {
    let opts = native.options();
    // `tmux_sessions()` y `tmux_pane_inventory()`: excepciones sin capturar
    // (plazo → 504, el resto → 500); un código de salida distinto de 0 → vacío.
    let (sessions, listed) = tokio::join!(
        light::tmux_sessions(&opts.tmux),
        opts.tmux.run(&["list-panes", "-a", "-F", PANE_FORMAT])
    );
    let sessions = sessions?;
    let listed = listed.map_err(|e| Fault::Error(e.uncaught()))?;
    let mut panes = if listed.ok {
        parse_pane_inventory(&listed.stdout)
    } else {
        Vec::new()
    };
    agent_procs::resolve_foreground(opts.proc_root.as_path(), &mut panes);
    let tmux_panes = listed.ok.then(|| {
        panes
            .iter()
            .map(|p| p.pane.clone())
            .collect::<BTreeSet<_>>()
    });
    let scan_opts = opts.clone();
    let registry = native.registry.clone();
    let scanned =
        tokio::task::spawn_blocking(move || scan(&scan_opts, &panes, &sessions, &registry))
            .await
            .map_err(|_| failure())?;
    let Scanned {
        conf,
        usage_file,
        agents,
    } = scanned;
    let declined = |conf, usage_file| Live {
        panes: None,
        tmux_panes: tmux_panes.clone(),
        conf,
        usage_file,
    };
    let (labels, maps) = match agents {
        Ok(found) => found,
        Err(StateFault::Decline) => return Ok(declined(conf, usage_file)),
        Err(fault) => return Err(fault.into()),
    };
    // Deduplicado por `(session, pane, agent)` en el orden de `by_cwd`.
    let mut seen = HashSet::new();
    let mut infos = Vec::new();
    for (_, rows) in maps.by_cwd {
        for info in rows {
            if seen.insert((info.session.clone(), info.pane.clone(), info.agent.clone())) {
                infos.push(info);
            }
        }
    }
    // Los `git rev-parse` corren a la vez (acotados); el resultado no depende
    // del orden y uno por carpeta basta.
    let cwds: HashSet<String> = infos.iter().map(|i| i.cwd.clone()).collect();
    let roots: HashMap<String, String> = stream::iter(cwds)
        .map(|cwd| async move {
            let root = git_root_for_path(Path::new(&cwd)).await;
            (cwd, root)
        })
        .buffer_unordered(GIT_CONCURRENCY)
        .collect()
        .await;
    let mut out = Vec::with_capacity(infos.len());
    for info in infos {
        let git_root = roots.get(&info.cwd).cloned().unwrap_or_default();
        let agent = if info.agent.is_empty() {
            "claude".to_owned()
        } else {
            info.agent
        };
        let mut raw = Row::new();
        raw.insert("session".into(), info.session.into());
        raw.insert("pane".into(), info.pane.into());
        raw.insert("cwd".into(), info.cwd.into());
        raw.insert("git_root".into(), git_root.into());
        raw.insert("agent".into(), agent.into());
        raw.insert("pid".into(), info.pid.into());
        match usage_state::normalize_pane_identity(&Value::Object(raw), &labels, now) {
            Ok(pane) => out.push(pane),
            Err(UsageError::Unsure) => return Ok(declined(conf, usage_file)),
            Err(UsageError::Raises | UsageError::Overflow) => return Err(failure()),
        }
    }
    Ok(Live {
        panes: Some(out),
        tmux_panes,
        conf,
        usage_file,
    })
}

/// `usage_runtime_env()` (cc-dash:179): `cc-notify.conf`, `usage.env`, el
/// entorno capturado al arrancar (D7) y `read_usage_settings`, en ese orden.
pub(crate) fn runtime_env(
    conf: &ConfFile,
    usage_file: &ConfFile,
    process: &BTreeMap<String, String>,
    settings: Vec<(String, Value)>,
) -> Result<Row, Fault> {
    let mut env = Row::new();
    for (key, value) in conf.env_pairs()?.into_iter().chain(usage_file.env_pairs()?) {
        env.insert(key, value.into());
    }
    for (key, value) in process {
        env.insert(key.clone(), value.clone().into());
    }
    for (key, value) in settings {
        env.insert(key, value);
    }
    Ok(env)
}

/// GET `/usage/state` sin bordes ni importación (pasos 1–9 de la tarea).
pub async fn compute(native: &Native) -> Result<UsageStateReply, Fault> {
    let opts = native.options();
    // 1. Punto de declinar: `read_usage_settings` por el carril de uso.
    let settings = native
        .usage
        .with(|u| usage_read::usage_settings(&u.conn))
        .await?
        .map_err(|e| match e {
            ReadError::Raises => failure(),
            ReadError::Sql(_) | ReadError::Undecodable | ReadError::Unsure => Fault::Decline,
        })?;
    // Desde aquí nunca `Fault::Decline` (D1).
    let now = (opts.clock)().div_euclid(1000);
    // 2. Paneles vivos (y los dos archivos de entorno, en el mismo salto).
    let Live {
        panes: live,
        tmux_panes,
        conf,
        usage_file,
    } = live_panes(native, now).await.map_err(no_decline)?;
    let record = live.is_some();
    let live = live.unwrap_or_default();
    // 4. Entorno.
    let env = runtime_env(&conf, &usage_file, &opts.usage_env, settings)?;
    // 5. Límites (efecto: puede lanzar el refresco).
    let limits = native.limits.get(&native.refresh_deps());
    // 8. `record_pane` de cada panel vivo (errores ignorados, el `except
    // Exception: pass` de 7253): todos en una transacción, en un trabajo del
    // carril de escritura (el de importación) que la respuesta no espera (R3).
    // El Python registra antes del memo, pero el memo no lee `usage_panes`
    // cuando hay vivos: el orden no cambia la respuesta.
    if record && opts.usage_effects && !live.is_empty() {
        record_latest_panes(&native.usage_engine, &native.import_lane, live.clone());
    }
    let (alerts, recent) = native
        .usage
        .with_retry(|u| -> Result<_, ReadError> {
            let alerts = usage_read::list_alerts(&u.conn, ALERTS_LIMIT)?;
            let recent = usage_read::recent_interactions(&u.conn, 1)?;
            Ok((alerts, recent))
        })
        .await
        .map_err(no_decline)?
        .map_err(|_| failure())?;
    // 6. Memo.
    let memo = native
        .usage_engine
        .state(native, &live, env.clone())
        .await?;
    // 7. La respuesta parte de una copia superficial del memo: en Rust se
    // escriben las entradas del memo con estas claves encima.
    let mut over: Vec<(&str, Value)> = Vec::new();
    let mut put = |key: &'static str, value: Value| match over.iter_mut().find(|(k, _)| *k == key) {
        Some(slot) => slot.1 = value,
        None => over.push((key, value)),
    };
    let rows: Vec<Value> = limits.rows.into_iter().map(Value::Object).collect();
    put("limits", Value::Array(rows));
    let mut health = match usage_state::credential_health(&env) {
        Value::Object(map) => map,
        _ => return Err(failure()),
    };
    for (key, value) in limits.health {
        health.insert(key, value);
    }
    put("credential_health", Value::Object(health));
    put(
        "alerts",
        Value::Array(alerts.into_iter().map(Value::Object).collect()),
    );
    // D5: `attach_token_counts` sobre las filas cacheadas, y `enrich_limits`.
    // `windows` sale del memo: ninguna clave de encima la toca.
    let attached: Vec<Value> = native
        .limits
        .attach_tokens(&memo.windows)
        .into_iter()
        .map(Value::Object)
        .collect();
    let lang = lang_of(conf.cc_lang()?.as_deref(), &opts.usage_env);
    let enriched = allocation::enrich_limits(&attached, (opts.clock)() as f64 / 1000.0, lang)
        .map_err(|_| failure())?;
    put("limits", Value::Array(enriched));
    let last = recent.into_iter().next().map_or(Value::Null, Value::Object);
    put("lastInteraction", last);
    // 9. Cuerpo.
    let body = memo.body(&over).map_err(|_| failure())?;
    Ok(UsageStateReply {
        body,
        state: memo,
        live_panes: live,
        live_declined: !record,
        tmux_panes,
    })
}

#[cfg(test)]
mod tests {
    use super::{Lane, Row, UsageEngine, UsageImportBackend, record_latest_panes};
    use serde_json::json;
    use std::sync::{Arc, mpsc};

    fn pane(id: &str) -> Row {
        let Ok(serde_json::Value::Object(row)) = serde_json::to_value(json!({
            "tmux_session": "s", "tmux_pane": id, "pane_pwd": "/r", "agent": "claude",
            "last_seen_at": 1, "started_at": 1,
        })) else {
            unreachable!()
        };
        row
    }

    /// Con el carril de escritura ocupado, tres vueltas dejan un solo trabajo
    /// pendiente y se escribe el último juego de paneles.
    #[tokio::test]
    async fn record_panes_keeps_only_the_latest_live_set() {
        let dir = std::env::temp_dir().join(format!("2e-panes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("comandos-usage.sqlite");
        drop(comandos_store::usage::open_usage_db_at(&db).unwrap());
        let lane = Arc::new(Lane::<UsageImportBackend>::new(db.clone()));
        let engine = Arc::new(UsageEngine::default());
        let (release, wait) = mpsc::channel::<()>();
        let busy = {
            let lane = lane.clone();
            tokio::spawn(async move { lane.with(move |_| wait.recv()).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        for id in ["%1", "%2", "%3"] {
            record_latest_panes(&engine, &lane, vec![pane(id)]);
        }
        assert!(engine.panes.lock().unwrap().queued);
        release.send(()).unwrap();
        assert!(matches!(busy.await.unwrap(), Ok(Ok(()))));
        for _ in 0..100 {
            if !engine.panes.lock().unwrap().queued {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let Ok(ids) = lane
            .with(|u| {
                let mut stmt = u.conn.prepare("select tmux_pane from usage_panes").unwrap();
                stmt.query_map([], |r| r.get::<_, String>(0))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            })
            .await
        else {
            panic!("carril");
        };
        assert_eq!(ids, ["%3"]);
        lane.shutdown().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// M3 de la revisión final: una reconstrucción del memo que ya corre en el
    /// carril se guarda aunque quien la pidió se haya ido (un cliente que corta
    /// por plazo). La petición siguiente la reutiliza en vez de reconstruir.
    #[tokio::test]
    async fn abandoned_rebuild_is_kept() {
        let home = std::env::temp_dir().join(format!("2e-memo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let opts = crate::dash::native::NativeOptions::for_home(&home, home.join("state.sqlite3"));
        std::fs::create_dir_all(&opts.hooks).unwrap();
        // Bastantes turnos en la ventana para que la reconstrucción dure.
        {
            let mut conn = comandos_store::usage::open_usage_db_at(&opts.usage_db).unwrap();
            comandos_store::usage::ensure_schema(&conn).unwrap();
            let now = (opts.clock)().div_euclid(1000);
            let tx = conn.transaction().unwrap();
            {
                let mut insert = tx
                    .prepare(
                        "insert into usage_turns(id,provider,agent,tmux_session,tmux_pane,pane_pwd,\
                         git_root,model,turn_started_at,turn_finished_at,total_tokens,cost_usd,\
                         source,confidence,harness_account) \
                         values(?,?,?,?,?,?,?,?,?,?,?,0.001,'test','exact','main')",
                    )
                    .unwrap();
                for i in 0..80_000i64 {
                    let b = i / 500;
                    insert
                        .execute(rusqlite::params![
                            format!("t-{i:08}"),
                            ["claude", "codex"][(b % 2) as usize],
                            "claude",
                            format!("s-{}", b % 30),
                            format!("%{}", b % 60),
                            format!("/r/p-{}", b % 20),
                            format!("/r/p-{}", b % 20),
                            format!("m-{}", b % 5),
                            now - 10 - i * 10,
                            now - i * 10,
                            1_000 + i,
                        ])
                        .unwrap();
                }
            }
            tx.commit().unwrap();
        }
        let native = crate::dash::native::Native::new(opts);
        let engine = native.usage_engine.clone();
        {
            let first = engine.state(&native, &[], Row::new());
            tokio::pin!(first);
            loop {
                tokio::select! {
                    biased;
                    done = &mut first => panic!("terminó antes de abandonarla: {:?}", done.is_ok()),
                    () = tokio::time::sleep(std::time::Duration::from_millis(1)) => {
                        if engine.builds() == 1 {
                            break;
                        }
                    }
                }
            }
            // `first` se suelta aquí con la reconstrucción en curso en el carril.
        }
        assert!(engine.state(&native, &[], Row::new()).await.is_ok());
        assert_eq!(
            engine.builds(),
            1,
            "la segunda petición reutiliza la abandonada"
        );
        native.shutdown().await;
        let _ = std::fs::remove_dir_all(&home);
    }
}
