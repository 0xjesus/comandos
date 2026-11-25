//! `comandos dash`: frente del tablero en 127.0.0.1:4777.
//!
//! Rust aplica la puerta de seguridad portada (transporte + `dashboard_access`)
//! y sirve los estáticos; lo demás se reenvía al `cc-dash` Python heredado en
//! `legacy_port`. Fase 2a: los estáticos los sirve `statics` y lo demás lo
//! reenvía `forward` con cabeceras y cuerpo intactos. Fase 2b: las rutas de
//! `native` se responden en Rust (salvo `Decline`); `--no-native` vuelve a la 2a.
pub mod forward;
pub mod native;
pub mod router;
pub mod statics;
pub mod term;
pub mod token;
pub mod web;

pub use token::{load_token, token_path};

use crate::{AssetExists, Config, Handler, HandlerError, Limits, Reply, Request};
use http::{Method, StatusCode};
use native::{Background, Cut};
use router::RouteClass;
use std::{
    collections::BTreeSet,
    fmt, io,
    net::{Ipv4Addr, SocketAddr},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{net::TcpListener, sync::watch};

pub const DEFAULT_PORT: u16 = 4777;
pub const DEFAULT_LEGACY_PORT: u16 = 4781;
pub const LEGACY_PORT_ENV: &str = "COMANDOS_DASH_LEGACY_PORT";
pub const DASH_DIR_ENV: &str = "COMANDOS_DASH_DIR";
pub const NATIVE_ENV: &str = "COMANDOS_DASH_NATIVE";
pub const TRACE_ENV: &str = "COMANDOS_DASH_TRACE_FORWARD";
/// Raíz del checkout del Python heredado (`REPO_ROOT`, `bin/cc-dash:1370`).
pub const REPO_ENV: &str = comandos_core::repo::REPO_ENV;
/// `COMANDOS_DASH_USAGE_EFFECTS=0` (o `--no-usage-effects`): la sombra no
/// refresca límites (ni red ni escrituras) ni aplica efectos de uso (D1 b, R4).
pub const USAGE_EFFECTS_ENV: &str = "COMANDOS_DASH_USAGE_EFFECTS";
/// `legacy` (por omisión en la 2f) o `front` (2g): dueño de los hilos de fondo (D6).
pub const BACKGROUND_ENV: &str = "COMANDOS_DASH_BACKGROUND";
/// Cortes desactivados, separados por comas (`tabs,ops`): D3.
pub const CUTS_OFF_ENV: &str = "COMANDOS_DASH_CUTS_OFF";

#[derive(Clone)]
pub struct DashConfig {
    /// Serve the compiled Rust dashboard by default; explicit web queries still override.
    pub native_web: bool,
    pub term: term::TermMode,
    pub webterm_compat: Vec<u16>,
    pub shadow_readonly: bool,
    pub no_usage_effects: bool,
    pub term_replay_dir: Option<PathBuf>,
    pub port: u16,
    /// Puerto del `cc-dash` Python al que se reenvía lo no nativo.
    pub legacy_port: u16,
    pub home: PathBuf,
    /// Raíz de estáticos; sus entradas suelen ser symlinks al repositorio.
    pub dash_dir: PathBuf,
    /// Artefactos web versionados de `xtask web-build`.
    pub web_dir: PathBuf,
    pub token: Vec<u8>,
    /// `~/.claude/hooks/dash-token`: la puerta lo relee en cada petición (solo
    /// lo fija `from_env`; con `None` vale `token`, como en las pruebas).
    pub token_file: Option<PathBuf>,
    /// Falso con `--no-native` o `COMANDOS_DASH_NATIVE=0`: todo se reenvía (2a).
    pub native: bool,
    /// `app-state.sqlite3`, resuelto como `lib/app_state.py`.
    pub state_db: PathBuf,
    /// `COMANDOS_DASH_TRACE_FORWARD=1`: una línea en stderr por reenvío.
    pub trace_forward: bool,
    /// Checkout del heredado (`config/model-tiers.json`); `None` si no se sabe.
    pub repo_root: Option<PathBuf>,
    /// Falso con `--no-usage-effects` o `COMANDOS_DASH_USAGE_EFFECTS=0` (sombra).
    pub usage_effects: bool,
    /// `--background=legacy|front` o `COMANDOS_DASH_BACKGROUND`. `None`: sin
    /// elegir; `build` deja el de las opciones (`legacy` salvo inyección).
    pub background: Option<Background>,
    /// `--cuts-off=a,b` o `COMANDOS_DASH_CUTS_OFF`: se unen a los inyectados.
    pub cuts_off: BTreeSet<Cut>,
    /// `$XDG_RUNTIME_DIR/comandos-dash-declines-<puerto>.json`; solo lo fija
    /// `from_env` (sin `XDG_RUNTIME_DIR`, `None`: el censo no se escribe).
    pub census_path: Option<PathBuf>,
}

impl fmt::Debug for DashConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // El token nunca aparece en trazas.
        f.debug_struct("DashConfig")
            .field("port", &self.port)
            .field("legacy_port", &self.legacy_port)
            .field("home", &self.home)
            .field("dash_dir", &self.dash_dir)
            .field("web_dir", &self.web_dir)
            .field("token", &"<oculto>")
            .field("token_file", &self.token_file)
            .field("native", &self.native)
            .field("native_web", &self.native_web)
            .field("state_db", &self.state_db)
            .field("trace_forward", &self.trace_forward)
            .field("repo_root", &self.repo_root)
            .field("usage_effects", &self.usage_effects)
            .field("background", &self.background)
            .field("cuts_off", &self.cuts_off)
            .field("census_path", &self.census_path)
            .finish()
    }
}

