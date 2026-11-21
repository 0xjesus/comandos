//! POST/GET `/session/message`: el mensaje de un cliente remoto llega una sola
//! vez, nunca a otro pane, y su recibo sobrevive a un reinicio del frente.
//!
//! Cada pane corre `cat > outN` en el tmux privado de la prueba (`-S`): el
//! archivo dice exactamente lo que la aplicación recibió (una línea por Enter;
//! nada mientras el texto siga en la línea de entrada).
mod support;

use serde_json::Value;
use std::{path::PathBuf, time::Duration};
use support::{FakeLegacy, TestHome, front, get, request_body, run_tmux, tmux_available};

const PATH: &str = "/session/message";

/// `s1` con dos panes (`%0`, `%1`; el activo es `%1`).
fn sink_session(home: &TestHome) -> [PathBuf; 2] {
    let out = [home.root.join("out0"), home.root.join("out1")];
    let cat = |p: &PathBuf| format!("cat > {}", p.display());
    run_tmux(
        home,
        &["new-session", "-d", "-s", "s1", "-x", "120", "-y", "30", &cat(&out[0])],
    );
    run_tmux(home, &["split-window", "-h", "-t", "=s1:", &cat(&out[1])]);
    out
}

fn read(path: &PathBuf) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Espera a que `path` tenga `want` (o vence y devuelve lo que haya).
async fn wait_for(path: &PathBuf, want: &str) -> String {
    for _ in 0..40 {
        if read(path) == want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Un poco más: un segundo pegado tendría tiempo de aparecer.
    tokio::time::sleep(Duration::from_millis(300)).await;
    read(path)
}

fn screen(home: &TestHome, pane: &str) -> String {
    run_tmux(home, &["capture-pane", "-p", "-J", "-t", pane])
}

async fn post(port: u16, body: &str) -> (u16, Value) {
    let wire = request_body(port, "POST", PATH, "", body).await;
    let value = serde_json::from_slice(&wire.body).unwrap_or(Value::Null);
    (wire.status, value)
}

async fn lookup(port: u16, id: &str) -> (u16, Value) {
    let wire = get(port, &format!("{PATH}?clientMessageId={id}")).await;
    let value = serde_json::from_slice(&wire.body).unwrap_or(Value::Null);
    (wire.status, value)
}

#[tokio::test]
async fn delivered_then_duplicate_types_once_even_after_a_restart() {
    if !tmux_available() {
        return;
    }
    let home = TestHome::new("msg-once");
    let out = sink_session(&home);
    let legacy = FakeLegacy::start().await;
    let first_front = front(&home, legacy.port, home.options()).await;
    let body = r#"{"clientMessageId":"m-1","session":"s1","pane":"%0","text":"hola mundo"}"#;
    let (status, first) = post(first_front.port, body).await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["ok"], true);
    assert_eq!(first["status"], "delivered");
    assert_eq!(first["duplicate"], false);
    assert_eq!(first["clientMessageId"], "m-1");
    assert_eq!(first["pane"], "%0");
    assert_eq!(first["chars"], 10);
    assert_eq!(first["deliveredAtMs"], support::NOW_MS);
    let (status, again) = post(first_front.port, body).await;
    assert_eq!(status, 200, "{again}");
    assert_eq!(again["duplicate"], true);
    assert_eq!(again["deliveredAtMs"], first["deliveredAtMs"]);
    assert_eq!(wait_for(&out[0], "hola mundo\n").await, "hola mundo\n", "una sola vez");
    assert_eq!(screen(&home, "%0").matches("hola mundo").count(), 1);
    assert_eq!(read(&out[1]), "", "nada en el otro pane");
    let (status, receipt) = lookup(first_front.port, "m-1").await;
    assert_eq!(status, 200);
    assert_eq!(receipt["status"], "delivered");
    let (status, unknown) = lookup(first_front.port, "nadie-lo-mando").await;
    assert_eq!(status, 404);
    assert_eq!(unknown["status"], "unknown");
    assert_eq!(unknown["ok"], false);
    // El mismo id con otro texto es un error del cliente, nunca un envío.
    let other = r#"{"clientMessageId":"m-1","session":"s1","pane":"%0","text":"otro"}"#;
    let (status, reused) = post(first_front.port, other).await;
    assert_eq!(status, 422, "{reused}");
    assert_eq!(reused["code"], "invalid");
    first_front.stop().await;
    // Otro frente sobre el mismo HOME: el recibo sigue y no se vuelve a pegar.
    let second_front = front(&home, legacy.port, home.options()).await;
    let (status, replayed) = post(second_front.port, body).await;
    assert_eq!(status, 200, "{replayed}");
    assert_eq!(replayed["duplicate"], true);
    assert_eq!(lookup(second_front.port, "m-1").await.0, 200);
    assert_eq!(wait_for(&out[0], "hola mundo\n").await, "hola mundo\n");
    second_front.stop().await;
    assert!(legacy.requests().is_empty(), "nunca se reenvía");
}

