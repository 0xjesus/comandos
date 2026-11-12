//! Corte `tabs` (plan 2f-1, Tareas 3 y 5): crear y revivir sesiones.
//! POST `/recover-tab` (9540), `/ensure` (9713), `/new` (9737), `/shell`
//! (9758), `/up` (9774), `/ssh-connect` (9130, `ssh_connect` 7750) y
//! `/ssh-new-tab` (9137, `ssh_open_new_tab` 7801) de `bin/cc-dash`, el
//! registro de la terminal rápida fuera de la barra (`quick_register`, que
//! llama `quick.rs`) y las piezas que las otras tareas
//! del corte reutilizan: `agent_launch` (5948), `agent_set` (5965),
//! `scope_cmd` (5420), `tmux_new_session` (5441), `ensure_shell_window`
//! (5458), `focus_session` (5533) y `select_claude_window` (7735).
//!
//! `/recover-tab` va tras el preámbulo de `do_POST` sin
//! `resolve_project_session` (en el Python está antes de esa llamada); las
//! otras cuatro, tras `target::post_target`.
//!
//! Linux: sin `systemd-run` (`opts.scope == None`) las cinco declinan antes de nada
//! (R2 del pre-flight de la 2d): un servidor tmux que naciera fuera de un scope
//! del gestor de usuario viviría en el cgroup del frente y moriría con él.
//! Darwin lanza la cola tmux directamente y no requiere ese scope.
//!
//! `Decline` solo antes del primer efecto: cada efecto (orden de tmux que muta,
//! escritura del registro o de `app-focus.json`, `wmctrl`, la terminal) marca
//! la petición (`EFFECTS`, local de su tarea) y un `Decline` tras la marca es
//! un 500 con su línea en stderr (ruling 2 del sub-plan). Los archivos del
//! registro se leen antes de crear la sesión, así un registro incierto da el
//! 500 sin sesión nueva.
//!
//! Cancelación: cada petición corre en su propia tarea, contada en
//! `Native::tasks` (`spawn_handle`, también al apagar); si el cliente se va, la sesión y el registro terminan igual que en el Python.
//!
//! Efectos en vivo (los del Python, en su orden): `systemd-run --user --scope
//! --collect --quiet tmux new-session -d -s <s> …` (plazo 15 s), `new-window -d
//! -t =<s> -n shell …`, `select-window -t =<s>:shell`, `switch-client -c
//! <cliente más activo> -t =<s>` cuando la sesión no tiene clientes,
//! `app-focus.json`, el registro de pestañas, la terminal (`systemd-run --user
//! --collect --quiet <terminal> …`) y `wmctrl` con el pulso keep-above. Nunca
//! mata ni renombra sesiones; todo destino es `=<sesión pedida>`.
//!
//! SSH (T5), en vivo: la prueba de llave `ssh -o BatchMode=yes -o
//! ConnectTimeout=6 <host> true` (14 s, proceso asíncrono, nunca un hilo
//! retenido), `ssh -O check <host>` (3 s, solo consulta el socket de control),
//! `systemd-run --user --scope --collect --quiet tmux new-session -d -s
//! ssh-<host>|sshtab-<host>-<i> -n ssh "ssh <host>; exec $SHELL"`,
//! `set-option -t <s> mouse off` (sin `=`, como el Python), `send-keys` al
//! pane de `=ssh-<host>:` y el registro. El `ssh` es `opts.ssh` (el mismo que
//! `/state`). Diferencias del Python que se reproducen tal cual: un host con
//! `.` da una sesión que tmux renombra (`.` → `_`) y los `=<s>` siguientes no
//! la encuentran; un alias que empieza por `-` (lo admite `SSH_HOST_RE`)
//! llega a `ssh` como opción.
use super::{
    Answer, Entry, Fault, Key, Native, NativeOptions, NativeRoute, Verb, light, procs, py, reply,
    states::gather::session_labels,
    tabs::tab_registry::{self, RegistryError},
    target,
    tmux::{Output, Program, RunError, run_program},
};
use crate::{HandlerError, Request};
use comandos_core::json::{response_dumps, truthy};
use comandos_core::text::shlex_quote;
use comandos_runtime::{model_catalog::catalog_paths, providers, ssh_config};
use http::StatusCode;
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

mod launch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionsRoute {
    RecoverTab,
    Ensure,
    New,
    Shell,
    Up,
    SshConnect,
    SshNewTab,
    SessionNew,
    AccountAdd,
    AccountRemove,
    AccountRename,
}

impl SessionsRoute {
    /// La ruta del Python (`self.path == …`).
    pub const fn path(self) -> &'static str {
        match self {
            SessionsRoute::RecoverTab => "/recover-tab",
            SessionsRoute::Ensure => "/ensure",
            SessionsRoute::New => "/new",
            SessionsRoute::Shell => "/shell",
            SessionsRoute::Up => "/up",
            SessionsRoute::SshConnect => "/ssh-connect",
            SessionsRoute::SshNewTab => "/ssh-new-tab",
            SessionsRoute::SessionNew => "/session-new",
            SessionsRoute::AccountAdd => "/account/add",
            SessionsRoute::AccountRemove => "/account/remove",
            SessionsRoute::AccountRename => "/account/rename",
        }
    }
}

const fn entry(route: SessionsRoute) -> Entry {
    Entry {
        verb: Verb::Post,
        key: Key::Raw(route.path()),
        route: NativeRoute::Sessions(route),
    }
}

pub const ROUTES: &[Entry] = &[
    entry(SessionsRoute::RecoverTab),
    entry(SessionsRoute::Ensure),
    entry(SessionsRoute::New),
    entry(SessionsRoute::Shell),
    entry(SessionsRoute::Up),
    entry(SessionsRoute::SshConnect),
    entry(SessionsRoute::SshNewTab),
    entry(SessionsRoute::SessionNew),
    entry(SessionsRoute::AccountAdd),
    entry(SessionsRoute::AccountRemove),
    entry(SessionsRoute::AccountRename),
];

/// `AGENT_LAUNCH` (5935), literal.
pub const AGENT_LAUNCH: [(&str, &str); 8] = [
    ("claude", "claude --continue 2>/dev/null || claude"),
    ("codex", "codex resume --last 2>/dev/null || codex"),
    ("grok", "grok --continue 2>/dev/null || grok"),
    ("opencode", "opencode --continue 2>/dev/null || opencode"),
    ("gemini", "gemini"),
    ("agy", "agy --continue 2>/dev/null || agy"),
    ("aider", "aider"),
    ("acp", "cc-acp"),
];

/// Lo que `tmux_new_session` antepone al lanzador del agente.
const PROVIDERS_ENV: &str = "set -a; . \"$HOME/.claude/hooks/providers.env\" 2>/dev/null; set +a; ";

/// Plazo de `subprocess.run(scope_cmd(...), timeout=15)`.
const SCOPE_SECONDS: u64 = 15;

/// Plazo de `wm()` en `focus_session`.
const WMCTRL_SECONDS: u64 = 2;

/// `threading.Timer(0.6, drop)` del pulso keep-above.
const KEEP_ABOVE_MS: u64 = 600;

/// `TERMINALS` (4811): nombre (también la clase de ventana) en su orden.
const TERMINALS: [&str; 3] = ["kitty", "tilix", "gnome-terminal"];