/// `[puerto] [--no-open] [--legacy-port N] [--no-native] [--no-usage-effects]
/// [--background=legacy|front] [--cuts-off=a,b] [--web-native]`.
/// Como el Python, el primer argumento numérico es el puerto y los demás
/// argumentos se ignoran; `--no-open` se acepta y no hace nada (nunca se abre
/// navegador). `--no-native` reenvía todo lo no estático, como en la Fase 2a;
/// `--no-usage-effects` (la sombra) apaga los efectos de uso y la red de límites.
/// Devuelve `dash_dir`, `state_db` por defecto y token vacío: `from_env` los resuelve.
pub fn parse_args(
    args: &[String],
    home: &Path,
    legacy_env: Option<&str>,
) -> Result<DashConfig, String> {
    parse_args_env(args, home, legacy_env, None, None)
}

/// `parse_args` con `COMANDOS_DASH_BACKGROUND` y `COMANDOS_DASH_CUTS_OFF`: sin
/// bandera manda el entorno (vacío = sin valor), con la misma validación.
pub fn parse_args_env(
    args: &[String],
    home: &Path,
    legacy_env: Option<&str>,
    background_env: Option<&str>,
    cuts_env: Option<&str>,
) -> Result<DashConfig, String> {
    let mut port = None;
    let mut background_flag = None;
    let mut cuts_flag = None;
    let mut legacy_flag = None;
    let mut native = true;
    let mut native_web = false;
    let mut usage_effects = true;
    let mut term = term::TermMode::Off;
    let mut webterm_compat = Vec::new();
    let mut shadow_readonly = false;
    let mut no_usage_effects = false;
    let mut term_replay_dir = None;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if word == "--web-native" {
            native_web = true;
        } else if word == "--term" || word.starts_with("--term=") {
            let value = word
                .strip_prefix("--term=")
                .map(String::from)
                .or_else(|| words.next().cloned())
                .ok_or_else(|| "--term requiere modo".to_string())?;
            term = term::TermMode::parse(&value)?;
        } else if word == "--webterm-compat" || word.starts_with("--webterm-compat=") {
            let value = word
                .strip_prefix("--webterm-compat=")
                .map(String::from)
                .or_else(|| words.next().cloned())
                .ok_or_else(|| "--webterm-compat requiere puertos".to_string())?;
            webterm_compat = parse_compat(&value)?;
        } else if word == "--shadow-readonly" {
            shadow_readonly = true;
        } else if word == "--no-usage-effects" {
            no_usage_effects = true;
            usage_effects = false;
        } else if word == "--term-replay-dir" || word.starts_with("--term-replay-dir=") {
            let value = word
                .strip_prefix("--term-replay-dir=")
                .map(String::from)
                .or_else(|| words.next().cloned())
                .ok_or_else(|| "--term-replay-dir requiere directorio".to_string())?;
            term_replay_dir = Some(PathBuf::from(value));
        } else if word == "--no-native" {
            native = false;
        } else if word == "--legacy-port" {
            let value = words
                .next()
                .ok_or_else(|| "--legacy-port requiere un puerto".to_string())?;
            legacy_flag = Some(parse_port("--legacy-port", value)?);
        } else if let Some(value) = word.strip_prefix("--legacy-port=") {
            legacy_flag = Some(parse_port("--legacy-port", value)?);
        } else if word == "--background" {
            let value = words
                .next()
                .ok_or_else(|| "--background requiere legacy o front".to_string())?;
            background_flag = Some(parse_background("--background", value)?);
        } else if let Some(value) = word.strip_prefix("--background=") {
            background_flag = Some(parse_background("--background", value)?);
        } else if word == "--cuts-off" {
            let value = words
                .next()
                .ok_or_else(|| "--cuts-off requiere una lista de cortes".to_string())?;
            cuts_flag = Some(parse_cuts("--cuts-off", value)?);
        } else if let Some(value) = word.strip_prefix("--cuts-off=") {
            cuts_flag = Some(parse_cuts("--cuts-off", value)?);
        } else if port.is_none() && !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
            port = Some(parse_port("puerto", word)?);
        }
    }
    let legacy_port = match (legacy_flag, legacy_env.filter(|v| !v.is_empty())) {
        (Some(port), _) => port,
        (None, Some(value)) => parse_port(LEGACY_PORT_ENV, value)?,
        (None, None) => DEFAULT_LEGACY_PORT,
    };
    let background = match (background_flag, background_env.filter(|v| !v.is_empty())) {
        (Some(background), _) => Some(background),
        (None, Some(value)) => Some(parse_background(BACKGROUND_ENV, value)?),
        (None, None) => None,
    };
    let cuts_off = match (cuts_flag, cuts_env) {
        (Some(cuts), _) => cuts,
        (None, Some(value)) => parse_cuts(CUTS_OFF_ENV, value)?,
        (None, None) => BTreeSet::new(),
    };
    let port = port.unwrap_or(DEFAULT_PORT);
    if port == legacy_port {
        return Err(format!(
            "el puerto heredado ({legacy_port}) no puede ser el del tablero"
        ));
    }
    Ok(DashConfig {
        native_web,
        term,
        webterm_compat,
        shadow_readonly,
        no_usage_effects,
        term_replay_dir,
        port,
        legacy_port,
        home: home.to_path_buf(),
        dash_dir: default_dash_dir(home),
        web_dir: PathBuf::from("target/web"),
        token: Vec::new(),
        token_file: None,
        native,
        state_db: home.join(".local/state/comandos/app-state.sqlite3"),
        trace_forward: false,
        repo_root: None,
        usage_effects,
        background,
        cuts_off,
        census_path: None,
    })
}

