//! Full installer proof using the retained release payload, never fabricated daemons.
//! Required: COMANDOS_INSTALL_NATIVE_PAYLOAD and COMANDOS_INSTALL_NATIVE_ARTIFACTS
//! (absolute paths). COMANDOS_INSTALL_NATIVE_CLI optionally selects an actual
//! release executable instead of CARGO_BIN_EXE_comandos. Services/font registration
//! are injected; all filesystem and
//! agents setup operations are real and confined to this test's private HOME.
#![cfg(target_os = "linux")]

use comandos_cli::install::{assets, full, plan::Action, platform::Platform};
use comandos_server::{ReplyBody, Request, dash};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static SEQ: AtomicU64 = AtomicU64::new(0);
struct Private(PathBuf);
impl Private {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "comandos-install-native-payload-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Private {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn input(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("missing {name}")));
    assert!(
        path.is_absolute() && path.is_dir(),
        "invalid {name}: {}",
        path.display()
    );
    path
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            assert!(!meta.is_symlink(), "payload symlink {}", path.display());
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            if meta.is_dir() {
                out.insert(rel, None);
                visit(root, &path, out);
            } else {
                assert!(meta.is_file(), "special payload entry {}", path.display());
                out.insert(rel, Some(fs::read(path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir(destination).unwrap();
    for (rel, bytes) in tree(source) {
        let at = destination.join(rel);
        if let Some(bytes) = bytes {
            fs::write(at, bytes).unwrap();
        } else {
            fs::create_dir(at).unwrap();
        }
    }
}
fn regular_native(source: &Path, destination: &Path) -> Vec<u8> {
    let source = fs::canonicalize(source).unwrap_or_else(|e| panic!("{}: {e}", source.display()));
    let meta = fs::symlink_metadata(&source).unwrap();
    assert!(meta.is_file() && meta.permissions().mode() & 0o111 != 0);
    let bytes = fs::read(&source).unwrap();
    assert!(
        bytes.starts_with(b"\x7fELF"),
        "not a real Linux native artifact {}",
        source.display()
    );
    fs::write(destination, &bytes).unwrap();
    fs::set_permissions(destination, fs::Permissions::from_mode(0o755)).unwrap();
    bytes
}
fn bytes_mode(path: &Path, bytes: &[u8], mode: u32) {
    let meta = fs::symlink_metadata(path).unwrap();
    assert!(
        meta.is_file(),
        "regular embedded resource {}",
        path.display()
    );
    assert_eq!(fs::read(path).unwrap(), bytes, "bytes {}", path.display());
    assert_eq!(
        meta.permissions().mode() & 0o777,
        mode,
        "mode {}",
        path.display()
    );
}

#[test]
#[ignore = "requires retained final Linux native artifacts and full web payload"]
fn full_installer_preserves_complete_native_payload_and_declares_legacy_replacements() {
    assert_eq!(
        Platform::current(),
        Platform::LinuxNative,
        "this is the Linux full-tree proof"
    );
    assert_ne!(
        std::env::var("COMANDOS_RETIRE_TELEGRAM").as_deref(),
        Ok("1"),
        "retirement is outside this fixture"
    );
    let payload = input("COMANDOS_INSTALL_NATIVE_PAYLOAD");
    let artifacts = input("COMANDOS_INSTALL_NATIVE_ARTIFACTS");
    let expected_web = tree(&payload);
    assert_eq!(
        expected_web.values().filter(|v| v.is_some()).count(),
        63,
        "retained full payload changed"
    );
    let manifest: comandos_core::web_assets::Manifest =
        serde_json::from_slice(expected_web[Path::new("manifest.json")].as_ref().unwrap()).unwrap();
    manifest.check_paths().unwrap();
    assert_eq!(
        manifest.files.len(),
        61,
        "manifest artifact count excludes both records"
    );
    let receipt: Value = serde_json::from_slice(
        expected_web[Path::new(".comandos-web-build.json")]
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["producer"], "comandos-xtask-web-build");
    assert_eq!(receipt["version"], 1);
    assert_eq!(
        receipt["manifest_sha256"],
        format!(
            "{:x}",
            Sha256::digest(expected_web[Path::new("manifest.json")].as_ref().unwrap())
        )
    );
    let hashes = receipt["files"].as_object().unwrap();
    assert_eq!(hashes.len(), 61);
    for rel in manifest.files.values() {
        assert_eq!(
            hashes[rel],
            format!(
                "{:x}",
                Sha256::digest(expected_web[Path::new(rel)].as_ref().unwrap())
            ),
            "producer hash {rel}"
        );
    }
    let declared: BTreeSet<_> = manifest
        .files
        .values()
        .map(PathBuf::from)
        .chain([
            PathBuf::from("manifest.json"),
            PathBuf::from(".comandos-web-build.json"),
        ])
        .collect();
    assert_eq!(
        declared,
        expected_web
            .iter()
            .filter_map(|(p, v)| v.as_ref().map(|_| p.clone()))
            .collect(),
        "unmanifested/missing web files, with exactly the two declared producer/manifest records"
    );

    let private = Private::new();
    let home = private.0.join("home");
    fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
    let source = private.0.join("input-release");
    fs::create_dir(&source).unwrap();
    let cli_source = std::env::var_os("COMANDOS_INSTALL_NATIVE_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_comandos")));
    assert!(
        cli_source.is_absolute(),
        "COMANDOS_INSTALL_NATIVE_CLI must be absolute"
    );
    let cli = regular_native(&cli_source, &source.join("comandos"));
    let separate: BTreeMap<_, _> = ["comandos-app", "comandos-notifyd", "cc-model-proxy"]
        .into_iter()
        .map(|name| {
            (
                name,
                regular_native(&artifacts.join(name), &source.join(name)),
            )
        })
        .collect();
    copy_tree(&payload, &source.join("web"));
    let actions = comandos_cli::install::plan::plan(&home, Platform::LinuxNative, &source);
    let mut external = Vec::new();
    let mut operations = Vec::new();
    let mut runner = |action: &Action| -> Result<(), String> {
        match action {
            Action::InstallFonts(path) => {
                assert_eq!(path, &home.join(".local/share/fonts/comandos"));
                external.push("fonts");
                Ok(())
            }
            Action::Systemctl { home: at, args } => {
                assert_eq!(at, &home);
                assert_eq!(args, &["--user", "daemon-reload"]);
                external.push("daemon-reload");
                Ok(())
            }
            Action::ExtensionOperation {
                home: at,
                journal,
                operation,
                quiescent,
            } => {
                assert_eq!(at, &home);
                assert!(journal.starts_with(&home));
                assert_eq!(operation, "agents-setup");
                operations.push(operation.clone());
                let result =
                    comandos_cli::install::extension_worker::execute(at, journal, operation);
                *quiescent.borrow_mut() = true;
                result
            }
            _ => panic!("unexpected external operation: {action:?}"),
        }
    };
    let args = vec![
        "--home".into(),
        home.to_string_lossy().into_owned(),
        "--release".into(),
        source.to_string_lossy().into_owned(),
    ];
    assert_eq!(full::run_with(&args, &mut runner).unwrap(), 0);
    assert_eq!(external, ["daemon-reload", "fonts"]);
    assert_eq!(operations, ["agents-setup"]);
    let installed_cli = fs::canonicalize(home.join(".local/share/comandos/bin/comandos")).unwrap();
    assert!(installed_cli.starts_with(home.join(".local/share/comandos/releases")));
    bytes_mode(&installed_cli, &cli, 0o755);
    let installed_web = installed_cli.parent().unwrap().join("web");
    assert_eq!(
        tree(&installed_web),
        expected_web,
        "entire installed web tree, including manifest and empty directories"
    );
    for (rel, bytes) in &expected_web {
        if let Some(bytes) = bytes {
            bytes_mode(&installed_web.join(rel), bytes, 0o644);
        }
    }
    for action in &actions {
        match action {
            Action::Link { name, at, .. } => {
                let resolved = fs::canonicalize(at).unwrap();
                assert!(
                    resolved.starts_with(&home),
                    "alias escapes private installation {}",
                    at.display()
                );
                let artifact = match name.as_str() {
                    "cc-app" => Some("comandos-app"),
                    "cc-notifyd" => Some("comandos-notifyd"),
                    "cc-model-proxy" => Some("cc-model-proxy"),
                    _ => None,
                };
                if let Some(artifact) = artifact {
                    bytes_mode(&resolved, &separate[artifact], 0o755);
                    let receipt: Value = serde_json::from_slice(
                        &fs::read(resolved.parent().unwrap().join("manifest.json")).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(receipt["artifact"], artifact);
                    assert_eq!(
                        receipt["sha256"],
                        format!("{:x}", Sha256::digest(&separate[artifact]))
                    );
                } else {
                    assert_eq!(resolved, installed_cli, "alias {}", at.display());
                }
            }
            Action::Mkdir(path, _) => assert!(path.is_dir(), "directory {}", path.display()),
            Action::WriteIfAbsent { path, bytes, mode } => bytes_mode(path, bytes, *mode),
            Action::WriteUnit { path, bytes, .. } => bytes_mode(path, bytes, 0o644),
            Action::Write { path, bytes, mode } => bytes_mode(path, bytes, *mode),
            _ => {}
        }
    }
    let baseline: Value =
        serde_json::from_str(include_str!("fixtures/install-legacy-linux.json")).unwrap();
    for old in baseline["entries"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|name| name.starts_with(".local/bin/"))
    {
        if matches!(
            old.as_str(),
            ".local/bin/cc-app-mac" | ".local/bin/cc_usage.py"
        ) {
            continue;
        }
        let at = home.join(old);
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, Action::Link { at: planned, .. } if planned == &at)),
            "frozen legacy alias absent from native plan: {}",
            at.display()
        );
        assert!(at.is_symlink() && fs::canonicalize(&at).unwrap().starts_with(&home));
    }
    for resource in assets::CONFIG {
        bytes_mode(&home.join(resource.path), resource.bytes, resource.mode);
    }
    for font in assets::FONTS {
        bytes_mode(
            &home.join(".local/share/fonts/comandos").join(font.path),
            font.bytes,
            font.mode,
        );
    }
    // Prove independence from the transient input release, not just byte parity while it exists.
    fs::remove_dir_all(&source).unwrap();
    for (name, args, expected) in [
        ("comandos", &["--version"][..], "comandos"),
        ("cc-dash", &["--help"][..], "uso: comandos dash"),
    ] {
        let output = Command::new(home.join(".local/bin").join(name))
            .args(args)
            .env_clear()
            .env("HOME", &home)
            .env("PATH", "/usr/bin:/bin")
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("XDG_CACHE_HOME", home.join(".cache"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "installed {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(expected));
    }
    let web_manifest = dash::web::Manifest::load(&installed_web).unwrap();
    let page = dash::web::native_page::bundle(&web_manifest, &installed_web).unwrap();
    let terminal = dash::web::native_term_page::bundle(&web_manifest, &installed_web).unwrap();
    assert_eq!(page.components.len(), 48);
    assert_eq!(terminal.components, ["term-main", "term-tail"]);
    legacy_frontend_replacements(
        &home,
        &page.components,
        &page.assets,
        &terminal.assets,
        &manifest,
        &installed_web,
    );
    // Explicit fixture opt-in, not a claim that installation enables native web by default.
    fs::write(
        home.join(".claude/hooks/comandos-web.json"),
        json!({"on":page.components,"shadow":[]}).to_string(),
    )
    .unwrap();
    installed_http(&home, &installed_web, &manifest);
    eprintln!(
        "FULL installer: 61 exact manifest artifacts + manifest/producer records (63 regular web files total); complete alias/resource/font destinations; 33 frontend replacements; Darwin-only alias + internal usage replacement; installed CLI and source-less native HTTP admitted"
    );
}

fn legacy_frontend_replacements(
    home: &Path,
    components: &[String],
    assets: &BTreeMap<String, String>,
    term_assets: &BTreeMap<String, String>,
    manifest: &comandos_core::web_assets::Manifest,
    web: &Path,
) {
    // These are replacements, not byte equality with the retired Python/static tree.
    // Source contracts: native_page::bundle, native_term_page::bundle, embedded
    // component inventory; Phase 6 artifact table (cc-app-mac and cc_usage.py).
    let baseline: Value =
        serde_json::from_str(include_str!("fixtures/install-legacy-linux.json")).unwrap();
    let replacements = [
        ("", "bundle"),
        ("analytics-render.js", "retired"),
        ("analytics.css", "retired"),
        ("analytics.js", "retired"),
        ("assets", "assets"),
        ("buttons.css", "/buttons.css"),
        ("chain-builder.js", "retired"),
        ("command-sidebar.js", "command-sidebar"),
        ("device-drafts.js", "term-main"),
        ("extensions.css", "extensions"),
        ("extensions.html", "extensions"),
        ("extensions.js", "extensions"),
        ("icon-192.png", "/icon-192.png"),
        ("icon-512.png", "/icon-512.png"),
        ("icons", "iconos"),
        ("index.html", "bundle"),
        ("manifest.webmanifest", "/manifest.webmanifest"),
        ("news-reader.js", "news-reader"),
        ("notifications.js", "retired"),
        ("pomodoro.js", "retired"),
        ("push-settings.js", "push-settings"),
        ("quick-terminal.js", "quick-terminal"),
        ("session-config.js", "session-config"),
        ("session-controls.js", "term-main"),
        ("sw.js", "sw"),
        ("term.html", "term-main"),
        ("ui-sounds.js", "ui-sounds"),
        ("vendor", "vendor"),
        ("work-marks.js", "work-marks"),
        ("workspace-dock.js", "workspace-dock"),
        ("workspace-layout.js", "workspace-layout"),
        ("workspace.css", "/workspace.css"),
        ("workspace.js", "workspace"),
    ];
    let old: BTreeSet<_> = baseline["entries"]
        .as_object()
        .unwrap()
        .keys()
        .filter_map(|path| {
            path.strip_prefix(".claude/hooks/dash")
                .map(|s| s.trim_start_matches('/'))
        })
        .collect();
    assert_eq!(old, replacements.iter().map(|(old, _)| *old).collect());
    assert_eq!(old.len(), 33);
    assert!(!home.join(".claude/hooks/dash").exists());
    for (old, replacement) in replacements {
        match replacement {
            // Deleted from the dashboard (Analytics, Pomodoro, notices drawer,
            // Cadenas): no native replacement and no packaged asset.
            "retired" => {
                let stem = old.trim_end_matches(".js").trim_end_matches(".css");
                assert!(
                    !components.iter().any(|v| v == stem),
                    "retired {old} still a native component"
                );
                assert!(
                    !assets.contains_key(&format!("/{old}")),
                    "retired {old} still packaged"
                );
            }
            "bundle" => assert!(manifest.files.contains_key("comandos_native_page.json")),
            "assets" => {
                assert!(!assets.is_empty());
                assert_eq!(
                    term_assets
                        .keys()
                        .filter(|v| v.starts_with("/assets/fonts/"))
                        .count(),
                    4
                );
            }
            "term-main" => assert!(
                manifest
                    .files
                    .contains_key("comandos_native_term_page.json")
            ),
            "vendor" => {
                for id in ["vendor-markdown-it", "vendor-purify"] {
                    assert!(components.iter().any(|v| v == id));
                }
            }
            "sw" => {
                for key in [
                    "comandos_web_sw.js",
                    "comandos_web_sw_bg.wasm",
                    "comandos_web_sw_boot.js",
                    "comandos_web_sw_component.json",
                ] {
                    assert!(manifest.files.contains_key(key));
                }
                let receipt: Value = serde_json::from_slice(
                    &fs::read(web.join(&manifest.files["comandos_web_sw_component.json"])).unwrap(),
                )
                .unwrap();
                assert_eq!(receipt["id"], "sw");
                assert_eq!(receipt["artifact"], "comandos-web-sw");
            }
            root if root.starts_with('/') => {
                assert!(
                    assets.contains_key(root),
                    "legacy {old} -> native asset {root}"
                );
            }
            id => assert!(
                components.iter().any(|v| v == id),
                "legacy {old} -> native component {id}"
            ),
        }
    }
    for alias in ["cc-app-mac", "cc_usage.py"] {
        assert!(
            baseline["entries"]
                .get(format!(".local/bin/{alias}"))
                .is_some()
        );
        assert!(!home.join(".local/bin").join(alias).exists());
    }
    // Darwin's App binary is a separate platform artifact. Usage is internal Rust
    // state/backend, not a compatibility executable named cc_usage.py.
    assert!(dash::native::route(&"GET".parse().unwrap(), "/usage/state").is_some());
}

fn installed_http(home: &Path, web: &Path, manifest: &comandos_core::web_assets::Manifest) {
    // Real HTTP handler and compiled admission, without a TCP listener, native
    // application census, term actors, GTK, tmux or any user/default socket.
    dash::runtime().unwrap().block_on(async {
        let mut cfg = dash::parse_args(&[], home, None).unwrap();
        cfg.native = false;
        cfg.usage_effects = false;
        cfg.repo_root = None;
        cfg.web_dir = web.to_path_buf();
        cfg.term = dash::term::TermMode::Off;
        cfg.token_file = None;
        cfg.token = b"private-install-token".to_vec();
        let (server, native) = dash::build(cfg, None);
        assert!(native.is_none());
        for (url, mode, boot) in [
            ("/?web=native", "native", "comandos_web_boot.js"),
            (
                "/term/?web=native&arg=private-unused",
                "native-term",
                "comandos_term_web_boot.js",
            ),
        ] {
            let reply = (server.handler)(request(url)).await.unwrap();
            assert_eq!(reply.status.as_u16(), 200, "native installed page {url}");
            let ReplyBody::Bytes(bytes) = reply.body else {
                panic!("unexpected page stream")
            };
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(text.contains(&format!("name=\"comandos-web-mode\" content=\"{mode}\"")));
            assert!(text.contains(&format!("src=\"/web/{}\"", manifest.files[boot])));
        }
        for rel in manifest.files.values() {
            let reply = (server.handler)(request(&format!("/web/{rel}")))
                .await
                .unwrap();
            assert_eq!(reply.status.as_u16(), 200, "installed asset HTTP {rel}");
            let ReplyBody::Bytes(bytes) = reply.body else {
                panic!("unexpected asset stream {rel}")
            };
            assert_eq!(
                bytes.as_ref(),
                fs::read(web.join(rel)).unwrap(),
                "HTTP bytes {rel}"
            );
        }
    });
}
fn request(target: &str) -> Request {
    Request {
        method: "GET".parse().unwrap(),
        target: target.into(),
        peer: "127.0.0.1:42424".parse().unwrap(),
        headers: vec![("host".into(), "127.0.0.1".into())],
        data: None,
        body: Default::default(),
        internal_producer: false,
    }
}