fn failure() -> Fault {
    Fault::Error(HandlerError::Failure)
}

tokio::task_local! {
    /// ¿Hubo ya un efecto en esta petición? Lo fija `mark_effect`.
    static EFFECTS: Arc<AtomicBool>;
}

/// Justo antes de un efecto. Fuera de una petición de estas rutas (otra
/// tarea del corte que reutiliza las piezas) no hace nada: esa ruta lleva su
/// propia cuenta.
fn mark_effect() {
    let _ = EFFECTS.try_with(|flag| flag.store(true, Ordering::Release));
}

/// Tras el primer efecto ya no se declina (ruling 2): un `Decline` es un 500
/// con su línea en stderr.
fn settle(path: &str, answer: Answer, effects: &AtomicBool) -> Answer {
    match answer {
        Err(Fault::Decline) if effects.load(Ordering::Acquire) => {
            eprintln!("comandos dash: estado incierto tras un efecto; {path} responde 500");
            Err(failure())
        }
        other => other,
    }
}

/// Un trabajo de disco; su pánico es una excepción sin capturar.
async fn blocking<T, F>(job: F) -> Result<T, Fault>
where
    F: FnOnce() -> Result<T, Fault> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|_| failure())?
}

/// `tmux(*args)` (plazo 5 s) cuyas excepciones el Python no captura.
async fn tmux(opts: &NativeOptions, args: &[&str]) -> Result<Output, Fault> {
    opts.tmux
        .run(args)
        .await
        .map_err(|e| Fault::Error(e.uncaught()))
}

/// `tmux` de una orden que muta (marca el efecto antes).
async fn mutate(opts: &NativeOptions, args: &[&str]) -> Result<Output, Fault> {
    mark_effect();
    tmux(opts, args).await
}

pub async fn answer(native: &Arc<Native>, route: SessionsRoute, request: &Request) -> Answer {
    // R2: sin scope de systemd ninguna de las que lo usan se atiende aquí
    // (`/session-new` y `/account/add` no lo usan, como el Python).
    let scoped = !matches!(
        route,
        SessionsRoute::SessionNew
            | SessionsRoute::AccountAdd
            | SessionsRoute::AccountRemove
            | SessionsRoute::AccountRename
    );
    if scoped && native.options().scope.is_none() {
        return Err(Fault::Decline);
    }
    let data = light::data(request)?.clone();
    let effects = Arc::new(AtomicBool::new(false));
    let job = native
        .tasks()
        .spawn_handle({
            let native = Arc::clone(native);
            let effects = Arc::clone(&effects);
            EFFECTS.scope(effects, async move { run(&native, route, data).await })
        })
        .map_err(|_| failure())?;
    let answer = job.await.map_err(|_| failure())?;
    settle(route.path(), answer, &effects)
}

async fn run(native: &Native, route: SessionsRoute, data: Map<String, Value>) -> Answer {
    // Las tres van antes de `resolve_project_session` en el `do_POST` del Python.
    match route {
        SessionsRoute::RecoverTab => return recover_tab(native, &data).await,
        SessionsRoute::SshConnect => return ssh_connect(native, &data).await,
        SessionsRoute::SshNewTab => return ssh_new_tab(native, &data).await,
        // Antes de `resolve_project_session`, como en el `do_POST` (T4).
        SessionsRoute::SessionNew => return launch::session_new(native, &data).await,
        SessionsRoute::AccountAdd => return launch::account_add(native, &data).await,
        SessionsRoute::AccountRemove => return launch::account_remove(native, &data).await,
        SessionsRoute::AccountRename => return launch::account_rename(native, &data).await,
        _ => {}
    }
    let value = Value::Object(data);
    let pt = match target::post_target(native, route.path(), &value).await {
        Ok(pt) => pt,
        Err(answer) => return answer,
    };
    let Value::Object(data) = value else {
        return Err(failure());
    };
    match route {
        SessionsRoute::Ensure => ensure(native, &pt, &data, raw_cwd(&data)?).await,
        SessionsRoute::New => new(native, &pt, &data).await,
        SessionsRoute::Shell => shell(native, &pt, &data, raw_cwd(&data)?).await,
        SessionsRoute::Up => up(native, &pt, &data, raw_cwd(&data)?).await,
        SessionsRoute::RecoverTab
        | SessionsRoute::SshConnect
        | SessionsRoute::SshNewTab
        | SessionsRoute::SessionNew
        | SessionsRoute::AccountAdd
        | SessionsRoute::AccountRemove
        | SessionsRoute::AccountRename => Err(failure()),
    }
}

/// `data.get("cwd", "")` sin `str()`: un valor verdadero que no es texto
/// llega a `os.path.isdir`/`isabs` con un comportamiento que no se reproduce
/// (un entero es un descriptor). Se declina antes de nada.
fn raw_cwd(data: &Map<String, Value>) -> Result<String, Fault> {
    match data.get("cwd") {
        Some(Value::String(c)) => Ok(c.clone()),
        Some(v) if truthy(v) => Err(Fault::Decline),
        _ => Ok(String::new()),
    }
}

// ------------------------------------------------------------------ piezas

/// `str(value or fallback)`: un valor falso cae a `fallback`; un escalar
/// verdadero da su `str()`; un flotante o un contenedor verdadero declina.
fn text_or(value: Option<&Value>, fallback: &str) -> Result<String, Fault> {
    match value {
        Some(value) if truthy(value) => py::str_scalar(value).ok_or(Fault::Decline),
        _ => Ok(fallback.to_owned()),
    }
}

/// `os.path.isdir(cwd)` con el directorio de trabajo del frente (el `stat`
/// en un hilo de bloqueo: nunca en el runtime).
async fn is_dir(opts: &NativeOptions, cwd: &str) -> Result<bool, Fault> {
    if cwd.is_empty() {
        return Ok(false);
    }
    let path = opts.cwd.join(cwd);
    blocking(move || Ok(path.is_dir())).await
}

/// Una ruta de `find_project_dir` como texto (no UTF-8: declina).
fn dir_text(dir: Option<PathBuf>) -> Result<String, Fault> {
    match dir {
        Some(dir) => dir.to_str().map(str::to_owned).ok_or(Fault::Decline),
        None => Ok(String::new()),
    }
}

/// `find_project_dir(sess)` en un hilo de bloqueo, como texto (`""` si no hay).
async fn project_dir(opts: &NativeOptions, sess: &str) -> Result<String, Fault> {
    let (home, sess) = (opts.home.clone(), sess.to_owned());
    blocking(move || dir_text(target::find_project_dir(&home, &sess)?)).await
}

/// `os.path.expanduser("~")`.
fn home_text(opts: &NativeOptions) -> Result<String, Fault> {
    opts.home.to_str().map(str::to_owned).ok_or(Fault::Decline)
}

/// `read_conf()` (4819) del frente. Un archivo que no se lee como UTF-8 (o
/// cualquier `OSError` salvo «no existe») hace que el Python lance: 500.
fn read_conf(opts: &NativeOptions) -> Result<Vec<(String, String)>, Fault> {
    providers::read_conf(&opts.hooks.join("cc-notify.conf")).map_err(|_| failure())
}

fn conf_get<'a>(conf: &'a [(String, String)], key: &str) -> Option<&'a str> {
    conf.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// `agent_launch(agent)` (5948). Bloquea (lee `cc-notify.conf`).