fn parse_background(name: &str, value: &str) -> Result<Background, String> {
    Background::parse(value)
        .ok_or_else(|| format!("{name}: dueño de fondo no válido (legacy|front): {value}"))
}

/// Lista separada por comas; los huecos se ignoran y un nombre desconocido
/// (también `base`, que no se apaga por corte) es un error de uso.
fn parse_cuts(name: &str, value: &str) -> Result<BTreeSet<Cut>, String> {
    value
        .split(',')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            Cut::parse(part).ok_or_else(|| {
                format!(
                    "{name}: corte desconocido (tabs|ops|services|news|residue): {}",
                    part.trim()
                )
            })
        })
        .collect()
}

fn parse_compat(value: &str) -> Result<Vec<u16>, String> {
    let mut ports = Vec::new();
    for value in value.split(',') {
        let port = parse_port("--webterm-compat", value)?;
        if port == 0 || ports.contains(&port) {
            return Err("puerto compat vacío, cero o duplicado".into());
        }
        ports.push(port);
    }
    Ok(ports)
}
fn validate_term(cfg: &DashConfig) -> Result<(), String> {
    if !cfg.webterm_compat.is_empty() && cfg.term == term::TermMode::Off {
        return Err("compat requiere --term ttyd/native".into());
    }
    if cfg
        .webterm_compat
        .iter()
        .any(|p| *p == cfg.port || *p == cfg.legacy_port)
    {
        return Err("puerto compat coincide con tablero/heredado".into());
    }
    Ok(())
}
fn parse_port(name: &str, value: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .map_err(|_| format!("{name}: puerto no válido: {value}"))
}

