#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]
use comandos_app::ui::extensions::{extension_message, extension_uri, message_owned, shelf_height};
#[test]
fn shelf_height_keeps_original_minimum_small_window_and_truncation() {
    assert_eq!(shelf_height(720.0, None), 345.0);
    assert_eq!(shelf_height(150.0, Some(1000.0)), 180.0);
    assert_eq!(shelf_height(1000.0, Some(321.75)), 321.0);
    assert_eq!(shelf_height(400.0, Some(-1.0)), 240.0);
}
#[test]
fn close_only_from_current_extension_webview_and_exact_origin_target() {
    let uri = extension_uri("http://127.0.0.1:7337", "fixture", "%7", "grok", "test").unwrap();
    assert!(uri.contains("pane=%257"));
    assert_eq!(
        extension_message("close").unwrap(),
        serde_json::json!({"command":"close"})
    );
    assert_eq!(
        extension_message("{\"session\":\"other\"}").unwrap(),
        serde_json::Value::Null
    );
    assert!(message_owned(&uri, &uri, "http://127.0.0.1:7337", 2, 2));
    assert!(!message_owned(&uri, &uri, "http://127.0.0.1:7337", 1, 2));
    assert!(!message_owned(
        "http://127.0.0.1:7337/extensions.html?session=other",
        &uri,
        "http://127.0.0.1:7337",
        2,
        2
    ));
    assert!(!message_owned(
        "http://evil.invalid/extensions.html",
        &uri,
        "http://127.0.0.1:7337",
        2,
        2
    ));
    assert!(extension_uri("http://127.0.0.1:7337", "fixture", "%bad", "grok", "test").is_err());
}
#[test]
fn shelf_document_is_readonly_authoritative_and_live_cas_writes_only_legacy() {
    use comandos_app::{config::parse_args, guard::WriteGuard, state_files::StateFiles};
    use comandos_store::unified::{self, Mode};
    use std::os::unix::fs::PermissionsExt;
    for (i, mode) in [Mode::Legacy, Mode::Mirror, Mode::Unified, Mode::Sealed]
        .into_iter()
        .enumerate()
    {
        let root = std::env::temp_dir().join(format!("t17-shelf-store-{}-{i}", std::process::id()));
        for d in ["home", "hooks", "run", "tmp"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
            std::fs::set_permissions(root.join(d), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let cfg = parse_args(
            &[
                "--mode".into(),
                "live".into(),
                "--hooks-dir".into(),
                root.join("hooks").display().to_string(),
            ],
            false,
            &|k| match k {
                "HOME" => Some(root.join("home").display().to_string()),
                "XDG_RUNTIME_DIR" => Some(root.join("run").display().to_string()),
                "TMPDIR" => Some(root.join("tmp").display().to_string()),
                _ => None,
            },
        )
        .unwrap();
        let files = StateFiles::new(cfg.clone(), WriteGuard::from_config(&cfg, ":fixture"));
        let old = serde_json::json!({"height":300,"unknown":{"ñ":"😀"}});
        files.write("app-extension-shelf.json", &old).unwrap();
        let db = unified::open_unified(&unified::unified_path(cfg.home())).unwrap();
        let mut authoritative = old.clone();
        authoritative["height"] = serde_json::json!(880);
        unified::doc_put(
            &db,
            "hooks/app-extension-shelf.json",
            "app-ui",
            comandos_core::json::response_dumps(&authoritative)
                .unwrap()
                .as_bytes(),
            unified::Origin::Import,
            1,
        )
        .unwrap();
        unified::set_mode(&db, "app-ui", mode, "fixture", 1).unwrap();
        assert_eq!(
            files
                .read_pane_document("app-extension-shelf.json")
                .unwrap(),
            if matches!(mode, Mode::Unified | Mode::Sealed) {
                authoritative
            } else {
                old.clone()
            }
        );
        let expected = files
            .read_pane_document("app-extension-shelf.json")
            .unwrap();
        let mut next = expected.clone();
        next["height"] = serde_json::json!(410);
        assert!(files.write_shelf_when(&expected, &next, || true).unwrap());
        assert_eq!(files.read("app-extension-shelf.json").unwrap(), next);
        assert!(
            !files
                .write_shelf_when(&expected, &serde_json::json!({"height":500}), || true)
                .unwrap()
        );
    }
}

#[path = "support/t17_oracle.rs"]
mod oracle;
#[test]
fn actual_original_shelf_math_messages_and_separator_css_match_native() {
    use serde_json::json;
    let cases = [
        (720.0, 0.0),
        (150.0, 1000.0),
        (1000.0, 321.75),
        (400.0, -1.0),
        (2400.0, 0.0),
    ];
    let original = oracle::original(json!({"op":"shelf","cases":cases,"statuses":[null]}));
    for ((total, saved), expected) in cases
        .into_iter()
        .zip(original["heights"].as_array().unwrap())
    {
        assert_eq!(shelf_height(total, Some(saved)), expected.as_f64().unwrap());
    }
    let themes: serde_json::Value =
        serde_json::from_str(include_str!("../src/ui/desktop-themes.json")).unwrap();
    for (index, name) in [
        "noche",
        "dia",
        "calido",
        "termius",
        "bruno",
        "superglass",
        "neon",
        "contraste",
        "ubuntu",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            comandos_app::ui::extensions::shelf_separator_css(themes.get(name).unwrap()),
            original["css"][index].as_str().unwrap()
        );
    }
    for (raw, close) in [
        ("close", true),
        (" close", false),
        ("{\"command\":\"close\"}", false),
    ] {
        let outcome = oracle::original(
            json!({"op":"extension-message","raw":raw,"uri":"http://127.0.0.1:7337/extensions.html?session=fixture&pane=%257","statuses":[null]}),
        );
        assert_eq!(outcome["closes"], if close { 1 } else { 0 });
        assert_eq!(!extension_message(raw).unwrap().is_null(), close);
    }
}

#[test]
fn shelf_delayed_publication_and_replayed_actions_use_the_real_owner_scope() {
    use comandos_app::{
        config::parse_args, guard::WriteGuard, state_files::StateFiles, ui::snippets::Scope,
    };
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("t17-shelf-replay-{}", std::process::id()));
    for d in ["home", "hooks", "run", "tmp"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
        std::fs::set_permissions(root.join(d), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let cfg = parse_args(
        &[
            "--mode".into(),
            "live".into(),
            "--hooks-dir".into(),
            root.join("hooks").display().to_string(),
        ],
        false,
        &|k| match k {
            "HOME" => Some(root.join("home").display().to_string()),
            "XDG_RUNTIME_DIR" => Some(root.join("run").display().to_string()),
            "TMPDIR" => Some(root.join("tmp").display().to_string()),
            _ => None,
        },
    )
    .unwrap();
    let files = StateFiles::new(cfg.clone(), WriteGuard::from_config(&cfg, ":private-shelf"));
    let old = serde_json::json!({"height":300,"unknown":"fixture"});
    files.write("app-extension-shelf.json", &old).unwrap();
    // The domain writer now acquires the shared lock; a stale ticket must not recreate it.
    std::fs::remove_file(cfg.hooks_dir().join("app-extension-shelf.json.lock")).unwrap();
    let owner = Scope::default();
    let delayed = owner.ticket();
    owner.close();
    assert!(
        !files
            .write_shelf_when(&old, &serde_json::json!({"height":500}), || delayed
                .current())
            .unwrap()
    );
    assert_eq!(files.read("app-extension-shelf.json").unwrap(), old);
    assert!(
        !cfg.hooks_dir()
            .join("app-extension-shelf.json.lock")
            .exists()
    );
    let replacement = Scope::default();
    let current = replacement.ticket();
    assert!(current.current());
    assert!(!delayed.current());
    replacement.advance();
    assert!(!current.current());
    let next = replacement.ticket();
    let reads = std::cell::Cell::new(0);
    assert!(
        !files
            .write_shelf_when(&old, &serde_json::json!({"height":500}), || {
                reads.set(reads.get() + 1);
                if reads.get() == 2 {
                    replacement.close();
                }
                next.current()
            })
            .unwrap()
    );
    assert_eq!(files.read("app-extension-shelf.json").unwrap(), old);
    let path = cfg.hooks_dir().join("app-extension-shelf.json");
    WriteGuard::from_config(&cfg, ":private-shelf")
        .write_atomic(&path, b"MOVED", "app-state.")
        .unwrap();
    assert!(
        files
            .read_pane_document("app-extension-shelf.json")
            .is_err()
    );
    assert!(
        files
            .write_shelf_when(&old, &serde_json::json!({"height":500}), || true)
            .is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), b"MOVED");
}
#[test]
fn start_here_javascript_quotes_payload_into_the_actual_original_global_target() {
    use comandos_app::ui::bridge::{JsFunction, js_call};
    let args = [
        serde_json::json!("fixture"),
        serde_json::json!("%7"),
        serde_json::json!("/tmp/fixture 'quoted' \"new\""),
    ];
    let original = oracle::original(
        serde_json::json!({"op":"wizard","session":args[0],"pane":args[1],"cwd":args[2].as_str().unwrap().trim(),"statuses":[null]}),
    );
    let original_body = original["scripts"][0]
        .as_str()
        .unwrap()
        .strip_prefix("nsOpenForPane(")
        .unwrap()
        .strip_suffix(")")
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&format!("[{original_body}]")).unwrap(),
        serde_json::json!([args[0], args[1], args[2].as_str().unwrap().trim()])
    );
    let script = js_call(JsFunction::NewSessionForPane, &args).unwrap();
    let body = script
        .split("window.nsOpenForPane(")
        .nth(1)
        .unwrap()
        .strip_suffix(");")
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&format!("[{body}]")).unwrap();
    assert_eq!(parsed, serde_json::json!(args));
    assert!(script.starts_with("if(typeof window.nsOpenForPane!=="));
}

