//! Preámbulo de destino de los POST de sesión (`do_POST`, `bin/cc-dash`
//! 9483–9565) y la identidad de un pane, compartidos por 2f-1 (`/send`,
//! `/paste`, `/key`, `/focus`, `/kill`, `/ensure`, `/new`, `/shell`, `/up`,
//! `/export`) y 2f-2 (`/session/configure`, `/account/switch`,
//! `/harness/switch`, `/model/switch`, `/model/switch-cancel`).
//!
//! Todo aquí solo lee: tmux (`list-sessions`, `display-message`,
//! `list-panes -a`, con el plazo de 5 s de `tmux()`), `H/state/*.json`,
//! `~/codebase`, `/proc` y los archivos de pestañas. Declinar aquí sigue siendo
//! «antes de efectos». Las lecturas de disco y `/proc` van en `spawn_blocking`
//! (a lo sumo dos saltos por resolución, el segundo solo si hay agentes).
use super::{
    Answer, Fault, Native, NativeOptions, light, py, reply, states::gather::session_labels,
};
use crate::HandlerError;
use comandos_runtime::session_configuration as sc;
use comandos_runtime::{
    agent_procs::{
        self, AgentInfo, AgentMaps, AgentProc, PANE_FORMAT, agent_pane_maps,
        agent_procs_for_agents, parse_pane_inventory, process_owners,
    },
    model_catalog::catalog_paths,
    providers::{self, RegistryCache},
};
use http::StatusCode;
use serde_json::{Map, Value, json};
use std::{
    collections::{HashMap, HashSet},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

/// Rutas de configuración: el Python NO sustituye la sesión por la resuelta.
pub const CONFIG_ROUTES: [&str; 3] = ["/session/configure", "/account/switch", "/model/switch"];

/// `sess`, `target` (`=<sess>`), `pane` (`=<sess>:` o el pane exacto vivo) y
/// lo que devolvió `resolve_project_session`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTarget {
    pub sess: String,
    pub target: String,
    pub pane: String,
    pub resolved: Option<(String, String)>,
}

/// `ValueError` de `_pane_identity` (texto literal) o lo que no se sabe hacer.
pub enum TargetError {
    Value(String),
    Fault(Fault),
}

impl From<Fault> for TargetError {
    fn from(fault: Fault) -> Self {
        TargetError::Fault(fault)
    }
}

fn failure() -> Fault {
    Fault::Error(HandlerError::Failure)
}

/// Un trabajo de disco o `/proc`; su pánico es una excepción sin capturar.
async fn blocking<T, F>(job: F) -> Result<T, Fault>
where
    F: FnOnce() -> Result<T, Fault> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|_| failure())?
}

/// `glob("*.json")` sobre `dir`: nombres sin punto inicial, en el orden de
/// `read_dir`. Un directorio que falta o no se lee: lista vacía (`glob` se
/// traga el `OSError`), salvo que no sea «no existe»: entonces no se sabe.
fn json_files(dir: &Path) -> Result<Vec<PathBuf>, Fault> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(Fault::Decline),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| Fault::Decline)?;
        let name = entry.file_name();
        let bytes = name.as_bytes();
        if bytes.starts_with(b".") || !bytes.ends_with(b".json") {
            continue;
        }
        out.push(entry.path());
    }
    Ok(out)
}

/// El bucle común de `resolve_project_session` (6183) y `state_agent` (5428):
/// el primer `H/state/*.json` legible cuyo `session_name(project)` es `sess`.
/// Un registro que no es objeto, o con un `project` que no es texto, hace que
/// el Python lance (`AttributeError`/`TypeError`, 500): se declina.
pub fn first_state_record(state: &Path, sess: &str) -> Result<Option<Map<String, Value>>, Fault> {
    for path in json_files(state)? {
        // `open()` de un directorio: `IsADirectoryError` → `continue`.
        if path.is_dir() {
            continue;
        }
        let Some(doc) = light::load(&path)? else {
            continue;
        };
        let Value::Object(doc) = doc else {
            return Err(Fault::Decline);
        };
        let project = match doc.get("project") {
            None => "",
            Some(Value::String(p)) => p.as_str(),
            Some(_) => return Err(Fault::Decline),
        };
        if py::session_name(project) == sess {
            return Ok(Some(doc));
        }
    }
    Ok(None)
}