fn default_dash_dir(home: &Path) -> PathBuf {
    home.join(".claude/hooks/dash")
}

/// `_dashboard_directory()` del Python: sin override, `~/.claude/hooks/dash`;
/// con override, `abspath(expanduser(..))` que debe ser un directorio legible.
pub fn dash_dir(home: &Path, override_dir: Option<&str>) -> Result<PathBuf, String> {
    let Some(raw) = override_dir.filter(|v| !v.is_empty()) else {
        return Ok(default_dash_dir(home));
    };
    let expanded = if raw == "~" {
        home.to_path_buf()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(raw)
    };
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map_err(|e| format!("{DASH_DIR_ENV}: sin directorio actual: {e}"))?
            .join(expanded)
    };
    let candidate = normalize(&absolute);
    // Directorio (tras symlinks) que se puede listar: equivale a R_OK | X_OK.
    if candidate.is_dir() && std::fs::read_dir(&candidate).is_ok() {
        Ok(candidate)
    } else {
        Err(format!(
            "{DASH_DIR_ENV} no es un directorio legible: {}",
            candidate.display()
        ))
    }
}

/// `COMANDOS_DASH_REPO` si está; si no, el destino canónico de
/// `<dash_dir>/index.html` dos niveles arriba. La regla es la de
/// `comandos-core` (compartida con `comandos-notifyd`); aquí se resuelve el
/// enlace, que es I/O.
pub fn repo_root(dash_dir: &Path, override_dir: Option<&str>) -> Option<PathBuf> {
    let index = std::fs::canonicalize(dash_dir.join("index.html")).ok();
    comandos_core::repo::repo_root_from(index.as_deref(), override_dir)
}

/// `os.path.normpath` léxico: quita `.` y resuelve `..` sin tocar el disco.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Configuración completa desde el proceso: `HOME`, `COMANDOS_DASH_LEGACY_PORT`,
/// `COMANDOS_DASH_DIR`, `COMANDOS_DASH_NATIVE`, `COMANDOS_DASH_TRACE_FORWARD`,
/// la ruta de `app-state.sqlite3` y el token (creado si falta).
pub fn from_env(args: &[String]) -> Result<DashConfig, StartError> {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| StartError::Usage("HOME no está definido".into()))?;
    let legacy = std::env::var(LEGACY_PORT_ENV).ok();
    let background = std::env::var(BACKGROUND_ENV).ok();
    let cuts = std::env::var(CUTS_OFF_ENV).ok();
    let mut cfg = parse_args_env(
        args,
        &home,
        legacy.as_deref(),
        background.as_deref(),
        cuts.as_deref(),
    )
    .map_err(StartError::Usage)?;
    // D7: el censo vive en el directorio de ejecución del usuario; sin él no
    // se escribe. Solo aquí (arranque real): las pruebas nunca lo alcanzan.
    cfg.census_path = comandos_runtime::platform::runtime_directory_from_env(true)
        .map_err(|e| StartError::Usage(format!("runtime directory: {e}")))?
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(native::census::file_name(cfg.port)));
    if !args
        .iter()
        .any(|a| a == "--term" || a.starts_with("--term="))
        && let Ok(value) = std::env::var("COMANDOS_DASH_TERM")
    {
        cfg.term = term::TermMode::parse(&value).map_err(StartError::Usage)?;
    }
    if !args
        .iter()
        .any(|a| a == "--webterm-compat" || a.starts_with("--webterm-compat="))
        && let Ok(value) = std::env::var("COMANDOS_DASH_WEBTERM_COMPAT")
    {
        cfg.webterm_compat = parse_compat(&value).map_err(StartError::Usage)?;
    }
    validate_term(&cfg).map_err(StartError::Usage)?;
    let override_dir = std::env::var(DASH_DIR_ENV).ok();
    cfg.dash_dir = dash_dir(&home, override_dir.as_deref()).map_err(StartError::Config)?;
    cfg.repo_root = repo_root(&cfg.dash_dir, std::env::var(REPO_ENV).ok().as_deref());
    let cwd = std::env::current_dir()
        .map_err(|e| StartError::Config(format!("directorio actual: {e}")))?;
    let workspace = cfg.repo_root.clone().unwrap_or_else(|| cwd.clone());
    let target = std::env::var_os("CARGO_TARGET_DIR");
    let target = target.as_deref().map(Path::new);
    let installed_web = if target.is_none_or(|path| path.as_os_str().is_empty()) {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("web")))
            .filter(|dir| dir.join(comandos_core::web_assets::MANIFEST_FILE).is_file())
    } else {
        None
    };
    cfg.web_dir = installed_web
        .unwrap_or_else(|| comandos_core::web_assets::out_dir_from(target, &cwd, &workspace));
    if std::env::var(NATIVE_ENV).is_ok_and(|v| v == "0") {
        cfg.native = false;
    }
    cfg.trace_forward = std::env::var(TRACE_ENV).is_ok_and(|v| v == "1");
    if std::env::var(USAGE_EFFECTS_ENV).is_ok_and(|v| v == "0") {
        cfg.usage_effects = false;
    }
    let state_override = std::env::var_os("COMANDOS_STATE_DB").map(PathBuf::from);
    let xdg = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
    cfg.state_db =
        comandos_runtime::state_path(None, state_override.as_deref(), xdg.as_deref(), Some(&home))
            .map_err(|e| StartError::Config(format!("app-state: {e}")))?;
    cfg.token = load_token(&home).map_err(|e| {
        StartError::Config(format!("dash-token ({}): {e}", token_path(&home).display()))
    })?;
    cfg.token_file = Some(token_path(&home));
    Ok(cfg)
}