#[test]
fn shadow_preview_shows_inventory_without_script_forms_or_credential_fields() {
    let data = serde_json::json!({"harness":"grok","secret":"FAKE_KEY_NOT_UI","inventory":{"mcps":[{"id":"m1","name":"Fake <&> MCP","size":{"tokens":100},"env":{"TOKEN":"FAKE_KEY_NOT_UI"}}],"skills":[{"id":"s1","name":"Fake skill","size":{"tokens":null}}]},"desired":{"mcps":{"m1":true},"skills":{"s1":false}}});
    let html = comandos_app::ui::extensions::shadow_preview(&data, "fixture", "%7", false);
    assert!(html.contains("Fake &lt;&amp;&gt; MCP"));
    assert!(html.contains("Fake skill"));
    assert!(html.contains("100 tokens"));
    assert!(html.contains("Solo lectura"));
    assert!(html.contains("%7"));
    for text in [
        "<script",
        "<form",
        "<button",
        "FAKE_KEY_NOT_UI",
        "fetch(",
        "localStorage",
    ] {
        assert!(!html.contains(text), "{text}");
    }
}

#[test]
fn shadow_inventory_get_is_pinned_before_and_after_inert_http() {
    use comandos_app::{
        config::RunMode,
        tmux::{TmuxError, TmuxOut},
        ui::clipboard::TmuxIo,
    };
    use std::{
        cell::{Cell, RefCell},
        path::{Path, PathBuf},
    };
    struct Pane {
        socket: PathBuf,
        record: RefCell<String>,
        cancel: Cell<bool>,
        reads: Cell<usize>,
    }
    impl TmuxIo for Pane {
        fn mode(&self) -> RunMode {
            RunMode::Shadow
        }
        fn socket(&self) -> &Path {
            &self.socket
        }
        fn read(&self, args: &[&str]) -> Result<TmuxOut, TmuxError> {
            assert_eq!(
                args,
                &[
                    "display-message",
                    "-p",
                    "-t",
                    "%7",
                    "#{pid}|#{session_id}|#{s/[$]//:session_id}|#{session_name}|#{pane_id}"
                ]
            );
            self.reads.set(self.reads.get() + 1);
            Ok(TmuxOut {
                code: 0,
                stdout: self.record.borrow().clone(),
                stderr: String::new(),
            })
        }
        fn mutate(&self, _: &[&str], _: Option<&[u8]>) -> Result<TmuxOut, TmuxError> {
            panic!("Shadow inventory cannot POST or mutate tmux")
        }
    }
    for race in [
        "none",
        "cancel-in-http",
        "recycle-in-http",
        "closed-before-http",
    ] {
        let pane = Pane {
            socket: std::env::temp_dir().join("extension-explicit-fake-S"),
            record: RefCell::new("111|$1|42|fixture|%7".into()),
            cancel: Cell::new(race == "closed-before-http"),
            reads: Cell::new(0),
        };
        let requests = Cell::new(0);
        let data = serde_json::json!({"inventory":{"skills":[{"name":"Fake skill","env":{"TOKEN":"FAKE_TEST_CREDENTIAL"}}]}});
        let result = comandos_app::ui::extensions::read_inventory_when(
            &pane,
            "fixture",
            "%7",
            "grok",
            || !pane.cancel.get(),
            |path| {
                requests.set(requests.get() + 1);
                assert_eq!(
                    path,
                    "/pane-extensions?session=fixture&pane=%257&harness=grok"
                );
                if race == "cancel-in-http" {
                    pane.cancel.set(true);
                }
                if race == "recycle-in-http" {
                    *pane.record.borrow_mut() = "111|$2|99|fixture|%7".into();
                }
                Ok(data.clone())
            },
        );
        if race == "none" {
            assert_eq!(result.unwrap(), data);
            assert_eq!(pane.reads.get(), 2);
        } else {
            assert!(result.is_err(), "{race}");
        }
        assert_eq!(requests.get(), usize::from(race != "closed-before-http"));
    }
}
