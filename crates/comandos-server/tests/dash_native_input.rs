//! Corte tabs, T6: teclas, foco, desplazamiento y exportación (`/send`,
//! `/paste`, `/key`, `/focus`, `/kill`, `/tmux-scroll`, `/export`) contra el
//! Python (gemelo).
//!
//! Confinamiento: cada lado del gemelo tiene su HOME temporal corto, su tmux
//! privado (`-S`) y su `fakebin` confinado (`PATH` = solo él). Las sesiones
//! las siembra la prueba por `support::run_tmux` (`-S` al socket privado,
//! entorno confinado) y sus paneles corren `cat`/`sh` del `fakebin`: lo que
//! las rutas teclean o pegan cae ahí y se lee con `capture-pane` por el mismo
//! `-S`. La terminal, `wmctrl`, `xdg-open` y Chrome son falsos del `fakebin`
//! (anotan; el Chrome de la exportación solo copia el HTML). `/kill` solo
//! mata `s1`, sembrada por la prueba, y se prueba que `hub` (400) sigue viva.
//! Cada prueba empieza por `assert_confined` (canario: `-S` privado en el
//! frente y en el guardián del Python) ANTES de cualquier tecla. La limpieza
//! es el `Drop` de cada `TestHome` (`kill-server -S` y después borrar).
mod support;

