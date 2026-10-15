//! Las teclas y señales con las que una operación de sesión cierra el agente
//! de un pane y teclea el siguiente (`bin/cc-dash`: `_pane_exit_current`
//! 2075, `_stop_owned_startup` 2697, `_restore_shell_tty` 1846,
//! `_send_shell_line`, `_send_configuration_command` 2691 y
//! `capture_handoff` 1926).
//!
//! Solo se señala el pid anotado y solo si su inicio (`/proc/<pid>/stat`
//! campo 22) sigue siendo el esperado; nunca un pid `<= 0` (el Python
//! llamaría `os.kill(0|-n, …)`, que alcanza grupos de procesos: aquí se
//! responde «no cerró»), nunca por patrón. Los errores son el `str(exc)` del
//! Python.
use crate::{
    agent_procs,
    session_configuration::{Env, Fail},
};
use comandos_core::text::{is_space, splitlines, strip};
use nix::{errno::Errno, sys::signal, unistd::Pid};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn sleep(env: &Env, seconds: f64) {
    (env.sleep)(Duration::from_secs_f64(seconds));
}

fn tmux(env: &Env, args: &[&str]) -> Result<crate::pane_typing::TmuxResult, String> {
    (env.tmux)(args)
}

fn start(env: &Env, pid: i64) -> String {
    agent_procs::process_start(&env.proc_root, pid)
}

/// `os.kill(int(pid), sig)`: `Ok` si se entregó; el `Errno` si no.
fn kill(pid: i64, sig: Option<signal::Signal>) -> Result<(), KillError> {
    if pid <= 0 {
        return Err(KillError::Refused);
    }
    let raw = i32::try_from(pid).map_err(|_| KillError::Overflow)?;
    signal::kill(Pid::from_raw(raw), sig).map_err(KillError::Errno)
}

enum KillError {
    Errno(Errno),
    /// `OverflowError` de `os.kill` con un pid fuera de `int`.
    Overflow,
    /// Pid `<= 0`: no se señala.
    Refused,
}

impl KillError {
    fn text(&self) -> String {
        match self {
            KillError::Errno(errno) => format!("[Errno {}] {}", *errno as i32, errno.desc()),
            KillError::Overflow => "signed integer is greater than maximum".to_owned(),
            KillError::Refused => String::new(),
        }
    }
}

/// `_pane_exit_current(pane, pid, from_harness, expected_start)` (2075):
/// sale del CLI actual y espera a que muera. `/exit` (Claude y ACP) o
/// Ctrl-C doble; en última instancia `SIGTERM` solo al pid anotado con el
/// mismo inicio.
pub fn exit_current(
    env: &Env,
    pane: &str,
    pid: i64,
    from_harness: &str,
    expected_start: Option<&str>,
) -> Result<bool, String> {
    if matches!(from_harness, "" | "shell") {
        return Ok(true);
    }
    if pid <= 0 {
        return Ok(false);
    }
    let started = start(env, pid);
    if expected_start.is_some_and(|e| e != started) {
        return Ok(false);
    }
    if started.is_empty() {
        // Solo `ProcessLookupError` se captura.
        return match kill(pid, None) {
            Ok(()) => Ok(false),
            Err(KillError::Errno(Errno::ESRCH)) => Ok(true),
            Err(other) => Err(other.text()),
        };
    }
    tmux(env, &["send-keys", "-t", pane, "Escape"])?;
    sleep(env, 0.12);
    tmux(env, &["send-keys", "-t", pane, "C-u"])?;
    sleep(env, 0.15);
    if matches!(from_harness, "claude" | "acp") {
        tmux(env, &["send-keys", "-t", pane, "Escape"])?;
        sleep(env, 0.1);
        tmux(env, &["send-keys", "-t", pane, "-l", "--", "/exit"])?;
        tmux(env, &["send-keys", "-t", pane, "Enter"])?;
    } else {
        tmux(env, &["send-keys", "-t", pane, "C-c"])?;
        sleep(env, 0.3);
        tmux(env, &["send-keys", "-t", pane, "C-c"])?;
    }
    for _ in 0..30 {
        sleep(env, 0.5);
        if start(env, pid) != started {
            return Ok(true);
        }
        if kill(pid, None).is_err() {
            return Ok(true);
        }
    }
    if start(env, pid) != started {
        return Ok(true);
    }
    if kill(pid, Some(signal::Signal::SIGTERM)).is_ok() {
        sleep(env, 1.0);
    }
    Ok(kill(pid, None).is_err())
}

