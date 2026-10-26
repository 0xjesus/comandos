//! Toda llamada a tmux de la app pasa por aquí, siempre con `-S <socket>`
//! explícito: nunca se cae por descuido en el servidor `default` del usuario.
//! El modo decide qué se puede hacer: la sombra solo lee.
use crate::config::{AppConfig, RunMode, TmuxServer};
use crate::guard::{GuardError, WriteGuard};
use crate::proc::{ProcSpec, run_when};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `systemd-run` arguments that start the tmux server on `socket` inside the transient
/// scope `unit`: the server outlives the app, and an empty server does not exit (as
/// `tmux.service` does). The scope stays while the server lives.
pub fn detached_server_argv(socket: &Path, unit: &str) -> Vec<OsString> {
    let mut argv: Vec<OsString> = ["--user", "--scope", "--collect", "--quiet", "-p", "ManagedOOMPreference=avoid"]
        .iter()
        .map(OsString::from)
        .collect();
    argv.push(format!("--unit={unit}").into());
    argv.extend(["tmux".into(), "-S".into(), socket.as_os_str().to_owned()]);
    argv.extend(["start-server", ";", "set-option", "-s", "exit-empty", "off"].iter().map(OsString::from));
    argv
}

/// Plazo de `tmuxc` (`bin/cc-app:414`).
pub const TMUX_TIMEOUT: Duration = Duration::from_secs(5);

/// Verbos que no cambian nada (con las reglas de `check_read_args`).
pub const READ_VERBS: &[&str] = &[
    "has-session",
    "list-sessions",
    "list-windows",
    "list-panes",
    "list-clients",
    "list-buffers",
    "display-message",
    "show-options",
    "show-buffer",
    "save-buffer",
    "capture-pane",
    "show-environment",
];

/// Verbos que la app usa para cambiar tmux (solo fuera de la sombra). Ni
/// `kill-server` ni `kill-session`: el único borrado es `kill_owned_session`.
pub const MUTATE_VERBS: &[&str] = &[
    "new-session",
    "new-window",
    "split-window",
    "send-keys",
    "select-pane",
    "select-window",
    "select-layout",
    "resize-pane",
    "resize-window",
    "set-option",
    "load-buffer",
    "paste-buffer",
    "delete-buffer",
    "respawn-pane",
    "move-window",
];

/// Shells cuya presencia exclusiva hace «ociosa» una sesión `term-*` (`close_tab`, 3386).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxError {
    ShadowRefused(String),
    Forbidden(String),
    BadArgs(String),
    Spawn(String),
    Timeout(String),
}

/// Como `subprocess.CompletedProcess` del Python.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxOut {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl TmuxOut {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

/// Prueba de que una sesión se puede borrar. Solo la crean `idle_scratch` y
/// `new_placeholder_session`; no se puede construir desde fuera.
#[derive(Debug)]
pub struct OwnedSession {
    name: String,
    socket: PathBuf,
    identity: String,
    idle: bool,
}

#[derive(Debug, Clone)]
pub struct TmuxCtl {
    mode: RunMode,
    socket: PathBuf,
    sandbox_home: Option<PathBuf>,
}

