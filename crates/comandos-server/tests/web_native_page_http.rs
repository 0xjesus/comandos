//! Real HTTP against disposable files, without browser or tmux processes.
use comandos_server::{
    Config,
    dash::{
        self, DashState,
        web::{
            WebState,
            registry::{Entry, Resolved, sha256_hex},
        },
    },
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};

const PAGE: &str = "legacy-exact";

struct Front {
    root: PathBuf,
    address: std::net::SocketAddr,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl Front {
    async fn start() -> Self {
        Self::start_readonly(false).await
    }
    async fn start_readonly(readonly: bool) -> Self {
        Self::start_config(readonly, false).await
    }
    async fn start_config(readonly: bool, native_default: bool) -> Self {
        Self::start_with_term(readonly, native_default, dash::term::TermMode::Off).await
    }
    async fn start_with_term(
        readonly: bool,
        native_default: bool,
        term: dash::term::TermMode,
    ) -> Self {
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "comandos-web-http-{}-{:x}",
            std::process::id(),
            u64::from_le_bytes(nonce)
        ));
        std::fs::create_dir_all(root.join(".claude/hooks/dash")).unwrap();
        std::fs::create_dir_all(root.join("web/fedcba987654")).unwrap();
        std::fs::write(root.join(".claude/hooks/dash/index.html"), PAGE).unwrap();
        std::fs::write(
            root.join(".claude/hooks/comandos-web.json"),
            r#"{"on":["quick-terminal"],"shadow":[]}"#,
        )
        .unwrap();
        std::fs::write(root.join("quick-terminal.js"), b"original-script").unwrap();
        std::fs::write(root.join("web/manifest.json"), r#"{"files":{"comandos_web_boot.js":"fedcba987654/boot.js","comandos_web.js":"fedcba987654/comandos_web.js","comandos_web_bg.wasm":"fedcba987654/comandos_web_bg.wasm"}}"#).unwrap();
        for name in ["boot.js", "comandos_web.js", "comandos_web_bg.wasm"] {
            std::fs::write(root.join("web/fedcba987654").join(name), b"fixture").unwrap();
        }
        let args = if native_default {
            vec!["--web-native".into()]
        } else {
            vec![]
        };
        let mut cfg = dash::parse_args(&args, &root, None).unwrap();
        cfg.native = false;
        cfg.shadow_readonly = readonly;
        cfg.term = term;
        if term != dash::term::TermMode::Off {
            std::fs::write(root.join(".claude/hooks/webterm-enabled"), b"").unwrap();
        }
        cfg.token = b"fixture-token".to_vec();
        cfg.repo_root = Some(root.clone());
        cfg.web_dir = root.join("web");
        let ids = comandos_core::web_assets::native_index_components().unwrap();
        std::fs::write(
            root.join(".claude/hooks/comandos-web.json"),
            serde_json::json!({"on":ids}).to_string(),
        )
        .unwrap();
        let mut web = WebState::new(&cfg);
        let mut entries = ids
            .iter()
            .map(|id| Entry::script(id, "compiled-only", "unused", &[], &[]))
            .collect::<Vec<_>>();
        entries[0].deps = vec![ids.last().unwrap().clone()];
        web.registry = Resolved::in_memory(entries, &[]);
        let assets = [
            "/comandos_web_sound.js",
            "/comandos_web_sound_bg.wasm",
            "/comandos_web_content.js",
            "/comandos_web_content_bg.wasm",
            "/workspace.css",
            "/buttons.css",
            "/manifest.webmanifest",
            "/icon-192.png",
            "/icon-512.png",
        ]
        .into_iter()
        .map(|url| {
            (
                url.to_string(),
                format!("native_{}", url.trim_start_matches('/')),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
        let descriptor = comandos_core::web_assets::NativePage {
            version: 1,
            template_sha256: sha256_hex(
                comandos_web_view::index_page::shell("es")
                    .into_string()
                    .as_bytes(),
            ),
            components: ids,
            assets: assets.clone(),
        };
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("web/manifest.json")).unwrap())
                .unwrap();
        let files = manifest["files"].as_object_mut().unwrap();
        for logical in assets.values() {
            files.insert(
                logical.clone(),
                serde_json::json!(format!("fedcba987654/{logical}")),
            );
            std::fs::write(
                root.join("web/fedcba987654").join(logical),
                b"native-static",
            )
            .unwrap();
        }
        files.insert(
            comandos_core::web_assets::NATIVE_PAGE_FILE.into(),
            serde_json::json!("fedcba987654/comandos_native_page.json"),
        );
        std::fs::write(
            root.join("web/fedcba987654/comandos_native_page.json"),
            serde_json::to_vec(&descriptor).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("web/manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let asset_exists = dash::asset_exists(&cfg.dash_dir);
        let state = Arc::new(DashState {
            web,
            config: cfg,
            asset_exists: asset_exists.clone(),
            native: None,
            term_target: dash::term::attach::TmuxTarget::Private(root.clone()),
            term_control: Arc::new(dash::term::lifecycle::Control::new(&root)),
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let config = Config {
            token: b"fixture-token".to_vec(),
            token_file: None,
            asset_exists,
            handler: dash::handler(state),
            websocket: None,
            limits: dash::limits(),
        };
        let (stop, rx) = watch::channel(false);
        let task = tokio::spawn(comandos_server::serve(listener, config, rx));
        Self {
            root,
            address,
            stop,
            task,
        }
    }
    async fn http(&self, method: &str, path: &str, body: &str, headers: &str) -> String {
        let mut stream = TcpStream::connect(self.address).await.unwrap();
        stream.write_all(format!("{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}", self.address, body.len()).as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        tokio::time::timeout(Duration::from_secs(12), stream.read_to_end(&mut raw))
            .await
            .unwrap()
            .unwrap();
        String::from_utf8(raw).unwrap()
    }
    async fn stop(self) {
        self.stop.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(3), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        std::fs::remove_dir_all(self.root).unwrap();
    }
}
fn nonce(page: &str) -> &str {
    page.split_once("data-k=\"")
        .unwrap()
        .1
        .split_once('"')
        .unwrap()
        .0
}

#[tokio::test]
async fn standalone_extensions_uses_wasm_without_legacy_source_and_its_own_readiness() {
    let front = Front::start_config(false, true).await;
    let legacy = front.root.join(".claude/hooks/dash/extensions.html");
    std::fs::write(
        &legacy,
        "legacy extensions<script src=\"/extensions.js\"></script>",
    )
    .unwrap();
    let off = front.http("GET", "/extensions.html?web=off", "", "").await;
    assert!(
        off.starts_with("HTTP/1.1 200") && off.contains("/extensions.js"),
        "{off}"
    );
    std::fs::remove_file(&legacy).unwrap();
    let page = front
        .http("GET", "/extensions.html?tab=session&pane=%251", "", "")
        .await;
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(page.contains("<main id=\"extensions\""));
    assert!(page.contains("name=\"comandos-web\" content=\"extensions\""));
    assert!(page.contains("/extensions.css") && page.contains("/buttons.css"));
    assert!(page.contains("src=\"/web/fedcba987654/boot.js\""));
    assert!(!page.contains("/extensions.js") && !page.contains("gate.js"));
    let k = nonce(&page);
    assert!(k.starts_with("native-extensions-"));
    for mounted in [
        serde_json::json!([]),
        serde_json::json!(["extensions", "extensions"]),
        serde_json::json!(["app"]),
    ] {
        let denied = front
            .http(
                "POST",
                "/web/ready",
                &serde_json::json!({"k": k, "mounted": mounted, "failed": []}).to_string(),
                "",
            )
            .await;
        assert!(denied.starts_with("HTTP/1.1 400"), "{denied}");
    }
    let payload = serde_json::json!({"k": k, "mounted": ["extensions"], "failed": []}).to_string();
    let ready = front.http("POST", "/web/ready", &payload, "").await;
    assert!(ready.starts_with("HTTP/1.1 200"), "{ready}");
    let replay = front.http("POST", "/web/ready", &payload, "").await;
    assert!(replay.starts_with("HTTP/1.1 400"), "{replay}");
    std::fs::remove_file(front.root.join("web/fedcba987654/boot.js")).unwrap();
    let missing = front.http("GET", "/extensions.html", "", "").await;
    assert!(missing.starts_with("HTTP/1.1 503"), "{missing}");
    front.stop().await;
}

#[tokio::test]
async fn embedded_native_viewport_uses_supported_keys_and_browser_keeps_keyboard_resize() {
    let front = Front::start().await;
    std::fs::remove_file(front.root.join(".claude/hooks/dash/index.html")).unwrap();
    let embedded = front.http("GET", "/?web=native&app=1", "", "").await;
    let browser = front.http("GET", "/?web=native", "", "").await;
    let flag_prefix = front.http("GET", "/?web=native&app=10", "", "").await;
    for page in [&embedded, &browser, &flag_prefix] {
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains("viewport-fit=cover"));
    }
    assert!(!embedded.contains("interactive-widget=resizes-content"));
    assert!(browser.contains("interactive-widget=resizes-content"));
    assert!(flag_prefix.contains("interactive-widget=resizes-content"));
    front.stop().await;
}

#[tokio::test]
async fn compiled_native_page_serves_without_original_sources_and_ready_requires_every_mount() {
    let front = Front::start().await;
    std::fs::remove_file(front.root.join(".claude/hooks/dash/index.html")).unwrap();
    std::fs::remove_file(front.root.join("quick-terminal.js")).unwrap();
    let page = front.http("GET", "/?web=native&app=1", "", "").await;
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert_eq!(page.matches("<!DOCTYPE html>").count(), 1);
    assert_eq!(page.matches("<html").count(), 1);
    assert_eq!(page.matches("<head>").count(), 1);
    assert!(page.contains("comandos-web-mode\" content=\"native"));
    assert!(page.find("id=\"servers\"").unwrap() < page.find("<script type=\"module\"").unwrap());
    assert!(
        !page.contains("gate.js")
            && !page.contains("web=off")
            && !page.contains("quick-terminal.js")
    );
    assert!(page.contains("/web/fedcba987654/native_workspace.css"));
    let ids = comandos_core::web_assets::native_index_components().unwrap();
    let meta = page
        .split_once("<meta name=\"comandos-web\" content=\"")
        .unwrap()
        .1
        .split_once('"')
        .unwrap()
        .0;
    assert_eq!(
        meta.split_whitespace().next(),
        ids.last().map(String::as_str)
    );
    let status = front.http("GET", "/web/status?web=native", "", "").await;
    assert!(
        status.starts_with("HTTP/1.1 200") && status.contains("native"),
        "{status}"
    );
    let k = nonce(&page);
    let denied = front
        .http(
            "POST",
            "/web/ready",
            &serde_json::json!({"k":k,"mounted":ids,"failed":[{"id":"fixture"}]}).to_string(),
            "",
        )
        .await;
    assert!(denied.starts_with("HTTP/1.1 400"));
    let denied = front
        .http(
            "POST",
            "/web/ready",
            &serde_json::json!({"k":k,"mounted":[],"failed":[]}).to_string(),
            "",
        )
        .await;
    assert!(denied.starts_with("HTTP/1.1 400"));
    let mut repeated = ids.clone();
    repeated.push(ids[0].clone());
    let denied = front
        .http(
            "POST",
            "/web/ready",
            &serde_json::json!({"k":k,"mounted":repeated,"failed":[]}).to_string(),
            "",
        )
        .await;
    assert!(denied.starts_with("HTTP/1.1 400"));
    let ready = front
        .http(
            "POST",
            "/web/ready",
            &serde_json::json!({"k":k,"mounted":ids,"failed":[]}).to_string(),
            "",
        )
        .await;
    assert!(ready.starts_with("HTTP/1.1 200"), "{ready}");
    let duplicate = front
        .http(
            "POST",
            "/web/ready",
            &serde_json::json!({"k":k,"mounted":ids,"failed":[]}).to_string(),
            "",
        )
        .await;
    assert!(duplicate.starts_with("HTTP/1.1 400"));
    let css = front
        .http("GET", "/web/fedcba987654/native_workspace.css", "", "")
        .await;
    assert!(css.starts_with("HTTP/1.1 200") && css.contains("native-static"));
    front.stop().await;
}
#[tokio::test]
async fn incomplete_native_selection_and_assets_refuse_without_legacy_fallback() {
    let front = Front::start().await;
    std::fs::write(
        front.root.join(".claude/hooks/comandos-web.json"),
        "{\"on\":[]}",
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(1050)).await;
    let reply = front.http("GET", "/?web=native", "", "").await;
    assert!(
        reply.starts_with("HTTP/1.1 503") && reply.contains("not selected on"),
        "{reply}"
    );
    assert!(!reply.contains("legacy-exact"));
    let off = front.http("GET", "/?web=off", "", "").await;
    assert!(off.ends_with("legacy-exact"), "{off}");
    front.stop().await;
    let front = Front::start().await;
    std::fs::remove_file(front.root.join("web/fedcba987654/native_workspace.css")).unwrap();
    let reply = front.http("GET", "/?web=native", "", "").await;
    assert!(reply.starts_with("HTTP/1.1 503"), "{reply}");
    assert!(!reply.contains("legacy-exact"));
    front.stop().await;
    let front = Front::start().await;
    std::fs::remove_file(
        front
            .root
            .join("web/fedcba987654/native_comandos_web_content_bg.wasm"),
    )
    .unwrap();
    let reply = front.http("GET", "/?web=native", "", "").await;
    assert!(reply.starts_with("HTTP/1.1 503"), "{reply}");
    assert!(!reply.contains("legacy-exact"));
    front.stop().await;
}
#[tokio::test]
async fn native_admission_requires_metadata_and_rejects_dependency_cycles() {
    use comandos_server::dash::web::{Manifest, Selection, native_page};
    use std::collections::BTreeSet;
    let front = Front::start().await;
    let ids = comandos_core::web_assets::native_index_components().unwrap();
    let selection = Selection {
        on: ids.iter().cloned().collect(),
        shadow: BTreeSet::new(),
    };
    let manifest = Manifest::load(&front.root.join("web")).unwrap();
    let mut entries = ids
        .iter()
        .map(|id| Entry::script(id, "source-not-installed", "unused", &[], &[]))
        .collect::<Vec<_>>();
    let omitted = entries.pop().unwrap();
    let registry = Resolved::in_memory(entries.clone(), &[]);
    assert!(
        native_page::admit(&registry, &selection, &manifest, &front.root.join("web"))
            .unwrap_err()
            .contains("component missing")
    );
    entries.push(omitted);
    let first = entries.first().unwrap().id.clone();
    let last = entries.last().unwrap().id.clone();
    entries.first_mut().unwrap().deps = vec![last];
    entries.last_mut().unwrap().deps = vec![first];
    let registry = Resolved::in_memory(entries, &[]);
    assert!(
        native_page::admit(&registry, &selection, &manifest, &front.root.join("web"))
            .unwrap_err()
            .contains("dependency cycle")
    );
    front.stop().await;
}

/// Bounded real-payload host for a separately owned remote browser. The caller
/// supplies only an xtask output; all HOME/config/database paths are disposable.
#[tokio::test]
#[ignore = "explicit real web-build payload and owned remote-browser smoke only"]
async fn serve_real_native_payload_for_remote_smoke() {
    let web_dir = PathBuf::from(
        std::env::var_os("COMANDOS_SMOKE_WEB_DIR").expect("explicit web-build directory"),
    );
    assert!(web_dir.is_absolute());
    let root = std::env::temp_dir().join(format!("comandos-native-smoke-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir_all(root.join(".claude/hooks")).unwrap();
    std::fs::write(
        root.join(".claude/hooks/comandos-web.json"),
        serde_json::json!({"on":comandos_core::web_assets::native_index_components().unwrap()})
            .to_string(),
    )
    .unwrap();
    let mut cfg = dash::parse_args(&[], &root, None).unwrap();
    cfg.native = false;
    cfg.shadow_readonly = true;
    cfg.token = b"fixture-token".to_vec();
    cfg.web_dir = web_dir;
    cfg.repo_root = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let web = WebState::new(&cfg);
    dash::web::native_page::admit(
        &web.registry,
        &web.selection(),
        &web.manifest(),
        &cfg.web_dir,
    )
    .expect("real native payload admission");
    let asset_exists = dash::asset_exists(&cfg.dash_dir);
    let state = Arc::new(DashState {
        web,
        config: cfg,
        asset_exists: asset_exists.clone(),
        native: None,
        term_target: dash::term::attach::TmuxTarget::Private(root.clone()),
        term_control: Arc::new(dash::term::lifecycle::Control::new(&root)),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    println!(
        "NATIVE_SMOKE_URL=http://{}/?web=native",
        listener.local_addr().unwrap()
    );
    let config = Config {
        token: b"fixture-token".to_vec(),
        token_file: None,
        asset_exists,
        handler: {
            let mut asset_urls =
                dash::web::native_page::bundle(&state.web.manifest(), &state.config.web_dir)
                    .unwrap()
                    .assets
                    .into_keys()
                    .collect::<std::collections::HashSet<_>>();
            if let Ok(manifest) = dash::web::assets::Manifest::load_terminal(&state.config.web_dir)
                && let Ok(page) =
                    dash::web::native_term_page::bundle(&manifest, &state.config.web_dir)
            {
                asset_urls.extend(page.assets.into_keys());
            }
            let frontend = dash::handler(state);
            Arc::new(move |request| {
                let path = dash::router::path_of(&request.target);
                let native_page = request
                    .target
                    .split('?')
                    .nth(1)
                    .is_some_and(|query| query.split('&').any(|pair| pair == "web=native"));
                if (matches!(path, "/" | "/index.html" | "/term/") && native_page)
                    || path.starts_with("/web/")
                    || asset_urls.contains(path)
                    || (path == "/sw.js" && request.target.split('?').nth(1) == Some("native=1"))
                {
                    return frontend(request);
                }
                if path.starts_with("/assets/")
                    || matches!(path, "/" | "/index.html" | "/term/" | "/sw.js")
                {
                    return Box::pin(async {
                        Ok(comandos_server::Reply::bytes(
                            http::StatusCode::NOT_FOUND,
                            "text/plain",
                            b"private fixture: no legacy asset fallback".to_vec(),
                        ))
                    });
                }
                // No API request can reach the user's legacy daemon. The WASM
                // artifact and frontend transport are real; backend data is a fixture.
                let data = match path {
                    "/conf" => {
                        serde_json::json!({"_lang":"en","VOLUME":"50","SPEAK_DONE":"0","SPEAK_ATTENTION":"0"})
                    }
                    "/prefs" => {
                        serde_json::json!({"favorites":[],"theme":"dia","button_style":"sutil","tabs_layout":"row"})
                    }
                    "/remote-state" => {
                        serde_json::json!({"remoteOn":false,"terminalState":"off","urls":{},"qrAvailable":false})
                    }
                    "/ssh" | "/tabs" | "/state" | "/tab-history" => serde_json::json!([]),
                    "/chains" => serde_json::json!({"chains":[]}),
                    "/commands/catalog" => {
                        serde_json::json!({"catalog":{"commands":[],"groups":[]},"cliInPane":"codex"})
                    }
                    "/workspace" => serde_json::json!({"revision":1,"groups":[],"tabs":[]}),
                    "/notices" | "/notices/watch" => {
                        serde_json::json!({"notices":[],"rev":"0"})
                    }
                    _ => serde_json::json!({}),
                };
                let long_poll = path == "/notices/watch";
                Box::pin(async move {
                    if long_poll {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    comandos_server::Reply::json(http::StatusCode::OK, &data)
                })
            })
        },
        websocket: None,
        limits: dash::limits(),
    };
    let (stop, rx) = watch::channel(false);
    let task = tokio::spawn(comandos_server::serve(listener, config, rx));
    // Exit early by creating this private sentinel; otherwise close after 300s.
    println!("NATIVE_SMOKE_STOP={}", root.join("stop").display());
    for _ in 0..300 {
        if root.join("stop").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    stop.send(true).unwrap();
    task.await.unwrap().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn compiled_metadata_admits_page_without_retired_modules() {
    let front = Front::start().await;
    let registry = Resolved::embedded(None);
    let mut selection = dash::web::Selection {
        on: comandos_core::web_assets::native_index_components()
            .unwrap()
            .into_iter()
            .collect(),
        shadow: Default::default(),
    };
    let manifest = dash::web::Manifest::load(&front.root.join("web")).unwrap();
    let plan =
        dash::web::native_page::admit(&registry, &selection, &manifest, &front.root.join("web"))
            .unwrap();
    assert_eq!(plan.ids.len(), 43);
    for gone in [
        "analytics",
        "analytics-render",
        "pomodoro",
        "notifications",
        "chain-builder",
    ] {
        assert!(!plan.ids.iter().any(|id| id == gone), "{gone}");
    }
    selection.on.remove("command-sidebar");
    assert!(
        dash::web::native_page::admit(&registry, &selection, &manifest, &front.root.join("web"))
            .is_err()
    );
    front.stop().await;
}

#[tokio::test]
async fn native_worker_requires_own_artifact_and_preserves_legacy_route() {
    let front = Front::start().await;
    std::fs::write(front.root.join(".claude/hooks/dash/sw.js"), "legacy-worker").unwrap();
    let missing = front.http("GET", "/sw.js?native=1", "", "").await;
    assert!(missing.starts_with("HTTP/1.1 503"), "{missing}");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(front.root.join("web/manifest.json")).unwrap())
            .unwrap();
    for name in [
        "comandos_web_sw.js",
        "comandos_web_sw_bg.wasm",
        "comandos_web_sw_boot.js",
        "comandos_web_sw_component.json",
    ] {
        manifest["files"][name] = serde_json::json!(format!("fedcba987654/{name}"));
        std::fs::write(front.root.join("web/fedcba987654").join(name), if name.ends_with("_component.json") { include_str!("../../comandos-web-sw/components/sw.json") } else if name.ends_with("_boot.js") {"importScripts({{SW_MODULE}}); wasm_bindgen.initSync({module:new Uint8Array()}); wasm_bindgen.boot(true, {{PRECACHE}});"} else {"worker-fixture"}).unwrap();
    }
    std::fs::write(
        front.root.join("web/manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let native = front.http("GET", "/sw.js?native=1", "", "").await;
    assert!(native.starts_with("HTTP/1.1 200"), "{native}");
    assert!(native.contains("service-worker-allowed: /"));
    assert!(native.contains("cache-control: no-store"));
    assert!(native.contains("/web/fedcba987654/comandos_web_sw.js"));
    assert!(native.contains("/?web=native"));
    assert!(!native.contains("{{") || !native.contains("PRECACHE"));
    assert!(!native.contains("legacy-worker"));
    for path in ["/sw.js", "/sw.js?native=10"] {
        let legacy = front.http("GET", path, "", "").await;
        assert!(
            legacy.starts_with("HTTP/1.1 200") && legacy.ends_with("legacy-worker"),
            "{legacy}"
        );
    }
    std::fs::remove_file(front.root.join("web/fedcba987654/comandos_web_sw_bg.wasm")).unwrap();
    assert!(
        front
            .http("GET", "/sw.js?native=1", "", "")
            .await
            .starts_with("HTTP/1.1 503")
    );
    front.stop().await;
}

#[tokio::test]
async fn dedicated_compiled_terminal_needs_no_main_wasm_or_source_and_validates_its_own_ready() {
    let front = Front::start_with_term(false, false, dash::term::TermMode::Native).await;
    let ids = vec!["term-main".to_string(), "term-tail".to_string()];
    let assets = [
        "/buttons.css",
        "/icon-192.png",
        "/assets/fonts/JetBrainsMono/JetBrainsMonoNerdFontMono-Regular.ttf",
        "/assets/fonts/JetBrainsMono/JetBrainsMonoNerdFontMono-Italic.ttf",
        "/assets/fonts/JetBrainsMono/JetBrainsMonoNerdFontMono-Bold.ttf",
        "/assets/fonts/JetBrainsMono/JetBrainsMonoNerdFontMono-BoldItalic.ttf",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, url)| (url.into(), format!("term-static-{i}")))
    .collect::<std::collections::BTreeMap<String, String>>();
    let page = comandos_core::web_assets::NativePage {
        version: 1,
        template_sha256: sha256_hex(
            comandos_web_view::term_page::shell(&Default::default())
                .into_string()
                .as_bytes(),
        ),
        components: ids.clone(),
        assets: assets.clone(),
    };
    let mut files = std::collections::BTreeMap::new();
    for name in [
        "comandos_term_web_boot.js",
        "comandos_term_web.js",
        "comandos_term_web_bg.wasm",
    ] {
        files.insert(name.to_string(), format!("fedcba987654/{name}"));
        std::fs::write(
            front.root.join("web/fedcba987654").join(name),
            b"term-payload",
        )
        .unwrap();
    }
    for name in assets.values() {
        files.insert(name.clone(), format!("fedcba987654/{name}"));
        std::fs::write(
            front.root.join("web/fedcba987654").join(name),
            b"term-static",
        )
        .unwrap();
    }
    let descriptor = dash::web::native_term_page::DESCRIPTOR;
    files.insert(descriptor.into(), format!("fedcba987654/{descriptor}"));
    std::fs::write(
        front.root.join("web/fedcba987654").join(descriptor),
        serde_json::to_vec(&page).unwrap(),
    )
    .unwrap();
    std::fs::write(
        front.root.join("web/manifest.json"),
        serde_json::to_vec(&comandos_core::web_assets::Manifest { files }).unwrap(),
    )
    .unwrap();
    std::fs::remove_file(front.root.join(".claude/hooks/dash/index.html")).unwrap();
    let response = front
        .http(
            "GET",
            "/term/?arg=demo&auth=fixture-token&theme=termius",
            "",
            "",
        )
        .await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("x-comandos-term: native"));
    assert!(response.contains("content=\"native-term\""));
    assert_eq!(response.matches("<!DOCTYPE html>").count(), 1);
    assert!(!response.contains("<script src=\"../assets/xterm"));
    assert!(
        response.find("id=\"pane-chrome\"").unwrap()
            < response.find("comandos_term_web_boot.js").unwrap()
    );
    assert!(response.contains("url('/web/fedcba987654/term-static-"));
    let k = nonce(&response);
    assert!(k.starts_with("native-term-"));
    let denied = front
        .http(
            "POST",
            "/web/ready",
            &serde_json::json!({"k":k,"mounted":["term-main"],"failed":[]}).to_string(),
            "",
        )
        .await;
    assert!(denied.starts_with("HTTP/1.1 400"));
    let ready = front
        .http(
            "POST",
            "/web/ready",
            &serde_json::json!({"k":k,"mounted":ids,"failed":[]}).to_string(),
            "",
        )
        .await;
    assert!(ready.starts_with("HTTP/1.1 200"), "{ready}");
    let explicit = front.http("GET", "/term/?web=native", "", "").await;
    assert!(explicit.starts_with("HTTP/1.1 200"));
    assert!(explicit.contains("comandos_term_web_boot.js"));
    let font = front
        .http(
            "GET",
            "/assets/fonts/JetBrainsMono/JetBrainsMonoNerdFontMono-Regular.ttf",
            "",
            "",
        )
        .await;
    assert!(
        font.starts_with("HTTP/1.1 200") && font.contains("term-static"),
        "{font}"
    );
    std::fs::remove_file(
        front
            .root
            .join("web/fedcba987654/comandos_term_web_bg.wasm"),
    )
    .unwrap();
    let missing = front.http("GET", "/term/?web=native", "", "").await;
    assert!(missing.starts_with("HTTP/1.1 503"));
    assert!(!missing.contains("legacy-exact"));
    let default_missing = front.http("GET", "/term/?arg=demo", "", "").await;
    assert!(default_missing.starts_with("HTTP/1.1 503"));
    assert!(!default_missing.contains("<html"));
    front.stop().await;
}

#[tokio::test]
async fn native_default_serves_compiled_page_and_status_without_legacy_sources() {
    let front = Front::start_config(false, true).await;
    let original = front.http("GET", "/?web=off", "", "").await;
    assert!(original.starts_with("HTTP/1.1 200") && original.ends_with(PAGE));
    std::fs::remove_file(front.root.join(".claude/hooks/dash/index.html")).unwrap();
    let page = front.http("GET", "/?app=1", "", "").await;
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(nonce(&page).starts_with("native-"));
    let status = front.http("GET", "/web/status", "", "").await;
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    let status: serde_json::Value =
        serde_json::from_str(status.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(status["mode"], "native");
    front.stop().await;
}
