//! Rutas que el frente responde sin el Python (Fase 2b).
//!
//! Cada ruta reproduce byte a byte una rama de `bin/cc-dash`. Lo que toca la
//! base pasa por un único `BackendWorker<StateBackend>`; lo que toca tmux o
//! `fc-list`, por `tokio::process`. Si una entrada no se puede reproducir con
//! certeza, el manejador devuelve `Fault::Decline` ANTES de cualquier efecto
//! y el frente reenvía la petición original al heredado.
pub mod background;
pub mod catalog_cli;
pub mod catalogs;
pub mod census;
pub mod events;
pub mod feed;
pub mod files;
pub mod input;
pub mod lanes;
pub mod light;
pub mod news;
pub mod notices;
pub mod operations;
pub mod ops;
pub mod pomodoro;
pub mod procs;
pub mod push;
pub mod py;
pub mod query;
pub mod quick;
pub mod remote;
pub mod residue;
pub mod resources;
pub mod retired;
pub mod sessions;
pub mod settings;
pub mod snippets;
pub mod sidebar;
mod chat;
pub mod ssh;
pub mod state;
pub mod states;
pub mod tabs;
pub mod target;
pub mod terminal;
mod terminal_links;
pub mod tmux;
pub mod typing;
pub mod ui_log;
pub mod usage;
pub mod workspace;

use crate::{
    HandlerError, Reply, Request,
    blocking::{BackendCaller, BackendWorker},
    dash::router::path_of,
};
use comandos_core::usage_state::LocalZone;
use http::Method;
use serde_json::{Map, Value};
use state::{Refusal, StateBackend};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::OnceCell;

/// Trabajos en cola del worker de la base (sin contar el que corre).
pub const WORKER_CAPACITY: usize = 64;

/// GET `/usage/state` nativo con todos sus efectos (Tarea 8, D1) y el refresco
/// de límites al arrancar que va con ella. La puerta de ligereza (`xtask poll
/// --shadow` realista, `--max-pss-mib 56`) se mide con el `GLIBC_TUNABLES` de
/// producción (`comandos_core::malloc_tuning`, en el drop-in de `cc-dash`): sin
/// él la ruta funciona igual, pero el memo que se reconstruye tras cada
/// importación deja ≈ 61 MiB retenidos en las arenas de glibc.
pub const USAGE_STATE_NATIVE: bool = true;

/// Gracia de la primera importación de uso del frente tras arrancar (D1 c): un
/// hilo de importación del Python que estuviera en curso termina antes.
pub const USAGE_IMPORT_GRACE_MS: i64 = 75_000;
/// `COMANDOS_DASH_USAGE_IMPORT_GRACE_MS`: otra gracia (entero ≥ 0, en ms).
pub const USAGE_IMPORT_GRACE_ENV: &str = "COMANDOS_DASH_USAGE_IMPORT_GRACE_MS";

/// Cada dominio añade su variante en su tarea.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeRoute {
    Light(light::LightRoute),
    Events,
    Notices(notices::NoticesRoute),
    Workspace(workspace::WorkspaceRoute),
    Snippets(snippets::SnippetsRoute),
    UiLog,
    Pomodoro,
    Catalog(catalogs::CatalogRoute),
    Terminal(terminal::TerminalRoute),
    PaneType,
    ModelStatus,
    State,
    QuickTerminal,
    Usage(usage::UsageRoute),
    Retired,
    Sidebar,
    Chat,
    Feed(feed::FeedRoute),
    // Cortes de la 2f (D2): cada variante pertenece a un `Cut` (ver `cut`).
    Tabs(tabs::TabsRoute),
    Sessions(sessions::SessionsRoute),
    Input(input::InputRoute),
    Ops(ops::OpsRoute),
    Remote(remote::RemoteRoute),
    Ssh(ssh::SshRoute),
    Settings(settings::SettingsRoute),
    Cli(catalog_cli::CliRoute),
    Push(push::PushRoute),
    News(news::NewsRoute),
    Residue(residue::ResidueRoute),
}

/// Grupo de rutas que comparte un estado con un solo dueño (D2 del plan 2f):
/// se activa y se revierte entero con `COMANDOS_DASH_CUTS_OFF`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Cut {
    /// Rutas de 2b–2e: solo `COMANDOS_DASH_NATIVE=0` las apaga.
    Base,
    Tabs,
    Ops,
    Services,
    News,
    Residue,
}

impl Cut {
    /// Un nombre de la lista de `COMANDOS_DASH_CUTS_OFF` (separada por comas).
    /// `base` no se puede apagar por corte: no tiene nombre.
    pub fn parse(name: &str) -> Option<Cut> {
        match name.trim() {
            "tabs" => Some(Cut::Tabs),
            "ops" => Some(Cut::Ops),
            "services" => Some(Cut::Services),
            "news" => Some(Cut::News),
            "residue" => Some(Cut::Residue),
            _ => None,
        }
    }
}

impl NativeRoute {
    pub fn cut(self) -> Cut {
        match self {
            NativeRoute::Tabs(_) | NativeRoute::Sessions(_) | NativeRoute::Input(_) => Cut::Tabs,
            NativeRoute::Ops(_) => Cut::Ops,
            NativeRoute::Remote(_)
            | NativeRoute::Ssh(_)
            | NativeRoute::Settings(_)
            | NativeRoute::Cli(_)
            | NativeRoute::Push(_) => Cut::Services,
            NativeRoute::News(_) => Cut::News,
            NativeRoute::Residue(_) => Cut::Residue,
            // Sin `_`: una variante nueva tiene que decidir su corte aquí.
            NativeRoute::Light(_)
            | NativeRoute::Events
            | NativeRoute::Notices(_)
            | NativeRoute::Workspace(_)
            | NativeRoute::Snippets(_)
            | NativeRoute::UiLog
            | NativeRoute::Pomodoro
            | NativeRoute::Catalog(_)
            | NativeRoute::Terminal(_)
            | NativeRoute::PaneType
            | NativeRoute::ModelStatus
            | NativeRoute::State
            | NativeRoute::QuickTerminal
            | NativeRoute::Usage(_)
            | NativeRoute::Retired
            | NativeRoute::Sidebar
            | NativeRoute::Chat
            | NativeRoute::Feed(_) => Cut::Base,
        }
    }
}

