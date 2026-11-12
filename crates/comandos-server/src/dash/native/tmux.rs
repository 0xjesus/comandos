//! Procesos externos (`tmux`, `fc-list`) como `subprocess.run(..., text=True,
//! capture_output=True, timeout=…)` del Python: stdout/stderr decodificados
//! como UTF-8 estricto con saltos universales; al vencer el plazo se mata.
use super::py::repr_ascii;
use crate::HandlerError;
use std::{
    ffi::OsString,
    io,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct Program {
    pub path: PathBuf,
    /// Argumentos que van antes de los del llamador (dobles de prueba).
    pub prefix: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub env_remove: Vec<OsString>,
    /// `env_clear` antes de `env`: el hijo solo ve `env` (pruebas confinadas;
    /// en producción siempre `false`, el entorno del proceso como el Python).
    pub env_clear: bool,
}

/// Variables del frente que ningún hijo hereda: el ajuste de malloc del
/// drop-in de `cc-dash` es solo del frente. Un `tmux` que arranca el servidor
/// (o `systemd-run --scope … tmux new-session`) se lo pasaría a cada pane,
/// agente y terminal.
pub const CHILD_ENV_REMOVE: &[&str] = &[comandos_core::malloc_tuning::GLIBC_TUNABLES_ENV];

impl Program {
    /// Hereda el entorno del proceso, como `subprocess.run`.
    pub fn named(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            prefix: Vec::new(),
            env: Vec::new(),
            env_remove: Vec::new(),
            env_clear: false,
        }
    }

    /// La orden con el prefijo y el entorno del programa, sin las variables de
    /// `CHILD_ENV_REMOVE`. Todo hijo del frente sale de aquí.
    pub fn command(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.path);
        cmd.args(&self.prefix);
        if self.env_clear {
            cmd.env_clear();
        }
        for name in CHILD_ENV_REMOVE {
            cmd.env_remove(name);
        }
        for name in &self.env_remove {
            cmd.env_remove(name);
        }
        for (name, value) in &self.env {
            cmd.env(name, value);
        }
        cmd
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// `returncode == 0`.
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug)]
pub enum RunError {
    Timeout,
    Spawn(io::Error),
    Decode,
}