pub fn first_state_record_domain(
    home: &Path,
    state: &Path,
    sess: &str,
) -> Result<Option<Map<String, Value>>, Fault> {
    comandos_store::domains::caller::read(home, "session-status", |mode, db| {
        if !matches!(mode, comandos_store::unified::Mode::Unified | comandos_store::unified::Mode::Sealed) { return Ok(first_state_record(state, sess)); }
        let Some(db) = db else { return Ok(Err(Fault::Decline)); };
        let rows = db.prepare("SELECT body FROM session_status WHERE file_key NOT LIKE '.%' AND substr(file_key,-5)='.json' ORDER BY file_key")?.query_map([], |r| r.get::<_,Vec<u8>>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for body in rows {
            let doc = match light::value_from(super::files::bytes_strict(&body)) { Ok(Some(doc)) => doc, Ok(None) => continue, Err(e) => return Ok(Err(e)) };
            let Value::Object(doc) = doc else { return Ok(Err(Fault::Decline)); };
            let project = match doc.get("project") {None => "", Some(Value::String(p)) => p.as_str(), _ => return Ok(Err(Fault::Decline))};
            if py::session_name(project) == sess { return Ok(Ok(Some(doc))); }
        }
        Ok(Ok(None))
    }).map_err(|_| Fault::Decline)?
}
pub fn state_agent_domain(home: &Path, state: &Path, sess: &str) -> Result<String, Fault> {
    let Some(doc) = first_state_record_domain(home, state, sess)? else {
        return Ok("claude".into());
    };
    match doc.get("agent") {
        Some(Value::String(agent)) if !agent.is_empty() => Ok(agent.clone()),
        Some(value) if comandos_core::json::truthy(value) => Err(Fault::Decline),
        _ => Ok("claude".into()),
    }
}

/// `state_agent(sess)` (5428): `agent or "claude"` del primer registro.
pub fn state_agent(state: &Path, sess: &str) -> Result<String, Fault> {
    let Some(doc) = first_state_record(state, sess)? else {
        return Ok("claude".into());
    };
    match doc.get("agent") {
        Some(Value::String(agent)) if !agent.is_empty() => Ok(agent.clone()),
        Some(value) if comandos_core::json::truthy(value) => Err(Fault::Decline),
        _ => Ok("claude".into()),
    }
}

/// Entradas de `dir` como las da `glob('*')`: sin punto inicial, en el orden
/// de `read_dir`; un error de lectura da lista vacía.
fn glob_star(dir: &Path) -> Vec<(PathBuf, std::ffi::OsString)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| (e.path(), e.file_name()))
        .filter(|(_, name)| !name.as_bytes().starts_with(b"."))
        .collect()
}