pub(crate) fn agent_launch(opts: &NativeOptions, agent: &str) -> Result<String, Fault> {
    // `.upper()` de Python es Unicode: solo se reproduce en ASCII.
    if !agent.is_ascii() {
        return Err(Fault::Decline);
    }
    let key = format!(
        "AGENT_LAUNCH_{}",
        agent.replace(['-', '.'], "_").to_ascii_uppercase()
    );
    let conf = read_conf(opts)?;
    if let Some(custom) = conf_get(&conf, &key).filter(|c| !c.is_empty()) {
        return Ok(custom.to_owned());
    }
    let table = |name: &str| {
        AGENT_LAUNCH
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| *v)
    };
    Ok(table(agent)
        .or_else(|| table("claude"))
        .unwrap_or_default()
        .to_owned())
}

/// `agent_set()` (5965): `AGENTS` de la conf (o los de siempre) más los
/// harnesses del registro salvo `shell`. Sin `config/providers.json` el
/// `except: pass` deja solo la conf; un registro que el frente no reproduce
/// con certeza declina (solo se llama antes de efectos). Bloquea.
pub(crate) fn agent_set(opts: &NativeOptions) -> Result<BTreeSet<String>, Fault> {
    let conf = read_conf(opts)?;
    let agents = conf_get(&conf, "AGENTS");
    let repo = opts.repo_root.as_ref().ok_or(Fault::Decline)?;
    let file = repo.join("config/providers.json");
    if !file.exists() {
        return Ok(providers::agent_set(agents, &Value::Null));
    }
    let catalog = catalog_paths(
        &opts.home,
        &opts.cwd,
        opts.codex_home.as_deref(),
        opts.grok_home.as_deref(),
    );
    let registry = providers::RegistryCache::default()
        .load(&file, &catalog)
        .map_err(|_| Fault::Decline)?;
    Ok(providers::agent_set(agents, &registry))
}

/// `scope_cmd(argv)` (5420) con la cola `tail` + `args`: el `systemd-run` de
/// `opts.scope` tal cual (sus banderas `--user --scope --collect --quiet` ya
/// van en el prefijo, P18) seguido del programa de la cola con su prefijo. El
/// entorno de la cola (el socket privado en las pruebas) pasa al scope.
/// Linux: `None` sin `systemd-run` declina antes de efectos. Darwin usa la cola directa.
pub(crate) fn scope_cmd(opts: &NativeOptions, tail: &Program) -> Option<Program> {
    super::quick::platform_tmux(
        comandos_runtime::platform::host(),
        opts.scope.as_ref(),
        tail,
    )
}

/// `subprocess.run(scope_cmd(["tmux", *args]), capture_output=True,
/// text=True, timeout=15)`. `TimeoutExpired` no se captura (504); el resto
/// de excepciones, 500.
async fn run_scoped_tmux(opts: &NativeOptions, args: &[&str]) -> Result<Output, Fault> {
    let program = scope_cmd(opts, &opts.tmux.program).ok_or(Fault::Decline)?;
    run_program(&program, args, Duration::from_secs(SCOPE_SECONDS))
        .await
        .map_err(|error| match error {
            RunError::Timeout => Fault::Error(HandlerError::Timeout),
            RunError::Spawn(_) | RunError::Decode => failure(),
        })
}

/// `agent_launch(agent)` en un hilo de bloqueo.
async fn launch_of(opts: &NativeOptions, agent: &str) -> Result<String, Fault> {
    let (reads, owned) = (opts.clone(), agent.to_owned());
    blocking(move || agent_launch(&reads, &owned)).await
}

/// `tmux_new_session(sess, cwd, agent)` (5441): sesión en su scope con la
/// ventana del agente y, si nació, la ventana `shell`.
#[expect(dead_code, reason = "T4–T6")]
pub(crate) async fn tmux_new_session(
    native: &Native,
    sess: &str,
    cwd: &str,
    agent: &str,
) -> Result<Output, Fault> {
    let launch = launch_of(native.options(), agent).await?;
    new_session_with(native.options(), sess, cwd, &launch).await
}

/// `tmux_new_session` con el lanzador ya calculado (las rutas lo calculan
/// antes del primer efecto: así su `Decline` todavía puede reenviarse).
async fn new_session_with(
    opts: &NativeOptions,
    sess: &str,
    cwd: &str,
    launch: &str,
) -> Result<Output, Fault> {
    if scope_cmd(opts, &opts.tmux.program).is_none() {
        return Err(Fault::Decline);
    }
    let command = format!("{PROVIDERS_ENV}{launch}; exec $SHELL");
    mark_effect();
    let out = run_scoped_tmux(
        opts,
        &[
            "new-session",
            "-d",
            "-s",
            sess,
            "-n",
            "claude",
            "-c",
            cwd,
            &command,
        ],
    )
    .await?;
    if out.ok {
        let target = format!("={sess}");
        mutate(
            opts,
            &["new-window", "-d", "-t", &target, "-n", "shell", "-c", cwd],
        )
        .await?;
    }
    Ok(out)
}

/// `str.split()` sin argumentos.
fn split_ws(text: &str) -> Vec<&str> {
    text.split(py::is_space).filter(|w| !w.is_empty()).collect()
}

/// `ensure_shell_window(sess, cwd)` (5458).
pub(crate) async fn ensure_shell_window(
    native: &Native,
    sess: &str,
    cwd: &str,
) -> Result<(), Fault> {
    let opts = native.options();
    let target = format!("={sess}");
    let wins = tmux(
        opts,
        &["list-windows", "-t", &target, "-F", "#{window_name}"],
    )
    .await?;
    if split_ws(&wins.stdout).contains(&"shell") {
        return Ok(());
    }
    let meta = tab_registry::tab_metadata_for_session(native, sess)
        .await
        .map_err(|e| e.into_fault("ensure_shell_window"))?;
    let kind = meta.get("kind").and_then(Value::as_str).unwrap_or("");
    if matches!(kind, "ssh" | "ssh-tab") {
        let host = meta.get("host").and_then(Value::as_str).unwrap_or("");
        if ssh_config::is_host(host) {
            let command = format!("ssh {}; exec $SHELL", shlex_quote(host));
            mutate(
                opts,
                &["new-window", "-d", "-t", &target, "-n", "shell", &command],
            )
            .await?;
        }
        return Ok(());
    }
    let wcwd = if is_dir(opts, cwd).await? {
        cwd.to_owned()
    } else {
        match project_dir(opts, sess).await? {
            dir if dir.is_empty() => home_text(opts)?,
            dir => dir,
        }
    };
    mutate(
        opts,
        &[
            "new-window",
            "-d",
            "-t",
            &target,
            "-n",
            "shell",
            "-c",
            &wcwd,
        ],
    )
    .await?;
    Ok(())
}

