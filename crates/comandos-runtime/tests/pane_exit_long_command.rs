//! Long launch commands are read from a private file, never passed through the TTY.
#[path = "support/ops_lab.rs"]
mod ops_lab;
#[path = "support/private_tmux.rs"]
mod private_tmux;
use comandos_runtime::{pane_exit, pane_typing::TmuxResult};
use ops_lab::OpsLab;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn source_path(args: &[&str]) -> Option<PathBuf> {
    if args.first() != Some(&"send-keys") || !args.contains(&"-l") {
        return None;
    }
    let words = comandos_runtime::extension_launch::shlex_split(args.last()?).ok()?;
    (words.len() == 2 && words[0] == ".").then(|| PathBuf::from(&words[1]))
}

fn canonical(tty: &str) -> bool {
    let output = Command::new("stty")
        .args(["-F", tty, "-a"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let modes = String::from_utf8(output.stdout).unwrap();
    modes
        .split_whitespace()
        .any(|mode| mode.trim_end_matches(';') == "icanon")
}

#[test]
fn long_unicode_command_reaches_zsh_after_stty_sane_exactly() {
    let Some(lab) = OpsLab::start("long-zsh-sane") else {
        return;
    };
    let mut env = lab.env();
    pane_exit::send_shell_line(&env, &lab.other, "exec /bin/zsh -f").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while lab.show(&lab.other, "#{pane_current_command}") != "zsh" {
        assert!(Instant::now() < deadline, "private zsh did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    let tty = lab.show(&lab.other, "#{pane_tty}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while canonical(&tty) {
        assert!(
            Instant::now() < deadline,
            "zsh never entered its raw ZLE input mode"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let payload = "á🦀λ'\\$\" ".repeat(2500);
    let output = lab.home.join("exact-output");
    let quote = comandos_core::text::shlex_quote;
    let command = format!(
        "printf %s {} > {}",
        quote(&payload),
        quote(output.to_str().unwrap())
    );
    assert!(command.len() > 24 * 1024);
    let original = env.tmux.clone();
    let transport = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let seen = transport.clone();
    let expected = command.clone();
    env.tmux = Arc::new(move |args| {
        assert!(
            args.iter().all(|arg| arg.len() < 4096),
            "oversized tmux argument"
        );
        if let Some(path) = source_path(args) {
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert!(
                fs::read_to_string(&path)
                    .unwrap()
                    .ends_with(&format!("{expected}\n"))
            );
            seen.lock().unwrap().push(path);
        }
        original(args)
    });
    // Exact production ordering: restore_shell_tty resets ICANON while zsh
    // is already waiting in ZLE. Pasting a 24KiB line at this point truncates it.
    pane_exit::restore_shell_tty(&env, &lab.other).unwrap();
    assert!(
        canonical(&tty),
        "fixture must exercise the production ICANON limit"
    );
    pane_exit::send_shell_line(&env, &lab.other, &command).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if fs::read(&output).ok().as_deref() == Some(payload.as_bytes()) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "zsh after stty sane did not receive exact Unicode bytes"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let paths = transport.lock().unwrap();
    assert_eq!(paths.len(), 1);
    assert!(!paths[0].exists(), "self-deleting command file leaked");
}

fn rejected_transport(stage: &str) {
    let Some(lab) = OpsLab::start(stage) else {
        return;
    };
    let mut env = lab.env();
    let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let captured = calls.clone();
    let files = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let captured_files = files.clone();
    let stage = stage.to_string();
    let injected = stage.clone();
    env.tmux = Arc::new(move |args| {
        captured
            .lock()
            .unwrap()
            .push(args.iter().map(|s| s.to_string()).collect());
        let source = source_path(args);
        if let Some(path) = &source {
            assert!(path.exists(), "script deleted before shell can open it");
            captured_files.lock().unwrap().push(path.clone());
        }
        let enter = args.last() == Some(&"Enter");
        if enter {
            assert!(
                captured_files.lock().unwrap().iter().all(|p| p.exists()),
                "script deleted before Enter"
            );
        }
        if (injected == "send-failure" && source.is_some())
            || (injected == "enter-failure" && enter)
        {
            return Ok(TmuxResult {
                returncode: 1,
                stdout: String::new(),
                stderr: "private injected transport failure".into(),
            });
        }
        // Simulate a shell which never opens the script: no real terminal input.
        Ok(TmuxResult::default())
    });
    let result = pane_exit::send_shell_line(&env, &lab.other, &"x".repeat(25 * 1024));
    assert!(
        result.is_err(),
        "unacknowledged launch must not report success"
    );
    let files = files.lock().unwrap();
    assert_eq!(files.len(), 1, "launch did not use a short source command");
    assert!(
        files.iter().all(|p| !p.exists()),
        "temporary command file leaked after failure"
    );
    let calls = calls.lock().unwrap();
    let enters = calls
        .iter()
        .filter(|args| args.last().map(String::as_str) == Some("Enter"))
        .count();
    assert_eq!(
        enters,
        usize::from(stage != "send-failure"),
        "unexpected Enter after transport failure"
    );
    assert!(
        !calls.iter().any(|a| matches!(
            a.first().map(String::as_str),
            Some("load-buffer" | "paste-buffer")
        )),
        "large command still passes through the TTY"
    );
}
#[test]
fn send_failure_removes_script_without_enter() {
    rejected_transport("send-failure");
}
#[test]
fn enter_failure_removes_script_without_retry() {
    rejected_transport("enter-failure");
}
#[test]
fn shell_ack_timeout_removes_script_without_more_terminal_input() {
    rejected_transport("ack-timeout");
}

#[test]
fn a_relaunch_types_only_a_short_source_line() {
    let Some(lab) = OpsLab::start("short-relaunch") else {
        return;
    };
    let mut env = lab.env();
    pane_exit::send_shell_line(&env, &lab.other, "exec /bin/zsh -f").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while lab.show(&lab.other, "#{pane_current_command}") != "zsh" {
        assert!(Instant::now() < deadline, "private zsh did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = lab.home.join("relaunch-output");
    let quote = comandos_core::text::shlex_quote;
    let flags = " -u CLAUDECODE -u CODEX_HOME -u GROK_HOME".repeat(6);
    let command = format!(
        "env COMANDOS_OPERATION_ID=op-1 env{flags} sh -c {}",
        quote(&format!(
            "printf %s \"$COMANDOS_OPERATION_ID\" > {}",
            quote(output.to_str().unwrap())
        ))
    );
    assert!(command.len() > 250 && command.len() < 4096);
    let original = env.tmux.clone();
    let typed = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = typed.clone();
    env.tmux = Arc::new(move |args| {
        if args.first() == Some(&"send-keys") && args.contains(&"-l") {
            seen.lock().unwrap().push(args.last().unwrap().to_string());
        }
        original(args)
    });
    pane_exit::send_configuration_command(&env, &lab.other, &command).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while fs::read(&output).ok().as_deref() != Some(b"op-1".as_slice()) {
        assert!(Instant::now() < deadline, "relaunch did not run");
        std::thread::sleep(Duration::from_millis(20));
    }
    let typed = typed.lock().unwrap();
    assert_eq!(typed.len(), 1, "{typed:?}");
    assert!(typed[0].starts_with(". ") && typed[0].len() < 80, "{typed:?}");
    assert!(!typed[0].contains("COMANDOS_OPERATION_ID"));
    // Lo corto se sigue tecleando tal cual.
    drop(typed);
    pane_exit::send_configuration_command(&env, &lab.other, "exit").ok();
}
