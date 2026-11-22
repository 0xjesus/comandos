//! Dominio D: preferencias, pestañas y ratón de tmux respondidos por Rust.
//! El heredado es un puerto muerto: si el frente reenviara, sería 502.
mod support;

use comandos_server::dash::native::tmux::{Program, Tmux};
use std::{ffi::OsString, fs, time::Duration};
use support::http_golden::FrozenHttp;
use support::{FakeLegacy, TestHome, dead_port, front, get, request_body, tmux_available};

const MONO: &str = r#"{"family": "Monospace", "label": "Monospace del sistema", "a11y": false}"#;

fn new_tmux_sessions(home: &TestHome, names: &[&str]) {
    for name in names {
        let ok = home
            .tmux_command()
            .args(["new-session", "-d", "-s", name, "cat"])
            .status()
            .unwrap()
            .success();
        assert!(ok, "tmux new-session {name}");
    }
}

#[tokio::test]
async fn prefs_get_fills_defaults_after_file_keys() {
    let home = TestHome::new("prefs-get");
    home.write(
        "prefs.json",
        r#"{"theme": "dia", "favorites": ["a"], "zzz": 1}"#,
    );
    let front = front(&home, dead_port(), home.options()).await;
    let wire = get(front.port, "/prefs").await;
    assert_eq!(wire.status, 200);
    assert_eq!(wire.header("content-type"), Some("application/json"));
    assert_eq!(wire.header("cache-control"), Some("no-store"));
    assert_eq!(
        wire.text(),
        format!(
            r#"{{"theme": "dia", "favorites": ["a"], "zzz": 1, "font_family": "Ubuntu Sans Mono", "font_size": 13, "cursor_shape": "block", "cursor_blink": true, "terminal_padding": 8, "terminal_opacity": 100, "ligatures": true, "button_style": "sutil", "tabs_layout": "row", "fonts": [{MONO}]}}"#
        )
    );
    front.stop().await;
}

#[tokio::test]
async fn prefs_get_lists_installed_catalog_fonts() {
    let home = TestHome::new("prefs-fonts");
    let mut opts = home.options();
    // printf con dos `%.0s` se traga ": family".
    opts.fc_list = Program {
        path: "printf".into(),
        prefix: vec![OsString::from(
            "Hack\\nFira Code,Fira Code Retina\\n%.0s%.0s",
        )],
        env: vec![],
        env_remove: vec![],
        env_clear: false,
    };
    let front = front(&home, dead_port(), opts).await;
    let body = get(front.port, "/prefs").await.text();
    assert!(
        body.ends_with(&format!(
            r#""fonts": [{{"family": "Fira Code", "label": "Fira Code", "a11y": false}}, {{"family": "Hack", "label": "Hack", "a11y": false}}, {MONO}]}}"#
        )),
        "{body}"
    );
    front.stop().await;
}

