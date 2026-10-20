#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]
#[path = "support/t18_native.rs"]
mod native;
mod support;
use comandos_app::{
    config::RunMode,
    term::{
        engine::{Palette, SelectMode, Selection, TermEngine, selected_text},
        shadow::{ShadowReader, capture_when, capture_with},
    },
    tmux::TmuxOut,
};
use std::{
    cell::Cell,
    time::{Duration, Instant},
};
use support::tmux::TestTmux;

fn palette() -> Palette {
    Palette::xterm_default(
        [220, 220, 220],
        [20, 20, 20],
        [220, 220, 220],
        [20, 20, 20],
        [80, 80, 80],
    )
}
fn server_state(f: &support::tmux::Fixture) -> Vec<Vec<u8>> {
    let mut state: Vec<_> = [
        vec!["list-sessions"],
        vec!["list-windows", "-a"],
        vec![
            "list-panes",
            "-a",
            "-F",
            "#{pane_id}|#{pane_width}|#{pane_height}|#{pane_left}|#{pane_top}",
        ],
        vec!["list-clients"],
        vec!["list-buffers"],
        vec!["show-buffer", "-b", "owned"],
        vec!["show-environment", "-g"],
        vec!["show-environment", "-t", "=big"],
        vec!["show-options", "-w", "-t", "=big:"],
        vec!["show-options", "-g"],
        vec!["show-options", "-gw"],
    ]
    .iter()
    .map(|cmd| f.tmux.raw(cmd).stdout)
    .collect();
    let panes = f
        .tmux
        .raw(&["list-panes", "-t", "=big:", "-F", "#{pane_id}"]);
    for pane in String::from_utf8_lossy(&panes.stdout).lines() {
        state.push(
            f.tmux
                .raw(&["capture-pane", "-p", "-e", "-N", "-t", pane])
                .stdout,
        );
    }
    state
}
#[test]
fn all_panes_are_captured_without_clients_geometry_environment_or_buffer_writes() {
    let f = TestTmux::cold_for_mode(RunMode::Shadow).unwrap();
    f.tmux.new_session("big", 163, 44);
    assert!(
        f.tmux
            .raw(&["split-window", "-d", "-h", "-t", "=big:", "/bin/sh"])
            .status
            .success()
    );
    assert!(
        f.tmux
            .raw(&[
                "set-environment",
                "-t",
                "=big",
                "OWNED_SHADOW_SENTINEL",
                "same"
            ])
            .status
            .success()
    );
    assert!(
        f.tmux
            .raw(&["set-buffer", "-b", "owned", "private-buffer-ñ"])
            .status
            .success()
    );
    std::thread::sleep(Duration::from_millis(50));
    let before = server_state(&f);
    let frame = capture_when(&f.ctl, "big", &|| true).unwrap();
    assert_eq!((frame.cols, frame.rows), (163, 44));
    assert!(String::from_utf8_lossy(&frame.bytes).contains('│'));
    let mut engine = TermEngine::new(frame.cols, frame.rows, 0, palette(), false, Instant::now());
    engine.feed(&frame.bytes, Instant::now());
    assert!(engine.drain().replies.is_empty());
    assert_eq!(server_state(&f), before);
    assert!(f.ctl.attach_argv("big").is_empty());
    assert!(f.ctl.viewer_attach_argv("big").is_empty(), "las miniaturas tampoco se enganchan en Sombra");
    // Local resizing of an unrelated real PTY cannot feed back into the source.
    let p = comandos_app::term::pty::PtySession::spawn_with_env(
        &["/bin/cat".into()],
        80,
        24,
        f.config.home(),
        &f.env,
        true,
    )
    .unwrap();
    p.resize(22, 8).unwrap();
    assert_eq!(capture_when(&f.ctl, "big", &|| true).unwrap().cols, 163);
    let after = server_state(&f);
    assert_eq!(after, before);
    use sha2::Digest;
    let hashes = |state: &[Vec<u8>]| {
        state
            .iter()
            .map(|bytes| format!("{:x}", sha2::Sha256::digest(bytes)))
            .collect::<Vec<_>>()
    };
    println!(
        "{}",
        serde_json::json!({"before_sha256":hashes(&before),"after_sha256":hashes(&after),"server_geometry":[163,44],"shadow_clients":0,"pane_captures":before.len()-11})
    );
}
#[test]
fn grouped_capture_composes_ansi_unicode_selection_and_pinned_layout() {
    let f = TestTmux::for_mode(RunMode::Shadow).unwrap();
    f.tmux.new_session("big", 30, 8);
    let reads = Cell::new(0);
    let meta = "123|$1|@2|30|8|fixed|%3|0|0|30|8|2|0|1|1\n";
    let read = |commands: &[Vec<String>], _: Duration, _: &dyn Fn() -> bool| {
        reads.set(reads.get() + 1);
        if reads.get() == 1 {
            assert_eq!(commands.len(), 1);
            assert_eq!(commands[0][2], "=big:");
            Ok(TmuxOut {
                code: 0,
                stdout: meta.into(),
                stderr: String::new(),
            })
        } else {
            assert_eq!(commands.len(), 2);
            assert_eq!(commands[0][5], "%3");
            Ok(TmuxOut {
                code: 0,
                stdout: format!(
                    "\x1b[31mñandú 漢字\nred continued\x1b[0m\n{}{}",
                    "\n".repeat(6),
                    meta
                ),
                stderr: String::new(),
            })
        }
    };
    let frame = capture_with(&f.ctl, "big", &|| true, &read).unwrap();
    assert_eq!(reads.get(), 2);
    let mut engine = TermEngine::new(30, 8, 0, palette(), false, Instant::now());
    engine.feed(&frame.bytes, Instant::now());
    assert_eq!(
        selected_text(
            engine.engine(),
            &Selection {
                anchor: (0, 0),
                head: (0, 8),
                mode: SelectMode::Simple
            }
        ),
        "ñandú 漢字"
    );
    assert_eq!(engine.cursor().col, 2);
    assert_eq!(
        engine.diagnostic_grid(1000).unwrap()["cells"][0][0]["fg"],
        serde_json::json!(palette().ansi[1])
    );
    assert_eq!(
        engine.diagnostic_grid(1000).unwrap()["cells"][1][0]["fg"],
        serde_json::json!(palette().ansi[1])
    );
    let drained = engine.drain();
    assert!(drained.replies.is_empty() && drained.clipboard.is_none());
}
#[test]
fn changed_identity_layout_partial_output_or_cancellation_never_publishes() {
    let f = TestTmux::for_mode(RunMode::Shadow).unwrap();
    f.tmux.new_session("big", 30, 8);
    for after in [
        "123|$1|@9|30|8|fixed|%3|0|0|30|8|2|0|1|1\n",
        "123|$1|@2|30|8|changed|%3|0|0|30|8|2|0|1|1\n",
        "",
    ] {
        let reads = Cell::new(0);
        let before = "123|$1|@2|30|8|fixed|%3|0|0|30|8|2|0|1|1\n";
        let read = |_: &[Vec<String>], _: Duration, _: &dyn Fn() -> bool| {
            reads.set(reads.get() + 1);
            Ok(TmuxOut {
                code: 0,
                stdout: if reads.get() == 1 {
                    before.into()
                } else {
                    format!("{}{after}", "\n".repeat(8))
                },
                stderr: String::new(),
            })
        };
        assert!(capture_with(&f.ctl, "big", &|| true, &read).is_err());
    }
    let reads = Cell::new(0);
    assert!(
        capture_with(&f.ctl, "big", &|| false, &|_, _, _| {
            reads.set(reads.get() + 1);
            unreachable!()
        })
        .is_err()
    );
    assert_eq!(reads.get(), 0);
    assert!(capture_when(&f.ctl, "missing", &|| true).is_err());
    assert!(capture_when(&f.ctl, "big;kill-server", &|| true).is_err());
}
#[test]
fn hidden_reader_has_no_requests_coalesces_visible_work_and_drops_old_generation() {
    let f = TestTmux::for_mode(RunMode::Shadow).unwrap();
    f.tmux.new_session("big", 30, 8);
    let before = server_state(&f);
    let mut reader = ShadowReader::new(f.ctl.clone(), "big".into()).unwrap();
    assert!(!reader.request());
    assert!(reader.take().is_none());
    reader.set_visible(true);
    assert!(reader.request());
    assert!(!reader.request());
    reader.set_visible(false);
    reader.set_visible(true);
    let until = Instant::now() + Duration::from_secs(2);
    let mut requested = false;
    while Instant::now() < until {
        assert!(reader.take().is_none(), "old generation cannot be adopted");
        if reader.request() {
            requested = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(requested);
    let frame = loop {
        if let Some(frame) = reader.take() {
            break frame.unwrap();
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!((frame.cols, frame.rows), (30, 8));
    reader.set_visible(false);
    assert!(!reader.request());
    let start = Instant::now();
    drop(reader);
    assert!(start.elapsed() < Duration::from_millis(500));
    assert_eq!(server_state(&f), before);
}
#[test]
fn live_and_sandbox_retain_native_attach_and_cannot_be_shadow_sources() {
    for mode in [RunMode::Live, RunMode::Sandbox] {
        let f = TestTmux::for_mode(mode).unwrap();
        assert!(f.ctl.attach_argv("__keep")[2].contains("attach -t"));
        assert!(ShadowReader::new(f.ctl.clone(), "__keep".into()).is_err());
        assert!(capture_when(&f.ctl, "__keep", &|| true).is_err());
    }
}
#[test]
fn process_read_cancellation_terminates_only_owned_child_promptly() {
    let start = Instant::now();
    let spec = comandos_app::proc::ProcSpec {
        program: "/bin/sleep".into(),
        args: vec!["5".into()],
        stdin: None,
        env: vec![],
        clear_env: true,
        env_remove: vec![],
        cwd: None,
        timeout: Duration::from_secs(10),
    };
    let result =
        comandos_app::proc::run_when(&spec, &|| start.elapsed() < Duration::from_millis(30));
    assert!(result.is_err());
    assert!(start.elapsed() < Duration::from_millis(500));
}

#[test]
fn actual_view_spawn_send_resize_and_snapshot_methods_use_readonly_transport() {
    let f = TestTmux::for_mode(RunMode::Shadow).unwrap();
    f.tmux.new_session("big", 163, 44);
    let view = include_str!("../src/term/view.rs");
    let methods = [
        "fn spawn(self:",
        "fn send(self:",
        "fn send_event(self:",
        "fn resize(&self,",
        "fn poll_shadow(&self,",
    ]
    .map(|needle| native::body(&view[view.find("impl Inner {").unwrap()..], needle))
    .join("\n");
    let template = include_str!("support/shadow_view_probe.rs.txt");
    let source = format!("{template}\nimpl Inner{{{methods}}}")
        .replace("super::shadow::", "comandos_app::term::shadow::");
    let result = native::execute_source(
        &source,
        &serde_json::json!({"env":f.env,"hooks":f.config.hooks_dir()}),
    );
    assert_eq!(result["shadow_pty"], false);
    assert_eq!(result["visible_snapshot_adopted"], true);
    assert_eq!(result["live_real_pty"], true);
    println!("{result}");
    assert_eq!(f.tmux.session_size("big"), (163, 44));
    assert!(f.tmux.raw(&["list-clients"]).stdout.is_empty());
}

#[test]
fn cancellation_after_each_read_and_metadata_limits_fail_before_adoption() {
    let f = TestTmux::for_mode(RunMode::Shadow).unwrap();
    f.tmux.new_session("big", 30, 8);
    let meta = "123|$1|@2|30|8|fixed|%3|0|0|30|8|2|0|1|1\n";
    for cancel_at in [1, 2] {
        let reads = Cell::new(0);
        let active = Cell::new(true);
        let read = |_: &[Vec<String>], _: Duration, _: &dyn Fn() -> bool| {
            reads.set(reads.get() + 1);
            let stdout = if reads.get() == 1 {
                meta.into()
            } else {
                format!("{}{meta}", "\n".repeat(8))
            };
            if reads.get() == cancel_at {
                active.set(false);
            }
            Ok(TmuxOut {
                code: 0,
                stdout,
                stderr: String::new(),
            })
        };
        assert!(capture_with(&f.ctl, "big", &|| active.get(), &read).is_err());
        assert_eq!(reads.get(), cancel_at);
    }
    for invalid in [
        meta.replace("|30|8|fixed", "|65535|65535|fixed"),
        meta.replace("|%3|", "|%x|"),
        meta.replace("|30|8|2|0|1|1", "|31|8|2|0|1|1"),
        meta.replace("|1|1\n", "|9|1\n"),
    ] {
        let reads = Cell::new(0);
        let read = |_: &[Vec<String>], _: Duration, _: &dyn Fn() -> bool| {
            reads.set(reads.get() + 1);
            Ok(TmuxOut {
                code: 0,
                stdout: invalid.clone(),
                stderr: String::new(),
            })
        };
        assert!(capture_with(&f.ctl, "big", &|| true, &read).is_err());
        assert_eq!(
            reads.get(),
            1,
            "invalid metadata cannot reach grouped capture"
        );
    }
}

#[test]
fn private_tmux_unicode_and_color_reach_the_actual_snapshot_engine() {
    let f = TestTmux::for_mode(RunMode::Shadow).unwrap();
    f.tmux.new_session("big", 30, 8);
    assert!(
        f.tmux
            .raw(&[
                "send-keys",
                "-t",
                "=big:",
                "printf '\\033[2J\\033[H\\033[31mñandú 漢字\\033[0m'; sleep 5",
                "Enter"
            ])
            .status
            .success()
    );
    let end = Instant::now() + Duration::from_secs(2);
    let frame = loop {
        let frame = capture_when(&f.ctl, "big", &|| true).unwrap();
        let mut engine = TermEngine::new(30, 8, 0, palette(), false, Instant::now());
        engine.feed(&frame.bytes, Instant::now());
        let text = selected_text(
            engine.engine(),
            &Selection {
                anchor: (0, 0),
                head: (0, 8),
                mode: SelectMode::Simple,
            },
        );
        if text == "ñandú 漢字" {
            assert_eq!(
                engine.diagnostic_grid(1000).unwrap()["cells"][0][0]["fg"],
                serde_json::json!(palette().ansi[1])
            );
            break frame;
        }
        assert!(
            Instant::now() < end,
            "snapshot text did not preserve private tmux Unicode: {text:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!((frame.cols, frame.rows), (30, 8));
    assert!(f.tmux.raw(&["list-clients"]).stdout.is_empty());
}