/// `_stop_owned_startup(pid, started)` (2697): `SIGTERM` al destino que
/// arrancó esta operación (sin pulsar Enter en un diálogo sin aceptar).
pub fn stop_owned_startup(env: &Env, pid: i64, started: &str) -> Result<bool, String> {
    if pid <= 0 || start(env, pid) != started {
        return Ok(false);
    }
    // Inicio ilegible: el proceso probablemente ya murió y su pid podría ser
    // de otro. Desviación (el Python manda `SIGTERM` igual): solo se pregunta
    // con la señal 0, como `exit_current`.
    if started.is_empty() {
        return Ok(matches!(
            kill(pid, None),
            Err(KillError::Errno(Errno::ESRCH))
        ));
    }
    match kill(pid, Some(signal::Signal::SIGTERM)) {
        Ok(()) => {}
        Err(KillError::Errno(Errno::ESRCH)) => return Ok(true),
        Err(other) => return Err(other.text()),
    }
    for _ in 0..30 {
        if start(env, pid) != started {
            return Ok(true);
        }
        sleep(env, 0.1);
    }
    Ok(false)
}

/// `_restore_shell_tty(pane)` (1846): `stty sane` en la tty del pane.
pub fn restore_shell_tty(env: &Env, pane: &str) -> Result<(), String> {
    let out = tmux(env, &["display-message", "-p", "-t", pane, "#{pane_tty}"])?;
    let tty = strip(&out.stdout);
    if tty.starts_with("/dev/pts/") {
        run(&["stty", "sane", "-F", tty], 3)?;
    }
    Ok(())
}

/// `_send_shell_line(pane, command)`: teclea el comando y Enter (sin
/// `paste-buffer -p`, que zsh mostraría como `^[[200~`).
pub fn send_shell_line(env: &Env, pane: &str, command: &str) -> Result<(), String> {
    // Both tmux's command protocol and a terminal reset to ICANON have small
    // line limits. Keep the complete launch outside argv and outside the TTY.
    if command.len() >= 4096 {
        return send_scripted_shell_line(env, pane, command);
    }
    let r = tmux(env, &["send-keys", "-t", pane, "-l", "--", command])?;
    if r.returncode != 0 {
        let message = strip(&r.stderr);
        return Err(if message.is_empty() {
            "tmux send-keys fallo".to_owned()
        } else {
            message.to_owned()
        });
    }
    tmux(env, &["send-keys", "-t", pane, "Enter"])?;
    Ok(())
}

struct ShellScript(PathBuf);
impl Drop for ShellScript {
    fn drop(&mut self) {
        // Success is acknowledged by the sourced script unlinking itself.
        // On failure/timeout invalidate only this invocation's pending script.
        let _ = fs::remove_file(&self.0);
    }
}

fn checked_tmux(env: &Env, args: &[&str]) -> Result<(), String> {
    let result = tmux(env, args)?;
    if result.returncode == 0 {
        Ok(())
    } else {
        let message = strip(&result.stderr);
        Err(if message.is_empty() {
            "tmux shell transport failed".into()
        } else {
            message.into()
        })
    }
}

fn send_scripted_shell_line(env: &Env, pane: &str, command: &str) -> Result<(), String> {
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|e| e.to_string())?;
    let name = format!("comandos-shell-{:032x}", u128::from_ne_bytes(nonce));
    let path = std::env::temp_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(name);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let script = ShellScript(path);
    let path = script
        .0
        .to_str()
        .ok_or("temporary command path is not UTF-8")?;
    let quoted = comandos_core::text::shlex_quote(path);
    let source = format!(". {quoted}");
    if source.len() >= 4096 {
        return Err("temporary source command exceeds terminal line limit".into());
    }
    // The current shell keeps the opened source descriptor after unlink. This
    // is the acknowledgement that it can read the entire original command;
    // no payload bytes travel through the terminal's canonical input buffer.
    write!(
        file,
        "command /bin/rm -f -- {quoted} || return $?\n{command}\n"
    )
    .map_err(|e| e.to_string())?;
    drop(file);
    checked_tmux(env, &["send-keys", "-t", pane, "-l", "--", &source])?;
    checked_tmux(env, &["send-keys", "-t", pane, "Enter"])?;
    // Keep the source alive until the shell opens it. Never remove it directly
    // after send-keys: tmux acknowledging input does not mean zsh consumed it.
    for _ in 0..50 {
        match script.0.symlink_metadata() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
            Ok(_) => sleep(env, 0.1),
        }
    }
    Err("el shell no abrió el archivo temporal del comando".into())
}

