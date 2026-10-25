//! The original thirty command names share one registry with later UI slices.
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

pub const COMMAND_NAMES: [&str; 30] = [
    "split",
    "split_ssh",
    "kill_pane",
    "toggle_window",
    "select_pane",
    "next_tab",
    "prev_tab",
    "mru_toggle",
    "focus_page",
    "tab_reorder",
    "mosaic",
    "mosaic_zoom",
    "side_panel",
    "terminals_visible",
    "reload_dashboard",
    "font_scale",
    "open_switcher",
    "tabs_overview",
    "help",
    "snippets",
    "new_local_tab",
    "open_xterm_tab",
    "open_wizard",
    "start_ai_here",
    "copy_selection",
    "paste_clipboard",
    "copy_reply",
    "window",
    "paned_position",
    "quit",
];
#[derive(Debug, Clone, PartialEq)]
pub struct AppCommand {
    pub name: String,
    pub args: Value,
}
pub use comandos_desktop::command::{CommandError, string_arg};
pub type Handler = Rc<dyn Fn(&Value) -> Result<(), CommandError>>;
#[derive(Default)]
pub struct Registry {
    handlers: RefCell<BTreeMap<&'static str, Handler>>,
}
impl Registry {
    pub fn install(&self, name: &'static str, handler: Handler) {
        self.handlers.borrow_mut().insert(name, handler);
    }
    pub fn invoke(&self, name: &str, args: &Value) -> Result<(), CommandError> {
        // Clone before calling: a consumer may install/remove another handler.
        let handler = self
            .handlers
            .borrow()
            .get(name)
            .cloned()
            .ok_or_else(|| CommandError::MissingConsumer(name.into()))?;
        handler(args)
    }
    pub fn dispatch(&self, command: &AppCommand) -> Result<(), CommandError> {
        self.invoke(&command.name, &command.args)
    }
    pub fn clear(&self) {
        self.handlers.borrow_mut().clear();
    }
}
pub fn parse_command(value: &Value) -> Result<AppCommand, CommandError> {
    if !value.is_object() {
        return Err(CommandError::Invalid("command must be an object".into()));
    }
    let name = value
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::Invalid("command must be a string".into()))?;
    if !COMMAND_NAMES.contains(&name) {
        return Err(CommandError::Unknown(name.into()));
    }
    let args = value
        .get("args")
        .filter(|v| comandos_core::json::truthy(v))
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !args.is_object() && !ignores_arguments(name) {
        return Err(CommandError::Invalid("args must be an object".into()));
    }
    Ok(AppCommand {
        name: name.into(),
        args,
    })
}

fn ignores_arguments(name: &str) -> bool {
    matches!(
        name,
        "toggle_window"
            | "next_tab"
            | "prev_tab"
            | "mru_toggle"
            | "reload_dashboard"
            | "open_switcher"
            | "tabs_overview"
            | "help"
            | "snippets"
            | "new_local_tab"
            | "open_wizard"
            | "copy_selection"
            | "paste_clipboard"
            | "copy_reply"
            | "quit"
    )
}