/// `select_claude_window(sess)` (7735): la ventana `claude`; si tmux la
/// renombró, la que corre `claude`; si ninguna, la primera.
pub(crate) async fn select_claude_window(native: &Native, sess: &str) -> Result<(), Fault> {
    let opts = native.options();
    let claude = format!("={sess}:claude");
    if mutate(opts, &["select-window", "-t", &claude]).await?.ok {
        return Ok(());
    }
    let target = format!("={sess}");
    let panes = tmux(
        opts,
        &[
            "list-panes",
            "-s",
            "-t",
            &target,
            "-F",
            "#{window_index}|#{pane_current_command}",
        ],
    )
    .await?;
    for line in py::splitlines(&panes.stdout) {
        let (idx, cmd) = line.split_once('|').unwrap_or((line, ""));
        if cmd == "claude" {
            let window = format!("={sess}:{idx}");
            mutate(opts, &["select-window", "-t", &window]).await?;
            return Ok(());
        }
    }
    let first = format!("={sess}:^");
    mutate(opts, &["select-window", "-t", &first]).await?;
    Ok(())
}

/// `l.split(None, 1)` de una línea no vacía de `list-clients`, desempaquetada
/// en `(actividad, nombre)` y ordenada por `int(actividad)` descendente
/// (estable, como `sorted(..., reverse=True)`). Una línea sin nombre o una
/// actividad que no es entero lanzan en el Python (500).
fn order_clients(stdout: &str) -> Result<Vec<String>, Fault> {
    let mut rows: Vec<(i64, String)> = Vec::new();
    for line in py::splitlines(stdout) {
        if line.is_empty() {
            continue;
        }
        let rest = line.trim_start_matches(py::is_space);
        let (first, tail) = rest.split_once(py::is_space).ok_or_else(failure)?;
        let name = tail.trim_start_matches(py::is_space);
        if name.is_empty() {
            return Err(failure());
        }
        let activity = py::int(first).map_err(|_| failure())?;
        rows.push((activity, name.to_owned()));
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.0));
    Ok(rows.into_iter().map(|(_, name)| name).collect())
}

/// `wmctrl` (por ruta del `PATH` del frente) con `gui_env()`; `None` sin él.
fn wmctrl(opts: &NativeOptions) -> Option<Program> {
    let path = procs::which_in(opts.search_path.as_deref(), "wmctrl")?;
    let mut program = opts.program(path);
    program.env.extend(procs::gui_env_for(opts));
    Some(program)
}

/// `wm(*args)`: `returncode == 0`; cualquier excepción es `False`.
async fn wm(program: Option<&Program>, args: &[&str]) -> bool {
    let Some(program) = program else {
        return false;
    };
    mark_effect();
    matches!(
        run_program(program, args, Duration::from_secs(WMCTRL_SECONDS)).await,
        Ok(out) if out.ok
    )
}

/// `raise_win(match_flag, name)`: activa la ventana y da el pulso keep-above
/// (`add,above` y, 0,6 s después, en una tarea suelta, `remove,above`).
async fn raise_win(native: &Native, program: Option<&Program>, flag: &str, name: &str) -> bool {
    let with = |rest: &[&str]| -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        if !flag.is_empty() {
            args.push(flag.to_owned());
        }
        args.extend(rest.iter().map(|a| (*a).to_owned()));
        args
    };
    let activate = with(&["-a", name]);
    let refs: Vec<&str> = activate.iter().map(String::as_str).collect();
    let ok = wm(program, &refs).await;
    if ok {
        let add = with(&["-r", name, "-b", "add,above"]);
        let refs: Vec<&str> = add.iter().map(String::as_str).collect();
        wm(program, &refs).await;
        let remove = with(&["-r", name, "-b", "remove,above"]);
        let program = program.cloned();
        // Como el `threading.Timer`: no lo espera nadie.
        let _ = native.tasks().spawn(async move {
            tokio::time::sleep(Duration::from_millis(KEEP_ABOVE_MS)).await;
            let refs: Vec<&str> = remove.iter().map(String::as_str).collect();
            wm(program.as_ref(), &refs).await;
        });
    }
    ok
}

/// `pick_terminal()` (5386): `TERMINAL_CMD` de la conf si existe en el `PATH`;
/// si no, la primera de `TERMINALS`. Devuelve el nombre y su ruta. Bloquea.
fn pick_terminal(opts: &NativeOptions) -> Result<Option<(&'static str, PathBuf)>, Fault> {
    let conf = read_conf(opts)?;
    let want = conf_get(&conf, "TERMINAL_CMD").unwrap_or("");
    let found = |name: &str| procs::which_in(opts.search_path.as_deref(), name);
    for name in TERMINALS {
        if name == want
            && let Some(path) = found(name)
        {
            return Ok(Some((name, path)));
        }
    }
    for name in TERMINALS {
        if let Some(path) = found(name) {
            return Ok(Some((name, path)));
        }
    }
    Ok(None)
}

/// `argf(sess)` de `TERMINALS` tras el ejecutable.
fn terminal_args(name: &str, sess: &str) -> Vec<String> {
    let target = format!("={sess}");
    match name {
        "tilix" => vec!["-e".into(), format!("tmux attach -t {target}")],
        "gnome-terminal" => vec![
            "--".into(),
            "tmux".into(),
            "attach".into(),
            "-t".into(),
            target,
        ],
        _ => vec!["tmux".into(), "attach".into(), "-t".into(), target],
    }
}

/// `spawn_terminal(sess)` (5397): la terminal en su propia unidad
/// (`systemd-run --user --collect --quiet`) si hay `systemd-run`, con
/// `gui_env()`, sin esperarla. El ejecutable de la terminal va por su ruta
/// absoluta (del `PATH` del frente), no por nombre. Un fallo al lanzar es la
/// excepción de `Popen` (500).
async fn spawn_terminal(native: &Native, sess: &str) -> Result<Option<String>, Fault> {
    let opts = native.options();
    let reads = opts.clone();
    let picked = blocking(move || {
        let terminal = pick_terminal(&reads)?;
        #[cfg(target_os = "linux")]
        let runner = procs::which_in(reads.search_path.as_deref(), "systemd-run");
        #[cfg(not(target_os = "linux"))]
        let runner: Option<PathBuf> = None;
        Ok((terminal, runner))
    })
    .await?;
    let (Some((name, path)), runner) = picked else {
        return Ok(Some(
            "No encontre terminal (instala kitty: sudo apt install kitty)".into(),
        ));
    };
    let tail = terminal_args(name, sess);
    let (program, args): (Program, Vec<OsString>) = match runner {
        Some(runner) => {
            let mut args: Vec<OsString> = ["--user", "--collect", "--quiet"]
                .iter()
                .map(OsString::from)
                .collect();
            args.push(path.into_os_string());
            args.extend(tail.into_iter().map(OsString::from));
            (opts.program(runner), args)
        }
        None => (
            opts.program(path),
            tail.into_iter().map(OsString::from).collect(),
        ),
    };
    mark_effect();
    procs::spawn_detached(&program, &args, &procs::gui_env_for(opts)).map_err(|_| failure())?;
    Ok(None)
}