#[derive(Debug)]
pub enum StartError {
    /// Argumentos o entorno mal formados: salida 2.
    Usage(String),
    /// Configuración inutilizable (como el `SystemExit` del Python): salida 1.
    Config(String),
}

/// Límites del frente. Las clases de ruta de la Fase 2b los afinarán por ruta.
pub fn limits() -> Limits {
    Limits {
        connections: 256,
        websockets: 64,
        header_bytes: 64 * 1024,
        buffered_wire_bytes: 20_000_000 + 64 * 1024,
        header_timeout: Duration::from_secs(30),
        body_timeout: Duration::from_secs(30),
        handler_timeout: Duration::from_secs(120),
        write_timeout: Duration::from_secs(30),
        shutdown_grace: Duration::from_secs(2),
    }
}

/// `isfile(join(DASH, path.lstrip("/")))`: los enlaces de `dash_dir` apuntan
/// al repositorio, así que basta con que el destino resuelto sea un archivo.
pub fn asset_exists(dash_dir: &Path) -> AssetExists {
    let root = dash_dir.to_path_buf();
    Arc::new(move |path: &str| {
        std::fs::metadata(root.join(path.trim_start_matches('/'))).is_ok_and(|m| m.is_file())
    })
}

/// Estado compartido por las ramas del enrutador.
pub struct DashState {
    pub web: web::WebState,
    pub config: DashConfig,
    pub asset_exists: AssetExists,
    pub native: Option<Arc<native::Native>>,
    pub term_target: term::attach::TmuxTarget,
    pub term_control: Arc<term::lifecycle::Control>,
}

pub fn handler(state: Arc<DashState>) -> Handler {
    Arc::new(move |request| {
        let state = state.clone();
        Box::pin(async move { handle(&state, request).await })
    })
}

