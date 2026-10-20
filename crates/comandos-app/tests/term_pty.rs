#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]
mod support;
use comandos_app::config::RunMode;
use comandos_app::term::pty::{PtySession, ReadOutcome};
use comandos_app::term::settle::{Settle, initial_size};
use std::time::{Duration, Instant};
use support::tmux::TestTmux;

fn read_until(p: &mut PtySession, needle: &str, within: Duration) -> String {
    let mut buf = [0u8; 4096];
    let mut got = Vec::new();
    let end = Instant::now() + within;
    while Instant::now() < end {
        match p.read_chunk(&mut buf) {
            ReadOutcome::Data(n) => got.extend_from_slice(&buf[..n]),
            ReadOutcome::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
            ReadOutcome::Closed => break,
        }
        if String::from_utf8_lossy(&got).contains(needle) {
            break;
        }
    }
    String::from_utf8_lossy(&got).into_owned()
}

#[test]
fn settle_waits_for_quiet_allocation() {
    let mut s = Settle::new(250, 1500, 0);
    s.on_alloc(10);
    s.on_alloc(100); // 174 → 168 → 163 columnas al arrancar
    s.on_alloc(200);
    assert!(!s.due(449));
    assert!(s.due(450));
    assert!(s.fired());
    assert!(!s.fired(), "solo una vez");
    assert_eq!(s.next_check_ms(500), None);
}

#[test]
fn settle_cap_fires_even_while_resizing() {
    let mut s = Settle::new(250, 1500, 0);
    for t in (0..1600).step_by(100) {
        s.on_alloc(t);
        if t < 1500 {
            assert!(!s.due(t), "{t}");
        }
    }
    assert!(s.due(1500));
    assert_eq!(Settle::new(250, 1500, 0).next_check_ms(10), Some(1500));
}

#[test]
fn attach_before_allocation_uses_tmux_size() {
    // Asignación de 1 px (pantalla bloqueada): manda el tamaño que tmux ya tiene.
    assert_eq!(
        initial_size((1, 1), 8.0, 18.0, (20.0, 30.0), Some((163, 44))),
        (163, 44)
    );
    // Sin dato de tmux: 80×24, como VTE.
    assert_eq!(
        initial_size((1, 822), 8.0, 18.0, (20.0, 30.0), None),
        (80, 24)
    );
    // Con tamaño real: (1300 − 20) / 8 = 160 columnas, (822 − 30) / 18 = 44 filas.
    assert_eq!(
        initial_size((1300, 822), 8.0, 18.0, (20.0, 30.0), Some((10, 10))),
        (160, 44)
    );
    // Ventana diminuta: mínimos 2×1.
    assert_eq!(
        initial_size((10, 10), 8.0, 18.0, (20.0, 30.0), None),
        (2, 1)
    );
}