fn valid_session_name(name: &str) -> bool {
    // SESSION_RE de bin/cc-app:3572 (el mismo que cc-dash).
    !name.is_empty()
        && name.len() <= 80
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn valid_target(t: &str) -> bool {
    let id = |p: char| {
        t.strip_prefix(p)
            .is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
    };
    if id('%') || id('@') || id('$') {
        return true;
    }
    let body = t.strip_prefix('=').unwrap_or(t);
    let (sess, rest) = match body.split_once(':') {
        Some((s, r)) => (s, Some(r)),
        None => (body, None),
    };
    valid_session_name(sess)
        && rest.is_none_or(|r| {
            r.is_empty()
                || r == "^"
                || r.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        })
}

/// Reglas de lectura (F07, F09): sin `#(`, `-p` obligatorio donde sin él se
/// muta, `save-buffer` solo a stdout, objetivos `-t` válidos.
pub fn check_read_args(args: &[&str]) -> Result<(), TmuxError> {
    let verb = *args
        .first()
        .ok_or_else(|| TmuxError::BadArgs("sin verbo".into()))?;
    if !READ_VERBS.contains(&verb) {
        return Err(TmuxError::Forbidden(verb.to_string()));
    }
    if let Some(bad) = args.iter().find(|a| a.contains("#(")) {
        return Err(TmuxError::BadArgs(format!(
            "formato que ejecuta comandos: {bad}"
        )));
    }
    reject_command_chain(args)?;
    if matches!(verb, "display-message" | "capture-pane") {
        let mut printed = false;
        let mut it = args.iter().skip(1);
        while let Some(arg) = it.next() {
            match *arg {
                "-p" => printed = true,
                "-t" | "-c" | "-F" | "-b" | "-S" | "-E" => {
                    it.next()
                        .ok_or_else(|| TmuxError::BadArgs(format!("{arg} sin valor")))?;
                }
                "-a" | "-v" | "-l" | "-C" | "-e" | "-J" | "-N" | "-P" | "-q" => {}
                other if other.starts_with('-') => {
                    return Err(TmuxError::BadArgs(format!(
                        "opción de lectura desconocida: {other}"
                    )));
                }
                _ => break,
            }
        }
        if !printed {
            return Err(TmuxError::BadArgs(format!("{verb} sin -p")));
        }
    }
    if verb == "save-buffer" && args.last() != Some(&"-") {
        return Err(TmuxError::BadArgs("save-buffer solo a stdout (-)".into()));
    }
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if *a == "-t" {
            let target = it
                .next()
                .ok_or_else(|| TmuxError::BadArgs("-t sin valor".into()))?;
            if !valid_target(target) {
                return Err(TmuxError::BadArgs(format!("objetivo no válido: {target}")));
            }
        }
    }
    Ok(())
}

// tmux interpreta separadores y llaves como listas de comandos incluso sin shell.
fn reject_command_chain(args: &[&str]) -> Result<(), TmuxError> {
    if args
        .iter()
        .any(|a| a.contains(';') || matches!(*a, "{" | "}"))
    {
        return Err(TmuxError::BadArgs("lista de comandos tmux".into()));
    }
    Ok(())
}