pub fn valid_xterm_session(session: &str) -> bool {
    !session.is_empty()
        && session.len() <= 32
        && session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

/// Xterm-specific names fall back before creating their tab identity.
pub fn xterm_session_arg(args: &Value) -> Result<&str, CommandError> {
    let session = args
        .get("session")
        .filter(|v| comandos_core::json::truthy(v))
        .map_or(Ok("local"), |v| {
            v.as_str()
                .ok_or_else(|| CommandError::Invalid("session must be a string".into()))
        })?;
    Ok(if valid_xterm_session(session) {
        session
    } else {
        "local"
    })
}

pub fn deferred_consumer(name: &str) -> Option<&'static str> {
    Some(match name {
        "mosaic" => "T18.mosaic",
        "mosaic_zoom" => "T18.mosaic_zoom",
        "side_panel" => "T18.left_panel",
        "open_switcher" => "T15.switcher",
        "tabs_overview" => "T15.overview",
        "help" => "T15.help",
        "snippets" => "T16.snippets",
        "open_wizard" => "T14.wizard",
        "start_ai_here" => "T14.start_ai_here",
        "copy_selection" => "T16.copy_selection",
        "paste_clipboard" => "T16.paste_clipboard",
        "copy_reply" => "T16.copy_reply",
        _ => return None,
    })
}
pub fn deferred_arguments(name: &str, args: &Value) -> Result<Value, CommandError> {
    Ok(match name {
        "mosaic" => {
            let state = string_arg(args, "state", "toggle")?;
            if !["on", "off", "toggle"].contains(&state) {
                return Err(CommandError::Invalid("invalid mosaic state".into()));
            }
            json!({"state":state})
        }
        "mosaic_zoom" => json!({"session":args.get("session"),"zoom_session":args.get("session")}),
        "side_panel" => json!({"hidden":!args.get("on").is_some_and(comandos_core::json::truthy)}),
        "start_ai_here" => {
            // This command permits an omitted target (keyboard/current terminal).
            // Preserve other invalid types so the consumer's string_arg rejects them.
            let optional = |key| {
                args.get(key)
                    .filter(|v| !v.is_null())
                    .cloned()
                    .unwrap_or(json!(""))
            };
            json!({"session":optional("session"),"pane":optional("pane")})
        }
        "open_switcher" | "tabs_overview" | "help" | "snippets" | "open_wizard"
        | "copy_selection" | "paste_clipboard" | "copy_reply" => json!({}),
        _ => return Err(CommandError::Unknown(name.into())),
    })
}
pub fn valid_pane(pane: &str) -> bool {
    pane.strip_prefix('%')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}