async fn handle(state: &DashState, request: Request) -> Result<Reply, HandlerError> {
    if request.method == Method::GET && router::path_of(&request.target) == "/web/status" {
        return web::routes::handle(state, web::WebRoute::Status, &request).await;
    }
    // El handshake del arranque solo cambia memoria del frente. Debe liberar
    // también la compuerta de la sombra, sin habilitar escrituras del tablero.
    if request.method == Method::POST && router::path_of(&request.target) == "/web/ready" {
        return web::routes::handle(state, web::WebRoute::Ready, &request).await;
    }
    // Pure rendering only: no Native, store or mutation, including readonly shadow.
    if request.method == Method::POST && router::path_of(&request.target) == "/web/markdown" {
        return web::routes::handle(state, web::WebRoute::Markdown, &request).await;
    }
    if state.config.shadow_readonly && request.method != Method::GET {
        return Reply::json(
            StatusCode::OK,
            &serde_json::json!({"ok":true,"shadow":true}),
        );
    }
    if state.config.term != term::TermMode::Off
        && (router::path_of(&request.target) == "/term"
            || router::path_of(&request.target).starts_with("/term/"))
        && !state.term_control.refresh().await
    {
        return not_found();
    }
    if request.method == Method::GET
        && router::path_of(&request.target) == "/term/"
        && crate::dash::web::routes::query_value(&request.target, "web").as_deref()
            == Some("native")
    {
        return web::native_term_page::serve(state);
    }
    if state.config.term != term::TermMode::Off
        && let Some(route) = term::routes::route(&request.method, &request.target)
    {
        return term::routes::http_route(state, route, &request.target).await;
    }
    let live = state.native.as_ref().filter(|n| n.enabled());
    if live.is_none()
        && let Some(reply) = state.native.as_ref().and_then(|n| n.typing_retry(&request))
    {
        return Ok(reply);
    }
    // `/session/message` no tiene rama en el Python: con el conjunto apagado o
    // ausente se responde aquí (o 503), nunca se reenvía.
    if live.is_none() && native::message::claims(&request.method, &request.target) {
        return native::message::without_live(state.native.as_ref(), &request).await;
    }
    // Conjunto apagado (esquema rechazado o retirado): lo nativo se reenvía y
    // también cuenta como declinación en el censo (D7).
    if live.is_none()
        && let Some(native) = state.native.as_ref()
        && native::route(&request.method, &request.target).is_some()
    {
        native.census().note(&request.method, &request.target);
    }
    let web_exists = state.web.route_exists();
    let class = router::classify_with_web(
        &request.method,
        &request.target,
        &*state.asset_exists,
        live.is_some(),
        &web_exists,
    );
    match class {
        RouteClass::Web(route) => web::routes::handle(state, route, &request).await,
        RouteClass::Native(route) => {
            let Some(native) = live else {
                return forward_to_legacy(state, request).await;
            };
            match native.dispatch(route, &request).await? {
                native::Outcome::Reply(reply) => Ok(reply),
                // A read declines when the store is busy for a moment: try it
                // once more before the legacy server (usually not running, so
                // the gear's Temas/Terminal came up empty on a 502).
                native::Outcome::Decline if request.method == Method::GET => {
                    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                    match native.dispatch(route, &request).await? {
                        native::Outcome::Reply(reply) => Ok(reply),
                        native::Outcome::Decline => forward_to_legacy(state, request).await,
                    }
                }
                native::Outcome::Decline => forward_to_legacy(state, request).await,
            }
        }
        RouteClass::Static => statics::serve(&state.config.dash_dir, &request).await,
        // HEAD solo existe para estáticos; a una ruta API es 404 (el Python
        // devolvía 404 HTML vía SimpleHTTPRequestHandler.do_HEAD).
        RouteClass::Forward if request.method == Method::HEAD => not_found(),
        RouteClass::Forward => forward_to_legacy(state, request).await,
    }
}

async fn forward_to_legacy(state: &DashState, request: Request) -> Result<Reply, HandlerError> {
    if state.config.trace_forward {
        eprintln!("{}", trace_line(&request.method, &request.target));
    }
    let legacy = SocketAddr::from((Ipv4Addr::LOCALHOST, state.config.legacy_port));
    forward::relay(legacy, request).await
}

/// Sin consulta: los `deviceId` y tokens nunca llegan a los registros.
pub fn trace_line(method: &Method, target: &str) -> String {
    format!(
        "comandos dash: reenvío {method} {}",
        router::path_of(target)
    )
}

pub fn not_found() -> Result<Reply, HandlerError> {
    Reply::json(
        StatusCode::NOT_FOUND,
        &serde_json::json!({"error": "No encontrado"}),
    )
}

/// Transporte + conjunto nativo. `opts` sustituye a las opciones derivadas
/// de `cfg` (las pruebas inyectan reloj, tmux y `fc-list`).
pub fn build(
    cfg: DashConfig,
    opts: Option<native::NativeOptions>,
) -> (Config, Option<Arc<native::Native>>) {
    assemble(cfg, opts).0
}