/// `text=True`: UTF-8 estricto con saltos universales.
pub(crate) fn universal(bytes: Vec<u8>) -> Result<String, RunError> {
    let text = String::from_utf8(bytes).map_err(|_| RunError::Decode)?;
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

pub async fn run_program(
    program: &Program,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, RunError> {
    run(program, args, None, timeout).await
}

/// `run_program` con `cwd=…`: un directorio que no existe (o no se puede
/// abrir) da `Spawn`, como el `FileNotFoundError` de `subprocess.run`.
pub async fn run_program_in(
    program: &Program,
    args: &[&str],
    cwd: &Path,
    timeout: Duration,
) -> Result<Output, RunError> {
    run(program, args, Some(cwd), timeout).await
}

async fn run(
    program: &Program,
    args: &[&str],
    cwd: Option<&Path>,
    timeout: Duration,
) -> Result<Output, RunError> {
    let raw = run_raw(program, args, cwd, timeout).await?;
    Ok(Output {
        ok: raw.ok,
        stdout: universal(raw.stdout)?,
        stderr: universal(raw.stderr)?,
    })
}

/// La salida sin decodificar: `run_many` la parte antes de decodificar.
struct Raw {
    ok: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

async fn run_raw(
    program: &Program,
    args: &[&str],
    cwd: Option<&Path>,
    timeout: Duration,
) -> Result<Raw, RunError> {
    let mut cmd = program.command();
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(RunError::Spawn)?;
    // Al vencer, el futuro se suelta con el hijo dentro y kill_on_drop lo mata.
    let out = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| RunError::Timeout)?
        .map_err(RunError::Spawn)?;
    Ok(Raw {
        ok: out.status.success(),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// Tope de bytes de argumentos por invocación de `run_many`: un mensaje de
/// tmux (imsg) admite 16 KiB y lleva además el entorno de trabajo.
const BATCH_ARG_BYTES: usize = 6000;

/// Línea que separa las salidas de un lote; aleatoria para que ningún pane
/// la contenga.
fn batch_marker() -> Option<String> {
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).ok()?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Some(format!("comandos-lote-{hex}"))
}

/// Las salidas de las órdenes que terminaron, en orden: lo que hay antes de
/// cada línea de marca. Lo que sigue a la última marca es de una orden que no
/// llegó a terminar y se descarta.
fn split_batch(stdout: &[u8], marker: &str) -> Vec<Vec<u8>> {
    let line = format!("{marker}\n");
    let line = line.as_bytes();
    let mut done = Vec::new();
    let mut rest = stdout;
    while let Some(at) = rest.windows(line.len()).position(|w| w == line) {
        done.push(rest[..at].to_vec());
        rest = &rest[at + line.len()..];
    }
    done
}

/// Ruta del socket que tmux usaría con `TMUX_TMPDIR=socket_dir`
/// (`<socket_dir>/tmux-<uid>/default`), para pasarla explícita con `-S`.
pub fn private_socket(socket_dir: &Path) -> PathBuf {
    socket_dir
        .join(format!("tmux-{}", nix::unistd::getuid().as_raw()))
        .join("default")
}

#[derive(Debug, Clone)]
pub struct Tmux {
    pub program: Program,
    pub timeout: Duration,
}

#[derive(Debug)]
pub enum TmuxError {
    Timeout {
        args: Vec<String>,
        after: Duration,
    },
    /// No arrancó (o falló su E/S): la clase y el `errno`, si lo hay.
    Spawn(io::ErrorKind, Option<i32>),
    Decode,
}

impl Tmux {
    /// Producción: `tmux` del PATH con el entorno del proceso (`TMUX_TMPDIR`,
    /// `TMUX`), igual que el Python. Plazo de `tmux()` (5715): 5 s.
    pub fn system() -> Self {
        Self {
            program: Program::named("tmux"),
            timeout: Duration::from_secs(5),
        }
    }

    /// Servidor privado de pruebas: nunca el del usuario. `-f /dev/null`: si esta
    /// llamada arranca el servidor, nace sin `~/.tmux.conf` (que lee
    /// `~/.claude/hooks` y corre `cc-status.sh` en la barra de estado).
    ///
    /// El socket va explícito con `-S`, no solo por `TMUX_TMPDIR`: tmux 3.2a
    /// ignora en silencio un `TMUX_TMPDIR` cuyo directorio no existe y cae en
    /// `/tmp/tmux-<uid>/default`, el servidor real del usuario. Con `-S` un
    /// directorio borrado da «no server running», nunca un `kill-server` ajeno.
    pub fn private(socket_dir: &Path) -> Self {
        let socket = private_socket(socket_dir);
        if let Some(parent) = socket.parent() {
            // 0700 como lo crea tmux: con bits de «otros» rechaza el directorio.
            let _ = std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent);
        }
        let mut program = Program::named("tmux");
        program.prefix = vec![
            "-f".into(),
            "/dev/null".into(),
            "-S".into(),
            socket.into_os_string(),
        ];
        program.env.push(("TMUX_TMPDIR".into(), socket_dir.into()));
        program.env_remove.push("TMUX".into());
        Self {
            program,
            timeout: Duration::from_secs(5),
        }
    }

    pub async fn run(&self, args: &[&str]) -> Result<Output, TmuxError> {
        run_program(&self.program, args, self.timeout)
            .await
            .map_err(|error| match error {
                RunError::Timeout => TmuxError::Timeout {
                    args: args.iter().map(|a| (*a).to_owned()).collect(),
                    after: self.timeout,
                },
                RunError::Spawn(e) => TmuxError::Spawn(e.kind(), e.raw_os_error()),
                RunError::Decode => TmuxError::Decode,
            })
    }

    /// Varias órdenes de solo lectura con las mínimas invocaciones de tmux:
    /// cada cliente nuevo le cuesta al servidor (un solo hilo, el mismo que
    /// alimenta las terminales) mucho más que la orden en sí. `Some` es la
    /// salida de una orden que terminó bien; `None`, una que falló, que no
    /// entró en un lote o cuya salida no se pudo separar: quien llama la
    /// repite con `run` para obtener su resultado real. Ningún argumento
    /// puede terminar en `;` (tmux lo leería como separador).
    pub async fn run_many(&self, commands: &[Vec<String>]) -> Result<Vec<Option<Output>>, TmuxError> {
        let mut found = vec![None; commands.len()];
        let Some(marker) = batch_marker() else {
            return Ok(found);
        };
        let order: Vec<usize> = (0..commands.len())
            .filter(|&i| !commands[i].is_empty() && commands[i].iter().all(|a| !a.ends_with(';')))
            .collect();
        let mut next = 0;
        while next < order.len() {
            let mut args: Vec<&str> = Vec::new();
            let (mut bytes, mut end) = (0, next);
            while end < order.len() {
                let command = &commands[order[end]];
                let size = command.iter().map(|a| a.len() + 1).sum::<usize>() + marker.len() + 24;
                if end > next && bytes + size > BATCH_ARG_BYTES {
                    break;
                }
                if end > next {
                    args.push(";");
                }
                args.extend(command.iter().map(String::as_str));
                args.extend([";", "display-message", "-p", marker.as_str()]);
                bytes += size;
                end += 1;
            }
            let raw = run_raw(&self.program, &args, None, self.timeout)
                .await
                .map_err(|error| match error {
                    RunError::Timeout => TmuxError::Timeout {
                        args: args.iter().map(|a| (*a).to_owned()).collect(),
                        after: self.timeout,
                    },
                    RunError::Spawn(e) => TmuxError::Spawn(e.kind(), e.raw_os_error()),
                    RunError::Decode => TmuxError::Decode,
                })?;
            let done = split_batch(&raw.stdout, &marker);
            let sent = end - next;
            if done.len() > sent {
                // Un pane mostraba la marca: nada de este lote es fiable.
                next = end;
                continue;
            }
            let finished = done.len();
            for (i, stdout) in done.into_iter().enumerate() {
                found[order[next + i]] = universal(stdout).ok().map(|stdout| Output {
                    ok: true,
                    stdout,
                    stderr: String::new(),
                });
            }
            // tmux corta la lista en la primera orden que falla: esa queda
            // sin salida y se sigue con las de después.
            next += if finished == sent { sent } else { finished + 1 };
        }
        Ok(found)
    }

    /// `run` desde un hilo de bloqueo (librerías síncronas de `comandos-runtime`).
    /// El reactor, los timers y el reaper los mueve el hilo del runtime, que
    /// está en `Runtime::block_on`; aquí solo se espera el resultado.
    pub fn run_blocking(
        &self,
        handle: &tokio::runtime::Handle,
        args: &[&str],
    ) -> Result<Output, TmuxError> {
        handle.block_on(self.run(args))
    }
}

fn seconds(after: Duration) -> String {
    if after.subsec_nanos() == 0 {
        after.as_secs().to_string()
    } else {
        format!("{}", after.as_secs_f64())
    }
}

impl TmuxError {
    /// `str(exc)` del Python cuando la ruta captura la excepción
    /// (`get_tmux_mouse`/`set_tmux_mouse`). `None`: no hay texto seguro → declinar.
    pub fn python_message(&self) -> Option<String> {
        match self {
            TmuxError::Timeout { args, after } => {
                let mut parts = vec![repr_ascii("tmux")?];
                for arg in args {
                    parts.push(repr_ascii(arg)?);
                }
                Some(format!(
                    "Command '[{}]' timed out after {} seconds",
                    parts.join(", "),
                    seconds(*after)
                ))
            }
            TmuxError::Spawn(io::ErrorKind::NotFound, _) => {
                Some("[Errno 2] No such file or directory: 'tmux'".into())
            }
            TmuxError::Spawn(..) | TmuxError::Decode => None,
        }
    }

    /// Ramas donde el Python NO captura: `TimeoutExpired` → 504, el resto → 500.
    pub fn uncaught(&self) -> HandlerError {
        match self {
            TmuxError::Timeout { .. } => HandlerError::Timeout,
            _ => HandlerError::Failure,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_splits_at_each_marker_and_drops_the_unfinished_tail() {
        let out = b"uno\nM\nM\ndos\nmas\nM\na medias";
        assert_eq!(
            split_batch(out, "M"),
            vec![b"uno\n".to_vec(), b"".to_vec(), b"dos\nmas\n".to_vec()]
        );
        assert!(split_batch(b"can't find pane", "M").is_empty());
    }

    #[test]
    fn every_child_drops_the_front_malloc_tuning() {
        let mut program = Program::named("tmux");
        program.env.push(("TMUX_TMPDIR".into(), "/x".into()));
        let cmd = program.command();
        let envs: Vec<_> = cmd.as_std().get_envs().collect();
        assert!(
            envs.contains(&(std::ffi::OsStr::new("GLIBC_TUNABLES"), None)),
            "{envs:?}"
        );
        assert!(envs.contains(&(
            std::ffi::OsStr::new("TMUX_TMPDIR"),
            Some(std::ffi::OsStr::new("/x"))
        )));
    }
}