pub fn session_arg<'a>(args: &'a Value, default: &'a str) -> Result<&'a str, CommandError> {
    let value = args
        .get("session")
        .filter(|v| comandos_core::json::truthy(v));
    let key = value.map_or(Ok(default), |v| {
        v.as_str()
            .ok_or_else(|| CommandError::Invalid("session must be a string".into()))
    })?;
    if !crate::tab_actions::valid_session(key) {
        return Err(CommandError::Invalid("invalid session".into()));
    }
    Ok(key)
}
pub fn python_int(value: &Value) -> Result<i32, CommandError> {
    comandos_core::focus::integer_string(value)
        .map_err(|e| CommandError::Invalid(e.to_string()))?
        .parse()
        .map_err(|_| CommandError::Invalid("integer outside GTK range".into()))
}
pub fn font_scale(current: f64, delta: &Value) -> Result<f64, CommandError> {
    if !comandos_core::json::truthy(delta) {
        return Ok(1.0);
    }
    let scale = current + comandos_core::focus::float(delta).map_err(CommandError::Invalid)?;
    // Preserve Python max(0.5, min(2.5, scale)), including its NaN operand order.
    Ok(if scale < 2.5 { scale.max(0.5) } else { 2.5 })
}
pub fn split_flags(side: &str) -> Result<Vec<String>, CommandError> {
    Ok(match side {
        "right" => vec!["-h"],
        "left" => vec!["-h", "-b"],
        "down" => vec!["-v"],
        "up" => vec!["-v", "-b"],
        _ => return Err(CommandError::Invalid("invalid split side".into())),
    }
    .into_iter()
    .map(str::to_string)
    .collect())
}
pub fn ssh_host(session: &str) -> Option<&str> {
    if let Some(host) = session.strip_prefix("ssh-") {
        return (!host.is_empty()).then_some(host);
    }
    let (host, index) = session.strip_prefix("sshtab-")?.rsplit_once('-')?;
    (!host.is_empty() && !index.is_empty() && index.chars().all(|c| c.is_ascii_digit()))
        .then_some(host)
}
pub fn active_pane<T: crate::restore::RestoreTmux>(
    tmux: &T,
    session: &str,
) -> Result<Option<String>, CommandError> {
    if !crate::tab_actions::valid_session(session) {
        return Err(CommandError::Invalid("invalid current session".into()));
    }
    let target = format!("={session}:");
    let out = tmux
        .read(&["display-message", "-p", "-t", &target, "#{pane_id}"])
        .map_err(CommandError::Failed)?;
    let pane = out.trim();
    Ok(valid_pane(pane).then(|| pane.into()))
}
pub fn execute_split<T: crate::restore::RestoreTmux>(
    tmux: &T,
    session: &str,
    side: &str,
    ssh: bool,
    cancelled: impl Fn() -> bool,
) -> Result<(), CommandError> {
    execute_split_at(tmux, session, side, ssh, None, cancelled)
}
pub fn execute_split_at<T: crate::restore::RestoreTmux>(
    tmux: &T,
    session: &str,
    side: &str,
    ssh: bool,
    pane: Option<&str>,
    cancelled: impl Fn() -> bool,
) -> Result<(), CommandError> {
    execute_split_at_expected(tmux, session, side, ssh, pane, None, cancelled)
}
pub fn execute_split_at_expected<T: crate::restore::RestoreTmux>(
    tmux: &T,
    session: &str,
    side: &str,
    ssh: bool,
    pane: Option<&str>,
    expected: Option<&str>,
    cancelled: impl Fn() -> bool,
) -> Result<(), CommandError> {
    let flags = split_flags(side)?;
    if tmux.mode() == crate::config::RunMode::Shadow || cancelled() {
        return Err(CommandError::Refused("split cancelled or shadow".into()));
    }
    let target = if let Some(pane) = pane {
        if !valid_pane(pane) {
            return Err(CommandError::Invalid("invalid split pane".into()));
        }
        let actual = tmux
            .read(&["display-message", "-p", "-t", pane, "#{session_name}"])
            .map_err(CommandError::Failed)?;
        if actual.trim() != session {
            return Err(CommandError::Refused("split pane moved".into()));
        }
        pane.to_string()
    } else {
        active_pane(tmux, session)?.unwrap_or_else(|| format!("={session}:"))
    };
    let identity = if pane.is_some() {
        let stamp = tmux
            .read(&[
                "display-message",
                "-p",
                "-t",
                &target,
                "#{session_name}|#{pid}|#{session_id}|#{s/[$]//:session_id}|#{pane_id}",
            ])
            .map_err(CommandError::Failed)?;
        if stamp.trim().is_empty() || expected.is_some_and(|s| s != stamp.trim()) {
            return Err(CommandError::Refused("split identity changed".into()));
        }
        Some(stamp.trim().to_string())
    } else {
        None
    };
    if cancelled() {
        return Err(CommandError::Refused("split cancelled".into()));
    }
    let mut args = vec!["split-window".into()];
    args.extend(flags);
    args.extend(["-t".into(), target.clone()]);
    if let Some(host) = ssh.then(|| ssh_host(session)).flatten() {
        args.push(format!("ssh {host}"));
    } else {
        let out = tmux
            .read(&[
                "display-message",
                "-p",
                "-t",
                &target,
                "#{pane_current_path}",
            ])
            .map_err(CommandError::Failed)?;
        let cwd = out.trim();
        if !cwd.is_empty() {
            args.extend(["-c".into(), cwd.into()]);
        }
    }
    if cancelled() {
        return Err(CommandError::Refused("split cancelled".into()));
    }
    if let Some(stamp) = identity {
        let now = tmux
            .read(&[
                "display-message",
                "-p",
                "-t",
                &target,
                "#{session_name}|#{pid}|#{session_id}|#{s/[$]//:session_id}|#{pane_id}",
            ])
            .map_err(CommandError::Failed)?;
        if now.trim() != stamp || cancelled() {
            return Err(CommandError::Refused("split identity changed".into()));
        }
    }
    tmux.mutate(&args.iter().map(String::as_str).collect::<Vec<_>>(), None)
        .map_err(CommandError::Failed)?;
    Ok(())
}
pub fn execute_select_pane<T: crate::restore::RestoreTmux>(
    tmux: &T,
    pane: &str,
    cancelled: bool,
) -> Result<(), CommandError> {
    if !valid_pane(pane) {
        return Err(CommandError::Invalid("invalid pane id".into()));
    }
    if cancelled || tmux.mode() == crate::config::RunMode::Shadow {
        return Err(CommandError::Refused(
            "select pane cancelled or shadow".into(),
        ));
    }
    tmux.mutate(&["select-pane", "-t", pane], None)
        .map_err(CommandError::Failed)?;
    Ok(())
}
pub fn install(app: &Rc<super::app::App>) {
    app.install_app_commands();
}