/// El `try` de `focus_session` con la app abierta: `app-focus.json` con
/// `json.dump` y `open(…, "w")` (sin escritura atómica, como el Python). Lo
/// que el Python captura (tmux, E/S) no escribe nada; un archivo del registro
/// incierto es el 500 del ruling 2.
async fn write_focus(native: &Native, sess: &str, win: &str) -> Result<(), Fault> {
    let opts = native.options();
    let hooks = opts.hooks.clone();
    let home = opts.home.clone();
    let files = blocking(move || {
        let tabs = light::tab_labels_domain(&home, &hooks);
        let history = light::read_tab_history_domain(&home, &hooks);
        Ok((tabs, history))
    })
    .await?;
    let (tabs, history) = match files {
        (Ok(tabs), Ok(history)) => (tabs, history),
        (Err(Fault::Decline), _) | (_, Err(Fault::Decline)) => {
            eprintln!("comandos dash: registro de pestañas incierto; app-focus.json responde 500");
            return Err(failure());
        }
        _ => return Ok(()),
    };
    let Ok(live) = light::tmux_sessions(&opts.tmux).await else {
        return Ok(());
    };
    let label = session_labels(tabs, &live, &history)
        .get(sess)
        .cloned()
        .unwrap_or_else(|| sess.to_owned());
    let Some(ts) = serde_json::Number::from_f64((opts.clock_seconds)()) else {
        return Ok(());
    };
    let doc = json!({"session": sess, "win": win, "label": label, "ts": Value::Number(ts)});
    let Ok(text) = response_dumps(&doc) else {
        return Ok(());
    };
    let path = opts.hooks.join("app-focus.json");
    let (home, now) = (opts.home.clone(), (opts.clock)());
    mark_effect();
    let _ = tokio::task::spawn_blocking(move || {
        comandos_store::domains::commands::publish(
            &home,
            "app-focus.json",
            text.as_bytes(),
            now,
            || Ok(std::fs::write(path, &text)?),
        )
    })
    .await;
    Ok(())
}

/// `focus_session(sess, win)` (5533): `Some(error)` si no hay sesión o no
/// hay terminal que abrir.
pub(crate) async fn focus_session(
    native: &Native,
    sess: &str,
    win: &str,
) -> Result<Option<String>, Fault> {
    let opts = native.options();
    let target = format!("={sess}");
    if !tmux(opts, &["has-session", "-t", &target]).await?.ok {
        return Ok(Some(format!("No hay sesion tmux '{sess}'")));
    }
    let local = tmux(
        opts,
        &["list-clients", "-t", "=local", "-F", "#{client_name}"],
    )
    .await?;
    let app_open = !py::strip(&local.stdout).is_empty();
    if app_open {
        write_focus(native, sess, win).await?;
    } else {
        let mine = tmux(
            opts,
            &["list-clients", "-t", &target, "-F", "#{client_name}"],
        )
        .await?;
        if py::strip(&mine.stdout).is_empty() {
            let all = tmux(
                opts,
                &["list-clients", "-F", "#{client_activity} #{client_name}"],
            )
            .await?;
            let clients = order_clients(&all.stdout)?;
            let Some(first) = clients.first() else {
                return spawn_terminal(native, sess).await;
            };
            mutate(opts, &["switch-client", "-c", first, "-t", &target]).await?;
        }
    }
    let reads = opts.clone();
    let program = blocking(move || Ok(wmctrl(&reads))).await?;
    let program = program.as_ref();
    if app_open {
        // Respaldo por si el monitor de archivos de la app no dispara.
        raise_win(native, program, "-x", "comandos").await;
    } else if !raise_win(native, program, "", sess).await
        && !raise_win(native, program, "-x", "comandos").await
    {
        let reads = opts.clone();
        if let Some((name, _)) = blocking(move || pick_terminal(&reads)).await? {
            raise_win(native, program, "-x", name).await;
        }
    }
    Ok(None)
}

/// Los archivos del registro que una ruta escribirá, leídos antes de crear
/// la sesión: uno incierto es el 500 del ruling 2 sin sesión nueva.
async fn registry_preflight(native: &Native, route: &str, history: bool) -> Result<(), Fault> {
    let opts = native.options();
    let hooks = opts.hooks.clone();
    let home = opts.home.clone();
    let read_opts = opts.clone();
    let checked: Result<(), RegistryError> = tokio::task::spawn_blocking(move || {
        tab_registry::load_document(
            &tab_registry::domain_document(&read_opts, tab_registry::TABS_FILE)?,
            None,
            json!({}),
        )?;
        tab_registry::read_metadata_domain(&read_opts, None)?;
        if history {
            light::read_tab_history_domain(&home, &hooks).map_err(|fault| match fault {
                Fault::Decline => RegistryError::Unsure(hooks.join(tab_registry::TAB_HISTORY_FILE)),
                other => RegistryError::Fault(other),
            })?;
        }
        Ok(())
    })
    .await
    .map_err(|_| failure())?;
    checked.map_err(|e| e.into_fault(route))
}

/// `data.get("agent") or state_agent(sess)` con el `agent.replace` de
/// `agent_launch`: un agente verdadero que no es texto lanza (500).
async fn requested_agent(
    opts: &NativeOptions,
    data: &Map<String, Value>,
    sess: &str,
) -> Result<String, Fault> {
    match data.get("agent") {
        Some(Value::String(a)) if !a.is_empty() => Ok(a.clone()),
        Some(v) if truthy(v) => Err(failure()),
        _ => {
            let (state, sess) = (opts.hooks.join("state"), sess.to_owned());
            blocking(move || target::state_agent(&state, &sess)).await
        }
    }
}

/// El arranque común de `/ensure`, `/shell` y `/up` cuando la sesión no vive:
/// `cwd` (o el del proyecto), 400 sin él, el agente y su lanzador (todo antes
/// del primer efecto) y `tmux_new_session`. Devuelve el `cwd` que queda (el
/// Python lo reasigna) o la respuesta de error.
async fn revive(
    native: &Native,
    route: &str,
    sess: &str,
    data: &Map<String, Value>,
    cwd: String,
) -> Result<Result<String, Answer>, Fault> {
    let opts = native.options();
    let cwd = if is_dir(opts, &cwd).await? {
        cwd
    } else {
        project_dir(opts, sess).await?
    };
    if cwd.is_empty() {
        return Ok(Err(reply(
            StatusCode::BAD_REQUEST,
            &json!({"error": format!("No encontre el directorio de '{sess}' en ~/codebase")}),
        )));
    }
    let agent = requested_agent(opts, data, sess).await?;
    let launch = launch_of(opts, &agent).await?;
    registry_preflight(native, route, false).await?;
    let out = new_session_with(opts, sess, &cwd, &launch).await?;
    if !out.ok {
        return Ok(Err(created_error(&out, "No se pudo crear la sesion")));
    }
    Ok(Ok(cwd))
}

/// `500 {"error": r.stderr.strip() or fallback}`.
fn created_error(out: &Output, fallback: &str) -> Answer {
    let text = py::strip(&out.stderr);
    let text = if text.is_empty() { fallback } else { text };
    reply(StatusCode::INTERNAL_SERVER_ERROR, &json!({"error": text}))
}

/// `write_tab_metadata(sess, kind, host=…, cwd=…)` como efecto de `route`.
async fn write_meta(
    native: &Native,
    route: &str,
    sess: &str,
    kind: &str,
    host: &str,
    cwd: &str,
) -> Result<(), Fault> {
    mark_effect();
    tab_registry::write_tab_metadata(native, sess, kind, host, cwd)
        .await
        .map(|_| ())
        .map_err(|e| e.into_fault(route))
}

// ------------------------------------------------------------------ rutas

