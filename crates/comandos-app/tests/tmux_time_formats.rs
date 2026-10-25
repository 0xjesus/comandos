//! tmux 3.2a dereferences NULL in `format_find` when a time format
//! (`session_created`, `session_activity`, `window_activity`, `client_activity`…)
//! is expanded for a target that no longer exists: `display-message -t %gone
//! '#{session_created}'` kills the whole server. That is how all of Jesús's
//! sessions died on 24-oct-2026 21:10:51. ComandOS asks for a pane's identity right
//! when panes close, so no command with a target may carry a time format.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod support;

use std::path::Path;

const TIME_FORMATS: [&str; 7] = [
    "session_created",
    "session_activity",
    "session_last_attached",
    "window_activity",
    "client_activity",
    "client_created",
    "pane_start_time",
];

/// Lines that may name a time format: `list-*` commands only expand existing
/// objects, so the value is never NULL there.
const SAFE: [&str; 2] = [
    // `list-panes -a -F PANE_FORMAT`
    "pub const PANE_FORMAT: &str = \"#{session_name}|#{pane_id}|#{pane_pid}|#{pane_current_command}|#{pane_current_path}|#{session_activity}|#{pane_active}|#{window_active}\";",
    // `list-clients -F …`
    "&[\"list-clients\", \"-F\", \"#{client_activity} #{client_name}\"],",
];

fn scan(dir: &Path, found: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.starts_with("//") || SAFE.contains(&line) {
                    continue;
                }
                if TIME_FORMATS.iter().any(|f| line.contains(&format!("#{{{f}")) || line.contains(&format!(":{f}}}"))) {
                    found.push(format!("{}:{}: {line}", path.display(), n + 1));
                }
            }
        }
    }
}

#[test]
fn no_tmux_command_expands_a_time_format_for_a_target() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut found = Vec::new();
    for name in ["comandos-app", "comandos-app-mac", "comandos-server", "comandos-runtime", "comandos-core", "comandos-desktop", "comandos-cli"] {
        let src = crates.join(name).join("src");
        if src.is_dir() {
            scan(&src, &mut found);
        }
    }
    assert!(found.is_empty(), "time formats crash tmux 3.2a on a vanished target:\n{}", found.join("\n"));
}

/// The identity stamp that replaced `session_created` stays empty, not fatal, when
/// the pane or session is already gone (private server only).
#[test]
fn identity_of_a_vanished_target_is_empty_and_the_server_survives() {
    let Some(f) = support::tmux::TestTmux::for_mode(comandos_app::config::RunMode::Sandbox) else {
        return;
    };
    f.tmux.new_session("alive", 80, 24);
    let format = "#{pid}|#{session_id}|#{s/[$]//:session_id}|#{pane_id}";
    let live = f.tmux.raw(&["display-message", "-p", "-t", "=alive:", format]);
    let live = String::from_utf8(live.stdout).unwrap();
    let fields: Vec<&str> = live.trim().split('|').collect();
    assert_eq!(fields.len(), 4, "{live}");
    assert_eq!(fields[1].strip_prefix('$'), Some(fields[2]), "{live}");
    for target in ["=gone", "=gone:", "%999", "$99", "@99"] {
        let out = f.tmux.raw(&["display-message", "-p", "-t", target, format]);
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.trim().split('|').skip(1).all(str::is_empty), "{target}: {text}");
    }
    let alive = f.tmux.raw(&["has-session", "-t", "=alive"]);
    assert!(alive.status.success(), "the private server must survive every lookup");
}
