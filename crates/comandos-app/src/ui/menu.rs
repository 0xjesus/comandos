//! Original terminal menu order; handlers bind the originating terminal instance.
use std::{ffi::OsString, path::PathBuf};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Copy,
    Paste,
    CopyReply,
    ExportTxt,
    ExportPdf,
    Split { side: &'static str, ssh: bool },
    ToggleWindow,
    CloseSplit,
    StartAI,
    NewLocal,
    NewXterm,
    OpenProject,
    ExistingSessions,
    Rename,
    CloseTab,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuRow {
    Action {
        action: MenuAction,
        label: String,
        enabled: bool,
    },
    Separator,
    Marks,
}
pub fn terminal_rows(
    english: bool,
    session: Option<&str>,
    pane: Option<&str>,
    npanes: usize,
    has_key: bool,
) -> Vec<MenuRow> {
    let mut rows = Vec::new();
    let mut item = |action, es: &str, en: &str, enabled| {
        rows.push(MenuRow::Action {
            action,
            label: if english { en } else { es }.into(),
            enabled,
        })
    };
    item(
        MenuAction::Copy,
        "Copiar                   Ctrl+C",
        "Copy                     Ctrl+C",
        true,
    );
    item(
        MenuAction::Paste,
        "Pegar                    Ctrl+V",
        "Paste                    Ctrl+V",
        true,
    );
    item(
        MenuAction::CopyReply,
        "Copiar respuesta de Claude",
        "Copy Claude's reply",
        true,
    );
    item(
        MenuAction::ExportTxt,
        "Guardar respuesta como .txt",
        "Save reply as .txt",
        true,
    );
    item(
        MenuAction::ExportPdf,
        "Guardar respuesta como PDF",
        "Save reply as PDF",
        true,
    );
    rows.push(MenuRow::Separator);
    let ssh = session.and_then(super::app_commands::ssh_host).is_some();
    for (side, es, en) in [
        ("right", "derecha", "right"),
        ("left", "izquierda", "left"),
        ("down", "abajo", "down"),
        ("up", "arriba", "up"),
    ] {
        let (es, en) = if ssh {
            (
                format!("Split {es:<11}(otra conexion al server)"),
                format!("Split {en:<8} (new connection to server)"),
            )
        } else {
            (format!("Split {es}"), format!("Split {en}"))
        };
        rows.push(MenuRow::Action {
            action: MenuAction::Split { side, ssh },
            label: if english { en } else { es },
            enabled: true,
        });
    }
    if ssh {
        rows.push(MenuRow::Action {
            action: MenuAction::Split {
                side: "right",
                ssh: false,
            },
            label: if english {
                "Local split (your machine, right)"
            } else {
                "Split local (tu maquina, derecha)"
            }
            .into(),
            enabled: true,
        });
    }
    let mut item = |action, es: &str, en: &str, enabled| {
        rows.push(MenuRow::Action {
            action,
            label: if english { en } else { es }.into(),
            enabled,
        })
    };
    item(
        MenuAction::ToggleWindow,
        "Alternar ventana 1/2     Ctrl-b l",
        "Toggle window 1/2        Ctrl-b l",
        true,
    );
    item(
        MenuAction::CloseSplit,
        "Cerrar ESTE split",
        "Close THIS split",
        pane.is_some() && npanes > 1 && session.is_some(),
    );
    rows.push(MenuRow::Separator);
    let mut item = |action, es: &str, en: &str, enabled| {
        rows.push(MenuRow::Action {
            action,
            label: if english { en } else { es }.into(),
            enabled,
        })
    };
    item(
        MenuAction::StartAI,
        "Iniciar sesión de IA aquí…",
        "Start AI session here…",
        true,
    );
    item(
        MenuAction::NewLocal,
        "Nueva terminal aqui       Ctrl+T",
        "New terminal here         Ctrl+T",
        true,
    );
    item(
        MenuAction::NewXterm,
        "Terminal experimental (xterm.js)  Ctrl+Shift+T",
        "Experimental terminal (xterm.js)  Ctrl+Shift+T",
        true,
    );
    item(
        MenuAction::OpenProject,
        "Abrir proyecto...",
        "Open project...",
        true,
    );
    item(
        MenuAction::ExistingSessions,
        "Abrir sesion existente",
        "Open existing session",
        true,
    );
    rows.push(MenuRow::Separator);
    rows.push(MenuRow::Marks);
    let mut item = |action, es: &str, en: &str, enabled| {
        rows.push(MenuRow::Action {
            action,
            label: if english { en } else { es }.into(),
            enabled,
        })
    };
    item(
        MenuAction::Rename,
        "Renombrar pestana...",
        "Rename tab...",
        has_key,
    );
    item(MenuAction::CloseTab, "Cerrar pestana", "Close tab", has_key);
    rows
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenIntent {
    Open,
    Reveal,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPlan {
    pub program: String,
    pub args: Vec<OsString>,
    pub reveal: Option<PathBuf>,
}
pub fn clean_local_path(url: &str) -> Option<PathBuf> {
    let stripped = url.trim();
    let raw = if stripped.starts_with("file:") {
        let (path, host) = glib::filename_from_uri(stripped).ok()?;
        if host
            .as_ref()
            .is_some_and(|h| !h.is_empty() && h.as_str() != "localhost")
        {
            return None;
        }
        path.to_str()?.to_string()
    } else {
        stripped.into()
    };
    // Keep real filenames ending in digits or a period; otherwise remove
    // sentence punctuation and line[:column].
    let mut raw = raw.as_str();
    if !std::path::Path::new(raw).exists() {
        raw = crate::term::links::trim_trailing_punctuation(raw);
    }
    for _ in 0..2 {
        if std::path::Path::new(raw).exists() {
            break;
        }
        if let Some((path, _)) = raw
            .rsplit_once(':')
            .filter(|(_, number)| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
        {
            raw = path;
        } else {
            break;
        }
    }
    let path = if let Some(tail) = raw.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME")?).join(tail)
    } else if raw == "~" {
        PathBuf::from(std::env::var_os("HOME")?)
    } else if raw.starts_with('/') {
        PathBuf::from(raw)
    } else {
        return None;
    };
    (!raw.contains('\0') && path.exists()).then_some(path)
}
/// Relative references must be resolved against the clicked pane, never the GUI cwd.
pub fn relative_file_reference(url: &str) -> bool {
    !url.starts_with(['/', '~'])
        && !url.contains("://")
        && !url.starts_with("file:")
        && url.contains('/')
        && !url.chars().any(char::is_control)
}
pub fn open_plan_in(
    url: &str,
    intent: OpenIntent,
    cwd: &std::path::Path,
) -> Result<OpenPlan, String> {
    if relative_file_reference(url) {
        if !cwd.is_absolute() {
            return Err("Carpeta del panel inválida".into());
        }
        let path = cwd.join(url);
        open_plan(path.to_str().ok_or("Ruta no UTF-8")?, intent)
    } else {
        open_plan(url, intent)
    }
}
pub fn open_plan(url: &str, intent: OpenIntent) -> Result<OpenPlan, String> {
    if let Some(path) = clean_local_path(url) {
        let path = if intent == OpenIntent::Reveal && !path.is_dir() {
            path.parent().ok_or("Ruta sin carpeta")?.to_path_buf()
        } else {
            path
        };
        return Ok(OpenPlan {
            program: "xdg-open".into(),
            args: vec![path.into_os_string()],
            reveal: (intent == OpenIntent::Reveal)
                .then(|| clean_local_path(url))
                .flatten(),
        });
    }
    if intent == OpenIntent::Reveal {
        return Err("Ruta local invalida".into());
    }
    if (url.starts_with("https://") || url.starts_with("http://"))
        && !url.bytes().any(|b| b < 32 || b == 127)
    {
        let rest = url.split_once("://").map(|(_, r)| r).unwrap_or("");
        if rest.is_empty() || rest.starts_with('/') {
            return Err("URL invalida".into());
        }
        Ok(OpenPlan {
            program: "xdg-open".into(),
            args: vec![url.into()],
            reveal: None,
        })
    } else {
        Err("Esquema o comando no permitido".into())
    }
}
#[derive(Debug, Clone)]
pub struct Context {
    pub session: Option<String>,
    pub pane: Option<String>,
    pub npanes: usize,
    pub sessions: Vec<String>,
    pub copy_pane: Option<String>,
    pub stamp: Option<String>,
}
pub fn pane_at(listing: &str, col: u16, row: u16) -> (Option<String>, usize) {
    let rows = listing
        .lines()
        .filter(|line| !line.is_empty() && line.matches('|').count() == 4)
        .collect::<Vec<_>>();
    for line in &rows {
        let fields = line.split('|').collect::<Vec<_>>();
        if let [pane, left, top, right, bottom] = fields.as_slice() {
            let numbers = [left, top, right, bottom].map(|s| s.parse::<i64>());
            if let [Ok(l), Ok(t), Ok(r), Ok(b)] = numbers
                && i64::from(col) >= l
                && i64::from(col) <= r
                && i64::from(row) >= t
                && i64::from(row) <= b
                && super::app_commands::valid_pane(pane)
            {
                return (Some((*pane).into()), rows.len());
            }
        }
    }
    (None, rows.len())
}
pub fn read_context(
    tmux: &impl super::clipboard::TmuxIo,
    tty: Option<&str>,
    point: (u16, u16),
    cancelled: impl Fn() -> bool,
) -> Result<Context, String> {
    read_context_inner(tmux, tty, point, cancelled, true)
}
pub fn capture_click(
    tmux: &impl super::clipboard::TmuxIo,
    tty: Option<&str>,
    point: (u16, u16),
    cancelled: impl Fn() -> bool,
) -> Result<Context, String> {
    read_context_inner(tmux, tty, point, cancelled, false)
}
fn read_context_inner(
    tmux: &impl super::clipboard::TmuxIo,
    tty: Option<&str>,
    point: (u16, u16),
    cancelled: impl Fn() -> bool,
    select: bool,
) -> Result<Context, String> {
    if tmux.mode() == crate::config::RunMode::Shadow || cancelled() {
        return Err("Menu cancelado o Shadow".into());
    }
    let session = crate::clipboard_bridge::term_session(tmux, tty).map_err(|e| e.to_string())?;
    let (pane, npanes) = if let Some(session) = &session {
        if cancelled() {
            return Err("Menu cancelado".into());
        }
        let target = format!("={session}:");
        let out = tmux
            .read(&[
                "list-panes",
                "-t",
                &target,
                "-F",
                "#{pane_id}|#{pane_left}|#{pane_top}|#{pane_right}|#{pane_bottom}",
            ])
            .map_err(|e| format!("{e:?}"))?;
        pane_at(&out.stdout, point.0, point.1)
    } else {
        (None, 0)
    };
    let stamp = if let (Some(session), Some(pane)) = (&session, &pane) {
        let out = tmux
            .read(&[
                "display-message",
                "-p",
                "-t",
                pane,
                "#{session_name}|#{pid}|#{session_id}|#{s/[$]//:session_id}|#{pane_id}",
            ])
            .map_err(|e| format!("{e:?}"))?;
        if !out.ok() || out.stdout.trim().split('|').next() != Some(session.as_str()) {
            return Err("El pane cambio de sesion".into());
        }
        Some(out.stdout.trim().to_string())
    } else {
        None
    };
    if let (Some(pane), Some(stamp)) = (&pane, &stamp) {
        let now = tmux
            .read(&[
                "display-message",
                "-p",
                "-t",
                pane,
                "#{session_name}|#{pid}|#{session_id}|#{s/[$]//:session_id}|#{pane_id}",
            ])
            .map_err(|e| format!("{e:?}"))?;
        if !now.ok() || now.stdout.trim() != stamp {
            return Err("El destino cambio".into());
        }
    }
    if select && let Some(pane) = &pane {
        if cancelled() {
            return Err("Menu cancelado".into());
        }
        let out = tmux
            .mutate(&["select-pane", "-t", pane], None)
            .map_err(|e| format!("{e:?}"))?;
        if !out.ok() {
            return Err("No se pudo seleccionar el pane".into());
        }
    }
    let (copy_pane, sessions) = if select {
        let copy_pane =
            crate::clipboard_bridge::copy_mode_pane(tmux, session.as_deref(), pane.as_deref())
                .map_err(|e| e.to_string())?;
        if cancelled() {
            return Err("Menu cancelado".into());
        }
        let out = tmux
            .read(&["list-sessions", "-F", "#{session_name}"])
            .map_err(|e| format!("{e:?}"))?;
        let sessions = if out.ok() {
            out.stdout
                .split_whitespace()
                .filter(|s| crate::tab_actions::valid_session(s))
                .map(str::to_string)
                .collect()
        } else {
            vec![]
        };
        (copy_pane, sessions)
    } else {
        (None, vec![])
    };
    Ok(Context {
        session,
        pane,
        npanes,
        sessions,
        copy_pane,
        stamp,
    })
}

pub fn reply_from_state(items: &serde_json::Value, session: &str) -> Option<(String, String)> {
    for item in items.as_array()? {
        if item.get("session").and_then(serde_json::Value::as_str) == Some(session)
            && let Some(text) = item
                .get("detail")
                .filter(|v| comandos_core::json::truthy(v))
                .or_else(|| item.get("last").filter(|v| comandos_core::json::truthy(v)))
                .and_then(serde_json::Value::as_str)
        {
            return Some((
                text.into(),
                item.get("project")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .into(),
            ));
        }
    }
    None
}
pub fn launch(
    plan: &OpenPlan,
    allowed: impl Fn() -> bool,
    run: &impl Fn(&crate::proc::ProcSpec) -> Result<crate::proc::ProcOutput, crate::proc::ProcError>,
    detached: &impl Fn(&str, &[OsString]) -> Result<(), crate::proc::ProcError>,
) -> Result<(), String> {
    if !allowed() {
        return Err("Apertura cancelada".into());
    }
    if let Some(path) = &plan.reveal {
        let uri = glib::filename_to_uri(path, None).map_err(|e| e.to_string())?;
        let variant = glib::Variant::from(uri.as_str()).print(false);
        let spec = crate::proc::ProcSpec {
            program: "gdbus".into(),
            args: [
                "call",
                "--session",
                "--dest",
                "org.freedesktop.FileManager1",
                "--object-path",
                "/org/freedesktop/FileManager1",
                "--method",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("[{variant}]"),
                "",
            ]
            .map(OsString::from)
            .to_vec(),
            stdin: None,
            env: vec![],
            clear_env: false,
            env_remove: vec![],
            cwd: None,
            timeout: std::time::Duration::from_secs(4),
        };
        if run(&spec).is_ok_and(|o| o.code == Some(0) && !o.timed_out) {
            return Ok(());
        }
        if !allowed() {
            return Err("Apertura cancelada".into());
        }
    }
    detached(&plan.program, &plan.args).map_err(|e| format!("{e:?}"))
}
pub fn select_at(
    tmux: &impl super::clipboard::TmuxIo,
    tty: Option<&str>,
    point: (u16, u16),
    cancelled: impl Fn() -> bool,
) -> Result<(), String> {
    read_context(tmux, tty, point, cancelled).map(|_| ())
}

pub fn notification_payload(title: &str, body: &str) -> serde_json::Value {
    serde_json::json!({"title":title,"body":body,"kind":"done","project":title,"session":"local"})
}

pub fn link_rows(
    local: bool,
    english: bool,
) -> Vec<(&'static str, &'static str, Option<OpenIntent>)> {
    if local {
        vec![
            (
                "folder-open",
                if english {
                    "Open folder"
                } else {
                    "Abrir carpeta"
                },
                Some(OpenIntent::Reveal),
            ),
            (
                "file",
                if english {
                    "Open file"
                } else {
                    "Abrir archivo"
                },
                Some(OpenIntent::Open),
            ),
            (
                "copy",
                if english { "Copy path" } else { "Copiar ruta" },
                None,
            ),
        ]
    } else {
        vec![
            (
                "external-link",
                if english { "Open" } else { "Abrir" },
                Some(OpenIntent::Open),
            ),
            (
                "copy",
                if english { "Copy link" } else { "Copiar link" },
                None,
            ),
        ]
    }
}

pub fn context_current(
    tmux: &impl super::clipboard::TmuxIo,
    context: &Context,
    tty: Option<&str>,
    cancelled: impl Fn() -> bool,
) -> bool {
    if cancelled() || tmux.mode() == crate::config::RunMode::Shadow {
        return false;
    }
    if crate::clipboard_bridge::term_session(tmux, tty)
        .ok()
        .flatten()
        != context.session
    {
        return false;
    }
    match (&context.pane, &context.stamp) {
        (Some(pane), Some(stamp)) => tmux
            .read(&[
                "display-message",
                "-p",
                "-t",
                pane,
                "#{session_name}|#{pid}|#{session_id}|#{s/[$]//:session_id}|#{pane_id}",
            ])
            .is_ok_and(|out| out.ok() && out.stdout.trim() == stamp && !cancelled()),
        (None, None) => !cancelled(),
        _ => false,
    }
}

pub fn select_captured_click(
    tmux: &impl super::clipboard::TmuxIo,
    context: &Context,
    tty: Option<&str>,
    cancelled: impl Fn() -> bool,
) -> Result<(), String> {
    if !context_current(tmux, context, tty, &cancelled) {
        return Err("El destino del clic cambio".into());
    }
    if let Some(pane) = &context.pane {
        if cancelled() {
            return Err("Clic cancelado".into());
        }
        let out = tmux
            .mutate(&["select-pane", "-t", pane], None)
            .map_err(|e| format!("{e:?}"))?;
        if !out.ok() {
            return Err("No se pudo seleccionar el pane".into());
        }
    }
    Ok(())
}