/// POST `/recover-tab` (9540).
async fn recover_tab(native: &Native, data: &Map<String, Value>) -> Answer {
    const PATH: &str = "/recover-tab";
    let opts = native.options();
    let sess = match data.get("session") {
        None => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(failure()),
    };
    if !py::is_session(&sess) {
        return reply(
            StatusCode::BAD_REQUEST,
            &json!({"error": "Nombre de sesion invalido"}),
        );
    }
    let label = py::take_chars(&text_or(data.get("label"), &sess)?, 80);
    let cwd = text_or(data.get("cwd"), "")?;
    let raw_agent = match data.get("agent") {
        Some(value) if truthy(value) => py::str_scalar(value).ok_or(Fault::Decline)?,
        _ => {
            let (state, owned) = (opts.hooks.join("state"), sess.clone());
            blocking(move || target::state_agent(&state, &owned)).await?
        }
    };
    let mut agent = py::take_chars(&raw_agent, 16);
    let reads = opts.clone();
    let agents = blocking(move || agent_set(&reads)).await?;
    if !agents.contains(&agent) {
        agent = "claude".into();
    }
    let cwd = if is_dir(opts, &cwd).await? {
        cwd
    } else {
        let (home, from_label, owned) = (opts.home.clone(), py::session_name(&label), sess.clone());
        blocking(move || {
            match target::find_project_dir(&home, &from_label)? {
                Some(dir) => Ok(Some(dir)),
                None => target::find_project_dir(&home, &owned),
            }
            .and_then(dir_text)
        })
        .await?
    };
    if cwd.is_empty() {
        return reply(
            StatusCode::BAD_REQUEST,
            &json!({"error": format!("No encontre cwd para recuperar '{label}'")}),
        );
    }
    let target = format!("={sess}");
    if !tmux(opts, &["has-session", "-t", &target]).await?.ok {
        let launch = launch_of(opts, &agent).await?;
        registry_preflight(native, PATH, true).await?;
        let out = new_session_with(opts, &sess, &cwd, &launch).await?;
        if !out.ok {
            return created_error(&out, "No se pudo recuperar la sesion");
        }
    } else {
        registry_preflight(native, PATH, true).await?;
    }
    mark_effect();
    tab_registry::write_app_tab(native, &sess, &label)
        .await
        .map_err(|e| e.into_fault(PATH))?;
    write_meta(native, PATH, &sess, "project", "", &cwd).await?;
    tab_registry::remember_tab(native, &sess, Some(&label), &cwd, &agent, "recovered")
        .await
        .map_err(|e| e.into_fault(PATH))?;
    reply(
        StatusCode::OK,
        &json!({"ok": true, "session": sess, "label": label, "cwd": cwd}),
    )
}

/// POST `/ensure` (9713): sesión viva (la revive si hace falta) y la ventana
/// pedida, sin mover clientes ni ventanas.
async fn ensure(
    native: &Native,
    pt: &target::PostTarget,
    data: &Map<String, Value>,
    cwd: String,
) -> Answer {
    const PATH: &str = "/ensure";
    let opts = native.options();
    let sess = pt.sess.as_str();
    let mut cwd = cwd;
    if !tmux(opts, &["has-session", "-t", &pt.target]).await?.ok {
        match revive(native, PATH, sess, data, cwd).await? {
            Ok(revived) => cwd = revived,
            Err(answer) => return answer,
        }
    }
    if is_dir(opts, &cwd).await? {
        write_meta(native, PATH, sess, "project", "", &cwd).await?;
    } else {
        let meta = opts.hooks.join(tab_registry::TABS_META_FILE);
        let saved = blocking(move || {
            tab_registry::read_tab_metadata(&meta).map_err(|e| e.into_fault(PATH))
        })
        .await?;
        if !saved.get(sess).is_some_and(truthy) {
            let inferred = tab_registry::tab_metadata_for_session(native, sess)
                .await
                .map_err(|e| e.into_fault(PATH))?;
            let text = |key: &str| inferred.get(key).and_then(Value::as_str).unwrap_or("");
            write_meta(native, PATH, sess, text("kind"), text("host"), text("cwd")).await?;
        }
    }
    if matches!(data.get("win"), Some(Value::String(w)) if w == "shell") {
        ensure_shell_window(native, sess, &cwd).await?;
    }
    reply(StatusCode::OK, &json!({"ok": true, "session": sess}))
}

/// POST `/new` (9737): el proyecto de `~/codebase` con su agente; si no, una
/// terminal en blanco en el HOME.
async fn new(native: &Native, pt: &target::PostTarget, data: &Map<String, Value>) -> Answer {
    const PATH: &str = "/new";
    let opts = native.options();
    let sess = pt.sess.as_str();
    let proj = project_dir(opts, sess).await?;
    if !proj.is_empty() {
        if !tmux(opts, &["has-session", "-t", &pt.target]).await?.ok {
            let agent = requested_agent(opts, data, sess).await?;
            let launch = launch_of(opts, &agent).await?;
            registry_preflight(native, PATH, false).await?;
            let out = new_session_with(opts, sess, &proj, &launch).await?;
            if !out.ok {
                return created_error(&out, "No se pudo crear");
            }
        }
        write_meta(native, PATH, sess, "project", "", &proj).await?;
        return reply(
            StatusCode::OK,
            &json!({"ok": true, "kind": "project", "session": sess}),
        );
    }
    let home = home_text(opts)?;
    if !tmux(opts, &["has-session", "-t", &pt.target]).await?.ok {
        registry_preflight(native, PATH, false).await?;
        mark_effect();
        let out = run_scoped_tmux(opts, &["new-session", "-d", "-s", sess, "-c", &home]).await?;
        if !out.ok {
            return created_error(&out, "No se pudo crear");
        }
    }
    write_meta(native, PATH, sess, "shell", "", &home).await?;
    reply(
        StatusCode::OK,
        &json!({"ok": true, "kind": "shell", "session": sess}),
    )
}

/// POST `/shell` (9758): la ventana `shell` de la sesión, seleccionada y al
/// frente.
async fn shell(
    native: &Native,
    pt: &target::PostTarget,
    data: &Map<String, Value>,
    cwd: String,
) -> Answer {
    const PATH: &str = "/shell";
    let opts = native.options();
    let sess = pt.sess.as_str();
    let mut cwd = cwd;
    if !tmux(opts, &["has-session", "-t", &pt.target]).await?.ok {
        match revive(native, PATH, sess, data, cwd).await? {
            Ok(revived) => cwd = revived,
            Err(answer) => return answer,
        }
    }
    ensure_shell_window(native, sess, &cwd).await?;
    let window = format!("={sess}:shell");
    mutate(opts, &["select-window", "-t", &window]).await?;
    focus_session(native, sess, "shell").await?;
    reply(StatusCode::OK, &json!({"ok": true}))
}

/// POST `/up` (9774): revive si hace falta y trae la terminal.
async fn up(
    native: &Native,
    pt: &target::PostTarget,
    data: &Map<String, Value>,
    cwd: String,
) -> Answer {
    const PATH: &str = "/up";
    let opts = native.options();
    let sess = pt.sess.as_str();
    if !tmux(opts, &["has-session", "-t", &pt.target]).await?.ok
        && let Err(answer) = revive(native, PATH, sess, data, cwd).await?
    {
        return answer;
    }
    focus_session(native, sess, "claude").await?;
    reply(StatusCode::OK, &json!({"ok": true}))
}

// ------------------------------------------------------------- ssh (T5)

/// Plazo de la prueba de llave (`subprocess.run(..., timeout=14)`).
const SSH_PROBE_SECONDS: u64 = 14;