#[test]
fn pty_roundtrip_utf8_and_stty_resize() {
    let home = std::env::temp_dir().join(format!("app-pty-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let env = vec![
        ("HOME".into(), home.display().to_string()),
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("SHELL".into(), "/bin/sh".into()),
    ];
    let mut p = PtySession::spawn_with_env(
        &[
            "/bin/sh".into(),
            "-c".into(),
            r#"stty size; while IFS= read -r line; do printf '%s\n' "$line"; stty size; done"#
                .into(),
        ],
        100,
        30,
        &home,
        &env,
        true,
    )
    .unwrap();
    assert!(
        read_until(&mut p, "30 100", Duration::from_secs(3)).contains("30 100"),
        "tamaño ANTES del primer byte"
    );
    p.write("ñandú 漢字 🚀\n".as_bytes()).unwrap();
    assert!(read_until(&mut p, "🚀", Duration::from_secs(3)).contains("ñandú 漢字 🚀"));
    p.resize(120, 40).unwrap();
    p.write(b"resize-check\n").unwrap();
    assert!(
        read_until(&mut p, "40 120", Duration::from_secs(3)).contains("40 120"),
        "resize observado en el MISMO PTY"
    );
}

#[test]
fn pty_attach_keeps_session_size_until_settled() {
    let Some(f) = TestTmux::for_mode(RunMode::Sandbox) else {
        return;
    };
    // cc-app applies status=off before attaching (bin/cc-app:4067).
    // Configure only this private fixture; default tmux reserves one row.
    assert!(
        f.tmux
            .raw(&["set-option", "-g", "status", "off"])
            .status
            .success()
    );
    f.tmux.new_session("s1", 163, 44);
    let before = f.tmux.session_size("s1");
    // GTK aún no dio tamaño: el primer attach usa el de tmux, no 80×24.
    let (c, r) = initial_size((1, 1), 8.0, 18.0, (20.0, 30.0), f.ctl.window_size("s1"));
    assert_eq!((c, r), (163, 44));
    let p = PtySession::spawn_with_env(
        &f.ctl.attach_argv("s1"),
        c,
        r,
        f.config.home(),
        &f.env,
        true,
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        p.child_tty()
            .is_some_and(|tty| tty.starts_with("/dev/pts/"))
    );
    assert_eq!(
        f.tmux.session_size("s1"),
        before,
        "el attach no encoge la sesión"
    );
    // Cuando la asignación se estabiliza, un solo cambio al tamaño real.
    p.resize(120, 40).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(f.tmux.session_size("s1"), (120, 40));
    drop(p);
    std::thread::sleep(Duration::from_millis(300));
    let clients = f.tmux.raw(&["list-clients", "-F", "#{client_tty}"]);
    assert!(
        String::from_utf8_lossy(&clients.stdout).trim().is_empty(),
        "Drop cuelga el cliente"
    );
}

#[test]
fn claiming_takes_the_window_back_from_a_smaller_client() {
    let Some(f) = TestTmux::for_mode(RunMode::Sandbox) else {
        return;
    };
    assert!(
        f.tmux
            .raw(&["set-option", "-g", "status", "off"])
            .status
            .success()
    );
    f.tmux.new_session("s1", 120, 40);
    let attach = |cols, rows| {
        PtySession::spawn_with_env(&f.ctl.attach_argv("s1"), cols, rows, f.config.home(), &f.env, true)
            .unwrap()
    };
    let desktop = attach(120, 40);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(f.tmux.session_size("s1"), (120, 40));
    // A phone on the same session: with window-size latest the window shrinks to it,
    // and the desktop keeps showing text wrapped at the phone's width.
    let phone = attach(41, 20);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(f.tmux.session_size("s1"), (41, 20));
    // Focusing the desktop tab nudges its client with the same rows and columns.
    desktop.claim_size(120, 40).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(f.tmux.session_size("s1"), (120, 40), "el escritorio recupera la ventana");
    phone.claim_size(41, 20).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(f.tmux.session_size("s1"), (41, 20), "el teléfono la recupera igual");
    desktop.claim_size(120, 40).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(f.tmux.session_size("s1"), (120, 40), "cada empujón avisa, no solo el primero");
    drop(phone);
    drop(desktop);
}

#[test]
fn overview_tiles_never_shrink_the_window() {
    use comandos_app::term::settle::fits_shared_window;
    let Some(f) = TestTmux::for_mode(RunMode::Sandbox) else {
        return;
    };
    assert!(
        f.tmux
            .raw(&["set-option", "-g", "status", "off"])
            .status
            .success()
    );
    f.tmux.new_session("s1", 120, 40);
    let spawn = |argv: Vec<String>, cols, rows| {
        PtySession::spawn_with_env(&argv, cols, rows, f.config.home(), &f.env, true).unwrap()
    };
    let desktop = spawn(f.ctl.attach_argv("s1"), 120, 40);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(f.tmux.session_size("s1"), (120, 40));
    // A mosaic tile 8 columns wide: before, the window (and both of its panes)
    // shrank to it and Claude Code reprinted everything one letter per line.
    let tile = spawn(f.ctl.viewer_attach_argv("s1"), 8, 5);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(f.tmux.session_size("s1"), (120, 40), "la miniatura no encoge la ventana");
    tile.claim_size(8, 5).unwrap();
    tile.resize(9, 4).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(f.tmux.session_size("s1"), (120, 40), "ni al redimensionarse");
    assert!(!fits_shared_window((8, 5)) && !fits_shared_window((80, 4)));
    assert!(fits_shared_window((20, 5)) && fits_shared_window((41, 20)));
    drop(tile);
    drop(desktop);
}

fn confined_shell(script: &str) -> PtySession {
    let home = std::env::temp_dir().join(format!("app-pty-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let env = vec![
        ("HOME".into(), home.display().to_string()),
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("TMUX".into(), "forbidden".into()),
        ("NO_COLOR".into(), "forbidden".into()),
    ];
    PtySession::spawn_with_env(
        &["/bin/sh".into(), "-c".into(), script.into()],
        80,
        24,
        &home,
        &env,
        true,
    )
    .unwrap()
}

#[test]
fn environment_and_empty_reads_are_explicit() {
    let mut p = confined_shell(
        "printf '%s|%s|%s|%s|%s\\n' \"$TERM\" \"$COLORTERM\" \"$VTE_VERSION\" \"${TMUX-unset}\" \"${NO_COLOR-unset}\"",
    );
    assert_eq!(p.read_chunk(&mut []), ReadOutcome::WouldBlock);
    assert!(
        read_until(&mut p, "unset|unset", Duration::from_secs(2))
            .contains("xterm-256color|truecolor|6800|unset|unset")
    );
}

#[test]
fn backpressure_is_bounded_and_preserves_accepted_bytes() {
    use comandos_app::term::pty::MAX_PENDING_BYTES;
    let mut p = confined_shell(
        "stty -echo -icanon; printf 'READY'; sleep 1; dd bs=1 count=200000 2>/dev/null | wc -c",
    );
    assert!(read_until(&mut p, "READY", Duration::from_secs(2)).contains("READY"));
    assert!(p.write(&vec![b'x'; MAX_PENDING_BYTES + 1]).is_err());
    assert!(!p.has_pending(), "oversized input rejected atomically");
    p.write(&vec![b'x'; 200000]).unwrap();
    assert!(p.has_pending(), "stopped reader creates backpressure");
    let end = Instant::now() + Duration::from_secs(5);
    let mut output = String::new();
    while Instant::now() < end {
        p.flush_pending().unwrap();
        output.push_str(&read_until(&mut p, "200000", Duration::from_millis(20)));
        if output.contains("200000") {
            break;
        }
    }
    assert!(output.contains("200000"), "{output}");
    assert!(!p.has_pending());
}

#[test]
fn drop_escalates_only_its_own_group_and_reaps_leader() {
    let mut p = confined_shell("trap '' HUP TERM; printf 'READY'; while :; do sleep 10; done");
    let unrelated = confined_shell("printf 'OTHER'; sleep 10");
    assert!(read_until(&mut p, "READY", Duration::from_secs(2)).contains("READY"));
    let pid = p.pid();
    let start = Instant::now();
    drop(p);
    assert!(
        start.elapsed() < Duration::from_millis(100),
        "Drop cannot wait on child"
    );
    let path = format!("/proc/{pid}");
    let end = Instant::now() + Duration::from_secs(3);
    while std::path::Path::new(&path).exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !std::path::Path::new(&path).exists(),
        "leader remains unreaped"
    );
    assert!(std::path::Path::new(&format!("/proc/{}", unrelated.pid())).exists());
}

#[test]
fn settle_and_dimensions_remain_bounded_at_numeric_limits() {
    let mut s = Settle::new(250, 1500, u64::MAX - 1);
    s.on_alloc(u64::MAX - 1);
    assert!(!s.due(u64::MAX - 1));
    assert!(s.due(u64::MAX));
    assert_eq!(s.next_check_ms(u64::MAX), Some(u64::MAX));
    assert_eq!(
        initial_size((i32::MAX, i32::MAX), 0.0, f64::NAN, (0.0, 0.0), None),
        (u16::MAX, u16::MAX)
    );
}

#[test]
fn normal_exit_can_be_reaped_once_and_closed_read_is_reported() {
    let mut p = confined_shell("printf DONE; exit 7");
    assert!(read_until(&mut p, "DONE", Duration::from_secs(2)).contains("DONE"));
    let end = Instant::now() + Duration::from_secs(2);
    let mut status = None;
    while status.is_none() && Instant::now() < end {
        status = p.try_reap();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(status, Some(7));
    assert_eq!(p.try_reap(), None);
    assert_eq!(p.read_chunk(&mut [0; 10]), ReadOutcome::Closed);
}

#[test]
fn exited_leader_retains_group_cleanup_after_status_observation() {
    let mut p = confined_shell(
        "/bin/sh -c 'trap \"\" HUP TERM; while :; do sleep 10; done' & descendant=$!; printf 'DESC=%s\\n' \"$descendant\"; sleep 0.1; exit 0",
    );
    let text = read_until(&mut p, "DESC=", Duration::from_secs(2));
    let descendant: u32 = text
        .split("DESC=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while p.try_reap().is_none() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(p);
    let end = Instant::now() + Duration::from_secs(2);
    let state = || {
        std::fs::read_to_string(format!("/proc/{descendant}/stat"))
            .ok()
            .and_then(|s| {
                s.rsplit_once(") ")
                    .and_then(|(_, tail)| tail.chars().next())
            })
    };
    while state().is_some_and(|s| s != 'Z') && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    let alive = state().is_some_and(|s| s != 'Z');
    // On RED clean up only the synthetic descendant to avoid leaking it.
    if alive {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(i32::try_from(descendant).unwrap()),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    assert!(!alive, "descendant survived try_reap followed by Drop");
}

#[test]
fn exit_notification_waits_for_output_larger_than_one_dispatch() {
    use comandos_app::term::pty::PtyDrain;
    let mut p = confined_shell("head -c 270000 /dev/zero | tr '\\000' x; printf TAIL; exit 7");
    let mut drain = PtyDrain::default();
    let mut output = Vec::new();
    let end = Instant::now() + Duration::from_secs(5);
    while output.len() < 256000 && Instant::now() < end {
        drain.read_into(&mut p, |bytes| output.extend_from_slice(bytes));
        std::thread::sleep(Duration::from_millis(1));
    }
    while !drain.has_exited() && Instant::now() < end {
        drain.observe_exit(&mut p);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        drain.has_exited(),
        "leader should exit while last bytes remain buffered"
    );
    assert_eq!(drain.finished(), None, "exit callback precedes output EOF");
    while drain.finished().is_none() && Instant::now() < end {
        drain.read_into(&mut p, |bytes| output.extend_from_slice(bytes));
        drain.observe_exit(&mut p);
    }
    assert_eq!(drain.finished(), Some(7));
    assert_eq!(output.len(), 270004);
    assert!(output.ends_with(b"TAIL"));
    assert!(
        output
            .get(..270000)
            .unwrap()
            .iter()
            .all(|byte| *byte == b'x')
    );
}
