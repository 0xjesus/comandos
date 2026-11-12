#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use comandos_app::{
    config::RunMode,
    dash_client::DashClient,
    poll::{PollIntervals, PollUpdate, Poller, live_pref_snapshot},
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

#[test]
fn live_pref_snapshot_only_keeps_live_keys() {
    assert_eq!(
        live_pref_snapshot(&json!({
            "font_family": "JetBrains",
            "theme": "noche",
            "button_style": "arcade",
            "favorites": ["a"]
        })),
        json!({
            "font_family": "JetBrains",
            "font_size": null,
            "cursor_shape": null,
            "cursor_blink": null,
            "terminal_padding": null,
            "terminal_opacity": null,
            "theme": "noche",
            "tabs_layout": null
        })
    );
}

#[test]
fn disconnected_poller_stops_without_updates_or_sockets() {
    let client = DashClient::new(None, RunMode::Sandbox).unwrap();
    let generation = Arc::new(AtomicU64::new(42));
    let intervals = PollIntervals {
        state_prefs: Duration::from_millis(20),
        workspace: Duration::from_millis(20),
        marks: Duration::from_millis(20),
        ..PollIntervals::default()
    };
    let (poller, rx) = Poller::start_with_intervals(client, generation.clone(), intervals);
    std::thread::sleep(Duration::from_millis(40));
    poller.stop();
    assert!(rx.try_recv().is_err());
    assert_eq!(generation.load(Ordering::Acquire), 42);
}

#[test]
fn stopping_interrupts_long_idle_intervals_without_waiting_for_next_poll() {
    let client = DashClient::new(None, RunMode::Sandbox).unwrap();
    let intervals = PollIntervals {
        state_prefs: Duration::from_secs(30),
        workspace: Duration::from_secs(30),
        marks: Duration::from_secs(30),
        ..PollIntervals::default()
    };
    let (poller, rx) = Poller::start_with_intervals(client, Arc::new(AtomicU64::new(0)), intervals);
    std::thread::sleep(Duration::from_millis(30));
    let start = Instant::now();
    poller.stop();
    assert!(start.elapsed() < Duration::from_millis(500));
    assert!(rx.try_recv().is_err());
}