/// Plazo de `ssh -O check` en `ssh_state` (`timeout=3`).
const SSH_CHECK_SECONDS: u64 = 3;

/// `"Host desconocido; agregalo primero"` de las dos rutas.
const UNKNOWN_HOST: &str = "Host desconocido; agregalo primero";

/// `data.get("host", "")`: un valor que no es texto (también `null`) llega a
/// `SSH_HOST_RE.match` y lanza `TypeError` en el Python (500).
fn host_of(data: &Map<String, Value>) -> Result<String, Fault> {
    match data.get("host") {
        None => Ok(String::new()),
        Some(Value::String(host)) => Ok(host.clone()),
        Some(_) => Err(failure()),
    }
}

/// `any(h["host"] == host for h in parse_ssh_config())` (equivale a
/// `ssh_host_entry(host)`): lee el `~/.ssh/config` del HOME del frente en un
/// hilo de bloqueo; un error al leerlo (salvo «no existe») lanza en el Python.
async fn known_host(opts: &NativeOptions, host: &str) -> Result<bool, Fault> {
    let (home, host) = (opts.home.clone(), host.to_owned());
    blocking(move || {
        ssh_config::host_entry(&home, &host)
            .map(|entry| entry.is_some())
            .map_err(|_| failure())
    })
    .await
}

/// La prueba de llave: `ssh -o BatchMode=yes -o ConnectTimeout=6 <host> true`
/// con `text=True` y 14 s. Cualquier excepción (no arranca, vence el plazo,
/// salida que no es UTF-8) es `keyok=False, stderr="timeout"`. Proceso
/// asíncrono: la espera no ocupa ningún hilo; al vencer, `kill_on_drop` mata
/// el `ssh` como `subprocess.run`.
async fn ssh_probe(opts: &NativeOptions, host: &str) -> (bool, String) {
    mark_effect();
    let args = [
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=6",
        host,
        "true",
    ];
    match run_program(&opts.ssh, &args, Duration::from_secs(SSH_PROBE_SECONDS)).await {
        Ok(out) => (out.ok, out.stderr.to_lowercase()),
        Err(_) => (false, "timeout".into()),
    }
}

/// `subprocess.run(["ssh", "-O", "check", host], capture_output=True,
/// timeout=3).returncode == 0` dentro de `try/except Exception`: solo cuenta
/// el estado (la salida son bytes; no se decodifica).
async fn ssh_control_alive(opts: &NativeOptions, host: &str) -> bool {
    let program = &opts.ssh;
    let mut cmd = program.command();
    cmd.args(["-O", "check", host])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    matches!(
        tokio::time::timeout(Duration::from_secs(SSH_CHECK_SECONDS), cmd.output()).await,
        Ok(Ok(out)) if out.status.success()
    )
}

/// `ssh_state(sess)` (7714): `"ssh"` si algún pane corre `ssh`; `"mux"` si el
/// maestro de control de `ssh -O check` sigue vivo; `""` si no. Solo lee.
async fn ssh_state(opts: &NativeOptions, sess: &str) -> Result<&'static str, Fault> {
    let target = format!("={sess}");
    let panes = tmux(
        opts,
        &[
            "list-panes",
            "-s",
            "-t",
            &target,
            "-F",
            "#{pane_current_command}",
        ],
    )
    .await?;
    if panes.ok && split_ws(&panes.stdout).contains(&"ssh") {
        return Ok("ssh");
    }
    // `sess[4:]`: quien llama pasa `"ssh-" + host`.
    let host = sess.get(4..).unwrap_or("");
    if ssh_config::is_host(host) && ssh_control_alive(opts, host).await {
        return Ok("mux");
    }
    Ok("")
}

/// `scope_cmd(["tmux", "new-session", "-d", "-s", sess, "-n", "ssh", "ssh
/// <shlex.quote(host)>; exec $SHELL"])` con 15 s; `Err` con el texto del 400
/// (`r.stderr.strip() or "No se pudo crear la sesion"`) si tmux falla.
async fn new_ssh_session(
    opts: &NativeOptions,
    sess: &str,
    host: &str,
) -> Result<Result<(), String>, Fault> {
    let command = format!("ssh {}; exec $SHELL", shlex_quote(host));
    mark_effect();
    let out = run_scoped_tmux(
        opts,
        &["new-session", "-d", "-s", sess, "-n", "ssh", &command],
    )
    .await?;
    if out.ok {
        return Ok(Ok(()));
    }
    let text = py::strip(&out.stderr);
    Ok(Err(if text.is_empty() {
        "No se pudo crear la sesion".into()
    } else {
        text.to_owned()
    }))
}

/// `tmux("set-option", "-t", sess, "mouse", "off")`: sin `=`, tal cual el
/// Python.
async fn mouse_off(opts: &NativeOptions, sess: &str) -> Result<(), Fault> {
    mutate(opts, &["set-option", "-t", sess, "mouse", "off"]).await?;
    Ok(())
}

/// `400 {"error": note}` de las dos rutas cuando no hay sesión.
fn ssh_refused(note: &str) -> Answer {
    reply(StatusCode::BAD_REQUEST, &json!({"error": note}))
}

/// ¿El `stderr` (ya en minúsculas) de la prueba de llave pide contraseña?
fn asks_password(stderr: &str) -> bool {
    stderr.contains("denied") || stderr.contains("authentication")
}

/// POST `/ssh-connect` (9130, `ssh_connect` 7750): la sesión `ssh-<host>`;
/// verifica la llave antes de prometer nada.
async fn ssh_connect(native: &Native, data: &Map<String, Value>) -> Answer {
    const PATH: &str = "/ssh-connect";
    let opts = native.options();
    let host = host_of(data)?;
    if !ssh_config::is_host(&host) || !known_host(opts, &host).await? {
        return ssh_refused(UNKNOWN_HOST);
    }
    let sess = format!("ssh-{host}");
    let done = |connected: bool, note: Option<String>| {
        reply(
            StatusCode::OK,
            &json!({"ok": true, "session": sess, "connected": connected, "note": note}),
        )
    };
    write_meta(native, PATH, &sess, "ssh", &host, "").await?;
    let target = format!("={sess}");
    if tmux(opts, &["has-session", "-t", &target]).await?.ok {
        mouse_off(opts, &sess).await?;
        let state = ssh_state(opts, &sess).await?;
        if state == "ssh" {
            return done(true, None);
        }
        // El ssh del pane murió: relanzar la conexión en su pestaña.
        let pane = format!("={sess}:");
        let line = format!("ssh {host}");
        mutate(opts, &["send-keys", "-t", &pane, "-l", "--", &line]).await?;
        mutate(opts, &["send-keys", "-t", &pane, "Enter"]).await?;
        if state == "mux" {
            return done(
                true,
                Some(format!(
                    "Reconectado a {host} por el tunel vivo (sin password)"
                )),
            );
        }
        return done(
            false,
            Some("Reintentando conexion en su pestana; si pide password, tecleala ahi".into()),
        );
    }
    let (keyok, stderr) = ssh_probe(opts, &host).await;
    if let Err(note) = new_ssh_session(opts, &sess, &host).await? {
        return ssh_refused(&note);
    }
    mouse_off(opts, &sess).await?;
    if keyok {
        return done(true, None);
    }
    let note = if asks_password(&stderr) {
        format!(
            "{host} pide password: tecleala en su pestana (o corre cc-keys para no volver a hacerlo)"
        )
    } else {
        format!("{host} no responde (red caida o host apagado); la pestana quedo reintentando")
    };
    done(false, Some(note))
}