#[tokio::test]
async fn a_dead_pane_or_session_is_rejected_and_nothing_is_typed() {
    if !tmux_available() {
        return;
    }
    let home = TestHome::new("msg-gone");
    let out = sink_session(&home);
    let legacy = FakeLegacy::start().await;
    let front = front(&home, legacy.port, home.options()).await;
    let body = r#"{"clientMessageId":"g1","session":"s1","pane":"%99","text":"perdido"}"#;
    let (status, gone) = post(front.port, body).await;
    assert_eq!(status, 404, "{gone}");
    assert_eq!(gone["code"], "pane_gone");
    assert_eq!(gone["status"], "rejected");
    let body = r#"{"clientMessageId":"g2","session":"nadie","text":"perdido"}"#;
    let (status, gone) = post(front.port, body).await;
    assert_eq!(status, 404, "{gone}");
    assert_eq!(gone["code"], "session_gone");
    // Un rechazo no deja recibo: el mismo id se puede reintentar.
    assert_eq!(lookup(front.port, "g1").await.0, 404);
    for bad in [
        r#"{"session":"s1","text":"x"}"#,
        r#"{"clientMessageId":"a b","session":"s1","text":"x"}"#,
        r#"{"clientMessageId":"x","session":"s1","text":"   "}"#,
        r#"{"clientMessageId":"x","session":"s1","pane":"s1","text":"x"}"#,
        r#"{"clientMessageId":"x","session":"s1","text":"x","submit":"si"}"#,
        r#"{"clientMessageId":"x","text":"x"}"#,
        r#"{"clientMessageId":"x","session":"s1","text":"a\u001b[201~b"}"#,
    ] {
        let (status, invalid) = post(front.port, bad).await;
        assert_eq!(status, 422, "{bad}: {invalid}");
        assert_eq!(invalid["code"], "invalid", "{bad}");
    }
    let long = format!(
        r#"{{"clientMessageId":"x","session":"s1","text":"{}"}}"#,
        "y".repeat(20_001)
    );
    assert_eq!(post(front.port, &long).await.0, 422);
    assert_eq!(lookup(front.port, "a%20b").await.0, 400);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(read(&out[0]), "");
    assert_eq!(read(&out[1]), "");
    for pane in ["%0", "%1"] {
        let text = screen(&home, pane);
        assert!(!text.contains("perdido") && !text.contains('y'), "{pane}: {text}");
    }
    front.stop().await;
    assert!(legacy.requests().is_empty(), "nunca se reenvía");
}

#[tokio::test]
async fn multi_line_text_arrives_intact_and_submit_false_waits_in_the_input_line() {
    if !tmux_available() {
        return;
    }
    let home = TestHome::new("msg-lines");
    let out = sink_session(&home);
    let front = front(&home, support::dead_port(), home.options()).await;
    let body = r#"{"clientMessageId":"l1","session":"s1","pane":"%0","text":"uno\ndos\r\ntres"}"#;
    let (status, sent) = post(front.port, body).await;
    assert_eq!(status, 200, "{sent}");
    assert_eq!(sent["chars"], 12);
    assert_eq!(wait_for(&out[0], "uno\ndos\ntres\n").await, "uno\ndos\ntres\n");
    // Sin pane: el activo de la sesión, y el recibo dice cuál.
    let body = r#"{"clientMessageId":"l2","session":"s1","text":"pendiente","submit":false}"#;
    let (status, held) = post(front.port, body).await;
    assert_eq!(status, 200, "{held}");
    assert_eq!(held["pane"], "%1");
    assert_eq!(held["submitted"], false);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(read(&out[1]), "", "sin Enter la aplicación no lo recibe");
    assert!(screen(&home, "%1").contains("pendiente"));
    let body = r#"{"clientMessageId":"l3","session":"s1","pane":"%1","text":"!"}"#;
    assert_eq!(post(front.port, body).await.0, 200);
    assert_eq!(wait_for(&out[1], "pendiente!\n").await, "pendiente!\n");
    assert_eq!(read(&out[0]), "uno\ndos\ntres\n");
    front.stop().await;
}