use serde_json::{Value, json};
use std::time::Duration;
use support::{
    TestHome,
    frozen_twin::{Twin, TwinOpts, normalize},
    oracle::{FakeCall, confined_fakebin},
    run_tmux,
    tabs::{normalize_home, seed_registry},
    twin::tmux_stdin,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

/// `s1` con dos panes `cat` (el activo es el segundo) y `hub`.
fn seed(h: &TestHome) {
    seed_registry(h);
    run_tmux(
        h,
        &[
            "new-session",
            "-d",
            "-s",
            "s1",
            "-x",
            "100",
            "-y",
            "30",
            "cat",
        ],
    );
    run_tmux(h, &["split-window", "-t", "=s1:", "cat"]);
    run_tmux(h, &["new-session", "-d", "-s", "hub", "cat"]);
}

/// Canario: el tmux del frente y el guardián del Python llevan `-S` al
/// socket privado de su HOME; ningún socket es el del usuario.
fn assert_confined(t: &Twin) {
    for home in [&t.a, &t.b] {
        let socket = comandos_server::dash::native::tmux::private_socket(&home.tmux_dir());
        assert!(socket.starts_with(&home.root), "socket fuera del HOME");
        let guard = std::fs::read_to_string(home.root.join("fakebin/tmux")).unwrap();
        assert!(
            guard.contains(&format!("-S '{}'", socket.display())),
            "guardián sin -S privado: {guard}"
        );
        assert!(guard.contains(" -i "), "guardián sin env -i: {guard}");
        for fake in ["kitty", "tilix", "gnome-terminal", "wmctrl", "xdg-open"] {
            let text = std::fs::read_to_string(home.root.join("fakebin").join(fake)).unwrap();
            assert!(text.contains("fakebin.log"), "{fake} no es el falso");
        }
    }
    let opts = &t.front_options;
    assert_eq!(opts.tmux.program.path, t.a.root.join("fakebin/tmux"));
    support::assert_private_tmux(opts);
}

/// Un argumento comparable entre los dos lados: reloj, HOME, buffers de
/// pegado y rutas del `fakebin`.
fn comparable(home: &TestHome, arg: &str) -> String {
    let fakebin = format!("{}/", home.root.join("fakebin").display());
    let arg = arg.replace(&fakebin, "");
    let arg = regex::Regex::new(r"comandos-snip-[0-9a-f]{12}")
        .unwrap()
        .replace_all(&arg, "comandos-snip-X");
    let arg = regex::Regex::new(r"[0-9]{4}-[0-9]{2}-[0-9]{2}_[0-9]{4}")
        .unwrap()
        .replace_all(&arg, "FECHA");
    normalize_home(home, &normalize(&arg))
}

/// Las órdenes de tmux que mutan, comparables, y vacía los registros.
fn take_mutations(t: &Twin) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    let side = |home: &TestHome, calls: Vec<Vec<String>>| -> Vec<Vec<String>> {
        let _ = std::fs::remove_file(home.root.join("tmux.log"));
        calls
            .into_iter()
            .map(|c| c.iter().map(|a| comparable(home, a)).collect())
            .collect()
    };
    let a = t.tmux_mutations_a();
    let b: Vec<Vec<String>> =
        serde_json::from_value(t.observe("input-mutators-comparable", &Value::Null, || {
            let calls = support::twin::tmux_log(&t.b)
                .into_iter()
                .filter(|call| {
                    call.first()
                        .is_some_and(|name| support::twin::TMUX_MUTATORS.contains(&name.as_str()))
                })
                .map(|call| {
                    call.iter()
                        .map(|arg| comparable(&t.b, arg))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            json!(calls)
        }))
        .unwrap();
    (side(&t.a, a), side(&t.b, b))
}

/// Llamadas a los falsos, comparables, y vacía los registros.
/// Llamadas comparables de un lado: nombre y argumentos.
type Calls = Vec<(String, Vec<String>)>;

fn take_fakes(t: &Twin) -> (Calls, Calls) {
    let side = |home: &TestHome, calls: Vec<FakeCall>| {
        let _ = std::fs::remove_file(home.root.join("fakebin.log"));
        calls
            .into_iter()
            .map(|c| {
                let args = c.args.iter().map(|a| comparable(home, a)).collect();
                (c.name, args)
            })
            .collect::<Vec<_>>()
    };
    let a = t.fake_calls_a();
    let b: Vec<FakeCall> = t
        .observe("input-fake-calls-comparable", &Value::Null, || {
            json!(
                support::oracle::fake_calls(&t.b.root)
                    .iter()
                    .map(|call| json!([
                        call.name,
                        call.args
                            .iter()
                            .map(|arg| comparable(&t.b, arg))
                            .collect::<Vec<_>>()
                    ]))
                    .collect::<Vec<_>>()
            )
        })
        .as_array()
        .unwrap()
        .iter()
        .map(|call| FakeCall {
            name: call[0].as_str().unwrap().into(),
            args: serde_json::from_value(call[1].clone()).unwrap(),
        })
        .collect();
    (side(&t.a, a), side(&t.b, b))
}

/// `capture-pane` de los dos lados cuando dejan de cambiar (el `cat` escribe
/// su eco un poco después de que tmux reciba las teclas).
async fn captures(t: &Twin, target: &str) -> (String, String) {
    // The source observation is frozen once; native convergence is measured
    // against it in every mode without polling a synthetic source actor.
    let expected = t
        .observe("stable-pane-capture", &json!(target), || {
            let mut previous = String::new();
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(100));
                let now = run_tmux(&t.b, &["capture-pane", "-p", "-S", "-", "-t", target]);
                if now == previous {
                    return json!(now);
                }
                previous = now;
            }
            json!(previous)
        })
        .as_str()
        .unwrap()
        .to_owned();
    let mut previous = String::new();
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let now = t.tmux_a(&["capture-pane", "-p", "-S", "-", "-t", target]);
        if now == previous && now == expected {
            return (now, expected);
        }
        previous = now;
    }
    (previous, expected)
}