/// `ssh_new_tab_session(host)` (7791): `sshtab-<host>-<i>` (prefijo recortado
/// a 80 caracteres en total) para el primer `i` sin sesión; `""` si no hay.
async fn ssh_new_tab_session(opts: &NativeOptions, host: &str) -> Result<String, Fault> {
    let prefix = format!("sshtab-{host}");
    for i in 1..1000 {
        let suffix = format!("-{i}");
        let sess = format!(
            "{}{suffix}",
            py::take_chars(&prefix, 80usize.saturating_sub(suffix.chars().count()))
        );
        let target = format!("={sess}");
        if !tmux(opts, &["has-session", "-t", &target]).await?.ok {
            return Ok(sess);
        }
    }
    Ok(String::new())
}

/// POST `/ssh-new-tab` (9137, `ssh_open_new_tab` 7801): SIEMPRE una sesión y
/// una pestaña nuevas para un host guardado.
async fn ssh_new_tab(native: &Native, data: &Map<String, Value>) -> Answer {
    const PATH: &str = "/ssh-new-tab";
    let opts = native.options();
    let host = host_of(data)?;
    if !ssh_config::is_host(&host) || !known_host(opts, &host).await? {
        return ssh_refused(UNKNOWN_HOST);
    }
    let sess = ssh_new_tab_session(opts, &host).await?;
    if sess.is_empty() {
        return ssh_refused("No pude generar nombre de sesion SSH");
    }
    // El registro se lee antes de la prueba y de la sesión (ruling 2).
    registry_preflight(native, PATH, false).await?;
    let (keyok, stderr) = ssh_probe(opts, &host).await;
    if let Err(note) = new_ssh_session(opts, &sess, &host).await? {
        return ssh_refused(&note);
    }
    mouse_off(opts, &sess).await?;
    mark_effect();
    tab_registry::write_app_tab(native, &sess, &host)
        .await
        .map_err(|e| e.into_fault(PATH))?;
    write_meta(native, PATH, &sess, "ssh-tab", &host, "").await?;
    let note = if keyok {
        None
    } else if asks_password(&stderr) {
        Some(format!("{host} pide password en su nueva pestana"))
    } else {
        Some(format!(
            "{host} no responde o pide intervencion en su nueva pestana"
        ))
    };
    reply(
        StatusCode::OK,
        &json!({
            "ok": true,
            "session": sess,
            "label": host,
            "connected": keyok,
            "note": note,
        }),
    )
}

// ------------------------------------------- terminal rápida fuera de la barra

/// Ruta de las líneas de stderr del registro de la terminal rápida.
const QUICK_PATH: &str = "/terminal/quick";

/// Antes de cualquier efecto de POST `/terminal/quick` sin `place:"sidebar"`
/// (llama `quick.rs`): con el corte `tabs` apagado declina (la rama escribe el
/// registro, P12 del pre-flight); un archivo del registro incierto es el 500
/// del ruling 2 sin reclamo ni sesión.
pub(crate) async fn quick_register_ready(native: &Native) -> Result<(), Fault> {
    if super::cut_is_off(&native.options().cuts_off, super::Cut::Tabs) {
        return Err(Fault::Decline);
    }
    registry_preflight(native, QUICK_PATH, false).await
}

/// `quick_terminal_register(sess, label, cwd)` (5505): la pestaña `scratch` en
/// el registro y `workspace_sync(reason="user")` (un fallo de la
/// sincronización solo va a stderr). `Err` es el `str(exc)` que el `try` de
/// `open_quick_terminal` convierte en el 502 de lanzamiento.
pub(crate) async fn quick_register(
    native: &Native,
    sess: &str,
    label: &str,
    cwd: &str,
) -> Result<(), String> {
    let registered =
        tab_registry::register_app_tab(native, sess, Some(label), "scratch", "", cwd).await;
    if let Err(error) = registered {
        return Err(match error {
            // `[Errno N] <strerror>` sin el nombre del temporal de `mkstemp`
            // (aleatorio en el Python: no se reproduce).
            RegistryError::Io(error) => match error.raw_os_error() {
                Some(code) => {
                    let text = error.to_string();
                    let strerror = text
                        .strip_suffix(&format!(" (os error {code})"))
                        .unwrap_or(&text);
                    format!("[Errno {code}] {strerror}")
                }
                None => error.to_string(),
            },
            // Solo si el registro cambió tras `quick_register_ready`: el
            // Python lo leería con `load_json_file`; aquí, el fallo anotado.
            other => {
                let _ = other.into_fault(QUICK_PATH);
                "Error interno del tablero".into()
            }
        });
    }
    let hooks = native.options().hooks.clone();
    let now_seconds = (native.options().clock)() as f64 / 1000.0;
    let synced = native
        .with_state(move |b| super::workspace::sync_with_reason(b, &hooks, now_seconds, "user"))
        .await;
    match synced {
        Ok(Ok(_)) => {}
        Ok(Err(fault)) | Err(fault) => {
            let what = match fault {
                Fault::Decline => "no reproducible en el frente",
                Fault::Error(HandlerError::Timeout) => "tiempo agotado",
                Fault::Error(_) => "error interno",
            };
            eprintln!("workspace quick terminal {sess}: {what}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn shlex_quote_matches_python() {
        assert_eq!(shlex_quote(""), "''");
        assert_eq!(shlex_quote("srv-1.a_b"), "srv-1.a_b");
        assert_eq!(shlex_quote("a b"), "'a b'");
        assert_eq!(shlex_quote("it's"), "'it'\"'\"'s'");
        assert_eq!(shlex_quote("ñ"), "'ñ'");
    }

    #[test]
    fn clients_sort_by_activity_like_python() {
        assert_eq!(
            order_clients("10 /dev/pts/1\n30 a b\n\n30 c\n").ok(),
            Some(vec!["a b".to_owned(), "c".into(), "/dev/pts/1".into()])
        );
        assert!(order_clients("solo\n").is_err());
        assert!(order_clients("x y\n").is_err());
        assert_eq!(order_clients("").ok(), Some(Vec::new()));
    }

    #[test]
    fn terminal_argv_is_the_python_table() {
        assert_eq!(terminal_args("kitty", "s"), ["tmux", "attach", "-t", "=s"]);
        assert_eq!(terminal_args("tilix", "s"), ["-e", "tmux attach -t =s"]);
        assert_eq!(
            terminal_args("gnome-terminal", "s"),
            ["--", "tmux", "attach", "-t", "=s"]
        );
    }

    #[test]
    fn scope_cmd_keeps_flags_once() {
        let mut opts = NativeOptions::for_home(Path::new("/h"), PathBuf::from("/h/db"));
        opts.scope = Some(super::super::quick::scope_program("/usr/bin/systemd-run"));
        let program = scope_cmd(&opts, &opts.tmux.program).unwrap();
        let prefix: Vec<&str> = program.prefix.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(
            prefix,
            ["--user", "--scope", "--collect", "--quiet", "tmux"]
        );
        opts.scope = None;
        assert!(scope_cmd(&opts, &opts.tmux.program).is_none());
    }
}
