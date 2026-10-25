//! Automatic tmux buffers only. A single numeric watermark, no buffer cache.
use crate::{config::RunMode, tmux::TmuxError};
pub use comandos_desktop::state_files::MAX_BYTES;
pub const LIST_FORMAT: &str = "#{buffer_name}\t#{buffer_size}";
pub const POLL_MS: u64 = 350;
fn number_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}
fn top(listing: &str) -> Option<(String, String, i128)> {
    let mut best: Option<(String, String, i128)> = None;
    static AUTO: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = AUTO.get_or_init(|| {
        regex::Regex::new(r"^buffer(\d+)$").unwrap_or_else(|_| unreachable!("literal regex"))
    });
    for line in listing.lines() {
        let (name, size) = line.split_once('\t').unwrap_or((line, ""));
        let name = name.trim();
        let Some(capture) = pattern.captures(name) else {
            continue;
        };
        let Some(digits) = capture.get(1).map(|m| m.as_str()) else {
            continue;
        };
        let Ok(number) = comandos_core::focus::integer_string(&serde_json::json!(digits)) else {
            continue;
        };
        let Ok(size_text) =
            comandos_core::focus::integer_string(&serde_json::json!(if size.is_empty() {
                "0"
            } else {
                size
            }))
        else {
            continue;
        };
        let size = size_text
            .parse::<i128>()
            .unwrap_or(if size_text.starts_with('-') {
                i128::MIN
            } else {
                i128::MAX
            });
        if best
            .as_ref()
            .is_none_or(|(old, _, _)| number_cmp(&number, old).is_gt())
        {
            best = Some((number, name.into(), size));
        }
    }
    best
}
pub fn newest_auto(listing: &str) -> Option<String> {
    top(listing).map(|(_, name, _)| name)
}
#[derive(Debug, Default)]
pub struct Bridge {
    seen: Option<String>,
}
impl Bridge {
    pub fn poll(&mut self, listing: &str) -> Option<String> {
        let (number, name, size) = top(listing)?;
        let Some(seen) = self.seen.as_ref() else {
            self.seen = Some(number);
            return None;
        };
        match number_cmp(&number, seen) {
            std::cmp::Ordering::Less => {
                self.seen = Some(number);
                None
            }
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => {
                self.seen = Some(number);
                (size > 0 && size <= MAX_BYTES as i128).then_some(name)
            }
        }
    }
    pub fn read_next(
        &mut self,
        tmux: &impl crate::ui::clipboard::TmuxIo,
    ) -> Result<Option<String>, TmuxError> {
        if tmux.mode() == RunMode::Shadow {
            return Ok(None);
        }
        let out = tmux.read(&["list-buffers", "-F", LIST_FORMAT])?;
        let Some(name) = self.poll(if out.ok() { &out.stdout } else { "" }) else {
            return Ok(None);
        };
        let out = tmux.read(&["show-buffer", "-b", &name])?;
        Ok(
            (out.ok() && !out.stdout.is_empty() && out.stdout.len() <= MAX_BYTES)
                .then_some(out.stdout),
        )
    }
}
use crate::ui::clipboard::{ClipboardError, TmuxIo};
pub fn session_from_clients(listing: &str, tty: &str) -> Option<String> {
    for line in listing.lines() {
        if let Some((client, session)) = line.split_once('|')
            && client == tty
        {
            return Some(session.into());
        }
    }
    None
}
pub fn term_session(
    tmux: &impl TmuxIo,
    tty: Option<&str>,
) -> Result<Option<String>, ClipboardError> {
    let Some(tty) = tty.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let out = tmux
        .read(&["list-clients", "-F", "#{client_tty}|#{client_session}"])
        .map_err(|e| ClipboardError::Tmux(format!("{e:?}")))?;
    Ok(session_from_clients(&out.stdout, tty))
}
pub fn copy_mode_pane(
    tmux: &impl TmuxIo,
    session: Option<&str>,
    preferred: Option<&str>,
) -> Result<Option<String>, ClipboardError> {
    if let Some(pane) = preferred {
        let out = tmux
            .read(&["display-message", "-p", "-t", pane, "#{pane_in_mode}"])
            .map_err(|e| ClipboardError::Tmux(format!("{e:?}")))?;
        if out.ok() && crate::ui::app_commands::valid_pane(pane) && out.stdout.trim() == "1" {
            return Ok(Some(pane.into()));
        }
    }
    let Some(session) = session else {
        return Ok(None);
    };
    let target = format!("={session}:");
    let out = tmux
        .read(&[
            "list-panes",
            "-t",
            &target,
            "-F",
            "#{pane_id}|#{pane_in_mode}",
        ])
        .map_err(|e| ClipboardError::Tmux(format!("{e:?}")))?;
    if out.ok() {
        for line in out.stdout.lines() {
            if let Some((pane, "1")) = line.trim().split_once('|')
                && crate::ui::app_commands::valid_pane(pane)
            {
                return Ok(Some(pane.into()));
            }
        }
    }
    Ok(None)
}
pub fn exit_copy_mode(
    tmux: &impl TmuxIo,
    tty: Option<&str>,
    cancelled: impl Fn() -> bool,
) -> Result<bool, ClipboardError> {
    if tmux.mode() == RunMode::Shadow || cancelled() {
        return Err(ClipboardError::Cancelled);
    }
    let Some(session) = term_session(tmux, tty)? else {
        return Ok(false);
    };
    let target = format!("={session}:");
    let out = tmux
        .read(&[
            "display-message",
            "-p",
            "-t",
            &target,
            "#{pane_in_mode}|#{pid}|#{session_id}|#{s/[$]//:session_id}|#{pane_id}",
        ])
        .map_err(|e| ClipboardError::Tmux(format!("{e:?}")))?;
    if !out.ok() {
        return Err(ClipboardError::DestinationChanged);
    }
    let Some(("1", stamp)) = out.stdout.trim().split_once('|') else {
        return Ok(false);
    };
    if cancelled() {
        return Err(ClipboardError::Cancelled);
    }
    let now = tmux
        .read(&[
            "display-message",
            "-p",
            "-t",
            &target,
            "#{pid}|#{session_id}|#{s/[$]//:session_id}|#{pane_id}",
        ])
        .map_err(|e| ClipboardError::Tmux(format!("{e:?}")))?;
    if !now.ok() || now.stdout.trim() != stamp {
        return Err(ClipboardError::DestinationChanged);
    }
    if cancelled() {
        return Err(ClipboardError::Cancelled);
    }
    let pane = stamp
        .rsplit('|')
        .next()
        .filter(|p| crate::ui::app_commands::valid_pane(p))
        .ok_or(ClipboardError::DestinationChanged)?;
    let out = tmux
        .mutate(&["send-keys", "-t", pane, "-X", "cancel"], None)
        .map_err(|e| ClipboardError::Tmux(format!("{e:?}")))?;
    if !out.ok() {
        return Err(ClipboardError::Tmux("No se pudo salir de copy-mode".into()));
    }
    Ok(true)
}
pub fn copy_tmux_selection(
    tmux: &impl TmuxIo,
    pane: &str,
    cancelled: impl Fn() -> bool,
) -> Result<String, ClipboardError> {
    if tmux.mode() == RunMode::Shadow || cancelled() {
        return Err(ClipboardError::Cancelled);
    }
    if !crate::ui::app_commands::valid_pane(pane) {
        return Err(ClipboardError::DestinationChanged);
    }
    let capture = tmux
        .read(&[
            "display-message",
            "-p",
            "-t",
            pane,
            "#{pid}|#{session_id}|#{s/[$]//:session_id}|#{pane_id}",
        ])
        .map_err(|e| ClipboardError::Tmux(format!("{e:?}")))?;
    if !capture.ok() || capture.stdout.trim().is_empty() {
        return Err(ClipboardError::DestinationChanged);
    }
    let socket = tmux.socket().to_path_buf();
    let recheck = || -> Result<(), ClipboardError> {
        if cancelled() {
            return Err(ClipboardError::Cancelled);
        }
        if tmux.socket() != socket {
            return Err(ClipboardError::DestinationChanged);
        }
        let now = tmux
            .read(&[
                "display-message",
                "-p",
                "-t",
                pane,
                "#{pid}|#{session_id}|#{s/[$]//:session_id}|#{pane_id}",
            ])
            .map_err(|e| ClipboardError::Tmux(format!("{e:?}")))?;
        if cancelled() || !now.ok() || now.stdout.trim() != capture.stdout.trim() {
            return Err(ClipboardError::DestinationChanged);
        }
        Ok(())
    };
    recheck()?;
    let mut out = tmux
        .mutate(
            &["send-keys", "-t", pane, "-X", "copy-selection-no-clear"],
            None,
        )
        .map_err(|e| ClipboardError::Tmux(format!("{e:?}")))?;
    if !out.ok() {
        recheck()?;
        out = tmux
            .mutate(&["send-keys", "-t", pane, "-X", "copy-selection"], None)
            .map_err(|e| ClipboardError::Tmux(format!("{e:?}")))?;
    }
    if !out.ok() || cancelled() {
        return Err(ClipboardError::Tmux(
            "No se pudo copiar seleccion tmux".into(),
        ));
    }
    recheck()?;
    let out = tmux
        .read(&["save-buffer", "-"])
        .map_err(|e| ClipboardError::Tmux(format!("{e:?}")))?;
    recheck()?;
    if !out.ok() || out.stdout.len() > MAX_BYTES {
        return Err(ClipboardError::Tmux("Buffer invalido".into()));
    }
    Ok(out.stdout)
}