fn assemble(
    cfg: DashConfig,
    opts: Option<native::NativeOptions>,
) -> ((Config, Option<Arc<native::Native>>), Arc<DashState>) {
    let term_target = opts
        .as_ref()
        .map(|o| o.term_target.clone())
        .unwrap_or(term::attach::TmuxTarget::User);
    let asset_exists = asset_exists(&cfg.dash_dir);
    let token = cfg.token.clone();
    let native = (cfg.native && !cfg.shadow_readonly).then(|| {
        let mut o = opts.unwrap_or_else(|| {
            let mut o = native::NativeOptions::for_home(&cfg.home, cfg.state_db.clone());
            o.repo_root = cfg.repo_root.clone();
            o
        });
        o.dash_dir = Some(cfg.dash_dir.clone());
        o.standalone |= cfg.native_web;
        // El contexto de sugerencias habla con el MISMO heredado al que se reenvía.
        o.legacy = SocketAddr::from((Ipv4Addr::LOCALHOST, cfg.legacy_port));
        o.legacy_token = cfg.token.clone();
        // La sombra nunca tiene efectos de uso, tampoco con opciones inyectadas.
        o.usage_effects &= cfg.usage_effects;
        // B4: los cortes del arranque se suman a los inyectados; el dueño de
        // fondo solo cambia si vino por bandera o entorno; un censo inyectado
        // (pruebas) se respeta.
        o.cuts_off.extend(cfg.cuts_off.iter().copied());
        if let Some(background) = cfg.background {
            o.background = background;
        }
        if o.census_path.is_none() {
            o.census_path = cfg.census_path.clone();
        }
        Arc::new(native::Native::new(o))
    });
    let term_control = Arc::new(if cfg.shadow_readonly {
        term::lifecycle::Control::new(&cfg.home)
    } else {
        term::lifecycle::Control::new_live(&cfg.home)
    });
    let state = Arc::new(DashState {
        web: web::WebState::new(&cfg),
        config: cfg,
        asset_exists: asset_exists.clone(),
        native: native.clone(),
        term_target,
        term_control,
    });
    let config = Config {
        websocket: (state.config.term != term::TermMode::Off)
            .then(|| term::routes::ws_route(state.clone())),
        token,
        token_file: state.config.token_file.clone(),
        asset_exists,
        handler: handler(state.clone()),
        limits: limits(),
    };
    ((config, native), state)
}

/// Configuración del transporte para un `DashConfig` ya resuelto (API de la 2a).
pub fn transport_config(cfg: DashConfig) -> Config {
    build(cfg, None).0
}

/// Atiende con opciones nativas explícitas hasta que `shutdown` pase a `true`;
/// al terminar para el worker de la base.
pub async fn serve_with(
    listener: TcpListener,
    cfg: DashConfig,
    opts: Option<native::NativeOptions>,
    shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    validate_term(&cfg).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    term::lifecycle::persist(&cfg)?;
    let ((config, native), state) = assemble(cfg, opts);
    let mut census_task = None;
    if let Some(native) = &native {
        // Abre la base antes de atender: la primera petición no paga la migración.
        native.ready().await;
        // Productores retenidos hasta shutdown, según modo y corte; incluye
        // el refresco inicial de límites cuando Usage es nativo.
        native.start_background();
        // D7: el censo se escribe cada minuto (si cambió) y al apagar.
        if let Some(file) = native.options().census_path.clone() {
            census_task = Some(tokio::spawn(native::census::flush_every(
                native.census().clone(),
                file,
                native::census::FLUSH_PERIOD,
            )));
        }
    }
    let (stop, stop_rx) = watch::channel(*shutdown.borrow());
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(crate::serve(listener, config, stop_rx.clone()));
    tasks.spawn(term::lifecycle::run(state.clone(), stop_rx.clone()));
    let mut shutdown = shutdown;
    let mut served = Ok(());
    tokio::select! {
        _=shutdown.changed()=>{},
        result=tasks.join_next()=>{if let Some(result)=result {served=result.map_err(io::Error::other).and_then(|result|result);}},
    }
    let _ = stop.send(true);
    while let Some(result) = tasks.join_next().await {
        if served.is_ok() {
            served = result.map_err(io::Error::other).and_then(|result| result);
        }
    }
    if let Some(task) = census_task {
        task.abort();
    }
    if let Some(native) = native {
        native.shutdown().await;
        if let Some(file) = native.options().census_path.clone() {
            native::census::flush_in_background(native.census().clone(), file).await;
        }
    }
    served
}

/// Atiende en un listener ya abierto hasta que `shutdown` pase a `true`.
pub async fn serve_listener(
    listener: TcpListener,
    cfg: DashConfig,
    shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    serve_with(listener, cfg, None, shutdown).await
}

