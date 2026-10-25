use comandos_server::dash::term::attach::{AttachCommand, valid_session};
#[test]
fn closed_commands_and_exact_names() {
    for command in [
        AttachCommand::Version,
        AttachCommand::HasSession("s".into()),
        AttachCommand::ListSessionNames,
        AttachCommand::Attach {
            session: "s".into(),
            active_pane: true,
            ignore_size: false,
        },
        AttachCommand::Attach {
            session: "s".into(),
            active_pane: true,
            ignore_size: true,
        },
    ] {
        let args = command.args().join(" ");
        for forbidden in [
            "kill",
            "new-session",
            "set",
            "source",
            "unlink",
            "respawn",
            "detach -a",
            "server",
        ] {
            assert!(!args.contains(forbidden), "{args}");
        }
    }
    assert!(comandos_server::dash::term::attach::supports_active_pane(
        "tmux 3.2a"
    ));
    assert!(comandos_server::dash::term::attach::supports_active_pane(
        "tmux 4.0"
    ));
    assert!(!comandos_server::dash::term::attach::supports_active_pane(
        "tmux 3.1"
    ));
    assert!(!comandos_server::dash::term::attach::supports_active_pane(
        "unavailable"
    ));
    assert!(valid_session("a.b_c-12"));
    for name in ["", "=s", "s;x", "s:1", "%42", "s\n"] {
        assert!(!valid_session(name));
    }
    assert!(!valid_session(&"s".repeat(81)));
    assert_eq!(
        AttachCommand::Attach {
            session: "s".into(),
            active_pane: true,
            ignore_size: false,
        }
        .args(),
        ["attach", "-f", "active-pane", "-t", "=s"]
    );
    assert_eq!(
        AttachCommand::Attach {
            session: "s".into(),
            active_pane: true,
            ignore_size: true,
        }
        .args(),
        ["attach", "-f", "active-pane,ignore-size", "-t", "=s"]
    );
    // tmux before 3.2 has no `attach -f`: the flag is dropped with active-pane.
    assert_eq!(
        AttachCommand::Attach {
            session: "s".into(),
            active_pane: false,
            ignore_size: true,
        }
        .args(),
        ["attach", "-t", "=s"]
    );
}
#[path = "support/private_tmux.rs"]
mod private_tmux;
use comandos_server::dash::term::bridge::{BridgeLimits, Source, run_bridge};
use comandos_term::proto::{Dialect, Init, TTY_PREFS};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::watch,
    time::{sleep, timeout},
};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
const WAIT: Duration = Duration::from_secs(4);
async fn pair() -> (WebSocketStream<TcpStream>, WebSocketStream<TcpStream>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accept = async {
        tokio_tungstenite::accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap()
    };
    let connect = async {
        tokio_tungstenite::client_async(
            format!("ws://{addr}"),
            TcpStream::connect(addr).await.unwrap(),
        )
        .await
        .unwrap()
        .0
    };
    tokio::join!(accept, connect)
}
async fn wait_clients(tmux: &private_tmux::PrivateTmux, n: usize) {
    timeout(WAIT, async {
        while tmux.clients() != n {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
fn init() -> Init {
    Init {
        cols: 100,
        rows: 30,
        session: Some("t1".into()),
        ignore_size: false,
    }
}
#[tokio::test]
async fn attach_resize_raw_bytes_and_disconnect_reaps_only_client() {
    let Some(tmux) = private_tmux::PrivateTmux::start(&["t1", "t10"]) else {
        return;
    };
    let (server, mut client) = pair().await;
    let (_stop, rx) = watch::channel(false);
    let task = tokio::spawn(run_bridge(
        server,
        Dialect::V1,
        init(),
        Source::Pty {
            target: tmux.target(),
            session: None,
        },
        BridgeLimits::default(),
        rx,
    ));
    assert!(
        client
            .next()
            .await
            .unwrap()
            .unwrap()
            .into_data()
            .starts_with(b"1cc-webterm-attach (")
    );
    assert_eq!(
        client.next().await.unwrap().unwrap().into_data().as_ref(),
        comandos_term::proto::prefs(TTY_PREFS)
    );
    wait_clients(&tmux, 1).await;
    assert_eq!(tmux.size(), "100x30");
    client
        .send(Message::text("1{\"columns\":120,\"rows\":40}"))
        .await
        .unwrap();
    timeout(WAIT, async {
        while tmux.size() != "120x40" {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    client
        .send(Message::binary(b"0printf 'bridge-raw-ok\\n'\r".to_vec()))
        .await
        .unwrap();
    timeout(WAIT, async {
        while !tmux.capture().contains("bridge-raw-ok") {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    drop(client);
    let report = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert!(report.child_reaped);
    let pid = report.child_pid.unwrap();
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    wait_clients(&tmux, 0).await;
    assert!(tmux.tmux(&["has-session", "-t", "=t1"]).status.success());
    assert!(tmux.tmux(&["has-session", "-t", "=t10"]).status.success());
}
#[tokio::test]
async fn picker_ignores_reserved_and_invalid_session_cannot_escape() {
    let Some(tmux) =
        private_tmux::PrivateTmux::start(&["t1", "local", "hub", "control", "z with space"])
    else {
        return;
    };
    let (server, mut client) = pair().await;
    let (stop, rx) = watch::channel(false);
    let task = tokio::spawn(run_bridge(
        server,
        Dialect::Tty,
        init(),
        Source::Pty {
            target: tmux.target(),
            session: Some("nope".into()),
        },
        BridgeLimits::default(),
        rx,
    ));
    let mut text = String::new();
    timeout(WAIT, async {
        while !text.contains("Numero") {
            text.push_str(&String::from_utf8_lossy(
                &client.next().await.unwrap().unwrap().into_data(),
            ));
        }
    })
    .await
    .unwrap();
    assert!(text.contains("La sesion 'nope' ya no existe."));
    assert!(text.contains("1) t1"));
    assert!(text.contains("2) z with space"));
    assert!(!text.contains(") local"));
    assert_eq!(tmux.clients(), 0);
    client
        .send(Message::binary(b"02\r".to_vec()))
        .await
        .unwrap();
    wait_clients(&tmux, 1).await;
    assert_eq!(
        String::from_utf8_lossy(&tmux.tmux(&["list-clients", "-F", "#{session_name}"]).stdout)
            .trim(),
        "z with space"
    );
    stop.send(true).unwrap();
    let report = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert!(report.child_reaped);
    wait_clients(&tmux, 0).await;
}
#[tokio::test]
async fn replay_exact_bytes_and_shutdown() {
    let home = private_tmux::TestHome::new();
    let path = home.root.join("replay.bin");
    let data = b"\xff\0\x1b[31mraw\r\n";
    std::fs::write(&path, data).unwrap();
    let (server, mut client) = pair().await;
    let (stop, rx) = watch::channel(false);
    let task = tokio::spawn(run_bridge(
        server,
        Dialect::V1,
        init(),
        Source::Replay(path),
        BridgeLimits {
            outbox_bytes: 64,
            read_chunk: 16,
        },
        rx,
    ));
    let mut found = false;
    for _ in 0..3 {
        let msg = timeout(WAIT, client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if msg.into_data().as_ref() == comandos_term::proto::output(data) {
            found = true
        }
    }
    assert!(found);
    stop.send(true).unwrap();
    let stats = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(stats.child_pid, None);
    assert!(stats.max_outbox_bytes <= 64);
}
#[tokio::test]
async fn slow_client_bounds_outbox_and_shutdown_reaps() {
    let Some(tmux) = private_tmux::PrivateTmux::start(&["t1"]) else {
        return;
    };
    let (server, mut client) = pair().await;
    let (stop, rx) = watch::channel(false);
    let limits = BridgeLimits {
        outbox_bytes: 64 * 1024,
        read_chunk: 16 * 1024,
    };
    let task = tokio::spawn(run_bridge(
        server,
        Dialect::Tty,
        init(),
        Source::Pty {
            target: tmux.target(),
            session: None,
        },
        limits,
        rx,
    ));
    for _ in 0..2 {
        client.next().await.unwrap().unwrap();
    }
    wait_clients(&tmux, 1).await;
    assert!(
        tmux.tmux(&["send-keys", "-t", "=t1:0.0", "yes huge-output", "Enter"])
            .status
            .success()
    );
    sleep(Duration::from_millis(700)).await;
    stop.send(true).unwrap();
    let report = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert!(report.output_bytes > limits.outbox_bytes);
    assert!(report.max_outbox_bytes <= limits.outbox_bytes);
    assert!(report.child_reaped);
    wait_clients(&tmux, 0).await;
    // Stop the generator through its private pane; the session itself survives.
    tmux.tmux(&["send-keys", "-t", "=t1:0.0", "C-c"]);
    assert!(tmux.tmux(&["has-session", "-t", "=t1"]).status.success());
}
#[tokio::test]
async fn pause_resume_oversized_paste_and_bad_protocol() {
    let Some(tmux) = private_tmux::PrivateTmux::start(&["t1"]) else {
        return;
    };
    let (server, mut client) = pair().await;
    let (_stop, rx) = watch::channel(false);
    let task = tokio::spawn(run_bridge(
        server,
        Dialect::Tty,
        init(),
        Source::Pty {
            target: tmux.target(),
            session: None,
        },
        BridgeLimits::default(),
        rx,
    ));
    for _ in 0..2 {
        client.next().await.unwrap().unwrap();
    }
    wait_clients(&tmux, 1).await;
    // Drain startup output, then use resize as a server-side barrier after pause.
    client.send(Message::text("2")).await.unwrap();
    client
        .send(Message::text("1{\"columns\":121,\"rows\":41}"))
        .await
        .unwrap();
    timeout(WAIT, async {
        while tmux.size() != "121x41" {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    while timeout(Duration::from_millis(40), client.next())
        .await
        .is_ok()
    {}
    tmux.tmux(&[
        "send-keys",
        "-t",
        "=t1:0.0",
        "printf 'paused-marker\\n'",
        "Enter",
    ]);
    assert!(
        timeout(Duration::from_millis(120), client.next())
            .await
            .is_err()
    );
    client.send(Message::text("3")).await.unwrap();
    let mut text = String::new();
    timeout(WAIT, async {
        while !text.contains("paused-marker") {
            text.push_str(&String::from_utf8_lossy(
                &client.next().await.unwrap().unwrap().into_data(),
            ));
        }
    })
    .await
    .unwrap();
    client
        .send(Message::binary(vec![
            b'0';
            comandos_term::proto::MAX_CLIENT_FRAME
                + 1
        ]))
        .await
        .unwrap();
    client
        .send(Message::text("1{\"columns\":122,\"rows\":42}"))
        .await
        .unwrap();
    timeout(WAIT, async {
        while tmux.size() != "122x42" {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    client.send(Message::text("unknown")).await.unwrap();
    let close = timeout(WAIT, async {
        loop {
            let msg = client.next().await.unwrap().unwrap();
            if let Message::Close(frame) = msg {
                break frame.unwrap();
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(u16::from(close.code), 1003);
    let stats = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert!(stats.child_reaped);
    wait_clients(&tmux, 0).await;
}
#[tokio::test]
async fn invalid_target_is_picker_not_shell_interpolation_or_prefix_attach() {
    let Some(tmux) = private_tmux::PrivateTmux::start(&["t1", "t10"]) else {
        return;
    };
    let (server, mut client) = pair().await;
    let (_stop, rx) = watch::channel(false);
    let task = tokio::spawn(run_bridge(
        server,
        Dialect::Tty,
        init(),
        Source::Pty {
            target: tmux.target(),
            session: Some("t1;touch escaped".into()),
        },
        BridgeLimits::default(),
        rx,
    ));
    let mut text = String::new();
    timeout(WAIT, async {
        while !text.contains("Numero") {
            text.push_str(&String::from_utf8_lossy(
                &client.next().await.unwrap().unwrap().into_data(),
            ));
        }
    })
    .await
    .unwrap();
    assert!(!text.contains("La sesion"));
    assert_eq!(tmux.clients(), 0);
    assert!(!tmux.home.root.join("home/escaped").exists());
    drop(client);
    assert_eq!(
        timeout(WAIT, task)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .child_pid,
        None
    );
    assert!(tmux.tmux(&["has-session", "-t", "=t1"]).status.success());
    assert!(tmux.tmux(&["has-session", "-t", "=t10"]).status.success());
}
// A deterministic sink that never flushes its first output frame. It exposes
// every submitted frame, rather than trusting the bridge's own counters alone.
struct BlockedSocket {
    frames: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl futures_util::Stream for BlockedSocket {
    type Item = Result<Message, tokio_tungstenite::tungstenite::Error>;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Pending
    }
}
impl futures_util::Sink<Message> for BlockedSocket {
    type Error = tokio_tungstenite::tungstenite::Error;
    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn start_send(self: std::pin::Pin<&mut Self>, _: Message) -> Result<(), Self::Error> {
        self.frames
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        if self.frames.load(std::sync::atomic::Ordering::SeqCst) > 2 {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(Ok(()))
        }
    }
    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
}
#[tokio::test]
async fn blocked_sink_submits_one_output_and_shutdown_interrupts_flush() {
    let home = private_tmux::TestHome::new();
    let path = home.root.join("large-replay.bin");
    std::fs::write(&path, vec![b'x'; 1024 * 1024]).unwrap();
    let frames = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let socket = BlockedSocket {
        frames: frames.clone(),
    };
    let (stop, rx) = watch::channel(false);
    let task = tokio::spawn(run_bridge(
        socket,
        Dialect::V1,
        init(),
        Source::Replay(path),
        BridgeLimits {
            outbox_bytes: 4096,
            read_chunk: 16384,
        },
        rx,
    ));
    timeout(WAIT, async {
        while frames.load(std::sync::atomic::Ordering::SeqCst) < 3 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    sleep(Duration::from_millis(30)).await;
    assert_eq!(frames.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert!(!task.is_finished());
    stop.send(true).unwrap();
    let stats = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(stats.output_bytes, 4095);
    assert_eq!(stats.max_outbox_bytes, 4096);
}
#[tokio::test]
async fn no_sessions_uses_private_home_shell_and_reaps_on_exit() {
    let home = private_tmux::TestHome::new();
    let (server, mut client) = pair().await;
    let (_stop, rx) = watch::channel(false);
    let task = tokio::spawn(run_bridge(
        server,
        Dialect::V1,
        Init {
            cols: 80,
            rows: 24,
            session: None,
            ignore_size: false,
        },
        Source::Pty {
            target: comandos_server::dash::term::attach::TmuxTarget::Private(home.root.clone()),
            session: None,
        },
        BridgeLimits::default(),
        rx,
    ));
    let mut text = String::new();
    timeout(WAIT, async {
        while !text.contains("No hay sesiones.") {
            text.push_str(&String::from_utf8_lossy(
                &client.next().await.unwrap().unwrap().into_data(),
            ));
        }
    })
    .await
    .unwrap();
    client
        .send(Message::binary(
            b"0printf 'isolated-home:%s\\n' \"$HOME\"\r".to_vec(),
        ))
        .await
        .unwrap();
    let expected = format!("isolated-home:{}", home.root.join("home").display());
    timeout(WAIT, async {
        while !text.contains(&expected) {
            text.push_str(&String::from_utf8_lossy(
                &client.next().await.unwrap().unwrap().into_data(),
            ));
        }
    })
    .await
    .unwrap();
    client
        .send(Message::binary(b"0exit\r".to_vec()))
        .await
        .unwrap();
    let stats = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert!(stats.child_reaped);
    assert!(!std::path::Path::new(&format!("/proc/{}", stats.child_pid.unwrap())).exists());
}
#[tokio::test]
async fn disconnect_interrupts_valid_paste_into_stopped_private_client() {
    let Some(tmux) = private_tmux::PrivateTmux::start(&["t1"]) else {
        return;
    };
    let (server, mut client) = pair().await;
    let (_stop, rx) = watch::channel(false);
    let task = tokio::spawn(run_bridge(
        server,
        Dialect::Tty,
        init(),
        Source::Pty {
            target: tmux.target(),
            session: None,
        },
        BridgeLimits::default(),
        rx,
    ));
    for _ in 0..2 {
        client.next().await.unwrap().unwrap();
    }
    wait_clients(&tmux, 1).await;
    // This PID comes exclusively from list-clients on this test's explicit -S.
    let output = tmux.tmux(&["list-clients", "-F", "#{client_pid}"]);
    let pid = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGSTOP,
    )
    .unwrap();
    let mut paste = vec![b'x'; comandos_term::proto::MAX_INPUT + 1];
    paste[0] = b'0';
    client.send(Message::binary(paste)).await.unwrap();
    sleep(Duration::from_millis(100)).await;
    assert!(!task.is_finished());
    drop(client);
    let stats = timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(stats.child_pid, Some(pid as u32));
    assert!(stats.child_reaped);
    wait_clients(&tmux, 0).await;
    assert!(tmux.tmux(&["has-session", "-t", "=t1"]).status.success());
}
#[tokio::test]
async fn overloaded_input_reaps_stopped_private_client_before_global_shutdown() {
    let Some(tmux) = private_tmux::PrivateTmux::start(&["t1"]) else {
        return;
    };
    let (server, mut client) = pair().await;
    let (_stop, rx) = watch::channel(false);
    let task = tokio::spawn(run_bridge(
        server,
        Dialect::Tty,
        init(),
        Source::Pty {
            target: tmux.target(),
            session: None,
        },
        BridgeLimits::default(),
        rx,
    ));
    for _ in 0..2 {
        client.next().await.unwrap().unwrap();
    }
    wait_clients(&tmux, 1).await;
    let pid = String::from_utf8(tmux.tmux(&["list-clients", "-F", "#{client_pid}"]).stdout)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGSTOP,
    )
    .unwrap();
    let mut paste = vec![b'x'; comandos_term::proto::MAX_INPUT + 1];
    paste[0] = b'0';
    client.send(Message::binary(paste.clone())).await.unwrap();
    sleep(Duration::from_millis(50)).await;
    client.send(Message::binary(paste)).await.unwrap();
    // Exactly two full pastes are followed by Close; no global shutdown is sent.
    client.send(Message::Close(None)).await.unwrap();
    let mut close = None;
    timeout(WAIT, async {
        while let Some(msg) = client.next().await {
            match msg {
                Ok(Message::Close(frame)) => {
                    close = frame;
                    break;
                }
                Err(_) => break,
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(u16::from(close.unwrap().code), 1009);
    let stats = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert!(stats.child_reaped);
    assert_eq!(stats.child_pid, Some(pid as u32));
    wait_clients(&tmux, 0).await;
    assert!(tmux.tmux(&["has-session", "-t", "=t1"]).status.success());
}
#[tokio::test]
async fn picker_backspace_and_del_keep_trailing_crlf_command() {
    for (erase, split) in [(8u8, false), (127, false), (127, true)] {
        let Some(tmux) = private_tmux::PrivateTmux::start(&["t1", "t10"]) else {
            return;
        };
        let (server, mut client) = pair().await;
        let (_stop, rx) = watch::channel(false);
        let task = tokio::spawn(run_bridge(
            server,
            Dialect::Tty,
            init(),
            Source::Pty {
                target: tmux.target(),
                session: Some("missing".into()),
            },
            BridgeLimits::default(),
            rx,
        ));
        let mut text = String::new();
        timeout(WAIT, async {
            while !text.contains("Numero") {
                text.push_str(&String::from_utf8_lossy(
                    &client.next().await.unwrap().unwrap().into_data(),
                ));
            }
        })
        .await
        .unwrap();
        let mut input = vec![b'0', b'9', erase, b'1'];
        if split {
            input.push(b'\r');
            client.send(Message::binary(input)).await.unwrap();
            client
                .send(Message::binary(b"0\nprintf 'picker-tail-ok\\n'\r".to_vec()))
                .await
                .unwrap();
        } else {
            input.extend_from_slice(b"\r\nprintf 'picker-tail-ok\\n'\r");
            client.send(Message::binary(input)).await.unwrap();
        }
        timeout(WAIT, async {
            while !tmux.capture().contains("picker-tail-ok") {
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(tmux.clients(), 1);
        drop(client);
        assert!(
            timeout(WAIT, task)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .child_reaped
        );
    }
}
#[tokio::test]
async fn child_exit_drains_final_pty_bytes_before_close() {
    let home = private_tmux::TestHome::new();
    let (server, mut client) = pair().await;
    let (_stop, rx) = watch::channel(false);
    let task = tokio::spawn(run_bridge(
        server,
        Dialect::V1,
        Init {
            cols: 80,
            rows: 24,
            session: None,
            ignore_size: false,
        },
        Source::Pty {
            target: comandos_server::dash::term::attach::TmuxTarget::Private(home.root.clone()),
            session: None,
        },
        BridgeLimits {
            outbox_bytes: 4096,
            read_chunk: 64,
        },
        rx,
    ));
    let mut data = Vec::new();
    timeout(WAIT, async {
        while !String::from_utf8_lossy(&data).contains("No hay sesiones.") {
            data.extend_from_slice(&client.next().await.unwrap().unwrap().into_data());
        }
    })
    .await
    .unwrap();
    client.send(Message::text("2")).await.unwrap();
    client
        .send(Message::binary(
            b"0printf '\\106\\111\\116\\101\\114\\137\\104\\122\\101\\111\\116'; exit\r".to_vec(),
        ))
        .await
        .unwrap();
    timeout(WAIT, async {
        while let Some(msg) = client.next().await {
            match msg {
                Ok(Message::Binary(bytes)) => data.extend_from_slice(&bytes[1..]),
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert!(
        String::from_utf8_lossy(&data).contains("FINAL_DRAIN"),
        "{}",
        String::from_utf8_lossy(&data)
    );
    assert!(
        timeout(WAIT, task)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .child_reaped
    );
}
#[tokio::test]
async fn selector_ctrl_u_ctrl_w_and_ctrl_c_do_not_open_wrong_shell() {
    for input in [b"099\x151\r".as_slice(), b"01 88\x17\x08\r".as_slice()] {
        let Some(tmux) = private_tmux::PrivateTmux::start(&["t1"]) else {
            return;
        };
        let (server, mut client) = pair().await;
        let (_stop, rx) = watch::channel(false);
        let task = tokio::spawn(run_bridge(
            server,
            Dialect::Tty,
            init(),
            Source::Pty {
                target: tmux.target(),
                session: Some("missing".into()),
            },
            BridgeLimits::default(),
            rx,
        ));
        let mut text = String::new();
        timeout(WAIT, async {
            while !text.contains("Numero") {
                text.push_str(&String::from_utf8_lossy(
                    &client.next().await.unwrap().unwrap().into_data(),
                ));
            }
        })
        .await
        .unwrap();
        client.send(Message::binary(input.to_vec())).await.unwrap();
        wait_clients(&tmux, 1).await;
        drop(client);
        assert!(
            timeout(WAIT, task)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .child_reaped
        );
    }
    let Some(tmux) = private_tmux::PrivateTmux::start(&["t1"]) else {
        return;
    };
    let (server, mut client) = pair().await;
    let (_stop, rx) = watch::channel(false);
    let task = tokio::spawn(run_bridge(
        server,
        Dialect::Tty,
        init(),
        Source::Pty {
            target: tmux.target(),
            session: Some("missing".into()),
        },
        BridgeLimits::default(),
        rx,
    ));
    let mut text = String::new();
    timeout(WAIT, async {
        while !text.contains("Numero") {
            text.push_str(&String::from_utf8_lossy(
                &client.next().await.unwrap().unwrap().into_data(),
            ));
        }
    })
    .await
    .unwrap();
    client
        .send(Message::binary(b"01\x03\rprintf should-not-run\r".to_vec()))
        .await
        .unwrap();
    let close = timeout(WAIT, async {
        loop {
            if let Some(Ok(Message::Close(Some(frame)))) = client.next().await {
                break frame;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(u16::from(close.code), 1000);
    let stats = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(stats.child_pid, None);
    assert_eq!(tmux.clients(), 0);
    assert!(tmux.tmux(&["has-session", "-t", "=t1"]).status.success());
}
#[tokio::test]
async fn scoped_private_attach_and_free_shell_preserve_interaction_and_reap() {
    use comandos_server::dash::term::attach::TmuxTarget;
    let Some(tmux) = private_tmux::PrivateTmux::start(&["t1"]) else {
        return;
    };
    let source = tmux.home.root.join("scope_fixture.rs");
    let binary = tmux.home.root.join("scope-fixture");
    std::fs::write(&source,r#"use std::{env,fs,process::Command,os::unix::process::CommandExt};fn main(){let a:Vec<_>=env::args().skip(1).collect();assert_eq!(&a[..5],["--user","--scope","--collect","--quiet","--"]);let home=env::var("HOME").unwrap();fs::write(format!("{home}/scope-args"),a.join("\n")).unwrap();assert!(a[5]=="tmux"||a[5]=="/bin/sh");if a[5]=="tmux"{assert!(a.iter().any(|v|v=="-S"));}let error=Command::new(&a[5]).args(&a[6..]).exec();panic!("{error}");}"#).unwrap();
    assert!(
        std::process::Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap()
            .status
            .success()
    );
    for shell in [false, true] {
        let _ = std::fs::remove_file(tmux.home.root.join("home/scope-args"));
        let (server, mut client) = pair().await;
        let (_stop, rx) = watch::channel(false);
        let mut initial = init();
        if shell {
            initial.session = None;
        }
        let task = tokio::spawn(run_bridge(
            server,
            Dialect::Tty,
            initial,
            Source::Pty {
                target: TmuxTarget::PrivateScoped(tmux.home.root.clone(), binary.clone()),
                session: if shell { None } else { Some("t1".into()) },
            },
            BridgeLimits::default(),
            rx,
        ));
        client.next().await.unwrap().unwrap();
        client.next().await.unwrap().unwrap();
        if shell {
            client.next().await.unwrap().unwrap();
            client.send(Message::binary(b"0\r".to_vec())).await.unwrap();
        }
        timeout(WAIT, client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        client
            .send(Message::binary(b"0printf '%s%s\\n' scoped -ok\r".to_vec()))
            .await
            .unwrap();
        timeout(WAIT, async {
            loop {
                let msg = client.next().await.unwrap().unwrap();
                if String::from_utf8_lossy(&msg.into_data()).contains("scoped-ok") {
                    break;
                }
            }
        })
        .await
        .unwrap();
        timeout(WAIT, async {
            loop {
                if tmux.home.root.join("home/scope-args").is_file() {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let record = std::fs::read_to_string(tmux.home.root.join("home/scope-args")).unwrap();
        assert!(record.starts_with("--user\n--scope\n--collect\n--quiet\n--\n"));
        if shell {
            assert!(record.ends_with("/bin/sh"));
        } else {
            assert!(record.contains("\n-S\n"));
            assert!(record.ends_with("=t1"));
        }
        client.close(None).await.unwrap();
        assert!(
            timeout(WAIT, task)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .child_reaped
        );
        wait_clients(&tmux, 0).await;
        assert!(tmux.tmux(&["has-session", "-t", "=t1"]).status.success());
    }
}

/// A phone that crops one pane of a desktop split attaches with `ignore-size`: its
/// typing and resizes leave the shared window at the desktop's size.
#[tokio::test]
async fn an_ignore_size_phone_never_sizes_the_shared_window() {
    let Some(tmux) = private_tmux::PrivateTmux::start(&["t1"]) else {
        return;
    };
    assert!(tmux.tmux(&["set", "-g", "window-size", "latest"]).status.success());
    let window = || {
        String::from_utf8_lossy(
            &tmux
                .tmux(&["display", "-p", "-t", "=t1:", "#{window_width}x#{window_height}"])
                .stdout,
        )
        .trim()
        .to_owned()
    };
    let target = tmux.target();
    let bridge = |init: Init| {
        let target = target.clone();
        async move {
        let (server, mut client) = pair().await;
        let (stop, rx) = watch::channel(false);
        let task = tokio::spawn(run_bridge(
            server,
            Dialect::V1,
            init,
            Source::Pty {
                target,
                session: None,
            },
            BridgeLimits::default(),
            rx,
        ));
        client.next().await.unwrap().unwrap();
        client.next().await.unwrap().unwrap();
        (client, stop, task)
        }
    };
    let (mut desktop, _d, desktop_task) = bridge(init()).await;
    wait_clients(&tmux, 1).await;
    timeout(WAIT, async {
        while window() != "100x29" {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let (mut phone, _p, phone_task) = bridge(Init {
        cols: 45,
        rows: 30,
        session: Some("t1".into()),
        ignore_size: true,
    })
    .await;
    wait_clients(&tmux, 2).await;
    phone.send(Message::binary(b"0printf 'phone-typed\\n'\r".to_vec())).await.unwrap();
    phone.send(Message::text("1{\"columns\":41,\"rows\":20}")).await.unwrap();
    timeout(WAIT, async {
        while !tmux.capture().contains("phone-typed") {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    sleep(Duration::from_millis(300)).await;
    assert_eq!(window(), "100x29", "the phone must not take the window");
    desktop.send(Message::text("1{\"columns\":120,\"rows\":40}")).await.unwrap();
    timeout(WAIT, async {
        while window() != "120x39" {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    drop(phone);
    drop(desktop);
    timeout(WAIT, phone_task).await.unwrap().unwrap().unwrap();
    timeout(WAIT, desktop_task).await.unwrap().unwrap().unwrap();
    wait_clients(&tmux, 0).await;
}