#[tokio::test]
async fn prefs_set_applies_python_rules_and_writes_python_bytes() {
    let home = TestHome::new("prefs-set");
    home.write(
        "prefs.json",
        r#"{"favorites": ["b", "local", 3, "b"], "theme": "dia"}"#,
    );
    let front = front(&home, dead_port(), home.options()).await;
    let body = r#"{"favorite": {"session": "s1", "enabled": true}, "nfDismiss": {"k": 0}, "nfSnooze": {"x": 5, "y": true, "z": "n"}, "font_size": 99.5, "terminal_padding": -3, "theme": "neon", "notif_pos": "tr", "font_family": "  Hack  "}"#;
    let wire = request_body(front.port, "POST", "/prefs-set", "", body).await;
    assert_eq!(wire.status, 200, "{}", wire.text());
    assert_eq!(wire.text(), r#"{"ok": true, "favorites": ["b", "s1"]}"#);
    assert_eq!(
        fs::read_to_string(home.hooks().join("prefs.json")).unwrap(),
        r#"{"favorites": ["b", "s1"], "theme": "neon", "font_family": "Hack", "font_size": 28, "cursor_shape": "block", "cursor_blink": true, "terminal_padding": 0, "terminal_opacity": 100, "ligatures": true, "button_style": "sutil", "tabs_layout": "row", "nfDismiss": {"k": 1}, "nfSnooze": {"x": 5.0, "y": 1.0}, "notif_pos": "tr"}"#
    );
    front.stop().await;
}

#[tokio::test]
async fn prefs_set_errors_are_pythons() {
    let home = TestHome::new("prefs-err");
    let front = front(&home, dead_port(), home.options()).await;
    // `ensure_ascii`: los textos con tilde viajan como `\u00e1`.
    for (body, status, text) in [
        (
            r#"{"favorite": {"session": "local", "enabled": true}}"#,
            400,
            r#"{"error": "Favorito inv\u00e1lido"}"#,
        ),
        (
            r#"{"favorite": {"session": "x", "enabled": 1}}"#,
            400,
            r#"{"error": "Favorito inv\u00e1lido"}"#,
        ),
        (
            r#"{"font_size": NaN}"#,
            400,
            r#"{"error": "cannot convert float NaN to integer"}"#,
        ),
        (
            r#"{"font_size": Infinity}"#,
            500,
            r#"{"error": "Error interno del tablero"}"#,
        ),
    ] {
        let wire = request_body(front.port, "POST", "/prefs-set", "", body).await;
        assert_eq!(
            (wire.status, wire.text().as_str()),
            (status, text),
            "{body}"
        );
    }
    assert!(
        !home.hooks().join("prefs.json").exists(),
        "ningún error escribe"
    );
    let many: Vec<String> = (0..200).map(|i| format!("\"f{i}\"")).collect();
    home.write(
        "prefs.json",
        &format!("{{\"favorites\": [{}]}}", many.join(", ")),
    );
    let wire = request_body(
        front.port,
        "POST",
        "/prefs-set",
        "",
        r#"{"favorite": {"session": "otra", "enabled": true}}"#,
    )
    .await;
    assert_eq!(
        wire.text(),
        r#"{"error": "M\u00e1ximo de 200 favoritos alcanzado"}"#
    );
    front.stop().await;
}

#[tokio::test]
async fn prefs_set_unwritable_is_503() {
    use std::os::unix::fs::PermissionsExt;
    let home = TestHome::new("prefs-503");
    let front = front(&home, dead_port(), home.options()).await;
    fs::set_permissions(home.hooks(), fs::Permissions::from_mode(0o500)).unwrap();
    let wire = request_body(front.port, "POST", "/prefs-set", "", r#"{"theme": "neon"}"#).await;
    fs::set_permissions(home.hooks(), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(wire.status, 503);
    assert_eq!(
        wire.text(),
        r#"{"error": "No se pudieron guardar las preferencias"}"#
    );
    front.stop().await;
}

#[tokio::test]
async fn tabs_and_history_follow_registry_favorites_and_tmux() {
    if !tmux_available() {
        eprintln!("tmux no está instalado: se salta");
        return;
    }
    let home = TestHome::new("tabs");
    new_tmux_sessions(&home, &["local", "s1", "s2", "hub"]);
    home.write(
        "app-tabs.json",
        r#"{"s1": "Uno", "s2": "Dos", "hub": "H", "muerta": "M"}"#,
    );
    home.write("prefs.json", r#"{"favorites": ["s2"]}"#);
    home.write(
        "app-tabs-history.json",
        r#"[{"session": "s1"}, {"session": "vieja", "label": 5, "cwd": "rel", "ts": 12.9}, {"session": "s2x", "cwd": "/tmp", "agent": "codex", "reason": "crash", "ts": "7"}, "basura", {"session": "a b"}]"#,
    );
    let front = front(&home, dead_port(), home.options()).await;
    assert_eq!(
        get(front.port, "/tabs").await.text(),
        r#"[{"session": "local", "label": "\u2302 local", "closable": false}, {"session": "s2", "label": "Dos"}, {"session": "s1", "label": "Uno"}]"#
    );
    assert_eq!(
        get(front.port, "/tab-history").await.text(),
        r#"[{"session": "vieja", "label": "5", "cwd": "", "agent": "claude", "ts": 12, "reason": "closed", "alive": false}, {"session": "s2x", "label": "s2x", "cwd": "/tmp", "agent": "codex", "ts": 7, "reason": "crash", "alive": false}]"#
    );
    front.stop().await;
}

/// 21-nov-2026: «Salta a…» ofrecía «↺ Proyecto · recuperar» de una pestaña
/// cerrada junto al proyecto abierto del mismo nombre, y tocarla abría una
/// segunda pestaña igual. Una cerrada que se llama como una abierta no sale.
#[tokio::test]
async fn history_hides_closed_tabs_named_like_an_open_one() {
    if !tmux_available() {
        eprintln!("tmux no está instalado: se salta");
        return;
    }
    let home = TestHome::new("tabs-same-name");
    new_tmux_sessions(&home, &["local", "s1"]);
    home.write("app-tabs.json", r#"{"s1": "Proyecto"}"#);
    home.write(
        "app-tabs-history.json",
        r#"[{"session": "v1", "label": "PROYECTO", "cwd": "/p", "ts": 3}, {"session": "v2", "label": "Otro", "cwd": "/p", "ts": 2}]"#,
    );
    let front = front(&home, dead_port(), home.options()).await;
    assert_eq!(
        get(front.port, "/tab-history").await.text(),
        r#"[{"session": "v2", "label": "Otro", "cwd": "/p", "agent": "claude", "ts": 2, "reason": "closed", "alive": false}]"#
    );
    front.stop().await;
}

/// La llave de la voz de la app: sale del archivo del equipo, sin caché; sin
/// archivo (o sin una llave clara en él), 404. Una llave falsa armada en la
/// prueba: ninguna llave real en el repositorio.
#[tokio::test]
async fn voice_key_comes_from_the_desktop_file_only() {
    let home = TestHome::new("voice-key");
    let front = front(&home, dead_port(), home.options()).await;
    let wire = get(front.port, "/voice/openai-key").await;
    assert_eq!(wire.status, 404, "{}", wire.text());
    assert!(!wire.text().contains("sk-"), "{}", wire.text());
    let fake = format!("sk-{}", "prueba".repeat(4));
    let dir = home.root.join(".config/comandos");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("openai.key"), format!("OPENAI_API_KEY=\"{fake}\"\n")).unwrap();
    let wire = get(front.port, "/voice/openai-key").await;
    assert_eq!(wire.status, 200, "{}", wire.text());
    assert_eq!(wire.text(), format!(r#"{{"key": "{fake}"}}"#));
    assert_eq!(wire.header("cache-control"), Some("no-store"));
    // Desde fuera (Tailscale reenvía con X-Forwarded-For) sin token: 401, y
    // la llave no sale. Con el token del panel, sí.
    let outside = |token: String| {
        let port = front.port;
        async move {
            let wire = format!(
                "GET /voice/openai-key HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
                 X-Forwarded-For: 100.64.0.9\r\n{token}Connection: close\r\n\r\n"
            );
            let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut stream, wire.as_bytes()).await.unwrap();
            let mut out = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut out).await.unwrap();
            String::from_utf8_lossy(&out).into_owned()
        }
    };
    let denied = outside(String::new()).await;
    assert!(denied.starts_with("HTTP/1.1 401"), "{denied}");
    assert!(!denied.contains("sk-"), "{denied}");
    let allowed = outside(format!("X-Comandos-Token: {}\r\n", support::TOKEN)).await;
    assert!(allowed.starts_with("HTTP/1.1 200"), "{allowed}");
    assert!(allowed.contains(&fake), "con token");
    // Dos llaves distintas: no se adivina cuál.
    fs::write(dir.join("openai.key"), format!("{fake}\n{fake}otra\n")).unwrap();
    assert_eq!(get(front.port, "/voice/openai-key").await.status, 404);
    front.stop().await;
}

#[tokio::test]
async fn tab_models_and_active_tab() {
    let home = TestHome::new("models");
    home.write(
        "app-tab-models.json",
        r#"{"s1": {"panes": [{"pane": "%1"}]}, "mala": [1], "vacia": {}}"#,
    );
    let front = front(&home, dead_port(), home.options()).await;
    assert_eq!(
        get(front.port, "/tab-models?session=s1").await.text(),
        r#"{"session": "s1", "panes": [{"pane": "%1"}]}"#
    );
    assert_eq!(
        get(front.port, "/tab-models?session=vacia").await.text(),
        r#"{"session": "vacia", "panes": []}"#
    );
    assert_eq!(
        get(front.port, "/tab-models").await.text(),
        r#"{"session": "", "panes": []}"#
    );
    let bad = get(front.port, "/tab-models?session=mala").await;
    assert_eq!(
        (bad.status, bad.text().as_str()),
        (500, r#"{"error": "Error interno del tablero"}"#)
    );
    assert_eq!(get(front.port, "/active-tab").await.text(), "{}");
    home.write("app-tab-active.json", "[1]");
    assert_eq!(get(front.port, "/active-tab").await.status, 500);
    if tmux_available() {
        new_tmux_sessions(&home, &["s1"]);
        home.write("app-tab-active.json", r#"{"session": "s1", "x": 1}"#);
        let text = get(front.port, "/active-tab").await.text();
        assert!(
            text.starts_with(r#"{"session": "s1", "x": 1, "pane": "%"#),
            "{text}"
        );
    }
    front.stop().await;
}

#[tokio::test]
async fn tmux_mouse_get_and_set() {
    if !tmux_available() {
        eprintln!("tmux no está instalado: se salta");
        return;
    }
    let home = TestHome::new("mouse");
    new_tmux_sessions(&home, &["s1"]);
    let front = front(&home, dead_port(), home.options()).await;
    assert_eq!(
        get(front.port, "/tmux-mouse?session=s1").await.text(),
        r#"{"mouse": "off"}"#
    );
    let set = request_body(
        front.port,
        "POST",
        "/tmux-mouse",
        "",
        r#"{"session": "s1", "enabled": true}"#,
    )
    .await;
    assert_eq!(set.text(), r#"{"ok": true, "mouse": "on"}"#);
    assert_eq!(
        get(front.port, "/tmux-mouse?session=s1").await.text(),
        r#"{"mouse": "on"}"#
    );
    let none = get(front.port, "/tmux-mouse?session=nope").await;
    assert_eq!(
        (none.status, none.text().as_str()),
        (404, r#"{"error": "No hay sesion tmux 'nope'"}"#)
    );
    let bad = get(front.port, "/tmux-mouse?session=a%20b").await;
    assert_eq!(
        (bad.status, bad.text().as_str()),
        (400, r#"{"error": "Nombre de sesion invalido"}"#)
    );
    let typed = request_body(front.port, "POST", "/tmux-mouse", "", r#"{"session": 5}"#).await;
    assert_eq!(typed.status, 500, "SESSION_RE.match(int) es TypeError");
    front.stop().await;
}

#[tokio::test]
async fn tmux_missing_messages() {
    let home = TestHome::new("tmux-missing");
    let mut opts = home.options();
    opts.tmux = Tmux {
        program: Program::named("/no-existe/tmux"),
        timeout: Duration::from_secs(5),
    };
    let front = front(&home, dead_port(), opts).await;
    let caught = get(front.port, "/tmux-mouse?session=s1").await;
    assert_eq!(
        (caught.status, caught.text().as_str()),
        (
            500,
            r#"{"error": "[Errno 2] No such file or directory: 'tmux'"}"#
        )
    );
    let uncaught = get(front.port, "/tabs").await;
    assert_eq!(
        (uncaught.status, uncaught.text().as_str()),
        (500, r#"{"error": "Error interno del tablero"}"#)
    );
    front.stop().await;
}

#[tokio::test]
async fn tmux_hung_times_out() {
    let home = TestHome::new("tmux-hung");
    let mut opts = home.options();
    opts.tmux = Tmux {
        program: Program {
            path: "tail".into(),
            prefix: vec!["-f".into(), "/dev/null".into(), "--".into()],
            env: vec![],
            env_remove: vec![],
            env_clear: false,
        },
        timeout: Duration::from_millis(300),
    };
    let front = front(&home, dead_port(), opts).await;
    let uncaught = get(front.port, "/tabs").await;
    assert_eq!(uncaught.status, 504);
    let caught = get(front.port, "/tmux-mouse?session=s1").await;
    assert_eq!(
        caught.text(),
        r#"{"error": "Command '['tmux', 'has-session', '-t', '=s1']' timed out after 0.3 seconds"}"#
    );
    front.stop().await;
}

#[tokio::test]
async fn exotic_inputs_decline_to_legacy() {
    let home = TestHome::new("light-exotic");
    let legacy = FakeLegacy::start().await;
    home.write(
        "app-tabs-history.json",
        r#"[{"session": "s", "label": 1.5}]"#,
    );
    home.write("prefs.json", r#"{"favorites": "abc"}"#);
    home.write("app-tab-active.json", r#"{"session": 2.5}"#);
    let front = front(&home, legacy.port, home.options()).await;
    for target in ["/tab-history", "/tab-models?session=%FF", "/active-tab"] {
        assert_eq!(
            get(front.port, target).await.text(),
            r#"{"legacy": true}"#,
            "{target}"
        );
    }
    let wire = request_body(
        front.port,
        "POST",
        "/prefs-set",
        "",
        r#"{"favorite": {"session": "s", "enabled": true}}"#,
    )
    .await;
    assert_eq!(wire.text(), r#"{"legacy": true}"#);
    assert_eq!(
        fs::read_to_string(home.hooks().join("prefs.json")).unwrap(),
        r#"{"favorites": "abc"}"#,
        "declinar nunca escribe"
    );
    assert_eq!(legacy.requests().len(), 4);
    front.stop().await;
}

#[tokio::test]
async fn light_routes_match_python_oracle() {
    let home = TestHome::new("light-oracle");
    if tmux_available() {
        new_tmux_sessions(&home, &["local", "s1"]);
    }
    home.write("app-tabs.json", r#"{"s1": "Uno ñ"}"#);
    home.write("prefs.json", r#"{"favorites": ["s1"], "theme": "neon"}"#);
    home.write(
        "app-tab-models.json",
        r#"{"s1": {"panes": [{"pane": "%0", "model": "ñ"}]}}"#,
    );
    home.write("app-tab-active.json", r#"{"session": "s1"}"#);
    home.write(
        "app-tabs-history.json",
        r#"[{"session": "vieja", "ts": 3}]"#,
    );
    let py = FrozenHttp::new(
        &home,
        "server-http-light",
        &[
            "prefs.json",
            "prefs.json.lock",
            "app-tabs.json",
            "app-tabs-history.json",
            "app-tab-models.json",
            "app-tab-active.json",
        ],
    )
    .await;
    let front = front(&home, dead_port(), oracle_options(&home)).await;
    for target in [
        "/prefs",
        "/tabs",
        "/tab-history",
        "/tab-models?session=s1",
        "/active-tab",
        "/tmux-mouse?session=s1",
        "/tmux-mouse?session=nope",
    ] {
        let (a, b) = (py.get(target).await, get(front.port, target).await);
        assert_eq!(
            (a.status, a.header("content-type"), a.text()),
            (b.status, b.header("content-type"), b.text()),
            "{target}"
        );
    }
    front.stop().await;
}

/// Opciones del frente frente al oráculo: `fc-list` real con el mismo HOME
/// que el Python (las fuentes del usuario, `~/.local/share/fonts`, dependen de él).
fn oracle_options(home: &TestHome) -> comandos_server::dash::native::NativeOptions {
    let mut opts = home.options();
    opts.fc_list = Program::named("fc-list");
    opts.fc_list
        .env
        .push(("HOME".into(), home.root.clone().into()));
    opts
}

/// Status, tipo y cuerpo de una respuesta, para comparar con el oráculo.
fn seen(wire: &support::Wire) -> (u16, Option<String>, String) {
    (
        wire.status,
        wire.header("content-type").map(str::to_owned),
        wire.text(),
    )
}

#[tokio::test]
async fn light_errors_and_writes_match_python_oracle() {
    let home = TestHome::new("light-oracle-err");
    let tmux = tmux_available();
    if tmux {
        new_tmux_sessions(&home, &["s1"]);
    }
    let py = FrozenHttp::new(
        &home,
        "server-http-light",
        &[
            "prefs.json",
            "prefs.json.lock",
            "app-tabs.json",
            "app-tabs-history.json",
            "app-tab-models.json",
            "app-tab-active.json",
        ],
    )
    .await;
    let front = front(&home, dead_port(), oracle_options(&home)).await;
    let both = |target: &'static str| {
        let py = &py;
        let front = &front;
        async move {
            (
                seen(&py.get(target).await),
                seen(&get(front.port, target).await),
            )
        }
    };
    // Lecturas: cada archivo en sus formas raras que el Rust sí reproduce.
    home.write(
        "app-tab-models.json",
        r#"{"mala": [1], "vacia": {}, "texto": "x", "cero": 0, "nulo": null, "sin": {"panes": 0}, "ok": {"panes": [1, "ñ"]}}"#,
    );
    for target in [
        "/tab-models?session=mala",
        "/tab-models?session=vacia",
        "/tab-models?session=texto",
        "/tab-models?session=cero",
        "/tab-models?session=nulo",
        "/tab-models?session=sin",
        "/tab-models?session=ok&session=mala",
        "/tab-models?session=",
        "/tab-models",
    ] {
        let (a, b) = both(target).await;
        assert_eq!(a, b, "{target}");
    }
    for (active, target) in [
        ("[1]", "/active-tab"),
        ("null", "/active-tab"),
        ("roto", "/active-tab"),
        (r#"{"session": "a b", "z": "ñ"}"#, "/active-tab"),
        (r#"{"session": null}"#, "/active-tab"),
        (r#"{"session": "nope", "pane": "%9"}"#, "/active-tab"),
        (r#"{"pane": "x", "session": "s1"}"#, "/active-tab"),
    ] {
        home.write("app-tab-active.json", active);
        let (a, b) = both(target).await;
        assert_eq!(a, b, "{active}");
    }
    for history in [
        r#"[{"session": "x", "ts": "abc"}]"#,
        r#"[{"session": "x", "ts": [1]}]"#,
        r#"[{"session": "x", "ts": "1_000", "label": "", "agent": 0, "reason": "r"}]"#,
        r#"[{"session": null}, {"session": true}, {"session": 7, "cwd": 5}]"#,
        r#"{"no": "lista"}"#,
    ] {
        home.write("app-tabs-history.json", history);
        let (a, b) = both("/tab-history").await;
        assert_eq!(a, b, "{history}");
    }
    for prefs in [
        r#"{"favorites": null}"#,
        r#"{"favorites": [[1]]}"#,
        r#"{"favorites": [1, "s1", null]}"#,
        r#"[1]"#,
        "roto",
        "",
        "\u{feff}{\"favorites\": [\"s1\"], \"theme\": \"dia\"}",
    ] {
        home.write("prefs.json", prefs);
        for target in ["/tabs", "/prefs"] {
            let (a, b) = both(target).await;
            assert_eq!(a, b, "{prefs} {target}");
        }
    }
    // Formas raras de `app-tabs.json` contra `/tabs` y `/tab-history`.
    home.write("prefs.json", r#"{"favorites": ["s1"]}"#);
    home.write(
        "app-tabs-history.json",
        r#"[{"session": "s1"}, {"session": "x"}, {"session": "hub"}]"#,
    );
    for tabs in [
        "[1]",
        r#""s1""#,
        "null",
        "roto",
        r#"{"s1": 5, "x": ""}"#,
        r#"{"s1": "A", "x": "X", "s1": "B"}"#,
        r#"{"s1": "", "local": "L", "hub": "H"}"#,
        r#"{"s1": null, "x": ["y"]}"#,
        r#"{"hub": "H", "s1": "Uno ñ"}"#,
    ] {
        home.write("app-tabs.json", tabs);
        for target in ["/tabs", "/tab-history"] {
            let (a, b) = both(target).await;
            assert_eq!(a, b, "{tabs} {target}");
        }
    }
    for target in [
        "/tmux-mouse",
        "/tmux-mouse?session=a%20b",
        "/tmux-mouse?session=nope",
        "/tmux-mouse?session=s1",
    ] {
        let (a, b) = both(target).await;
        assert_eq!(a, b, "{target}");
    }
    // Escrituras: el mismo estado inicial para cada lado; se comparan la
    // respuesta y los bytes que quedan en disco.
    let prefs_path = home.hooks().join("prefs.json");
    // Flotantes guardados: la reescritura los vuelve a escribir con el `repr` del Python.
    let initial = r#"{"favorites": ["b", "local", 3, "b"], "theme": "dia", "nfSnooze": {"q": 1, "f": 1.10, "e": 1e5, "z": -0.0}, "x": [1.10, 1E-7, 123456789012345678.0]}"#;
    for body in [
        r#"{"favorite": {"session": "s1", "enabled": true}}"#,
        r#"{"favorite": {"session": "b", "enabled": false}}"#,
        r#"{"favorite": {"session": "b", "enabled": true}}"#,
        r#"{"favorite": {"session": "", "enabled": true}}"#,
        r#"{"favorite": {"session": "a\u0001", "enabled": true}}"#,
        r#"{"favorite": "s1"}"#,
        r#"{"favorite": {"session": "s1", "enabled": true}, "favorites": ["z"]}"#,
        r#"{"favorites": ["z", "", "local", 5, "z", "y"]}"#,
        r#"{"nfDismiss": {"a": 0, "b": null}, "nfSnooze": {"n": -0, "f": 1.5, "e": 1e400, "big": 12345678901234567890, "t": false, "s": "1"}}"#,
        r#"{"nfSnooze": {"x": 1e999}}"#,
        r#"{"nfSnooze": {"x": NaN}}"#,
        r#"{"font_size": NaN}"#,
        r#"{"font_size": Infinity}"#,
        r#"{"font_size": true, "terminal_padding": 1e30, "terminal_opacity": -1e30}"#,
        r#"{"font_size": 99999999999999999999999, "terminal_padding": 7.9, "terminal_opacity": "50"}"#,
        r#"{"font_family": "   ", "theme": "x", "button_style": "pixel", "tabs_layout": "rows", "cursor_shape": "ibeam", "cursor_blink": 0, "ligatures": false, "notif_pos": "free"}"#,
        r#"{"theme": ["neon"]}"#,
        "{}",
    ] {
        let mut sides = Vec::new();
        for reference in [true, false] {
            home.write("prefs.json", initial);
            let wire = if reference {
                py.request("POST", "/prefs-set", "", body).await
            } else {
                request_body(front.port, "POST", "/prefs-set", "", body).await
            };
            sides.push((seen(&wire), fs::read_to_string(&prefs_path).unwrap()));
        }
        assert_eq!(sides[0], sides[1], "{body}");
    }
    let fresh = py
        .request("POST", "/prefs-set", "", r#"{"theme": "neon"}"#)
        .await;
    assert_eq!(fresh.status, 200);
    for body in [
        r#"{"session": 5}"#,
        r#"{"session": null}"#,
        r#"{}"#,
        r#"{"session": "a b"}"#,
        r#"{"session": "nope"}"#,
        r#"{"session": "s1", "enabled": false}"#,
        r#"{"session": "s1", "enabled": null}"#,
        r#"{"session": "s1"}"#,
        r#"{"session": "s1", "enabled": 0}"#,
    ] {
        let a = seen(&py.request("POST", "/tmux-mouse", "", body).await);
        let b = seen(&request_body(front.port, "POST", "/tmux-mouse", "", body).await);
        assert_eq!(a, b, "{body}");
    }
    front.stop().await;
}

#[tokio::test]
async fn unsure_files_decline_to_legacy() {
    let home = TestHome::new("light-unsure");
    let legacy = FakeLegacy::start().await;
    let front = front(&home, legacy.port, home.options()).await;
    // Sustituto suelto: `json` del Python lo lee, el parser portado no.
    home.write("app-tab-active.json", r#"{"session": "\ud800"}"#);
    // Bytes que no son UTF-8: depende de la codificación del Python.
    fs::write(home.hooks().join("prefs.json"), b"{\"theme\": \"\xff\"}").unwrap();
    for target in ["/active-tab", "/prefs"] {
        assert_eq!(
            get(front.port, target).await.text(),
            r#"{"legacy": true}"#,
            "{target}"
        );
    }
    let wire = request_body(front.port, "POST", "/prefs-set", "", r#"{"theme": "neon"}"#).await;
    assert_eq!(wire.text(), r#"{"legacy": true}"#);
    assert_eq!(
        fs::read(home.hooks().join("prefs.json")).unwrap(),
        b"{\"theme\": \"\xff\"}",
        "declinar nunca escribe"
    );
    assert_eq!(legacy.requests().len(), 3);
    front.stop().await;
}

#[tokio::test]
async fn prefs_set_saves_selects_and_deletes_user_themes() {
    let home = TestHome::new("prefs-user-themes");
    let front = front(&home, dead_port(), home.options()).await;
    let noche = serde_json::json!({
        "fg": "#EAF0FB", "bg": "#0A0D13", "cursor": "#FFAE1A", "bar": "#0D1119",
        "dim": "#9AA6BF", "text": "#EAF0FB", "brand": "#ff6fa8", "line": "#222A3A",
        "line2": "#2E3852", "faint": "#5E6980", "panel": "#121722", "panel2": "#161C29",
        "pal": ["#232B3E", "#FF6B6B", "#2EE59D", "#FFAE1A", "#7AA5FF", "#B49CFF", "#4DD0E1", "#EAF0FB",
                "#5E6980", "#FF8F8F", "#6FF0BC", "#FFC55C", "#A3C0FF", "#CDBBFF", "#86E3F0", "#FFFFFF"],
    });
    let theme = serde_json::json!({"id": "u-rosa", "label": "Noche rosa", "base": "noche",
        "colors": noche, "treatment": "glass", "button_style": "arcade"});
    // Un tema propio que no existe todavía no se puede elegir.
    let wire = request_body(
        front.port,
        "POST",
        "/prefs-set",
        "",
        r#"{"theme": "u-rosa"}"#,
    )
    .await;
    assert_eq!(wire.status, 200);
    let body = serde_json::json!({"user_theme": theme, "theme": "u-rosa"}).to_string();
    let wire = request_body(front.port, "POST", "/prefs-set", "", &body).await;
    assert_eq!(wire.status, 200, "{}", wire.text());
    let prefs: serde_json::Value =
        serde_json::from_str(&get(front.port, "/prefs").await.text()).unwrap();
    assert_eq!(prefs["theme"], "u-rosa");
    assert_eq!(prefs["user_themes"][0]["colors"]["brand"], "#FF6FA8");
    let mut bad = theme.clone();
    bad["colors"]["bg"] = serde_json::json!("azul");
    let body = serde_json::json!({"user_theme": bad}).to_string();
    let wire = request_body(front.port, "POST", "/prefs-set", "", &body).await;
    assert_eq!(wire.status, 400);
    let wire = request_body(
        front.port,
        "POST",
        "/prefs-set",
        "",
        r#"{"user_theme_delete": "u-rosa"}"#,
    )
    .await;
    assert_eq!(wire.status, 200);
    let prefs: serde_json::Value =
        serde_json::from_str(&get(front.port, "/prefs").await.text()).unwrap();
    assert_eq!(prefs["theme"], "noche");
    assert_eq!(prefs["user_themes"], serde_json::json!([]));
    front.stop().await;
}