#[test]
fn poller_coalesces_repeated_state_and_live_prefs() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let _server = std::thread::spawn(move || {
        for stream in listener.incoming().take(32) {
            let mut stream = stream.unwrap();
            let mut buf = [0u8; 1024];
            let n = std::io::Read::read(&mut stream, &mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let body = if request.contains("/state") {
                r#"{"status":"same"}"#
            } else if request.contains("/prefs") {
                r#"{"theme":"noche","button_style":"ignored"}"#
            } else {
                "null"
            };
            let response = format!(
                "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    let client = DashClient::new(Some(&format!("http://127.0.0.1:{port}")), RunMode::Live).unwrap();
    let intervals = PollIntervals {
        state_prefs: Duration::from_millis(20),
        workspace: Duration::from_millis(100),
        marks: Duration::from_millis(100),
        ..PollIntervals::default()
    };
    let (poller, rx) = Poller::start_with_intervals(client, Arc::new(AtomicU64::new(3)), intervals);
    std::thread::sleep(Duration::from_millis(90));
    poller.stop();
    let updates: Vec<_> = rx.try_iter().collect();
    let state_count = updates
        .iter()
        .filter(|u| matches!(u, PollUpdate::State(_)))
        .count();
    let prefs_count = updates
        .iter()
        .filter(|u| matches!(u, PollUpdate::Prefs { .. }))
        .count();
    assert_eq!(state_count, 1, "{updates:?}");
    assert_eq!(prefs_count, 1, "{updates:?}");
}

#[test]
fn gtk_tick_coalesces_latest_state() {
    let mut batch = comandos_app::poll::PollBatch::default();
    batch.push(PollUpdate::State(json!({"seq":1})));
    batch.push(PollUpdate::Marks(json!({"marks":[]})));
    batch.push(PollUpdate::State(json!({"seq":2})));
    let updates: Vec<_> = batch.drain().collect();
    assert_eq!(
        updates,
        vec![
            PollUpdate::State(json!({"seq":2})),
            PollUpdate::Marks(json!({"marks":[]}))
        ]
    );
}

#[test]
fn favorites_only_changes_and_fresh_generation_reach_gtk() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    let count = Arc::new(AtomicU64::new(0));
    let requests = count.clone();
    let server = std::thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            };
            let mut buffer = [0u8; 2048];
            let n = std::io::Read::read(&mut stream, &mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..n]);
            let body = if request.contains("GET /prefs ") {
                let i = requests.fetch_add(1, Ordering::AcqRel);
                if i == 0 {
                    json!({"theme":"noche","favorites":[]})
                } else {
                    json!({"theme":"noche","favorites":["term-a"]})
                }
            } else {
                json!({})
            }
            .to_string();
            let response = format!(
                "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    let client = DashClient::new(Some(&format!("http://127.0.0.1:{port}")), RunMode::Live).unwrap();
    let generation = Arc::new(AtomicU64::new(0));
    let (poller, rx) = Poller::start_with_intervals(
        client,
        generation.clone(),
        PollIntervals {
            state_prefs: Duration::from_millis(20),
            workspace: Duration::from_secs(1),
            marks: Duration::from_secs(1),
            ..PollIntervals::default()
        },
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut prefs = Vec::new();
    while Instant::now() < deadline {
        if let Ok(PollUpdate::Prefs {
            value,
            favorite_generation,
        }) = rx.recv_timeout(Duration::from_millis(20))
        {
            prefs.push((value, favorite_generation));
            if prefs.len() == 2 {
                generation.store(1, Ordering::Release);
            }
            if prefs.len() == 3 {
                break;
            }
        }
    }
    poller.stop();
    stopped.store(true, Ordering::Release);
    server.join().unwrap();
    assert_eq!(prefs.len(), 3, "{prefs:?}");
    assert_eq!(prefs[0].0["favorites"], json!([]));
    assert_eq!(prefs[1].0["favorites"], json!(["term-a"]));
    assert_eq!(prefs[2].1, 1);
}

#[test]
fn marks_poll_uses_the_route_served_by_backend_and_original() {
    let original = std::env::var("COMANDOS_CC_APP_ORACLE")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../bin/cc-app").into());
    let script = r#"import ast,sys
node=next(n for n in ast.parse(open(sys.argv[1]).read()).body if isinstance(n,ast.FunctionDef) and n.name=='start_work_marks_fetch')
paths=[n.value for n in ast.walk(node) if isinstance(n,ast.Constant) and isinstance(n.value,str) and n.value.startswith('/')]
assert paths==['/work-marks'];print(paths[0])
"#;
    let out = comandos_app::proc::run(&comandos_app::proc::ProcSpec {
        program: "python3".into(),
        args: vec!["-c".into(), script.into(), original.into()],
        stdin: None,
        env: vec![],
        clear_env: false,
        env_remove: vec![],
        cwd: None,
        timeout: Duration::from_secs(5),
    })
    .unwrap();
    assert_eq!(out.code, Some(0));
    let path = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert!(
        include_str!("../../comandos-server/src/dash/native/events.rs")
            .contains(&format!("Key::Path(\"{path}\")"))
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = requests.clone();
    let server = std::thread::spawn(move || {
        while !stopped.load(Ordering::Acquire) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            };
            stream
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            let mut buf = [0u8; 2048];
            let n = std::io::Read::read(&mut stream, &mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let route = request
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .nth(1)
                .unwrap_or("");
            if matches!(route, "/marks" | "/work-marks") {
                seen.lock().unwrap().push(route.to_string());
            }
            let (status, body) = if route == path {
                (
                    "200 OK",
                    r#"{"marks":[{"scope":"session","key":"fixture","mark":"resolved","revision":1}],"panes":[]}"#,
                )
            } else if route == "/marks" {
                ("404 Not Found", "{}")
            } else {
                ("200 OK", "{}")
            };
            let response = format!(
                "HTTP/1.0 {status}\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    let client = DashClient::new(Some(&format!("http://127.0.0.1:{port}")), RunMode::Live).unwrap();
    let (poller, rx) = Poller::start_with_intervals(
        client,
        Arc::new(AtomicU64::new(0)),
        PollIntervals {
            marks: Duration::from_millis(20),
            state_prefs: Duration::from_secs(1),
            workspace: Duration::from_secs(1),
            ..PollIntervals::default()
        },
    );
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut adopted = None;
    while Instant::now() < deadline {
        if let Ok(PollUpdate::Marks(value)) = rx.recv_timeout(Duration::from_millis(20)) {
            adopted = Some(value);
            break;
        }
    }
    poller.stop();
    stop.store(true, Ordering::Release);
    server.join().unwrap();
    assert_eq!(
        adopted,
        Some(
            json!({"marks":[{"scope":"session","key":"fixture","mark":"resolved","revision":1}],"panes":[]})
        ),
        "routes {:?}",
        requests.lock().unwrap()
    );
    assert!(requests.lock().unwrap().iter().all(|p| p != "/marks"));
}