/// `TMUX_TMPDIR` como lo usa tmux 3.2a: si no es un directorio existente, `/tmp`.
fn tmux_tmpdir(env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    env("TMUX_TMPDIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

impl TmuxCtl {
    /// Único constructor: lee el modo y el servidor de la configuración.
    pub fn from_config(
        cfg: &AppConfig,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<TmuxCtl, TmuxError> {
        let uid = nix::unistd::getuid().as_raw();
        let user_dir = tmux_tmpdir(env).join(format!("tmux-{uid}"));
        let socket = match (cfg.mode(), cfg.tmux_server(), cfg.sandbox_root()) {
            (RunMode::Sandbox, TmuxServer::Private(label), Some(root)) => {
                root.join("tmux").join(label.as_str())
            }
            (RunMode::Sandbox, _, _) => {
                return Err(TmuxError::Forbidden("sandbox sin socket propio".into()));
            }
            (_, TmuxServer::User, _) => user_dir.join("default"),
            (_, TmuxServer::Private(label), _) => user_dir.join(label.as_str()),
        };
        if cfg.mode() == RunMode::Sandbox
            && (socket.starts_with(&user_dir) || socket.starts_with(format!("/tmp/tmux-{uid}")))
        {
            return Err(TmuxError::Forbidden(format!(
                "el socket del sandbox no puede vivir con los del usuario: {}",
                socket.display()
            )));
        }
        Ok(TmuxCtl {
            mode: cfg.mode(),
            socket,
            sandbox_home: cfg.sandbox_root().map(|root| root.join("home")),
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    pub fn mode(&self) -> RunMode {
        self.mode
    }

    /// Crea `<raíz>/tmux` (0700, como tmux exige) en sandbox; en otro modo no hace nada.
    pub fn prepare_socket_dir(&self, guard: &WriteGuard) -> Result<(), GuardError> {
        match (self.mode, self.socket.parent()) {
            (RunMode::Sandbox, Some(dir)) => {
                guard.create_dir_all(dir, 0o700)?;
                if let Some(home) = &self.sandbox_home {
                    guard.create_dir_all(home, 0o700)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn exec(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<TmuxOut, TmuxError> {
        self.exec_when(args, stdin, TMUX_TIMEOUT, &|| true)
    }

    /// Before a `new-session`: when no server is running, start one in a systemd unit
    /// of its own. A server born from this app lives in the app's cgroup, and closing
    /// the app kills that cgroup with every session in it (2026-09-04 and 2026-10-26).
    /// If this fails, `new-session` still starts the server the old way.
    fn ensure_server(&self) {
        if self.mode != RunMode::Live || !cfg!(target_os = "linux") {
            return;
        }
        if self.exec(&["list-sessions", "-F", "#{session_id}"], None).is_ok_and(|o| o.ok()) {
            return;
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let spec = ProcSpec {
            program: "systemd-run".into(),
            args: detached_server_argv(&self.socket, &format!("comandos-tmux-{stamp}.scope")),
            stdin: None,
            env: Vec::new(),
            clear_env: false,
            env_remove: vec!["TMUX".into(), "TMUX_PANE".into()],
            cwd: None,
            timeout: TMUX_TIMEOUT,
        };
        if let Err(error) = run_when(&spec, &|| true) {
            eprintln!("comandos-app: tmux en su propia unidad: {error:?}");
        }
    }

    fn exec_when(
        &self,
        args: &[&str],
        stdin: Option<&[u8]>,
        timeout: Duration,
        allowed: &dyn Fn() -> bool,
    ) -> Result<TmuxOut, TmuxError> {
        if !allowed() {
            return Err(TmuxError::Forbidden("cancelled".into()));
        }
        let mut argv: Vec<OsString> = vec!["-S".into(), self.socket.clone().into_os_string()];
        if self.mode == RunMode::Sandbox {
            // The private environment has no inherited locale. Force tmux's
            // UTF-8 client output so Unicode paths remain usable by consumers.
            argv.extend(["-u".into(), "-f".into(), "/dev/null".into()]);
        }
        argv.extend(args.iter().map(OsString::from));
        let spec = ProcSpec {
            program: "tmux".into(),
            args: argv,
            stdin: stdin.map(<[u8]>::to_vec),
            env: self.sandbox_home.as_ref().map_or_else(Vec::new, |home| {
                vec![
                    ("HOME".into(), home.clone().into_os_string()),
                    ("PATH".into(), "/usr/bin:/bin".into()),
                    ("SHELL".into(), "/bin/sh".into()),
                    ("TERM".into(), "xterm-256color".into()),
                    (
                        "XDG_CONFIG_HOME".into(),
                        home.join(".config").into_os_string(),
                    ),
                ]
            }),
            clear_env: self.mode == RunMode::Sandbox,
            env_remove: vec!["TMUX".into(), "TMUX_PANE".into()],
            cwd: None,
            timeout,
        };
        let out = run_when(&spec, allowed).map_err(|e| TmuxError::Spawn(format!("{e:?}")))?;
        if out.truncated {
            return Err(TmuxError::Spawn("tmux output exceeded limit".into()));
        }
        if out.timed_out {
            return Err(TmuxError::Timeout(args.join(" ")));
        }
        Ok(TmuxOut {
            code: out.code.unwrap_or(1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    pub fn read(&self, args: &[&str]) -> Result<TmuxOut, TmuxError> {
        check_read_args(args)?;
        self.exec(args, None)
    }

    /// Group individually validated read commands into one process, never shell input.
    pub(crate) fn read_batch_when(
        &self,
        commands: &[Vec<String>],
        timeout: Duration,
        allowed: &dyn Fn() -> bool,
    ) -> Result<TmuxOut, TmuxError> {
        if commands.is_empty() {
            return Err(TmuxError::BadArgs("empty read batch".into()));
        }
        let mut args = Vec::new();
        for command in commands {
            let refs: Vec<_> = command.iter().map(String::as_str).collect();
            check_read_args(&refs)?;
            if !args.is_empty() {
                args.push(";");
            }
            args.extend(refs);
        }
        self.exec_when(&args, None, timeout, allowed)
    }

    fn check_mutate(&self, args: &[&str]) -> Result<(), TmuxError> {
        let verb = *args
            .first()
            .ok_or_else(|| TmuxError::BadArgs("sin verbo".into()))?;
        if !MUTATE_VERBS.contains(&verb) {
            return Err(TmuxError::Forbidden(verb.to_string()));
        }
        if self.mode == RunMode::Shadow {
            return Err(TmuxError::ShadowRefused(verb.to_string()));
        }
        reject_command_chain(args)?;
        if let Some(bad) = args.iter().find(|a| a.contains("#(")) {
            return Err(TmuxError::BadArgs(format!(
                "formato que ejecuta comandos: {bad}"
            )));
        }
        Ok(())
    }

    pub fn mutate(&self, args: &[&str]) -> Result<TmuxOut, TmuxError> {
        self.check_mutate(args)?;
        if args.first() == Some(&"new-session") {
            self.ensure_server();
        }
        self.exec(args, None)
    }

    /// `load-buffer -` con el texto por stdin (`snip_paste`, F38).
    pub fn mutate_with_stdin(&self, args: &[&str], stdin: &[u8]) -> Result<TmuxOut, TmuxError> {
        self.check_mutate(args)?;
        self.exec(args, Some(stdin))
    }

    /// `Some` solo si `session` es `term-*` y todos sus panes son shells ociosos.
    pub fn idle_scratch(&self, session: &str) -> Result<Option<OwnedSession>, TmuxError> {
        if !session.starts_with("term-") || !valid_session_name(session) {
            return Ok(None);
        }
        let target = format!("={session}");
        let out = self.read(&[
            "list-panes",
            "-s",
            "-t",
            &target,
            "-F",
            "#{pane_current_command}",
        ])?;
        if crate::tab_actions::idle_scratch_commands(session, out.ok(), &out.stdout) {
            self.owned(session, true).map(Some)
        } else {
            Ok(None)
        }
    }

    fn owned(&self, name: &str, idle: bool) -> Result<OwnedSession, TmuxError> {
        let target = format!("={name}:");
        let out = self.read(&[
            "display-message",
            "-p",
            "-t",
            &target,
            "#{pid}:#{session_id}",
        ])?;
        if !out.ok() || out.stdout.trim().is_empty() {
            return Err(TmuxError::BadArgs("la sesión ya no existe".into()));
        }
        Ok(OwnedSession {
            name: name.into(),
            socket: self.socket.clone(),
            identity: out.stdout.trim().into(),
            idle,
        })
    }

    /// `new-session` de la restauración: devuelve la prueba de propiedad si tmux
    /// la creó (el nombre sale del `-s` de `args`).
    pub fn new_placeholder_session(
        &self,
        args: &[&str],
    ) -> Result<(TmuxOut, Option<OwnedSession>), TmuxError> {
        if args.first() != Some(&"new-session") {
            return Err(TmuxError::BadArgs(
                "new_placeholder_session solo crea sesiones".into(),
            ));
        }
        self.check_mutate(args)?;
        // -P/-F internos capturan identidad en la misma creación. Nunca buscar
        // por nombre después: un hook puede reemplazar la sesión antes de volver.
        let mut options = args.iter().skip(1);
        let mut session = None;
        let mut caller_print = false;
        let mut internal_format_added = false;
        let mut caller_format = "#{session_name}:";
        let mut argv: Vec<String> = vec!["new-session".into()];
        while let Some(arg) = options.next() {
            match *arg {
                "-P" => caller_print = true,
                "-F" => {
                    caller_format = options
                        .next()
                        .ok_or_else(|| TmuxError::BadArgs("-F sin valor".into()))?
                }
                "-d" | "-E" => argv.push((*arg).into()),
                "-s" => {
                    let name = options
                        .next()
                        .filter(|n| valid_session_name(n))
                        .ok_or_else(|| TmuxError::BadArgs("new-session sin -s válido".into()))?;
                    if session.replace((*name).to_string()).is_some() {
                        return Err(TmuxError::BadArgs("-s duplicado".into()));
                    }
                    argv.extend([(*arg).to_string(), (*name).to_string()]);
                }
                "-x" | "-y" | "-n" | "-c" | "-e" => {
                    let value = options
                        .next()
                        .ok_or_else(|| TmuxError::BadArgs(format!("{arg} sin valor")))?;
                    argv.extend([(*arg).to_string(), (*value).to_string()]);
                }
                other if other.starts_with('-') => {
                    return Err(TmuxError::BadArgs(format!(
                        "opción no permitida de placeholder: {other}"
                    )));
                }
                _ => {
                    // Las opciones internas van antes del programa y sus argumentos.
                    argv.extend([
                        "-P".into(),
                        "-F".into(),
                        format!("#{{pid}}:#{{session_id}}|{caller_format}"),
                    ]);
                    internal_format_added = true;
                    argv.push((*arg).into());
                    argv.extend(options.map(|value| (*value).to_string()));
                    break;
                }
            }
        }
        if !internal_format_added {
            argv.extend([
                "-P".into(),
                "-F".into(),
                format!("#{{pid}}:#{{session_id}}|{caller_format}"),
            ]);
        }
        let name = session.ok_or_else(|| TmuxError::BadArgs("new-session sin -s válido".into()))?;
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.ensure_server();
        let mut out = self.exec(&refs, None)?;
        let owned = if out.ok() {
            let (identity, requested) = out.stdout.split_once('|').ok_or_else(|| {
                TmuxError::BadArgs(format!(
                    "new-session no entregó identidad: {:?}",
                    out.stdout
                ))
            })?;
            let valid = identity.split_once(':').is_some_and(|(pid, id)| {
                !pid.is_empty()
                    && pid.bytes().all(|b| b.is_ascii_digit())
                    && id.starts_with('$')
                    && valid_target(id)
            });
            if !valid {
                return Err(TmuxError::BadArgs("identidad de creación inválida".into()));
            }
            let token = OwnedSession {
                name,
                socket: self.socket.clone(),
                identity: identity.into(),
                idle: false,
            };
            out.stdout = if caller_print {
                requested.to_string()
            } else {
                String::new()
            };
            Some(token)
        } else {
            None
        };
        Ok((out, owned))
    }

    pub fn kill_owned_session(&self, owned: OwnedSession) -> Result<TmuxOut, TmuxError> {
        if self.mode == RunMode::Shadow {
            return Err(TmuxError::ShadowRefused("kill-session".into()));
        }
        if owned.socket != self.socket {
            return Err(TmuxError::Forbidden("token de otro servidor".into()));
        }
        let current = self.owned(&owned.name, owned.idle)?;
        if current.identity != owned.identity {
            return Err(TmuxError::Forbidden(
                "sesión reemplazada desde la prueba de propiedad".into(),
            ));
        }
        if owned.idle && self.idle_scratch(&owned.name)?.is_none() {
            return Err(TmuxError::Forbidden(
                "la sesión dejó de estar ociosa".into(),
            ));
        }
        let target = owned
            .identity
            .split_once(':')
            .map(|(_, id)| id)
            .filter(|id| valid_target(id))
            .ok_or_else(|| TmuxError::BadArgs("identidad de sesión inválida".into()))?;
        self.exec(&["kill-session", "-t", target], None)
    }

    pub fn check_owned_session(&self, owned: &OwnedSession) -> Result<(), TmuxError> {
        if owned.socket != self.socket {
            return Err(TmuxError::Forbidden("token de otro servidor".into()));
        }
        if self.owned(&owned.name, owned.idle)?.identity != owned.identity {
            return Err(TmuxError::Forbidden(
                "sesión reemplazada desde la prueba de propiedad".into(),
            ));
        }
        Ok(())
    }

    /// Native interactive attach is unavailable in Shadow: even ignore-size
    /// changes a window when it is the only client. Shadow uses captures.
    pub fn attach_argv(&self, session: &str) -> Vec<String> {
        self.attach_with(session, "")
    }

    /// For overview tiles: the client never sizes the window (`ignore-size`), so a
    /// small tile shows part of the window instead of shrinking it for every client.
    pub fn viewer_attach_argv(&self, session: &str) -> Vec<String> {
        self.attach_with(session, "-f ignore-size ")
    }

    fn attach_with(&self, session: &str, flags: &str) -> Vec<String> {
        if self.mode == RunMode::Shadow {
            return Vec::new();
        }
        let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        let sandbox_prefix = self.sandbox_home.as_ref().map_or_else(String::new, |home| format!(
            "env -i HOME={} PATH=/usr/bin:/bin SHELL=/bin/sh TERM=xterm-256color XDG_CONFIG_HOME={} ",
            quote(&home.display().to_string()), quote(&home.join(".config").display().to_string())
        ));
        let isolated_config = if self.mode == RunMode::Sandbox {
            "-u -f /dev/null "
        } else {
            ""
        };
        let attach = format!(
            "{sandbox_prefix}tmux -S {} {isolated_config}attach {flags}-t {} || {{ echo '[sesion terminada — cierra esta pestana con la x]'; exec cat; }}",
            quote(&self.socket.display().to_string()),
            quote(&format!("={session}")),
        );
        vec!["/bin/sh".into(), "-c".into(), attach]
    }

    /// `_tmux_window_size` (749): tamaño que la sesión ya tiene, si es razonable.
    pub fn window_size(&self, session: &str) -> Option<(u16, u16)> {
        let target = format!("={session}:");
        let out = self
            .read(&[
                "display-message",
                "-p",
                "-t",
                &target,
                "#{window_width} #{window_height}",
            ])
            .ok()?;
        let mut it = out.stdout.split_whitespace().map(str::parse::<u16>);
        let (cols, rows) = (it.next()?.ok()?, it.next()?.ok()?);
        (cols >= 20 && rows >= 5).then_some((cols, rows))
    }
}