/// `_send_configuration_command(pane, command)` (2691): limpia la pantalla
/// visible tras el eco del comando (el prompt viejo queda en el historial,
/// fuera de la captura con la que se espera el destino).
///
/// Desviación (15-oct, Jesús: «pone comandos muy largos»): el relanzamiento
/// (`env COMANDOS_OPERATION_ID=… env -u … claude --resume …`) pasa siempre por
/// el archivo temporal que se lee a sí mismo; en la terminal solo queda
/// `. /tmp/comandos-shell-…`. Lo corto (un shell, `exit`) se teclea igual.
pub fn send_configuration_command(env: &Env, pane: &str, command: &str) -> Result<(), String> {
    let line = format!("printf '\\033[2J\\033[H'; {command}");
    if line.len() > SHORT_LINE {
        return send_scripted_shell_line(env, pane, &line);
    }
    send_shell_line(env, pane, &line)
}

/// Lo más largo que se teclea tal cual en un relanzamiento.
const SHORT_LINE: usize = 120;

/// `repr(str)` de Python para los mensajes de `TimeoutExpired`.
pub(crate) fn py_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `subprocess.run(argv, capture_output=True, timeout=…)`: la salida
/// estándar en bytes, o el texto de la excepción (`TimeoutExpired`,
/// `FileNotFoundError`, …). Al vencer el plazo mata el hijo, como el Python.
fn run(argv: &[&str], timeout: u64) -> Result<Vec<u8>, String> {
    let (program, args) = argv.split_first().ok_or_else(String::new)?;
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "[Errno 2] No such file or directory: {}",
                py_repr(program)
            ));
        }
        Err(error) => return Err(crate::session_configuration::io_text(&error)),
    };
    // Lectores con plazo: un nieto que herede los pipes (un hook de `git`)
    // no cuelga la operación. Como `subprocess.run`, el plazo cubre la salida
    // del hijo y el EOF de sus pipes; al vencer se mata el hijo, se abandonan
    // los lectores y se responde `TimeoutExpired`.
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut buf);
            }
            let _ = tx.send(buf);
        });
        rx
    };
    let out = reader(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = reader(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let (mut stdout, mut stderr_done, mut exited) = (None, false, false);
    while !(exited && stdout.is_some() && stderr_done) {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let list = argv
                .iter()
                .map(|a| py_repr(a))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "Command '[{list}]' timed out after {timeout} seconds"
            ));
        }
        if !exited {
            match child.try_wait() {
                Ok(Some(_)) => exited = true,
                Ok(None) => {}
                Err(error) => return Err(crate::session_configuration::io_text(&error)),
            }
        }
        if stdout.is_none() {
            match out.try_recv() {
                Ok(buf) => stdout = Some(buf),
                Err(mpsc::TryRecvError::Disconnected) => stdout = Some(Vec::new()),
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if !stderr_done && !matches!(err.try_recv(), Err(mpsc::TryRecvError::Empty)) {
            stderr_done = true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(stdout.unwrap_or_default())
}

/// `text=True`: UTF-8 estricto y saltos universales (`\r\n` y `\r` → `\n`).
fn text_mode(raw: Vec<u8>) -> Option<String> {
    let text = String::from_utf8(raw).ok()?;
    Some(text.replace("\r\n", "\n").replace('\r', "\n"))
}

/// `^[\s]*[─━=\-]{8,}` con el `\s` de Python.
fn is_rule(line: &str) -> bool {
    let rest = line.trim_start_matches(is_space);
    rest.chars()
        .take_while(|c| matches!(c, '─' | '━' | '=' | '-'))
        .count()
        >= 8
}

/// `str.rstrip()`.
fn rstrip(line: &str) -> &str {
    line.trim_end_matches(is_space)
}

/// `capture_handoff(pane, from, to, cwd, sid, transcript_path)` (1926): el
/// texto de traspaso con lo visible del pane y el estado de git; nada de
/// secretos ni del razonamiento interno.
pub fn capture_handoff(
    env: &Env,
    pane: &str,
    from_harness: &str,
    to_harness: &str,
    cwd: &str,
    sid: &str,
    transcript_path: &str,
) -> Result<String, Fail> {
    let git = |args: &[&str]| -> Option<String> {
        let mut argv = vec!["git", "-C", cwd];
        argv.extend_from_slice(args);
        let out = text_mode(run(&argv, 5).ok()?)?;
        Some(strip(&out).to_owned())
    };
    let (branch, st, diff) = match (
        git(&["rev-parse", "--abbrev-ref", "HEAD"]),
        git(&["status", "--short"]),
        git(&["diff", "--stat"]),
    ) {
        (Some(b), Some(s), Some(d)) => (b, s, d),
        _ => (String::new(), String::new(), String::new()),
    };
    let raw = (env.tmux)(&["capture-pane", "-p", "-t", pane, "-S", "-90"])
        .map_err(Fail::Py)?
        .stdout;
    let mut keep = Vec::new();
    for line in splitlines(&raw) {
        let s = rstrip(line);
        if s.is_empty() || is_rule(s) {
            continue;
        }
        if [
            "auto mode on",
            "bypass permissions",
            "for agents",
            "shift+tab to cycle",
            "esc to interrupt",
        ]
        .iter()
        .any(|m| s.contains(m))
        {
            continue;
        }
        keep.push(s);
    }
    let tail = keep[keep.len().saturating_sub(45)..].join("\n");
    let mut transcript = String::new();
    if !sid.is_empty() {
        // Sin ruta, el Python buscaría con `_transcript_path(sid)` en
        // `~/.claude/projects` y `~/.claude-accounts`: no se llega aquí (la
        // ruta exacta ya la fijó `_snapshot_transcript`).
        if transcript_path.is_empty() {
            return Err(Fail::Unsure);
        }
        transcript = format!(
            "\n## Transcript de {from_harness} (se conserva en disco)\n\
             Archivo: `{transcript_path}`\n\
             sessionId: `{sid}` — al VOLVER a {from_harness} ComandOS \
             lanza `--resume` con este id. La memoria interna del TUI \
             NO se copia a {to_harness}.\n"
        );
    }
    let branch = if branch.is_empty() { "?" } else { &branch };
    let st = if st.is_empty() {
        "(sin cambios sin commitear)"
    } else {
        &st
    };
    Ok(format!(
        "# Handoff de ComandOS — {from_harness} → {to_harness}\n\n\
         Vienes de una sesión de **{from_harness}** en `{cwd}`. Eres un CLI \
         distinto: NO tienes su memoria interna. Usa esto para continuar la MISMA tarea.\n\n\
         ## Estado git (rama {branch})\n```\n{st}\n\
         {diff}\n```\n\
         {transcript}\n\
         ## Últimos pasos visibles de la sesión anterior\n```\n{tail}\n```\n\n\
         Continúa desde aquí. Revisa archivos y git si necesitas más contexto. \
         Cuando termines de leer, puedes borrar este archivo.\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_and_repr_like_python() {
        assert!(is_rule("  ────────"));
        assert!(is_rule("\u{1c}--------x"));
        assert!(!is_rule("-------"));
        assert_eq!(py_repr("/dev/pts/3"), "'/dev/pts/3'");
        assert_eq!(py_repr("a'b"), "\"a'b\"");
    }

    #[test]
    fn run_reports_missing_program_and_timeout() {
        let missing = run(&["comandos-no-existe-xyz"], 1).unwrap_err();
        assert_eq!(
            missing,
            "[Errno 2] No such file or directory: 'comandos-no-existe-xyz'"
        );
        let slow = run(&["sleep", "5"], 1).unwrap_err();
        assert_eq!(slow, "Command '['sleep', '5']' timed out after 1 seconds");
        // Un nieto que hereda los pipes no cuelga la lectura: vence el plazo.
        let started = Instant::now();
        let held = run(&["sh", "-c", "sleep 5 & exit 0"], 1).unwrap_err();
        assert!(held.ends_with("timed out after 1 seconds"), "{held}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