/// D3: ¿declina el despachador esta ruta sin evaluar nada? `Cut::Base` nunca,
/// aunque alguien lo meta en la lista.
pub fn cut_is_off(cuts_off: &BTreeSet<Cut>, cut: Cut) -> bool {
    cut != Cut::Base && cuts_off.contains(&cut)
}

/// Qué hilos de fondo arranca el frente (D6 del plan 2f).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Background {
    pub pomodoro: bool,
    pub model_watch: bool,
    pub news: bool,
    pub limits_snapshot: bool,
    pub notices_push: bool,
    pub webterm_restore: bool,
}

impl Background {
    /// El Python sigue vivo: solo lo idempotente con él (el planificador de
    /// Pomodoro). Valor por omisión en toda la 2f.
    pub const fn legacy() -> Self {
        Self {
            pomodoro: true,
            model_watch: false,
            news: false,
            limits_snapshot: false,
            notices_push: false,
            webterm_restore: false,
        }
    }

    /// 2g: el frente es el único dueño.
    pub const fn front() -> Self {
        Self {
            pomodoro: true,
            model_watch: true,
            news: true,
            limits_snapshot: true,
            notices_push: true,
            webterm_restore: true,
        }
    }

    /// `legacy` o `front` (bandera `--background`, `COMANDOS_DASH_BACKGROUND`).
    pub fn parse(name: &str) -> Option<Background> {
        match name.trim() {
            "legacy" => Some(Background::legacy()),
            "front" => Some(Background::front()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Get,
    Post,
    /// `do_DELETE` del Python (2f).
    Delete,
    /// `do_HEAD`: ninguna tabla de 2b–2e lo usa; el residuo (2f-3/T7) lo
    /// reclamará con `Key::Prefix("/")` (R2 del preflight).
    Head,
}

/// Cómo compara el Python la ruta (sin decodificar `%XX`).
#[derive(Debug, Clone, Copy)]
pub enum Key {
    /// `urlsplit(self.path).path == p` o `self.path.startswith(p)`: se
    /// reclama solo la ruta exacta, con consulta opcional.
    Path(&'static str),
    /// `self.path == p or self.path.startswith(p + "?")`.
    ExactOrQuery(&'static str),
    /// `self.path == p`: sin consulta.
    Raw(&'static str),
    /// `self.path.startswith(p)` del Python sobre la ruta cruda (con consulta):
    /// el residuo del despachador (2f-3/T7) reclama así `/stateX`, `/prefs/…`.
    Prefix(&'static str),
}

impl Key {
    pub fn matches(self, target: &str) -> bool {
        match self {
            Key::Path(p) => path_of(target) == p,
            Key::ExactOrQuery(p) => {
                target == p
                    || target
                        .strip_prefix(p)
                        .is_some_and(|rest| rest.starts_with('?'))
            }
            Key::Raw(p) => target == p,
            Key::Prefix(p) => target.starts_with(p),
        }
    }
}

pub struct Entry {
    pub verb: Verb,
    pub key: Key,
    pub route: NativeRoute,
}

/// Una tabla por dominio; cada tarea añade la suya.
const TABLES: &[&[Entry]] = &[
    light::ROUTES,
    events::ROUTES,
    notices::ROUTES,
    workspace::ROUTES,
    snippets::ROUTES,
    ui_log::ROUTES,
    pomodoro::ROUTES,
    catalogs::ROUTES,
    terminal::ROUTES,
    typing::ROUTES,
    operations::ROUTES,
    states::ROUTES,
    quick::ROUTES,
    usage::ROUTES,
    retired::ROUTES,
    sidebar::ROUTES,
    chat::ROUTES,
    feed::ROUTES,
    // Cortes de la 2f. `residue` va la última: sus prefijos no deben tapar
    // ninguna entrada exacta.
    tabs::ROUTES,
    sessions::ROUTES,
    input::ROUTES,
    ops::ROUTES,
    remote::ROUTES,
    ssh::ROUTES,
    settings::ROUTES,
    catalog_cli::ROUTES,
    push::ROUTES,
    news::ROUTES,
    residue::ROUTES,
];

/// Cuántas tablas de `TABLES` son de la base (2b–2e): las primeras, antes de
/// las de los cortes de la 2f. Quien añada una tabla de base lo sube aquí.
#[cfg(test)]
const BASE_TABLES: usize = 18;

pub fn route(method: &Method, target: &str) -> Option<NativeRoute> {
    if *method == Method::GET
        && let Some(route) = residue::ordered_get_route(target)
    {
        return Some(route);
    }
    route_in_tables(method, target)
}

fn route_in_tables(method: &Method, target: &str) -> Option<NativeRoute> {
    let verb = if *method == Method::GET {
        Verb::Get
    } else if *method == Method::POST {
        Verb::Post
    } else if *method == Method::DELETE {
        Verb::Delete
    } else if *method == Method::HEAD {
        Verb::Head
    } else {
        return None;
    };
    TABLES
        .iter()
        .flat_map(|table| table.iter())
        .find(|entry| entry.verb == verb && entry.key.matches(target))
        .map(|entry| entry.route)
}

pub enum Outcome {
    Reply(Reply),
    /// Nada se escribió: el frente reenvía la petición original.
    Decline,
}

pub enum Fault {
    Decline,
    Error(HandlerError),
}

impl From<HandlerError> for Fault {
    fn from(error: HandlerError) -> Self {
        Fault::Error(error)
    }
}

pub type Answer = Result<Reply, Fault>;

/// `self._json(status, value)`: cuerpo con `json.dumps` del Python.
pub fn reply(status: http::StatusCode, value: &serde_json::Value) -> Answer {
    Reply::json(status, value).map_err(Fault::from)
}

/// El cuerpo de un `DELETE` ya admitido. La puerta de `do_DELETE` (tope de
/// 64 000 → 413 con cierre, JSON roto → 400 con cierre, no-objeto → 400) la
/// aplica el transporte antes del manejador (`dashboard_access`), igual que
/// la de `do_POST`: aquí solo queda el objeto. Sin él (no debería pasar) es un
/// fallo interno, como el `data.get` del Python sobre algo que no es `dict`.
pub fn delete_body(request: &Request) -> Result<&Map<String, Value>, Fault> {
    request
        .data
        .as_ref()
        .and_then(Value::as_object)
        .ok_or(Fault::Error(HandlerError::Failure))
}

/// Milisegundos Unix; las pruebas lo sustituyen.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub fn wall_clock_ms() -> i64 {
    comandos_runtime::now_ms().map_or(0, |ms| i64::try_from(ms).unwrap_or(i64::MAX))
}

/// Segundos Unix con la precisión de `time.time()`; las pruebas lo sustituyen.
pub type SecondsClock = Arc<dyn Fn() -> f64 + Send + Sync>;

/// One facts instance per state worker, created lazily at its first event.
/// The default native path keeps its concrete facts and allocates no box.
pub type EventFactsFactory =
    Arc<dyn Fn() -> Box<dyn crate::events_routes::Facts + Send> + Send + Sync>;

/// `time.time()` del Python: los nanosegundos de `CLOCK_REALTIME` (el de
/// `SystemTime`) convertidos como `_PyTime_AsSecondsDouble`.
pub fn python_time() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX));
    python_seconds(nanos)
}

/// `_PyTime_AsSecondsDouble`: segundos exactos si no hay fracción; si no,
/// `(double)ns / 1e9`. Mismo `double`, bit a bit, que escribe el Python.
pub fn python_seconds(nanos: i64) -> f64 {
    const NS: i64 = 1_000_000_000;
    if nanos % NS == 0 {
        (nanos / NS) as f64
    } else {
        nanos as f64 / 1e9
    }
}

/// `"desktop-" + re.sub(r"[^A-Za-z0-9_.-]", "-", os.uname().nodename or "local")[:60]`.
/// `/proc/sys/kernel/hostname` es el `nodename` de `uname`.
pub fn desktop_device() -> String {
    let node = comandos_runtime::platform::hostname()
        .map(|s| s.trim_end_matches('\n').to_owned())
        .unwrap_or_default();
    let node = if node.is_empty() {
        "local".to_owned()
    } else {
        node
    };
    let clean: String = node
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(60)
        .collect();
    format!("desktop-{clean}")
}

#[derive(Clone)]
pub struct NativeOptions {
    pub term_target: crate::dash::term::attach::TmuxTarget,
    pub state_db: PathBuf,
    /// `~/.claude/hooks` (el `HOOKS` del Python).
    pub hooks: PathBuf,
    pub clock: Clock,
    /// `time.time()` de las escrituras del journal (`recover_abandoned`).
    pub clock_seconds: SecondsClock,
    /// Optional event intake observations; None retains the native clock,
    /// UUID generator and kernel process identity without a per-request lock.
    pub events_facts: Option<EventFactsFactory>,
    /// `tmux` como lo llama el Python (entorno heredado, plazo 5 s).
    pub tmux: tmux::Tmux,
    /// `fc-list` de `_installed_font_families` (7603).
    pub fc_list: tmux::Program,
    /// `~/.claude/hooks/comandos-usage.sqlite`.
    pub usage_db: PathBuf,
    /// `~/.claude/hooks/session-operations.sqlite3` (journal de operaciones).
    pub journal_db: PathBuf,
    /// `DESKTOP_DEVICE` del Python (999).
    pub desktop_device: String,
    /// Checkout del heredado (`REPO_ROOT`): de él sale `config/model-tiers.json`.
    pub repo_root: Option<PathBuf>,
    /// Raíz de estáticos autorizada por DashConfig, nunca inferida del HOME.
    pub dash_dir: Option<PathBuf>,
    /// `HOME` del frente (`os.path.expanduser("~")` del Python).
    pub home: PathBuf,
    /// Raíz de `/proc` (las pruebas de capa usan una falsa; el frente, la real).
    pub proc_root: PathBuf,
    /// Heredado y token del tablero (`build` los fija). Desde la Tarea 5a de la
    /// 2e ningún cómputo nativo los lee: el contexto de sugerencias de GET
    /// `/state` ya no pide nada al heredado.
    pub legacy: SocketAddr,
    pub legacy_token: Vec<u8>,
    /// `PATH` del frente al arrancar, para `providers::which` (D9).
    pub search_path: Option<OsString>,
    /// Respaldos de `providers::which` tras el `PATH` (`_USER_BIN_DIRS`):
    /// `providers::USER_BIN_DIRS` en producción; el gemelo deja solo los del
    /// HOME temporal, nunca `/usr/local/bin`.
    pub user_bin_dirs: Vec<String>,
    /// Directorio de trabajo del frente (rutas relativas del catálogo y cuentas).
    pub cwd: PathBuf,
    /// `CODEX_HOME` / `GROK_HOME` del frente (catálogos de modelos).
    pub codex_home: Option<PathBuf>,
    pub grok_home: Option<PathBuf>,
    /// `ssh` de `ssh_state` (7708).
    pub ssh: tmux::Program,
    /// `systemd-run --user --scope --collect --quiet` de `scope_cmd` (5420) para
    /// lanzar la terminal rápida en Linux; `None` declina POST `/terminal/quick`
    /// allí. Darwin conserva el programa tmux directo y no consulta el scope.
    pub scope: Option<tmux::Program>,
    /// `quick_terminal_lib.default_base()` (carpetas de la terminal rápida).
    pub quick_base: PathBuf,
    /// Cliente de `api.anthropic.com/api/oauth/usage`; las pruebas ponen uno falso.
    pub oauth: Arc<dyn usage::limits::OauthHttp>,
    /// cc-notifyd (`127.0.0.1:4778`) de los avisos de nivel; las pruebas ponen
    /// uno falso que solo guarda los cuerpos.
    pub notifyd: Arc<dyn usage::pane_models::NotifyPost>,
    /// Feeds de noticias del vigilante de modelos (`news_watch`): las URLs del
    /// Python en producción; las pruebas los llevan a un servidor local.
    pub news_feeds: Arc<background::models::Feeds>,
    /// Falso en la sombra (`--no-usage-effects`): sin refresco de límites (ni red
    /// ni escrituras) ni el resto de efectos de uso de la fase.
    pub usage_effects: bool,
    /// Las claves de entorno que el Python de uso lee (D7), tomadas al arrancar.
    /// De `OPENAI_ADMIN_KEY`/`ANTHROPIC_ADMIN_KEY` solo la presencia (R6).
    pub usage_env: Arc<BTreeMap<String, String>>,
    /// Explicit environment snapshot for read-only extension inventories.
    pub extension_env: Arc<BTreeMap<String, String>>,
    /// Zona de `datetime.fromtimestamp` (la del proceso: `TZ` o `/etc/localtime`).
    pub zone: Arc<dyn LocalZone + Send + Sync>,
    /// Qué hilos de fondo arranca el frente (D6); `legacy` en toda la 2f.
    pub background: Background,
    /// Complete native dashboard: optional recommendation failures must not hide sessions.
    pub standalone: bool,
    /// Cortes desactivados (D3): sus rutas declinan sin evaluar nada.
    pub cuts_off: BTreeSet<Cut>,
    /// Archivo del censo de declinaciones (D7). `None`: no se escribe (las
    /// pruebas; `build` lo fija desde `DashConfig`).
    pub census_path: Option<PathBuf>,
    /// `XDG_STATE_HOME` del frente al arrancar (`media_dir` de noticias): las
    /// pruebas lo fijan al HOME temporal sin tocar el entorno del proceso.
    pub xdg_state_home: Option<PathBuf>,
    /// Puertos de salud de la terminal web (`4779`, `4780` en el Python): las
    /// pruebas los cambian para no tocar nunca la terminal web real.
    pub webterm_health_ports: [u16; 2],
    /// Entorno COMPLETO de los programas que lanza el frente fuera de tmux
    /// (`program`). `None` (producción): el del proceso, como el Python. Las
    /// pruebas confinadas fijan el del HOME temporal (sin `DISPLAY`, DBus ni el
    /// `PATH` del desarrollador).
    pub child_env: Option<Vec<(OsString, OsString)>>,
    /// `DISPLAY` que ve `procs::gui_env_for`: `None` (producción) = el del
    /// proceso; `Some(None)` = ausente; `Some(Some(v))` = `v`.
    pub display: Option<Option<OsString>>,
    /// Milisegundos desde el arranque antes de la primera importación de uso.
    pub usage_import_grace_ms: i64,
    /// `USAGE_STATE_NATIVE` en producción. Apagado, GET `/usage/state` declina
    /// antes de cualquier efecto y no hay refresco de límites al arrancar.
    pub usage_state_native: bool,
}

/// `USAGE_IMPORT_GRACE_ENV` si es un entero ≥ 0; si no, la de omisión.
pub fn usage_import_grace_from_env() -> i64 {
    std::env::var(USAGE_IMPORT_GRACE_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|ms| *ms >= 0)
        .unwrap_or(USAGE_IMPORT_GRACE_MS)
}

/// D7: lo que `usage_runtime_env` toma de `os.environ`.
pub const USAGE_ENV_KEYS: &[&str] = &[
    "COMANDOS_DAILY_BUDGET_USD",
    "COMANDOS_USAGE_DAILY_BUDGET_USD",
    "COMANDOS_CODEX_DAILY_TOKEN_LIMIT",
    "COMANDOS_CODEX_WEEKLY_TOKEN_LIMIT",
    "COMANDOS_CLAUDE_DAILY_TOKEN_LIMIT",
    "COMANDOS_CLAUDE_WEEKLY_TOKEN_LIMIT",
    "CODEX_DAILY_TOKEN_LIMIT",
    "CODEX_WEEKLY_TOKEN_LIMIT",
    "CLAUDE_DAILY_TOKEN_LIMIT",
    "CLAUDE_WEEKLY_TOKEN_LIMIT",
    "COMANDOS_USAGE_LOCAL_DAYS",
    "COMANDOS_USAGE_CLAUDE_MAX_FILES",
    "COMANDOS_USAGE_CODEX_MAX_FILES",
    "COMANDOS_CLAUDE_PROJECTS_DIR",
    "COMANDOS_OPENCODE_DB",
    "OPENAI_ADMIN_KEY",
    "ANTHROPIC_ADMIN_KEY",
    "LANG",
];

/// Claves cuyo valor es un secreto: se guarda `"1"` si están definidas y no
/// vacías (lo único que el Python mira de ellas es su veracidad).
const SECRET_ENV_KEYS: &[&str] = &["OPENAI_ADMIN_KEY", "ANTHROPIC_ADMIN_KEY"];

/// `usage_env` desde el entorno del proceso (`vars_os`: un valor que no es
/// UTF-8 se ignora en vez de entrar en pánico, C6).
pub fn usage_env_from_process() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (key, value) in std::env::vars_os() {
        let (Some(key), Some(value)) = (key.to_str(), value.to_str()) else {
            continue;
        };
        if !USAGE_ENV_KEYS.contains(&key) {
            continue;
        }
        let value = if SECRET_ENV_KEYS.contains(&key) {
            if value.is_empty() { "" } else { "1" }
        } else {
            value
        };
        out.insert(key.to_owned(), value.to_owned());
    }
    out
}

impl NativeOptions {
    pub fn for_home(home: &Path, state_db: PathBuf) -> Self {
        Self {
            term_target: crate::dash::term::attach::TmuxTarget::User,
            state_db,
            hooks: home.join(".claude/hooks"),
            clock: Arc::new(wall_clock_ms),
            clock_seconds: Arc::new(python_time),
            events_facts: None,
            tmux: tmux::Tmux::system(),
            fc_list: tmux::Program::named("fc-list"),
            usage_db: home.join(".claude/hooks/comandos-usage.sqlite"),
            journal_db: home.join(".claude/hooks/session-operations.sqlite3"),
            desktop_device: desktop_device(),
            repo_root: None,
            dash_dir: None,
            home: home.to_path_buf(),
            proc_root: PathBuf::from("/proc"),
            legacy: SocketAddr::from((Ipv4Addr::LOCALHOST, crate::dash::DEFAULT_LEGACY_PORT)),
            legacy_token: Vec::new(),
            search_path: std::env::var_os("PATH"),
            user_bin_dirs: comandos_runtime::providers::USER_BIN_DIRS
                .iter()
                .map(|d| (*d).to_owned())
                .collect(),
            cwd: std::env::current_dir().unwrap_or_else(|_| home.to_path_buf()),
            codex_home: env_path("CODEX_HOME"),
            grok_home: env_path("GROK_HOME"),
            ssh: tmux::Program::named("ssh"),
            scope: quick::find_scope(std::env::var_os("PATH").as_deref()),
            quick_base: quick::default_base(home),
            oauth: Arc::new(usage::limits::ReqwestOauth::default()),
            notifyd: Arc::new(usage::pane_models::HyperNotify::default()),
            news_feeds: Arc::new(background::models::Feeds::production()),
            usage_effects: true,
            usage_env: Arc::new(usage_env_from_process()),
            extension_env: Arc::new(
                ["XDG_CONFIG_HOME", "OPENCODE_CONFIG", "OPENCODE_CONFIG_DIR"]
                    .into_iter()
                    .filter_map(|k| std::env::var(k).ok().map(|v| (k.into(), v)))
                    .collect(),
            ),
            zone: Arc::new(chrono::Local),
            background: Background::legacy(),
            standalone: false,
            cuts_off: BTreeSet::new(),
            census_path: None,
            xdg_state_home: env_path("XDG_STATE_HOME"),
            webterm_health_ports: [4779, 4780],
            child_env: None,
            display: None,
            usage_import_grace_ms: usage_import_grace_from_env(),
            usage_state_native: USAGE_STATE_NATIVE,
        }
    }

    /// Un programa externo del frente (`path`, normalmente de
    /// `procs::which_in`) con el entorno de los hijos: el del proceso en
    /// producción; solo `child_env` si está fijado (pruebas confinadas).
    pub fn program(&self, path: impl Into<PathBuf>) -> tmux::Program {
        let mut program = tmux::Program::named(path);
        if let Some(env) = &self.child_env {
            program.env_clear = true;
            program.env = env.clone();
        }
        program
    }
}

/// Variable de entorno no vacía como ruta.
fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

pub struct Native {
    opts: NativeOptions,
    pub(crate) credits: Arc<sidebar::CreditsCache>,
    enabled: AtomicBool,
    pub(crate) news_scheduler_started: AtomicBool,
    background_owner: Mutex<Option<background::Owner>>,
    refusals: AtomicUsize,
    state: OnceCell<Option<BackendCaller<StateBackend>>>,
    worker: Mutex<Option<BackendWorker<StateBackend>>>,
    /// `_PREFS_LOCK` del Python.
    pub(crate) prefs_lock: tokio::sync::Mutex<()>,
    /// `_FONT_CACHE`: familias de `fc-list` y cuándo se leyeron.
    pub(crate) fonts: Mutex<Option<(std::time::Instant, std::collections::HashSet<String>)>>,
    pub(crate) resources: Arc<resources::Cache>,
    /// La revisión de avisos que comparten las esperas de `/notices/watch`.
    pub(crate) notice_feed: notices::RevisionFeed,
    /// Carril de la base de uso (`GET /pomodoro`, `GET /sovereignty`,
    /// `GET /analytics/week`, `GET /accounts`, `GET /extension-usage`); `Arc`
    /// para las tareas de fondo (D13).
    pub(crate) usage: Arc<lanes::Lane<lanes::UsageBackend>>,
    /// Caché de límites de proveedor (`_limits_cache`).
    pub(crate) limits: Arc<usage::limits::LimitsCache>,
    /// `_provider_registry_cache` de GET `/providers`, `/optimization/plans` y
    /// `/accounts` (se usa en hilos de bloqueo).
    pub(crate) registry: Arc<Mutex<comandos_runtime::providers::RegistryCache>>,
    /// Escritor de bordes de pane y `pane-models.txt` (latente hasta la Tarea 8).
    pub(crate) pane_models: Arc<usage::pane_models::PaneModelWriter>,
    /// `_TIER_LAST`/`_TIER_ALERTED` de los avisos de nivel (latente).
    pub(crate) tier_alerts: Mutex<usage::pane_models::TierAlerts>,
    /// `_ACCOUNT_PID_CACHE` de los valores de borde: caché propia, nunca la
    /// del escaneo de GET `/state` (su candado cubre todo el escaneo).
    pub(crate) pane_accounts: Arc<Mutex<comandos_runtime::agent_procs::AccountCache>>,
    /// Carril del journal de operaciones (`GET /model/status`).
    pub(crate) journal: lanes::Lane<lanes::JournalBackend>,
    /// Caché por `requestId` y candados de `POST /pane/type`.
    pub(crate) typing: Arc<typing::TypingState>,
    /// GET `/state`: caché de 1,2 s, vuelo único y cachés de los lectores.
    pub(crate) states: states::Engine,
    /// GET `/session-feed`: último cálculo, vuelo único y cachés de transcripts.
    pub(crate) feed: feed::FeedState,
    /// Censo de declinaciones (D7).
    pub(crate) census: Arc<census::DeclineCensus>,
    /// Tareas largas del frente (D12).
    pub(crate) tasks: Arc<procs::TaskTracker>,
    /// Foto de `/remote-state` (`_remote_state_cache`, 2f-3/T2).
    pub(crate) remote: remote::RemoteCache,
    /// Cachés del catálogo de CLIs y de los modelos de OpenCode (2f-3/T4).
    pub(crate) cli: catalog_cli::CliState,
    /// Memo de GET `/usage/state` (`cached_usage_state`) y su generación; `Arc`
    /// para que la importación suba la generación al terminar (D13).
    pub usage_engine: Arc<usage::state::UsageEngine>,
    /// Segundo carril sobre la base de uso: la importación y `record_pane` (D2, R3).
    pub(crate) import_lane: Arc<lanes::Lane<lanes::UsageImportBackend>>,
    /// Dueño de la importación de uso (D1): intervalo, gracia, candado entre frentes.
    pub(crate) import: Arc<usage::import::ImportOwner>,
}

impl Native {
    pub fn new(opts: NativeOptions) -> Self {
        Self {
            usage: Arc::new(lanes::Lane::new(opts.usage_db.clone())),
            import_lane: Arc::new(lanes::Lane::new(opts.usage_db.clone())),
            import: Arc::new(usage::import::ImportOwner::new((opts.clock)())),
            limits: Arc::default(),
            registry: Arc::default(),
            pane_models: Arc::default(),
            tier_alerts: Mutex::default(),
            pane_accounts: Arc::default(),
            journal: lanes::Lane::new(opts.journal_db.clone()),
            opts,
            credits: Arc::default(),
            enabled: AtomicBool::new(true),
            news_scheduler_started: AtomicBool::new(false),
            background_owner: Mutex::new(None),
            refusals: AtomicUsize::new(0),
            state: OnceCell::new(),
            worker: Mutex::new(None),
            prefs_lock: tokio::sync::Mutex::new(()),
            fonts: Mutex::new(None),
            resources: Arc::default(),
            notice_feed: notices::RevisionFeed::default(),
            typing: Arc::default(),
            states: states::Engine::default(),
            feed: feed::FeedState::default(),
            census: Arc::default(),
            tasks: Arc::default(),
            remote: remote::RemoteCache::default(),
            cli: catalog_cli::CliState::default(),
            usage_engine: Arc::default(),
        }
    }

    pub fn options(&self) -> &NativeOptions {
        &self.opts
    }

    /// El censo de declinaciones (D7).
    pub fn census(&self) -> &Arc<census::DeclineCensus> {
        &self.census
    }

    /// Las tareas largas del frente (D12).
    pub fn tasks(&self) -> &Arc<procs::TaskTracker> {
        &self.tasks
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Cuántas veces se escribió la línea de desactivación (0 o 1).
    pub fn refusals(&self) -> usize {
        self.refusals.load(Ordering::Acquire)
    }

    /// El carril de la base de uso (estado y líneas de apagado, para las pruebas).
    pub fn usage_lane(&self) -> &lanes::Lane<lanes::UsageBackend> {
        &self.usage
    }

    /// Las cachés que el escaneo de GET `/state` sostiene durante todo su salto
    /// (para las pruebas de que ninguna otra ruta las espera).
    pub fn state_caches(&self) -> &Arc<Mutex<states::gather::Blocking>> {
        &self.states.shared
    }

    /// La caché de límites de proveedor.
    pub fn limits(&self) -> &Arc<usage::limits::LimitsCache> {
        &self.limits
    }

    /// El escritor de bordes de pane.
    pub fn pane_models(&self) -> &Arc<usage::pane_models::PaneModelWriter> {
        &self.pane_models
    }

    /// Los avisos de nivel recordados (`_TIER_LAST`/`_TIER_ALERTED`).
    pub fn tier_alerts(&self) -> std::sync::MutexGuard<'_, usage::pane_models::TierAlerts> {
        self.tier_alerts.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Lo que la tarea de refresco de límites necesita (sin `&Native`, D13).
    pub fn refresh_deps(&self) -> usage::limits::RefreshDeps {
        usage::limits::RefreshDeps {
            opts: self.opts.clone(),
            usage: self.usage.clone(),
            import_lane: self.import_lane.clone(),
        }
    }

    /// Arranca una vez los productores y conserva sus señales de parada.
    /// Cada productor respeta su modo y corte; los límites pertenecen a Usage.
    pub fn start_background(self: &Arc<Self>) {
        let mut owner = self
            .background_owner
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if !self.enabled() || owner.is_some() || tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        if self.opts.usage_state_native {
            let _ = self.limits.get(&self.refresh_deps());
        }
        *owner = Some(background::Owner::start(self));
    }

    fn stop_background(&self) {
        // Solo señal: una operación larga ya lanzada puede terminar según D12.
        self.background_owner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
    }

    /// El dueño de la importación de uso (estado, para las pruebas).
    pub fn import_owner(&self) -> &Arc<usage::import::ImportOwner> {
        &self.import
    }

    /// El carril de la importación de uso (estado, para las pruebas).
    pub fn import_lane(&self) -> &lanes::Lane<lanes::UsageImportBackend> {
        &self.import_lane
    }

    /// Lo que la tarea de importación necesita, sin `&Native` (D13).
    pub fn import_deps(&self) -> usage::import::ImportDeps {
        usage::import::ImportDeps {
            opts: self.opts.clone(),
            import_lane: self.import_lane.clone(),
            engine: self.usage_engine.clone(),
        }
    }

    /// El carril del journal de operaciones (estado y líneas de apagado).
    pub fn journal_lane(&self) -> &lanes::Lane<lanes::JournalBackend> {
        &self.journal
    }

    fn disable(&self, refusal: &Refusal) {
        if self.enabled.swap(false, Ordering::AcqRel) {
            self.stop_background();
            self.refusals.fetch_add(1, Ordering::AcqRel);
            eprintln!("{}", refusal.message(&self.opts.state_db));
        }
    }

    /// Abre la base una sola vez (en un hilo de bloqueo) y arranca el worker.
    /// Devuelve si el conjunto nativo está activo.
    pub async fn ready(&self) -> bool {
        let caller = self
            .state
            .get_or_init(|| async {
                let path = self.opts.state_db.clone();
                let now_seconds = (self.opts.clock)() as f64 / 1000.0;
                let opened =
                    tokio::task::spawn_blocking(move || StateBackend::open(&path, now_seconds))
                        .await;
                match opened {
                    Ok(Ok(backend)) => match BackendWorker::start(WORKER_CAPACITY, backend) {
                        Ok(worker) => {
                            let caller = worker.caller();
                            *self.worker.lock().unwrap_or_else(|p| p.into_inner()) = Some(worker);
                            Some(caller)
                        }
                        Err(error) => {
                            self.disable(&Refusal::Unopened(error.to_string()));
                            None
                        }
                    },
                    Ok(Err(refusal)) => {
                        self.disable(&refusal);
                        None
                    }
                    Err(join) => {
                        self.disable(&Refusal::Unopened(join.to_string()));
                        None
                    }
                }
            })
            .await;
        caller.is_some() && self.enabled()
    }

    /// Un trabajo sobre la base. `Err(Fault::Decline)` si el conjunto está
    /// apagado, si la puerta de esquema rechaza ahora o si el worker ya se
    /// retiró. Un esquema más nuevo o desconocido y un worker retirado lo
    /// apagan; un fallo pasajero de la puerta declina solo esta petición. Solo el trabajo que entró en
    /// pánico responde `HandlerError::Failure`; los demás se reenvían.
    pub async fn with_state<T, F>(&self, job: F) -> Result<T, Fault>
    where
        F: FnOnce(&mut StateBackend) -> T + Send + 'static,
        T: Send + 'static,
    {
        if !self.ready().await {
            return Err(Fault::Decline);
        }
        let Some(Some(caller)) = self.state.get() else {
            return Err(Fault::Decline);
        };
        if caller.stopped() {
            self.disable(&Refusal::Retired);
            return Err(Fault::Decline);
        }
        // Marca de que el trabajo empezó: si no empezó, no hubo efectos y
        // se puede declinar con certeza.
        let started = Arc::new(AtomicBool::new(false));
        let mark = started.clone();
        let called = caller
            .call(move |backend: &mut StateBackend| {
                mark.store(true, Ordering::Release);
                backend.admit().map(|()| job(backend))
            })
            .await;
        match called {
            Ok(Ok(value)) => Ok(value),
            // Solo un esquema más nuevo o desconocido apaga el conjunto.
            Ok(Err(
                refusal @ (Refusal::Newer { .. } | Refusal::Incompatible(_) | Refusal::Retired),
            )) => {
                self.disable(&refusal);
                Err(Fault::Decline)
            }
            // Un fallo al consultar las versiones (p. ej. `SQLITE_BUSY`) no
            // dice nada del esquema: se reenvía esta petición y el conjunto
            // sigue (`admit` corre antes del trabajo, que no llegó a empezar).
            Ok(Err(Refusal::Unopened(_))) => Err(Fault::Decline),
            // El trabajo corrió y no respondió: entró en pánico y el worker
            // se retira. Esta petición es la única que recibe el 500.
            Err(error) if started.load(Ordering::Acquire) => {
                self.disable(&Refusal::Retired);
                Err(Fault::Error(error))
            }
            // Nunca empezó (worker parado o retirado en la carrera): sin
            // efectos, se reenvía al heredado.
            Err(_) => {
                if caller.stopped() {
                    self.disable(&Refusal::Retired);
                }
                Err(Fault::Decline)
            }
        }
    }

    /// Con el conjunto apagado todo se reenvía, salvo el reintento de un
    /// `POST /pane/type` que el frente ya tecleó: ese sale de su caché, porque
    /// el Python no lo conoce y volvería a teclear.
    pub fn typing_retry(&self, request: &Request) -> Option<Reply> {
        if route(&request.method, &request.target) != Some(NativeRoute::PaneType) {
            return None;
        }
        typing::retry_reply(&self.typing, request)
    }

    pub async fn dispatch(
        self: &Arc<Self>,
        route: NativeRoute,
        request: &Request,
    ) -> Result<Outcome, HandlerError> {
        // D3: un corte apagado declina sin evaluar nada (ni abrir la base).
        if cut_is_off(&self.opts.cuts_off, route.cut()) {
            return Ok(self.decline(request));
        }
        if !self.ready().await {
            if route == NativeRoute::PaneType
                && let Some(reply) = typing::retry_reply(&self.typing, request)
            {
                return Ok(Outcome::Reply(reply));
            }
            return Ok(self.decline(request));
        }
        match self.answer(route, request).await {
            Ok(reply) => Ok(Outcome::Reply(reply)),
            Err(Fault::Decline) => Ok(self.decline(request)),
            Err(Fault::Error(error)) => Err(error),
        }
    }

    /// Cuenta la declinación en el censo (D7) y la devuelve.
    pub fn decline(&self, request: &Request) -> Outcome {
        self.census.note(&request.method, &request.target);
        Outcome::Decline
    }

    async fn answer(self: &Arc<Self>, route: NativeRoute, request: &Request) -> Answer {
        match route {
            NativeRoute::Sidebar => sidebar::answer(self, request).await,
            NativeRoute::Chat => chat::answer(self, request).await,
            NativeRoute::Feed(route) => feed::answer(self, route, request).await,
            NativeRoute::Light(route) => light::answer(self, route, request).await,
            NativeRoute::Events => events::answer(self, request).await,
            NativeRoute::Notices(route) => notices::answer(self, route, request).await,
            NativeRoute::Workspace(route) => workspace::answer(self, route, request).await,
            NativeRoute::Snippets(route) => snippets::answer(self, route, request).await,
            NativeRoute::UiLog => ui_log::answer(self, request).await,
            NativeRoute::Pomodoro => pomodoro::answer(self).await,
            NativeRoute::Catalog(route) => catalogs::answer(self, route).await,
            NativeRoute::Terminal(route) => terminal::answer(self, route, request).await,
            NativeRoute::PaneType => typing::answer(self, request).await,
            NativeRoute::ModelStatus => operations::answer(self, request).await,
            NativeRoute::State => states::answer(self, &request.target).await,
            NativeRoute::QuickTerminal => quick::answer(self, request).await,
            NativeRoute::Usage(route) => usage::answer(self, route, request).await,
            NativeRoute::Retired => {
                let path = request
                    .target
                    .split_once('?')
                    .map_or(request.target.as_str(), |(path, _)| path);
                retired::answer(&request.method, path)
            }
            NativeRoute::Tabs(route) => tabs::answer(self, route, request).await,
            NativeRoute::Sessions(route) => sessions::answer(self, route, request).await,
            NativeRoute::Input(route) => input::answer(self, route, request).await,
            NativeRoute::Ops(route) => ops::answer(self, route, request).await,
            NativeRoute::Remote(route) => remote::answer(self, route, request).await,
            NativeRoute::Ssh(route) => ssh::answer(self, route, request).await,
            NativeRoute::Settings(route) => settings::answer(self, route, request).await,
            NativeRoute::Cli(route) => catalog_cli::answer(self, route, request).await,
            NativeRoute::Push(route) => push::answer(self, route, request).await,
            NativeRoute::News(route) => news::answer(self, route, request).await,
            NativeRoute::Residue(route) => residue::answer(self, route, request).await,
        }
    }

    /// GET `/state` con la caché del Python (`read_states_cached`); también la
    /// usará `/workspace/sort` en modo `by`.
    pub async fn states_cached(&self) -> Result<Arc<states::States>, Fault> {
        let clock = self.opts.clock.clone();
        self.states
            .cache
            .get(&*clock, || states::gather::compute(self))
            .await
            .map_err(Fault::from)
    }

    /// Para el worker (si arrancó) y espera a que termine su trabajo en curso.
    pub async fn shutdown(&self) {
        // Apagado ordenado: sin línea en stderr; lo que llegue tarde se reenvía.
        self.enabled.store(false, Ordering::Release);
        self.stop_background();
        let worker = self.worker.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(worker) = worker {
            let _ = worker.shutdown().await;
        }
        self.usage.shutdown().await;
        self.import_lane.shutdown().await;
        self.journal.shutdown().await;
        self.states.serial.shutdown().await;
    }
}

#[cfg(test)]
mod scaffold_tests {
    use super::*;

    #[test]
    fn delete_and_prefix_keys_match_like_python() {
        assert!(Key::Prefix("/state").matches("/state"));
        assert!(Key::Prefix("/state").matches("/stateful?x=1"));
        assert!(!Key::Prefix("/state").matches("/stat"));
        assert_eq!(
            route(&Method::DELETE, "/no-existe"),
            Some(NativeRoute::Residue(residue::ResidueRoute::DeleteUnknown))
        );
    }

    #[test]
    fn head_and_unknown_delete_reach_only_the_last_residual_table() {
        for path in ["/state", "/"] {
            assert_eq!(
                route(&Method::HEAD, path),
                Some(NativeRoute::Residue(residue::ResidueRoute::HeadFallback))
            );
        }
        assert_eq!(route(&Method::PUT, "/state"), None);
        assert!(TABLES.last().is_some_and(|table| {
            table.len() == residue::ROUTES.len()
                && table
                    .iter()
                    .all(|e| matches!(e.route, NativeRoute::Residue(_)))
        }));
        assert!(
            TABLES
                .iter()
                .flat_map(|t| t.iter())
                .all(|entry| (entry.verb != Verb::Head
                    || matches!(entry.route, NativeRoute::Residue(_)))
                    && (entry.verb != Verb::Delete
                        || matches!(entry.route, NativeRoute::Residue(_))
                        || matches!(entry.key, Key::Raw("/push/subscription"))))
        );
    }

    #[test]
    fn every_route_has_a_cut() {
        assert_eq!(NativeRoute::Retired.cut(), Cut::Base);
        assert_eq!(NativeRoute::PaneType.cut(), Cut::Base);
        // Toda entrada de las tablas de 2b–2e pertenece a la base (ningún corte
        // la apaga) y toda entrada de las tablas de la 2f, a su corte. La lista
        // explícita cubre las tablas de ambos carriles (2f-1 y 2f-3/2f-4); el
        // recuento debe igualar `TABLES.len()`.
        let base: [&[Entry]; 18] = [
            light::ROUTES,
            events::ROUTES,
            notices::ROUTES,
            workspace::ROUTES,
            snippets::ROUTES,
            ui_log::ROUTES,
            pomodoro::ROUTES,
            catalogs::ROUTES,
            terminal::ROUTES,
            typing::ROUTES,
            operations::ROUTES,
            states::ROUTES,
            quick::ROUTES,
            usage::ROUTES,
            retired::ROUTES,
            sidebar::ROUTES,
            chat::ROUTES,
            feed::ROUTES,
        ];
        let cuts: [&[Entry]; 11] = [
            tabs::ROUTES,
            sessions::ROUTES,
            input::ROUTES,
            ops::ROUTES,
            remote::ROUTES,
            ssh::ROUTES,
            settings::ROUTES,
            catalog_cli::ROUTES,
            push::ROUTES,
            news::ROUTES,
            residue::ROUTES,
        ];
        assert_eq!(base.len(), BASE_TABLES);
        assert_eq!(base.len() + cuts.len(), TABLES.len());
        assert!(
            base.iter()
                .flat_map(|table| table.iter())
                .all(|entry| entry.route.cut() == Cut::Base)
        );
        assert!(
            cuts.iter()
                .flat_map(|table| table.iter())
                .all(|entry| entry.route.cut() != Cut::Base)
        );
        // Y por posición: ninguna de las tablas que siguen a `BASE_TABLES` es
        // de la base. Un `BASE_TABLES` corto o largo falla aquí.
        for (i, table) in TABLES.iter().enumerate() {
            for entry in table.iter() {
                assert_eq!(entry.route.cut() == Cut::Base, i < BASE_TABLES);
            }
        }
    }

    #[test]
    fn base_is_never_cut() {
        let all = BTreeSet::from([
            Cut::Base,
            Cut::Tabs,
            Cut::Ops,
            Cut::Services,
            Cut::News,
            Cut::Residue,
        ]);
        assert!(!cut_is_off(&all, Cut::Base));
        assert!(cut_is_off(&all, Cut::Tabs));
        assert!(cut_is_off(&all, Cut::Residue));
        assert!(!cut_is_off(&BTreeSet::new(), Cut::Tabs));
        assert!(!cut_is_off(&BTreeSet::from([Cut::Ops]), Cut::Tabs));
    }

    #[test]
    fn background_names_parse() {
        assert_eq!(Background::parse(" front"), Some(Background::front()));
        assert_eq!(Background::parse("legacy"), Some(Background::legacy()));
        assert_eq!(Background::parse("Front"), None);
        assert_eq!(Background::parse(""), None);
    }

    #[test]
    fn cut_names_parse_like_the_env_list() {
        assert_eq!(Cut::parse("tabs"), Some(Cut::Tabs));
        assert_eq!(Cut::parse(" ops "), Some(Cut::Ops));
        assert_eq!(Cut::parse("services"), Some(Cut::Services));
        assert_eq!(Cut::parse("news"), Some(Cut::News));
        assert_eq!(Cut::parse("residue"), Some(Cut::Residue));
        assert_eq!(Cut::parse("base"), None);
        assert_eq!(Cut::parse("Tabs"), None);
        assert_eq!(Cut::parse(""), None);
    }
}