/// Solo IPv4 de bucle local, como el `ThreadingTCPServer(("127.0.0.1", port))`.
pub async fn bind(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await
}

pub async fn run(cfg: DashConfig, shutdown: watch::Receiver<bool>) -> io::Result<()> {
    let listener = bind(cfg.port).await?;
    serve_listener(listener, cfg, shutdown).await
}

/// Tope de hilos de bloqueo del frente. Con el de tokio (512) una ola de
/// peticiones simultáneas (los iframes de `term.html` que despiertan a la vez
/// con `visibilitychange`, cada `/terminal-panes` en su hilo) creaba 17 hilos
/// de golpe, y ninguno volvía a quedar 10 s ocioso: el pool despierta a sus
/// hilos por turno y GET `/state` daba cientos de saltos por cómputo, así que
/// la recolección se repartía entre todos y cada uno engordaba su propia arena
/// de glibc (en vivo tras la 2d: 4 → 24 hilos y 45 → 198 MiB de Pss). Ahora
/// `/state` tiene su hilo (`states::serial`) y una ola de `/terminal-panes`
/// ocupa como mucho un hilo (`terminal::PANES_GATE`), así que el pool queda
/// para el trabajo esporádico: estáticos, historial, tecleo, escrituras.
///
/// Ningún trabajo del pool espera a otro trabajo del pool (solo a procesos de
/// tmux que mueve el hilo del runtime; los candados que se toman dentro son
/// `try_lock` o, el de `/terminal-panes`, ya serializado por su puerta
/// asíncrona), así que el tope no puede interbloquear: lo que no cabe espera
/// en la cola. Ocho deja sitio a varios tecleos largos de `/pane/type`
/// sin frenar los estáticos.
pub const MAX_BLOCKING_THREADS: usize = 8;

/// Ocio tras el que un hilo de bloqueo se retira (tokio: 10 s).
pub const BLOCKING_KEEP_ALIVE: Duration = Duration::from_secs(2);

/// El runtime del frente: monohilo para la red y un pool de bloqueo acotado.
pub fn runtime() -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(MAX_BLOCKING_THREADS)
        .thread_keep_alive(BLOCKING_KEEP_ALIVE)
        .build()
}

/// Punto de entrada del binario: runtime monohilo, señales y código de salida.
pub fn main(args: &[String]) -> i32 {
    // Help must not load HOME, create the access token/runtime directory, or bind.
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "uso: comandos dash [PUERTO] [OPCIONES]\n\n\
             Puerto predeterminado: {DEFAULT_PORT}\n\
             Opciones:\n\
               --term off|ttyd|native\n\
               --term-replay-dir DIR\n\
               --webterm-compat PUERTOS\n\
               --legacy-port PUERTO\n\
               --no-native\n\
               --no-usage-effects\n\
               --shadow-readonly\n\
               --background legacy|front\n\
               --cuts-off LISTA\n\
               --no-open\n\
               --help, -h"
        );
        return 0;
    }
    let cfg = match from_env(args) {
        Ok(cfg) => cfg,
        Err(StartError::Usage(message)) => {
            eprintln!("comandos dash: {message}");
            return 2;
        }
        Err(StartError::Config(message)) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("comandos dash: runtime: {error}");
            return 1;
        }
    };
    runtime.block_on(serve_until_signal(cfg))
}

async fn serve_until_signal(cfg: DashConfig) -> i32 {
    use tokio::signal::unix::{SignalKind, signal};
    let port = cfg.port;
    let listener = match bind(port).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("comandos dash: no se pudo escuchar en 127.0.0.1:{port}: {error}");
            return 1;
        }
    };
    // Las señales se instalan antes del aviso: quien lo lee ya puede parar.
    let (Ok(mut term), Ok(mut int)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        eprintln!("comandos dash: no se pudieron instalar las señales");
        return 1;
    };
    let (stop, shutdown) = watch::channel(false);
    tokio::spawn(async move {
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
        let _ = stop.send(true);
    });
    {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(
            out,
            "Centro Claude corriendo en http://127.0.0.1:{port}  (Ctrl+C para salir)"
        );
        let _ = out.flush();
    }
    match serve_listener(listener, cfg, shutdown).await {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("comandos dash: {error}");
            1
        }
    }
}