#[tokio::test]
async fn input_routes_match_python() {
    let Some(t) = Twin::start("in", seed).await else {
        return;
    };
    assert_confined(&t);
    for (path, body) in [
        ("/send", r#"{"session":"s1","text":"hola ñ"}"#),
        ("/send", r#"{"session":"s1","text":"   "}"#),
        ("/send", r#"{"session":"s1","text":null}"#),
        ("/send", r#"{"session":"s1","text":42}"#),
        ("/send", r#"{"session":"nadie","text":"x"}"#),
        ("/send", r#"{"session":"mal nombre","text":"x"}"#),
        ("/paste", r#"{"session":"s1","text":"linea1\nlinea2"}"#),
        ("/paste", r#"{"session":"s1","text":" \n "}"#),
        ("/paste", r#"{"session":"nadie","text":"x"}"#),
        ("/key", r#"{"session":"s1","key":"Enter"}"#),
        ("/key", r#"{"session":"s1","key":"F1"}"#),
        ("/key", r#"{"session":"s1","key":1}"#),
        ("/key", r#"{"session":"s1","key":["y"]}"#),
        ("/key", r#"{"session":"s1","key":"y","pane":"%999"}"#),
        ("/key", r#"{"session":"s1","key":"y","pane":"pane"}"#),
        ("/key", r#"{"session":"s1","key":"y","pane":7}"#),
        ("/key", r#"{"session":"nadie","key":"n"}"#),
    ] {
        t.post(path, body).await.assert_same();
    }
    // Recortes de `/send` (4000) y tope de `/paste` (20000), y un pane exacto.
    let long = format!(r#"{{"session":"s1","text":"{}"}}"#, "a".repeat(4100));
    t.post("/send", &long).await.assert_same();
    let huge = format!(r#"{{"session":"s1","text":"{}"}}"#, "b".repeat(20001));
    t.post("/paste", &huge).await.assert_same();
    for (path, body) in [
        ("/paste", r#"{"session":"s1","pane":"%0","text":"al cero"}"#),
        ("/key", r#"{"session":"s1","pane":"%0","key":"Enter"}"#),
        (
            "/send",
            r#"{"session":"s1","pane":"%0","text":"cero otra vez"}"#,
        ),
    ] {
        t.post(path, body).await.assert_same();
    }
    let (a, b) = captures(&t, "=s1:").await;
    assert_eq!(a, b);
    assert!(a.contains("hola ñ") && a.contains("linea2"), "{a}");
    let (a, b) = captures(&t, "%0").await;
    assert_eq!(a, b);
    assert!(a.contains("al cero") && a.contains("cero otra vez"), "{a}");
    assert_eq!(
        t.tmux_a(&["list-buffers"]),
        "",
        "un buffer de pegado quedó vivo"
    );
    // Lo que recibió `load-buffer` por stdin, byte a byte, en los dos lados.
    let pasted = tmux_stdin(&t.a);
    let original_stdin: Vec<Vec<u8>> =
        serde_json::from_value(t.observe("load-buffer-stdin", &Value::Null, || {
            json!(tmux_stdin(&t.b))
        }))
        .unwrap();
    assert_eq!(pasted, original_stdin);
    assert_eq!(
        pasted,
        vec![b"linea1\nlinea2".to_vec(), b"al cero".to_vec()]
    );
    let (a, b) = take_mutations(&t);
    assert!(!a.is_empty());
    assert_eq!(a, b);
    for (path, body) in [
        ("/focus", r#"{"session":"s1"}"#),
        ("/focus", r#"{"session":"nadie"}"#),
        ("/tmux-scroll", r#"{"session":"s1","delta":-5}"#),
        ("/tmux-scroll", r#"{"session":"s1","delta":0}"#),
        ("/kill", r#"{"session":"hub"}"#),
        ("/kill", r#"{"session":"nadie"}"#),
        ("/kill", r#"{"session":"s1"}"#),
        ("/export", r#"{"session":"s1","format":"doc"}"#),
        ("/export", r#"{"session":"s1","format":null}"#),
        ("/export", r#"{"session":"nadie"}"#),
    ] {
        t.post(path, body).await.assert_same();
    }
    let (a, b) = take_mutations(&t);
    assert_eq!(a, b);
    let (a, b) = take_fakes(&t);
    assert_eq!(a, b);
    let sessions = t.tmux_a(&["list-sessions", "-F", "#{session_name}"]);
    assert!(sessions.contains("hub") && !sessions.contains("s1"));
    assert_eq!(
        sessions,
        t.tmux_b(&["list-sessions", "-F", "#{session_name}"])
    );
}

#[tokio::test]
async fn key_dead_pane_never_falls_back() {
    let Some(t) = Twin::start("kdead", seed).await else {
        return;
    };
    assert_confined(&t);
    let before = t.tmux_a(&["capture-pane", "-p", "-t", "=s1:"]);
    let run = t
        .post("/key", r#"{"session":"s1","key":"y","pane":"%77"}"#)
        .await;
    run.assert_same();
    assert_eq!(run.front.status, 404);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(t.tmux_a(&["capture-pane", "-p", "-t", "=s1:"]), before);
    let (a, b) = take_mutations(&t);
    assert!(a.is_empty(), "{a:?}");
    assert_eq!(a, b);
}

/// Un deslizón en la terminal del teléfono abre el historial de tmux (modo
/// copia); ahí las flechas del dock movían el historial y no el menú del
/// agente. `/key` sale del historial antes de teclear; Escape solo sale.
#[tokio::test]
async fn key_leaves_the_history_before_typing() {
    let Some(t) = Twin::start("khist", seed).await else {
        return;
    };
    assert_confined(&t);
    let pane = t.tmux_a(&["display-message", "-p", "-t", "=s1:", "#{pane_id}"]);
    let pane = pane.trim().to_owned();
    let in_mode = || t.tmux_a(&["display-message", "-p", "-t", &pane, "#{pane_in_mode}"]).trim().to_owned();
    t.tmux_a(&["copy-mode", "-t", &pane]);
    assert_eq!(in_mode(), "1");
    let body = format!(r#"{{"session":"s1","pane":"{pane}","key":"Escape"}}"#);
    let run = t.post_front("/key", &body).await;
    assert_eq!(run.status, 200, "{}", run.text());
    assert_eq!(in_mode(), "0", "Escape sale del historial");
    let keys = |log: &[Vec<String>]| -> Vec<String> {
        log.iter()
            .filter(|c| c.first().is_some_and(|v| v == "send-keys"))
            .map(|c| c.join(" "))
            .collect()
    };
    let sent = keys(&support::twin::tmux_log(&t.a));
    assert!(sent.iter().any(|c| c.ends_with("cancel")), "{sent:?}");
    assert!(!sent.iter().any(|c| c.ends_with(" Escape")), "Escape no llega al agente: {sent:?}");
    t.tmux_a(&["copy-mode", "-t", &pane]);
    let body = format!(r#"{{"session":"s1","pane":"{pane}","key":"Enter"}}"#);
    let run = t.post_front("/key", &body).await;
    assert_eq!(run.status, 200, "{}", run.text());
    assert_eq!(in_mode(), "0");
    let sent = keys(&support::twin::tmux_log(&t.a));
    assert!(sent.last().is_some_and(|c| c.ends_with(" Enter")), "{sent:?}");
}

/// Una petición como la manda `comandos-notifyd` (HTTP/1.0, sin token ni
/// `Connection`): solo cuenta el estado.
async fn notifyd_post(port: u16, path: &str, body: &str) -> u16 {
    let wire = format!(
        "POST {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(wire.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), stream.read_to_end(&mut out))
        .await
        .unwrap()
        .unwrap();
    let head = String::from_utf8_lossy(&out);
    head.split(' ').nth(1).unwrap().parse().unwrap()
}

#[tokio::test]
async fn notifyd_requests_get_python_statuses() {
    let Some(t) = Twin::start("nfyd", seed).await else {
        return;
    };
    assert_confined(&t);
    for (path, body) in [
        ("/key", r#"{"session":"s1","key":"y","pane":"%77"}"#),
        ("/key", r#"{"session":"s1","key":"n"}"#),
        ("/send", r#"{"session":"s1","text":"desde notifyd"}"#),
        ("/focus", r#"{"session":"s1"}"#),
    ] {
        let front = notifyd_post(t.front.port, path, body).await;
        let actual = if let Some(port) = t.source_port() {
            Some(notifyd_post(port, path, body).await)
        } else {
            None
        };
        let oracle = t
            .observe(
                "notifyd-http10-status",
                &json!({"path":path,"body":body}),
                || json!(actual.expect("record/check source listener")),
            )
            .as_u64()
            .unwrap() as u16;
        assert_eq!(front, oracle, "{path} {body}");
    }
    let (a, b) = take_mutations(&t);
    assert_eq!(a, b);
}

/// Panes con la app que pide ratón: SGR (`?1006`), UTF-8 (`?1005`) y estándar.
fn seed_mouse(h: &TestHome) {
    seed(h);
    for (name, modes) in [
        ("sgr", r"\033[?1000h\033[?1006h"),
        ("utf", r"\033[?1000h\033[?1005h"),
        ("std", r"\033[?1000h"),
    ] {
        let cmd = format!("printf '{modes}'; exec cat");
        run_tmux(
            h,
            &[
                "new-session",
                "-d",
                "-s",
                name,
                "-x",
                "80",
                "-y",
                "24",
                "sh",
                "-c",
                &cmd,
            ],
        );
    }
    run_tmux(h, &["split-window", "-h", "-t", "=std:", "cat"]);
}

#[tokio::test]
async fn tmux_scroll_matches_python() {
    let Some(t) = Twin::start("scroll", seed_mouse).await else {
        return;
    };
    assert_confined(&t);
    tokio::time::sleep(Duration::from_millis(300)).await;
    for body in [
        r#"{"session":"s1","delta":-3}"#,
        r#"{"session":"s1","delta":-3}"#,
        r#"{"session":"s1","delta":2}"#,
        r#"{"session":"s1","delta":99}"#,
        r#"{"session":"s1","delta":-100}"#,
        r#"{"session":"s1","delta":"abc"}"#,
        r#"{"session":"s1","delta":null}"#,
        r#"{"session":"s1","delta":true}"#,
        r#"{"session":"s1","delta":"7"}"#,
        r#"{"session":"s1","delta":1.9}"#,
        r#"{"session":"s1","delta":-1,"col":3}"#,
        r#"{"session":"s1","delta":-1,"col":3,"row":"x"}"#,
        r#"{"session":"s1","delta":-1,"col":-1,"row":2}"#,
        r#"{"session":"s1","delta":-1,"col":5000,"row":5000}"#,
        r#"{"session":"s1","delta":-1,"col":2,"row":2}"#,
        r#"{"session":"nadie","delta":-1}"#,
        r#"{"session":"nadie","delta":-1,"col":1,"row":1}"#,
        r#"{"session":"mal nombre","delta":-1}"#,
        r#"{"session":["x"],"delta":-1}"#,
        r#"{"session":"sgr","delta":-7,"col":4,"row":2}"#,
        r#"{"session":"sgr","delta":30}"#,
        r#"{"session":"utf","delta":-61,"col":79,"row":20}"#,
        r#"{"session":"std","delta":5,"col":70,"row":3}"#,
        r#"{"session":"std","delta":-2,"col":2,"row":3}"#,
    ] {
        t.post("/tmux-scroll", body).await.assert_same();
        let (a, b) = take_mutations(&t);
        assert_eq!(a, b, "{body}");
    }
}

/// Chrome falso de la exportación: copia el HTML al PDF pedido; con «NOPDF»
/// en el HTML falla sin PDF y deja una línea en stderr.
const FAKE_CHROME: &str = r#"#!/bin/sh
printf '%s\0' google-chrome "$@" "$(printf '\036')" >> "$HOME/fakebin.log"
out=""
for a in "$@"; do
  case "$a" in --print-to-pdf=*) out="${a#--print-to-pdf=}";; esac
  last="$a"
done
if grep -q SINSALIDA "$last"; then
  exit 1
fi
if grep -q NOPDF "$last"; then
  echo "  chrome se cayó  " >&2
  exit 3
fi
cp "$last" "$out"
"#;

fn seed_export(h: &TestHome) {
    seed(h);
    confined_fakebin(h, &[("google-chrome".into(), FAKE_CHROME.into())]);
    std::fs::create_dir_all(h.root.join("Downloads")).unwrap();
    let detail = "# Título <x>\n**negrita** y *cursiva*, `código` [enlace](http://e.x)\n\n| a | b |\n|---|:-:|\n| 1 & 2 | 'q' |\n```\nlet x = \"<y>\";\n```\n- uno\n  * dos\n---\nfin";
    for (file, record) in [
        (
            "a.json",
            json!({"session":"s1","status":"waiting","detail":detail,"ts":5}),
        ),
        (
            "b.json",
            json!({"session":"s2","project":"Mi proyecto/ñ","status":"waiting","last":"NOPDF\nsolo last","ts":4}),
        ),
        (
            "d.json",
            json!({"session":"s4","status":"waiting","detail":"SINSALIDA","ts":2}),
        ),
        (
            "c.json",
            json!({"session":"s3","status":"waiting","detail":"","ts":3}),
        ),
    ] {
        std::fs::write(h.hooks().join("state").join(file), record.to_string()).unwrap();
    }
}

/// Los archivos exportados de un HOME (nombre comparable → contenido).
fn exported(home: &TestHome) -> Vec<(String, String)> {
    let when = regex::Regex::new(r"[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}").unwrap();
    let mut out = Vec::new();
    for dir in [home.root.clone(), home.root.join("Downloads")] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("claude-") {
                continue;
            }
            let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
            let text = when.replace_all(&text, "CUANDO").into_owned();
            out.push((comparable(home, &name), text));
        }
    }
    out.sort();
    out
}

#[tokio::test]
async fn export_matches_python() {
    let opts = TwinOpts {
        response_normalizer: Some(("input-original-date-buffer-home", comparable)),
        fakebin_extra: vec![("google-chrome".into(), FAKE_CHROME.into())],
        oracle_env: vec![("TZ".into(), "America/Mexico_City".into())],
        ..TwinOpts::default()
    };
    let Some(t) = Twin::start_with("export", seed_export, opts).await else {
        return;
    };
    assert_confined(&t);
    for body in [
        r#"{"session":"s1"}"#,
        r#"{"session":"s1","format":"pdf"}"#,
        r#"{"session":"s2","format":"pdf"}"#,
        r#"{"session":"s2","format":"txt"}"#,
        r#"{"session":"s3","format":"txt"}"#,
        r#"{"session":"s4","format":"pdf"}"#,
    ] {
        let run = t.post("/export", body).await;
        assert_eq!(run.front.status, run.oracle.status, "{body}");
        let (front, oracle) = (body_of(&t.a, &run.front), body_of(&t.b, &run.oracle));
        assert_eq!(front, oracle, "{body}");
    }
    let expected: Vec<(String, String)> =
        serde_json::from_value(
            t.observe("exported-documents", &Value::Null, || json!(exported(&t.b))),
        )
        .unwrap();
    assert_eq!(exported(&t.a), expected);
    assert_eq!(exported(&t.a).len(), 3, "{:?}", exported(&t.a));
    let (a, b) = take_fakes(&t);
    assert_eq!(a, b);
    assert!(a.iter().any(|(name, _)| name == "xdg-open"), "{a:?}");
}

/// Cuerpo comparable (ruta del HOME y fecha normalizadas).
fn body_of(home: &TestHome, wire: &support::Wire) -> Value {
    let text = comparable(home, &wire.text());
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

#[test]
fn md_to_html_matches_python() {
    if !support::tmux_available() {
        // `TestHome` necesita su directorio de tmux; sin tmux no hay gemelo.
        return;
    }
    let home = TestHome::new_short("md");
    confined_fakebin(&home, &[]);
    let cases = [
        "",
        "hola",
        "# título\n## dos\n####### siete\n#sin espacio\n###### seis",
        "**a** **b** ***c*** ** d **",
        "*x* (*y*) a*b*c *z*, *w*! *v*? *u*: *t*; *s*. *r*)",
        " *x*\u{a0}*y*\u{1f}*z*\u{1f}",
        "*a\u{b}b* `c` ``d`` `e",
        "[a](b) [c](d e) [f]( g) [h]()",
        "| a | b |\n|---|---|\n| 1 | 2 |\n|  :-: | - |\n|x|\n||\n| a | b |   \n  | c |",
        "|---|\n| - |\n| x |",
        "```\n<a href=\"x\">'y'</a>\n  ```\n**fuera**",
        "```rust\nsin cerrar",
        "- uno\n* dos\n  - tres\n-cuatro\n\t* cinco",
        "---\n  -----  \n- - -\n--",
        "a\r\nb\rc\u{2028}d\u{1c}e\u{85}f",
        "<b>&amp;</b> \"q\" 'p'",
        "**neg *cur* neg** `**no**`",
        "ñandú *ñ* **ü** [ç](x)",
        "* *",
        "|a|b|\ntexto\n|c|d|",
    ];
    let dir = home.root.join("md");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("cases.json"), json!(cases).to_string()).unwrap();
    let code = format!(
        "import json\ncases = json.loads({cases})\nprint(json.dumps([dash.md_to_html(c) for c in cases]))",
        cases = serde_json::to_string(&json!(cases).to_string()).unwrap()
    );
    let out = support::http_golden::dash_files(
        &home,
        "server-input-markdown",
        &[],
        &code,
        &Default::default(),
    );
    let python: Vec<String> = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(python.len(), cases.len());
    for (case, want) in cases.iter().zip(&python) {
        assert_eq!(
            &comandos_server::dash::native::input::md_to_html(case),
            want,
            "caso {case:?}"
        );
    }
}

/// Cómputos de `/state` del frente: cada uno lee el inventario de panes una
/// vez (`list-panes -a -F <PANE_FORMAT>`).
fn computations(home: &TestHome) -> usize {
    support::twin::tmux_log(home)
        .iter()
        .filter(|call| {
            call.first().is_some_and(|v| v == "list-panes")
                && call.iter().any(|a| a == "-a")
                && call
                    .iter()
                    .any(|a| a == comandos_runtime::agent_procs::PANE_FORMAT)
        })
        .count()
}

#[tokio::test]
async fn export_burst_shares_one_state_computation() {
    // Ronda 1: `/export` relee `/state` dentro del vuelo único de su caché.
    // Con un tmux lento en el inventario, una ráfaga de exportaciones y
    // sondeos concurrentes hace un solo cómputo (nunca dos a la vez, así el
    // rastreador de configuración no se pisa).
    let opts = TwinOpts {
        response_normalizer: Some(("input-original-date-buffer-home", comparable)),
        fakebin_extra: vec![("google-chrome".into(), FAKE_CHROME.into())],
        oracle_env: vec![("TZ".into(), "America/Mexico_City".into())],
        front: Some(Box::new(|o| {
            // Un `tmux` que tarda 0,5 s en `list-panes -a` y delega en el
            // guardián (mismo prefijo `-f /dev/null -S <socket>` del frente).
            let guard = o.tmux.program.path.clone();
            let dir = guard
                .parent()
                .and_then(|p| p.parent())
                .unwrap()
                .join("slowbin");
            std::fs::create_dir_all(&dir).unwrap();
            let slow = dir.join("tmux");
            let sleep = support::oracle::real_program("sleep").unwrap();
            std::fs::write(
                &slow,
                format!(
                    "#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = -a ] && {} 0.5; done\nexec {} \"$@\"\n",
                    sleep.display(),
                    guard.display()
                ),
            )
            .unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755)).unwrap();
            o.tmux.program.path = slow;
        })),
        ..TwinOpts::default()
    };
    let Some(t) = Twin::start_with("burst", seed_export, opts).await else {
        return;
    };
    assert_confined_burst(&t);
    // Una sola exportación: cuántas lecturas del inventario hace un cómputo.
    let one = t.post_front("/export", r#"{"session":"s1"}"#).await;
    assert_eq!(one.status, 200, "{}", one.text());
    let per = computations(&t.a);
    assert!(per >= 1);
    let _ = std::fs::remove_file(t.a.root.join("tmux.log"));
    let port = t.front.port;
    let export = || support::request_body(port, "POST", "/export", "", r#"{"session":"s1"}"#);
    let poll = || support::get(port, "/state");
    let (a, b, c, d, e, f) = tokio::join!(export(), export(), export(), poll(), export(), poll());
    for wire in [&a, &b, &c, &e] {
        assert_eq!(wire.status, 200, "{}", wire.text());
    }
    for wire in [&d, &f] {
        assert_eq!(wire.status, 200, "{}", wire.text());
    }
    assert_eq!(computations(&t.a), per, "la ráfaga hizo más de un cómputo");
}

/// El canario de `assert_confined`, con el `tmux` lento delante del guardián.
fn assert_confined_burst(t: &Twin) {
    for home in [&t.a, &t.b] {
        let socket = comandos_server::dash::native::tmux::private_socket(&home.tmux_dir());
        let guard = std::fs::read_to_string(home.root.join("fakebin/tmux")).unwrap();
        assert!(guard.contains(&format!("-S '{}'", socket.display())));
        assert!(guard.contains(" -i "));
    }
    let opts = &t.front_options;
    assert_eq!(opts.tmux.program.path, t.a.root.join("slowbin/tmux"));
    let slow = std::fs::read_to_string(&opts.tmux.program.path).unwrap();
    assert!(slow.contains(&t.a.root.join("fakebin/tmux").display().to_string()));
    support::assert_private_tmux(opts);
}