/// `find_project_dir(sess)` (5409): `~/codebase/*` y después
/// `~/codebase/*/*`, el primer directorio cuyo `session_name(basename).lower()`
/// es `sess.lower()`. El `lower()` de Python es Unicode: con un nombre o una
/// sesión no ASCII que hubiera que comparar se declina antes de nada.
pub fn find_project_dir(home: &Path, sess: &str) -> Result<Option<PathBuf>, Fault> {
    if !sess.is_ascii() {
        return Err(Fault::Decline);
    }
    let want = sess.to_ascii_lowercase();
    let base = home.join("codebase");
    let check = |path: &Path, name: &std::ffi::OsStr| -> Result<bool, Fault> {
        // `os.path.isdir` sigue enlaces.
        if !path.is_dir() {
            return Ok(false);
        }
        // Un nombre que no es UTF-8 lleva sustitutos: nunca iguala a un texto ASCII.
        let Some(name) = name.to_str() else {
            return Ok(false);
        };
        if !name.is_ascii() {
            return Err(Fault::Decline);
        }
        Ok(py::session_name(name).to_ascii_lowercase() == want)
    };
    let top = glob_star(&base);
    for (path, name) in &top {
        if check(path, name)? {
            return Ok(Some(path.clone()));
        }
    }
    // `*/*`: solo los directorios del primer nivel (`entry.is_dir()` sigue enlaces).
    for (dir, _) in top.iter().filter(|(p, _)| p.is_dir()) {
        for (path, name) in glob_star(dir) {
            if check(&path, &name)? {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

/// `agent_process_aliases()` del Python sobre el registro y `cc-notify.conf`
/// del frente (lo mismo que hace el escaneo de `/state`).
type ProcessNames = (std::collections::BTreeSet<String>, HashMap<String, String>);
fn process_aliases(ctx: &Context) -> Result<ProcessNames, Fault> {
    let repo = ctx.repo_root.as_ref().ok_or(Fault::Decline)?;
    let catalog = catalog_paths(
        &ctx.home,
        &ctx.cwd,
        ctx.codex_home.as_deref(),
        ctx.grok_home.as_deref(),
    );
    let registry = RegistryCache::default()
        .load(&repo.join("config/providers.json"), &catalog)
        .map_err(|_| Fault::Decline)?;
    let conf =
        providers::read_conf(&ctx.hooks.join("cc-notify.conf")).map_err(|_| Fault::Decline)?;
    let conf_agents = conf
        .iter()
        .find(|(k, _)| k == "AGENTS")
        .map(|(_, v)| v.as_str());
    let agents = providers::agent_set(conf_agents, &registry);
    let aliases = providers::process_aliases(&agents, &registry);
    Ok((agents, aliases))
}

/// Lo que los trabajos de disco necesitan de las opciones.
#[derive(Clone)]
struct Context {
    home: PathBuf,
    hooks: PathBuf,
    proc_root: PathBuf,
    repo_root: Option<PathBuf>,
    cwd: PathBuf,
    codex_home: Option<PathBuf>,
    grok_home: Option<PathBuf>,
}

impl Context {
    fn of(opts: &NativeOptions) -> Self {
        Self {
            home: opts.home.clone(),
            hooks: opts.hooks.clone(),
            proc_root: opts.proc_root.clone(),
            repo_root: opts.repo_root.clone(),
            cwd: opts.cwd.clone(),
            codex_home: opts.codex_home.clone(),
            grok_home: opts.grok_home.clone(),
        }
    }
}

/// Lo que decide el primer salto de `resolve_project_session`.
enum Plan {
    /// Ningún registro: `None`.
    Nothing,
    /// La sesión del registro existe: `(sesión, pane si casa PANE_RE)`.
    Live { session: String, pane: String },
    /// Sin `cwd` (ni el del registro ni `find_project_dir`): `None`.
    NoCwd,
    /// Buscar el agente en `cwd`.
    Search {
        project: String,
        cwd: String,
        procs: Vec<AgentProc>,
        tabs: Vec<(String, String)>,
        history: Vec<Map<String, Value>>,
    },
}

/// `str(d.get(key) or "")` cuando solo importa si casa una expresión ASCII:
/// un valor que no es texto da `repr`s con comillas o corchetes que nunca
/// casan; los escalares con `str()` seguro se convierten.
fn text_field(doc: &Map<String, Value>, key: &str) -> Result<String, Fault> {
    match doc.get(key) {
        None => Ok(String::new()),
        Some(v) if !comandos_core::json::truthy(v) => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Array(_) | Value::Object(_)) => Ok(String::new()),
        Some(v) => py::str_scalar(v).ok_or(Fault::Decline),
    }
}

/// Primer salto (bloqueante): registros, `find_project_dir`, `agent_procs` y
/// los archivos de pestañas que `session_labels` leerá.
fn plan(ctx: &Context, sess: &str, live: &HashSet<String>) -> Result<Plan, Fault> {
    let Some(doc) = first_state_record_domain(&ctx.home, &ctx.hooks.join("state"), sess)? else {
        return Ok(Plan::Nothing);
    };
    let session = text_field(&doc, "session")?;
    if py::is_session(&session) && live.contains(&session) {
        return Ok(Plan::Live {
            session,
            pane: text_field(&doc, "pane")?,
        });
    }
    // `d.get("cwd", "") or find_project_dir(sess) or ""`: un `cwd` que no es
    // texto no tiene comparación segura con los de `/proc`.
    let cwd = match doc.get("cwd") {
        Some(Value::String(c)) if !c.is_empty() => c.clone(),
        Some(v) if comandos_core::json::truthy(v) => return Err(Fault::Decline),
        _ => match find_project_dir(&ctx.home, sess)? {
            Some(dir) => dir.to_str().ok_or(Fault::Decline)?.to_owned(),
            None => String::new(),
        },
    };
    if cwd.is_empty() {
        return Ok(Plan::NoCwd);
    }
    let (agents, aliases) = process_aliases(ctx)?;
    let procs = agent_procs_for_agents(&ctx.proc_root, &aliases, &agents)
        .map_err(|_| Fault::Decline)?
        .into_iter()
        .filter(|p| p.cwd == cwd)
        .collect();
    let project = match doc.get("project") {
        Some(Value::String(p)) => p.clone(),
        _ => String::new(),
    };
    Ok(Plan::Search {
        project,
        cwd,
        procs,
        tabs: light::tab_labels_domain(&ctx.home, &ctx.hooks)?,
        history: light::read_tab_history_domain(&ctx.home, &ctx.hooks)?,
    })
}

/// `choose_agent_pane(project, sess, candidates, labels)` (6143): misma
/// sesión; si no, la pestaña cuya etiqueta es el proyecto; si no, la primera.
pub fn choose_agent_pane<'a>(
    project: &str,
    sess: &str,
    candidates: &'a [AgentInfo],
    labels: &HashMap<String, String>,
) -> Option<&'a AgentInfo> {
    let first = candidates.first()?;
    if let Some(info) = candidates.iter().find(|i| i.session == sess) {
        return Some(info);
    }
    let labelled = candidates.iter().find(|info| {
        let label = labels.get(&info.session).map(String::as_str).unwrap_or("");
        !label.is_empty() && (label == project || py::session_name(label) == sess)
    });
    Some(labelled.unwrap_or(first))
}

/// `display-message -p -t <pane> '#{pane_id}'` y `.stdout.strip() == pane`.
/// Las excepciones del Python no se capturan.
async fn pane_alive(native: &Native, pane: &str) -> Result<bool, Fault> {
    let out = native
        .options()
        .tmux
        .run(&["display-message", "-p", "-t", pane, "#{pane_id}"])
        .await
        .map_err(|e| Fault::Error(e.uncaught()))?;
    Ok(py::strip(&out.stdout) == pane)
}

/// `resolve_project_session(sess)` (6183): el pane real del agente de un
/// proyecto sin sesión tmux propia, o `None`.
pub async fn resolve_project_session(
    native: &Native,
    sess: &str,
) -> Result<Option<(String, String)>, Fault> {
    let opts = native.options();
    let live = light::tmux_sessions(&opts.tmux).await?;
    let ctx = Context::of(opts);
    let owned = sess.to_owned();
    let planned = blocking(move || plan(&ctx, &owned, &live)).await?;
    let (project, cwd, procs, tabs, history) = match planned {
        Plan::Nothing | Plan::NoCwd => return Ok(None),
        Plan::Live { session, pane } => {
            let pane = match py::is_pane(&pane) {
                // `\d` de Python acepta dígitos Unicode.
                None => return Err(Fault::Decline),
                Some(true) if pane_alive(native, &pane).await? => pane,
                Some(_) => String::new(),
            };
            return Ok(Some((session, pane)));
        }
        Plan::Search {
            project,
            cwd,
            procs,
            tabs,
            history,
        } => (project, cwd, procs, tabs, history),
    };
    // `agent_pane_maps(procs)`: `tmux_pane_inventory()` siempre se lee.
    let listed = opts
        .tmux
        .run(&["list-panes", "-a", "-F", PANE_FORMAT])
        .await
        .map_err(|e| Fault::Error(e.uncaught()))?;
    let mut panes = if listed.ok {
        parse_pane_inventory(&listed.stdout)
    } else {
        Vec::new()
    };
    agent_procs::resolve_foreground(opts.proc_root.as_path(), &mut panes);
    let candidates = if procs.is_empty() {
        Vec::new()
    } else {
        let proc_root = opts.proc_root.clone();
        let cwd = cwd.clone();
        blocking(move || {
            let mut parents: HashMap<i64, i64> = HashMap::new();
            let mut parent = |pid: i64| {
                *parents
                    .entry(pid)
                    .or_insert_with(|| agent_procs::parent_pid(&proc_root, pid))
            };
            let owners = process_owners(&procs, &panes, &mut parent);
            let mut cmdline = |pid: i64| agent_procs::proc_cmdline(&proc_root, pid);
            let maps = agent_pane_maps(&procs, &panes, &owners, &mut cmdline, &mut parent);
            Ok(maps
                .by_cwd
                .into_iter()
                .find(|(c, _)| *c == cwd)
                .map(|(_, infos)| infos)
                .unwrap_or_default())
        })
        .await?
    };
    // `session_labels()`: pestañas, `tmux_sessions()` e historial.
    let live = light::tmux_sessions(&opts.tmux).await?;
    let labels = session_labels(tabs, &live, &history);
    Ok(choose_agent_pane(&project, sess, &candidates, &labels)
        .map(|info| (info.session.clone(), info.pane.clone())))
}

/// `agent_pane_maps(agent_procs())` (que siempre lee `list-panes -a`): los
/// agentes vivos emparejados con sus panes. Solo lee.
async fn live_agent_maps(native: &Native) -> Result<AgentMaps, Fault> {
    let opts = native.options();
    let ctx = Context::of(opts);
    let procs = blocking(move || {
        let (agents, aliases) = process_aliases(&ctx)?;
        agent_procs_for_agents(&ctx.proc_root, &aliases, &agents).map_err(|_| Fault::Decline)
    })
    .await?;
    let listed = opts
        .tmux
        .run(&["list-panes", "-a", "-F", PANE_FORMAT])
        .await
        .map_err(|e| Fault::Error(e.uncaught()))?;
    let mut panes = if listed.ok {
        parse_pane_inventory(&listed.stdout)
    } else {
        Vec::new()
    };
    agent_procs::resolve_foreground(opts.proc_root.as_path(), &mut panes);
    if procs.is_empty() {
        return Ok(AgentMaps::default());
    }
    let proc_root = opts.proc_root.clone();
    blocking(move || {
        let mut parents: HashMap<i64, i64> = HashMap::new();
        let mut parent = |pid: i64| {
            *parents
                .entry(pid)
                .or_insert_with(|| agent_procs::parent_pid(&proc_root, pid))
        };
        let owners = process_owners(&procs, &panes, &mut parent);
        let mut cmdline = |pid: i64| agent_procs::proc_cmdline(&proc_root, pid);
        Ok(agent_pane_maps(
            &procs,
            &panes,
            &owners,
            &mut cmdline,
            &mut parent,
        ))
    })
    .await
}

/// `agent_info_for_pane(pane)` (1591) para un pane que ya casa `PANE_RE`:
/// el primer agente de `by_cwd` en ese pane, o `None`. Solo lee.
pub async fn agent_info_for_pane(native: &Native, pane: &str) -> Result<Option<AgentInfo>, Fault> {
    Ok(live_agent_maps(native)
        .await?
        .by_cwd
        .into_iter()
        .flat_map(|(_, infos)| infos)
        .find(|info| info.pane == pane))
}

/// `_cwd_has_live_agent(cwd)` (4130): ¿hay un agente vivo cuya carpeta es
/// exactamente `cwd` (`bool(by_cwd.get(cwd))`)? Solo lee.
pub(crate) async fn cwd_has_live_agent(native: &Native, cwd: &str) -> Result<bool, Fault> {
    Ok(live_agent_maps(native)
        .await?
        .by_cwd
        .iter()
        .any(|(folder, infos)| folder == cwd && !infos.is_empty()))
}

/// Los campos de `_pane_identity`, en su orden (los del runtime: una sola
/// implementación para el frente y las operaciones de sesión).
pub use comandos_runtime::session_configuration::{IDENTITY_FIELDS, identity_key};

fn identity_fault(fail: sc::Fail) -> TargetError {
    match fail {
        sc::Fail::Py(text) => TargetError::Value(text),
        sc::Fail::Unsure => Fault::Decline.into(),
    }
}

/// `_pane_identity(sess, pane)` (6769): las ocho claves de tmux y
/// `server_start` (campo 22 de `/proc/<pid>/stat`, el 19 tras el último `)`),
/// con las piezas de `comandos_runtime::session_configuration`.
pub async fn pane_identity(
    native: &Native,
    sess: &str,
    pane: &str,
) -> Result<Map<String, Value>, TargetError> {
    match py::is_pane(pane) {
        None => return Err(Fault::Decline.into()),
        Some(false) => return Err(TargetError::Value("se necesita el panel exacto".into())),
        Some(true) => {}
    }
    let opts = native.options();
    let out = opts
        .tmux
        .run(&["display-message", "-p", "-t", pane, &sc::identity_format()])
        .await
        .map_err(|e| Fault::Error(e.uncaught()))?;
    let mut identity =
        sc::identity_from_output(out.ok, &out.stdout, sess, pane).map_err(identity_fault)?;
    let proc_root = opts.proc_root.clone();
    let pid = identity
        .get("pid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let start = blocking(move || Ok(sc::server_start(&proc_root, &pid)))
        .await?
        .map_err(identity_fault)?;
    identity.insert("server_start".into(), Value::String(start));
    Ok(identity)
}

/// El preámbulo de `do_POST`: valida `session`, la sustituye por la resuelta
/// salvo en las rutas de configuración, y elige el pane exacto si vive.
pub async fn post_target(native: &Native, path: &str, data: &Value) -> Result<PostTarget, Answer> {
    let data = data.as_object().ok_or(Err(failure()))?;
    // `data.get("session", "")` sin `str()`: un no-texto en `SESSION_RE.match`
    // es un `TypeError` (500).
    let raw = match data.get("session") {
        None => "",
        Some(Value::String(s)) => s.as_str(),
        Some(_) => return Err(Err(failure())),
    };
    if !py::is_session(raw) {
        return Err(reply(
            StatusCode::BAD_REQUEST,
            &json!({"error": "Nombre de sesion invalido"}),
        ));
    }
    let resolved = resolve_project_session(native, raw).await.map_err(Err)?;
    let sess = match &resolved {
        Some((real, _)) if !CONFIG_ROUTES.contains(&path) => real.clone(),
        _ => raw.to_owned(),
    };
    let target = format!("={sess}");
    let mut pane = format!("={sess}:");
    // `str(data.get("pane", "") or (resolved[1] if resolved else ""))`: un
    // valor falso cae al pane resuelto; un no-texto verdadero da un `str()` que
    // nunca casa PANE_RE.
    let want = match data.get("pane") {
        Some(Value::String(p)) if !p.is_empty() => p.clone(),
        Some(v) if comandos_core::json::truthy(v) => String::new(),
        _ => resolved
            .as_ref()
            .map(|(_, p)| p.clone())
            .unwrap_or_default(),
    };
    match py::is_pane(&want) {
        None => return Err(Err(Fault::Decline)),
        Some(true) if pane_alive(native, &want).await.map_err(Err)? => pane = want,
        Some(_) => {}
    }
    Ok(PostTarget {
        sess,
        target,
        pane,
        resolved,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(session: &str, pane: &str) -> AgentInfo {
        AgentInfo {
            session: session.into(),
            pane: pane.into(),
            cwd: "/x".into(),
            agent: "claude".into(),
            pid: 1,
        }
    }

    #[test]
    fn choose_prefers_session_then_label_then_first() {
        let c = [info("a", "%1"), info("b", "%2"), info("mi-p", "%3")];
        let none = HashMap::new();
        assert_eq!(
            choose_agent_pane("Mi.P", "mi-p", &c, &none).unwrap().pane,
            "%3"
        );
        let labels: HashMap<String, String> = [("b".to_owned(), "Mi.P".to_owned())].into();
        assert_eq!(
            choose_agent_pane("Mi.P", "otra", &c, &labels).unwrap().pane,
            "%2"
        );
        let labels: HashMap<String, String> = [("b".to_owned(), "otra.x".to_owned())].into();
        assert_eq!(
            choose_agent_pane("Mi.P", "otra-x", &c, &labels)
                .unwrap()
                .pane,
            "%2"
        );
        assert_eq!(
            choose_agent_pane("Mi.P", "nada", &c, &none).unwrap().pane,
            "%1"
        );
        assert!(choose_agent_pane("Mi.P", "nada", &[], &none).is_none());
    }

    #[test]
    fn identity_key_joins_six_fields() {
        let mut m = Map::new();
        for (k, v) in [("socket_path", "/s"), ("pid", "1"), ("pane_id", "%0")] {
            m.insert(k.into(), Value::from(v));
        }
        assert_eq!(identity_key(&m), "/s|1|||%0|");
    }
}
