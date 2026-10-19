use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "catalog-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn put(&self, p: &str, s: &str) {
        let p = self.0.join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }
    fn read(&self, p: &str) -> String {
        fs::read_to_string(self.0.join(p)).unwrap()
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn cli(h: &Path, action: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_comandos-extensions"))
        .arg("--home")
        .arg(h)
        .arg(action)
        .output()
        .unwrap()
}
fn ok(h: &Home, action: &str) -> Value {
    let out = cli(&h.0, action);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn import_sync_status_and_noop() {
    let h = Home::new();
    h.put(
        ".claude.json",
        r#"{"mcpServers":{"demo":{"command":"echo","args":[]}}}"#,
    );
    assert_eq!(ok(&h, "import")["servers"], 1);
    assert!(!cli(&h.0, "import").status.success());
    assert_eq!(ok(&h, "status")["servers"][0]["name"], "demo");
    assert_eq!(ok(&h, "sync")["configurations_changed"], 5);
    let original = h.read(".codex/config.toml");
    assert_eq!(
        ok(&h, "sync"),
        json!({"configurations_changed":0,"skill_links_changed":0})
    );
    assert_eq!(original, h.read(".codex/config.toml"));
}
#[test]
fn native_edits_disable_and_reenable_policy() {
    let h = Home::new();
    h.put(".codex/config.toml","model=\"keep\"\n# retained\n[mcp_servers.demo]\ncommand=\"echo\"\n[mcp_servers.demo.tools.write]\napproval_mode=\"approve\"\n");
    ok(&h, "import");
    ok(&h, "sync");
    assert!(h.read(".codex/config.toml").contains("# retained"));
    h.put(".claude.json", r#"{"mcpServers":{}}"#);
    ok(&h, "sync");
    assert_eq!(ok(&h, "status")["servers"][0]["enabled"], false);
    let mut c: Value =
        serde_json::from_str(&h.read(".config/comandos/extensions/catalog.json")).unwrap();
    c["servers"]["demo"]["enabled"] = json!(true);
    h.put(".config/comandos/extensions/catalog.json", &c.to_string());
    ok(&h, "sync");
    assert!(h.read(".codex/config.toml").contains("approve"));
    let codex = config::read_config(&h.0.join(".codex/config.toml")).unwrap();
    assert_eq!(codex["model"], "keep");
    assert_eq!(
        codex["mcp_servers"]["demo"]["tools"]["write"]["approval_mode"],
        "approve"
    );
}
#[test]
fn skills_complete_tree_reinstall_and_conflict() {
    let h = Home::new();
    h.put(".claude/skills/demo/SKILL.md", "old");
    h.put(".claude/skills/demo/resources/data", "resource");
    ok(&h, "import");
    ok(&h, "sync");
    assert_eq!(h.read(".agents/skills/demo/resources/data"), "resource");
    fs::remove_file(h.0.join(".grok/skills/demo")).unwrap();
    h.put(".grok/skills/demo/SKILL.md", "new");
    ok(&h, "sync");
    assert_eq!(h.read(".agents/skills/demo/SKILL.md"), "new");
    assert_eq!(h.read(".claude/skills/demo/SKILL.md"), "new");
    for (root, content) in [(".grok", "one"), (".claude", "two")] {
        fs::remove_file(h.0.join(format!("{root}/skills/demo"))).unwrap();
        h.put(&format!("{root}/skills/demo/SKILL.md"), content);
    }
    assert!(!cli(&h.0, "sync").status.success());
    assert_eq!(h.read(".grok/skills/demo/SKILL.md"), "one");
    assert_eq!(h.read(".claude/skills/demo/SKILL.md"), "two");
}
use comandos_extensions::{catalog as cat, config, skills};
fn fixture_catalog() -> Value {
    json!({"version":1,"servers":{"demo":{"command":"echo","args":[],"enabled":true}}})
}
fn sync(h: &Home, c: &Value) -> Value {
    let mut observed = json!({});
    cat::sync_configs(&h.0, c, "/launcher", &mut observed, None).unwrap();
    observed
}
#[test]
fn mail_only_secondary_clients_and_projects() {
    let h = Home::new();
    for (p, key) in [
        (".gemini/settings.json", "mcpServers"),
        (".config/Code/User/mcp.json", "servers"),
        (".config/github-copilot/intellij/mcp.json", "servers"),
    ] {
        h.put(
            p,
            &json!({"tools":{"keep":true},key:{"unrelated":{"url":"https://example.com/keep"}}})
                .to_string(),
        );
    }
    h.put(".claude.json",&json!({"projects":{"/work":{"mcpServers":{"demo":{"url":"https://different.example/mcp"},"gmail":{"url":"http://127.0.0.1:7288/mcp"}}}}}).to_string());
    let c = json!({"version":1,"servers":{"gmail":{"command":"mail"},"proton-mail":{"command":"mail"},"demo":{"command":"echo"}}});
    sync(&h, &c);
    for (p, key) in [
        (".gemini/settings.json", "mcpServers"),
        (".config/Code/User/mcp.json", "servers"),
        (".config/github-copilot/intellij/mcp.json", "servers"),
    ] {
        let d = config::read_config(&h.0.join(p)).unwrap();
        assert_eq!(d["tools"]["keep"], true);
        assert_eq!(d[key]["unrelated"]["url"], "https://example.com/keep");
        assert!(d[key].get("gmail").is_some());
        assert!(d[key].get("proton-mail").is_some());
        assert!(d[key].get("demo").is_none());
    }
    let d = config::read_config(&h.0.join(".claude.json")).unwrap();
    assert_eq!(
        d["projects"]["/work"]["mcpServers"]["demo"]["url"],
        "https://different.example/mcp"
    );
    assert_eq!(
        d["projects"]["/work"]["mcpServers"]["gmail"]["command"],
        "/launcher"
    );
    cat::save_snapshot(&h.0, &c, None).unwrap();
    let mut edited = Vec::new();
    for (p, key) in [
        (".gemini/settings.json", "mcpServers"),
        (".config/Code/User/mcp.json", "servers"),
        (".config/github-copilot/intellij/mcp.json", "servers"),
    ] {
        let path = h.0.join(p);
        let mut data = config::read_config(&path).unwrap();
        data[key]["unrelated"]["url"] = json!("https://example.com/changed");
        fs::write(&path, data.to_string()).unwrap();
        edited.push(path);
    }
    assert_eq!(
        cat::reconcile(&h.0, &c, "/launcher", &mut json!({})).unwrap(),
        c
    );
    sync(&h, &c);
    for path in &edited {
        assert_eq!(config::read_config(path).unwrap()["tools"]["keep"], true);
        let key = if path.ends_with("settings.json") {
            "mcpServers"
        } else {
            "servers"
        };
        assert_eq!(
            config::read_config(path).unwrap()[key]["unrelated"]["url"],
            "https://example.com/changed"
        );
    }
    cat::save_snapshot(&h.0, &c, None).unwrap();
    let before = edited
        .iter()
        .map(|p| fs::read(p).unwrap())
        .collect::<Vec<_>>();
    assert!(
        cat::sync_configs(&h.0, &c, "/launcher", &mut json!({}), None)
            .unwrap()
            .is_empty()
    );
    for (path, bytes) in edited.iter().zip(before) {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}
#[test]
fn import_alias_precedence_remote_rules_local_service_and_future_accounts() {
    let h = Home::new();
    h.put(".claude.json",r#"{"mcpServers":{"gmail":{"url":"https://example.com"},"linear-server":{"command":"first"},"linear":{"command":"second"},"unsafe.name":{"command":"x"},"node_repl":{"command":"x"},"playwright":{"command":"x"}}}"#);
    h.put(".codex/config.toml","[mcp_servers.gmail]\nurl='http://127.0.0.1:7000/mcp'\n[mcp_servers.chrome-current]\ncommand='old'\n[mcp_servers.off]\ncommand='echo'\nenabled=false\n");
    let c = cat::import_catalog(&h.0).unwrap();
    assert_eq!(c["servers"]["linear"]["command"], "first");
    assert_eq!(c["servers"]["gmail"]["url"], "http://127.0.0.1:7000/mcp");
    assert_eq!(
        c["servers"]["chrome-bg"]["command"],
        h.0.join(".local/bin/cc-browser-remote")
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(c["servers"]["off"]["enabled"], false);
    assert_eq!(c["servers"]["playwright"]["enabled"], false);
    assert!(c["servers"].get("node_repl").is_none());
    assert!(c["servers"].get("unsafe.name").is_none());
    sync(&h, &c);
    cat::save_snapshot(&h.0, &c, None).unwrap();
    h.put(
        ".claude-accounts/new/.claude.json",
        r#"{"mcpServers":{"extra":{"command":"extra-command"}}}"#,
    );
    let revised = cat::reconcile(&h.0, &c, "/launcher", &mut json!({})).unwrap();
    assert_eq!(revised["servers"]["extra"]["command"], "extra-command");
    std::os::unix::fs::symlink(
        h.0.join(".claude-accounts/new"),
        h.0.join(".claude-accounts/alias"),
    )
    .unwrap();
    assert!(
        !cat::targets(&h.0)
            .unwrap()
            .iter()
            .any(|t| t.path.to_string_lossy().contains("accounts/alias"))
    );
    sync(&h, &revised);
    for target in cat::targets(&h.0).unwrap() {
        // OpenCode y agy reciben solo el hub, que alcanza también a `extra`.
        let expected = if cat::uses_hub(target.kind) {
            "comandos"
        } else {
            "extra"
        };
        assert!(
            config::read_config(&target.path).unwrap()[target.key]
                .get(expected)
                .is_some()
        );
    }
}
#[test]
fn snapshots_and_export_reject_intervening_edits() {
    let h = Home::new();
    let c = fixture_catalog();
    let observed = sync(&h, &c);
    cat::save_snapshot(&h.0, &c, Some(&observed)).unwrap();
    let mut inputs = json!({});
    let revised = cat::reconcile(&h.0, &c, "/launcher", &mut inputs).unwrap();
    h.put(
        ".claude.json",
        r#"{"mcpServers":{"demo":{"command":"new"}}}"#,
    );
    assert!(cat::save_snapshot(&h.0, &c, Some(&observed)).is_err());
    assert!(cat::sync_configs(&h.0, &revised, "/launcher", &mut json!({}), Some(&inputs)).is_err());
    assert_eq!(
        config::read_config(&h.0.join(".claude.json")).unwrap()["mcpServers"]["demo"]["command"],
        "new"
    );
}
#[test]
fn simultaneous_catalog_native_and_multiple_native_conflicts_preserve_inputs() {
    let h = Home::new();
    let mut c = fixture_catalog();
    sync(&h, &c);
    cat::save_snapshot(&h.0, &c, None).unwrap();
    h.put(
        ".claude.json",
        r#"{"mcpServers":{"demo":{"command":"one"}}}"#,
    );
    c["servers"]["demo"]["command"] = json!("catalog-edit");
    assert!(
        cat::reconcile(&h.0, &c, "/launcher", &mut json!({}))
            .unwrap_err()
            .contains("Simultaneous")
    );
    c["servers"]["demo"]["command"] = json!("echo");
    h.put(".grok/config.toml", "[mcp_servers.demo]\ncommand='two'\n");
    assert!(
        cat::reconcile(&h.0, &c, "/launcher", &mut json!({}))
            .unwrap_err()
            .contains("Conflicting native")
    );
    assert!(h.read(".claude.json").contains("one"));
}
#[test]
fn grok_disable_list_and_client_owned_permission_edits() {
    let h = Home::new();
    let c = fixture_catalog();
    sync(&h, &c);
    cat::save_snapshot(&h.0, &c, None).unwrap();
    h.put(".grok/config.toml","disabled_mcp_servers=['unrelated','demo']\n[mcp_servers.demo]\ncommand='/launcher'\nargs=['serve','demo']\nrequired=true\nstartup_timeout_sec=80\n");
    let revised = cat::reconcile(&h.0, &c, "/launcher", &mut json!({})).unwrap();
    assert_eq!(revised["servers"]["demo"]["enabled"], false);
    sync(&h, &c);
    let d = config::read_config(&h.0.join(".grok/config.toml")).unwrap();
    assert_eq!(d["disabled_mcp_servers"], json!(["unrelated"]));
    assert_eq!(d["mcp_servers"]["demo"]["required"], true);
    assert_eq!(d["mcp_servers"]["demo"]["startup_timeout_sec"], 80);
    cat::save_snapshot(&h.0, &c, None).unwrap();
    h.put(
        ".grok/config.toml",
        &h.read(".grok/config.toml").replace("80", "90"),
    );
    assert_eq!(
        cat::reconcile(&h.0, &c, "/launcher", &mut json!({})).unwrap(),
        c
    );
}
#[test]
fn skill_provider_alias_relative_resources_materialization_and_external_links() {
    let h = Home::new();
    h.put(".claude/skills/demo/SKILL.md", "hello");
    h.put(".claude/skills/common/data", "resource");
    std::os::unix::fs::symlink("../common", h.0.join(".claude/skills/demo/resources")).unwrap();
    fs::create_dir_all(h.0.join(".agents/skills")).unwrap();
    std::os::unix::fs::symlink(
        h.0.join(".claude/skills/demo"),
        h.0.join(".agents/skills/demo"),
    )
    .unwrap();
    h.put("repo/external/SKILL.md", "external");
    std::os::unix::fs::symlink(
        h.0.join("repo/external"),
        h.0.join(".agents/skills/external"),
    )
    .unwrap();
    fs::create_dir_all(h.0.join(".codex")).unwrap();
    std::os::unix::fs::symlink(h.0.join(".agents/skills"), h.0.join(".codex/skills")).unwrap();
    skills::sync_skills(&h.0).unwrap();
    assert!(!h.0.join(".agents/skills/demo").is_symlink());
    assert!(h.0.join(".agents/skills/external").is_symlink());
    assert_eq!(h.read(".agents/skills/demo/resources/data"), "resource");
    assert_eq!(h.read(".claude/skills/demo/SKILL.md"), "hello");
    assert!(skills::sync_skills(&h.0).unwrap().is_empty());
}
#[test]
fn failed_skill_copy_keeps_original_and_existing_destination() {
    let h = Home::new();
    h.put(".agents/skills/demo/SKILL.md", "old");
    skills::sync_skills(&h.0).unwrap();
    fs::remove_file(h.0.join(".grok/skills/demo")).unwrap();
    h.put(".grok/skills/demo/SKILL.md", "new");
    std::os::unix::fs::symlink("missing", h.0.join(".grok/skills/demo/broken")).unwrap();
    assert!(skills::sync_skills(&h.0).is_err());
    assert_eq!(h.read(".agents/skills/demo/SKILL.md"), "old");
    assert_eq!(h.read(".grok/skills/demo/SKILL.md"), "new");
    assert!(!fs::read_dir(h.0.join(".agents/skills")).unwrap().any(|p| {
        p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".adopt")
    }));
}
#[test]
fn private_backups_symlink_and_concurrent_write_rejection() {
    use std::os::unix::fs::PermissionsExt;
    let h = Home::new();
    h.put("file", "original");
    let path = h.0.join("file");
    assert!(config::replace_config(&h.0, &path, Some(b"other"), b"new").is_err());
    assert!(config::replace_config(&h.0, &path, Some(b"original"), b"new").unwrap());
    let backups = fs::read_dir(config::state_dir(&h.0).join("backups"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read(&backups[0]).unwrap(), b"original");
    assert_eq!(
        fs::metadata(&backups[0]).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::write(&backups[0], b"owned").unwrap();
    h.put("file", "original");
    assert!(config::replace_config(&h.0, &path, Some(b"original"), b"new").is_err());
    assert_eq!(h.read("file"), "original");
    assert_eq!(fs::read(&backups[0]).unwrap(), b"owned");
    std::os::unix::fs::symlink(&path, h.0.join("alias")).unwrap();
    assert!(config::replace_config(&h.0, &h.0.join("alias"), Some(b"new"), b"bad").is_err());
    assert_eq!(h.read("file"), "original");
}
#[test]
fn jsonc_and_reserved_names_and_invalid_errors() {
    let h = Home::new();
    let p = h.0.join("config.jsonc");
    let d=config::Document::parse(&p,Some(br#"{/*comment*/"url":"https://host/path//?q=,}","literal":"/* keep */", "nested":{"$serde_json::private::Number":"123","$serde_json::private::RawValue":"opaque","x":1,},}"#)).unwrap();
    assert_eq!(d.data["nested"]["x"], 1);
    assert_eq!(d.data["literal"], "/* keep */");
    for raw in [
        br#"{"secret":"DO_NOT_LEAK","secret":1}"#.as_slice(),
        br#"{"nested":{"secret":"DO_NOT_LEAK","secret":1}}"#,
        &[0xff],
    ] {
        let e = config::Document::parse(&p, Some(raw)).err().unwrap();
        assert!(e.contains(p.to_str().unwrap()));
        assert!(!e.contains("DO_NOT_LEAK"));
    }
}
#[test]
fn toml_fingerprint_types_bigints_and_unknown_retention() {
    let h = Home::new();
    let p = h.0.join("config.toml");
    let text = "# unknown retained\nbig=9223372036854775808\nnegative=-9223372036854775809\ndate=1979-05-27\n[mcp_servers.demo]\ncommand=\"echo\"\nenabled=false\nother=true\nwhen=1979-05-27T07:32:00.123456789Z\nlocal=1979-05-27T07:32:00\ndate=1979-05-27\ntime=07:32:00.1\ntruth=[true,false]\n[[mcp_servers.demo.unknown]]\nfoo=true\n";
    let mut doc = config::Document::parse(&p, Some(text.as_bytes())).unwrap();
    assert_eq!(
        doc.native_fingerprints("mcp_servers").unwrap()["demo"],
        "822e3099d0d4c1342096dc0c363e02c974529da9134d48a135b3b48d61b48ced"
    );
    doc.data["mcp_servers"]["demo"] = cat::native_entry("codex", "demo", "/launcher", &json!({}));
    let rendered = String::from_utf8(doc.render().unwrap()).unwrap();
    assert!(rendered.contains("big=9223372036854775808"));
    assert!(rendered.contains("negative=-9223372036854775809"));
    assert!(rendered.contains("date=1979-05-27"));
    assert!(rendered.contains("# unknown retained"));
    let d = config::Document::parse(&p, Some(rendered.as_bytes())).unwrap();
    assert_eq!(d.data["big"].to_string(), "9223372036854775808");
}
#[test]
fn toml_new_bigint_policy_and_nonfinite_survive_sync() {
    let h = Home::new();
    h.put(".codex/config.toml","unknown=nan\n[mcp_servers.demo]\ncommand='echo'\nstartup_timeout_sec=nan\n[mcp_servers.node_repl]\ncommand='native'\nn=inf\n");
    let c = fixture_catalog();
    sync(&h, &c);
    let first = h.read(".codex/config.toml");
    sync(&h, &c);
    assert_eq!(h.read(".codex/config.toml"), first);
    let p = h.0.join("new.toml");
    let mut doc = config::Document::parse(&p, None).unwrap();
    doc.data = json!({"mcp_servers":{"demo":{"command":"/launcher","startup_timeout_sec":serde_json::Number::from_string_unchecked("9999999999999999999999999999999999999999".into())}}});
    let raw = doc.render().unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("9999999999999999999999999999999999999999"));
}
#[test]
fn invalid_toml_and_unserializable_native_policies_fail_safely() {
    let h = Home::new();
    let p = h.0.join("config.toml");
    for s in [
        "secret='DO_NOT_LEAK'\nsecret=1",
        "v=9223372036854775808_abc",
        "v=0xFFFFFFFFFFFFFFFFFF_",
        "[x]\nv=1\n[x]\nv=2",
    ] {
        assert!(config::Document::parse(&p, Some(s.as_bytes())).is_err());
    }
    for value in ["1979-05-27", "[true,false]"] {
        h.put(
            ".codex/config.toml",
            &format!("[mcp_servers.demo]\ncommand='echo'\nrequired={value}\n"),
        );
        let before = h.read(".codex/config.toml");
        assert!(
            cat::sync_configs(&h.0, &fixture_catalog(), "/launcher", &mut json!({}), None).is_err()
        );
        assert_eq!(h.read(".codex/config.toml"), before);
    }
}
#[test]
fn simultaneous_imports_obey_cross_process_sync_lock() {
    use std::{process::Stdio, time::Duration};
    let h = Home::new();
    let guard = config::SyncLock::new(&h.0).unwrap();
    let spawn = || {
        Command::new(env!("CARGO_BIN_EXE_comandos-extensions"))
            .arg("--home")
            .arg(&h.0)
            .arg("import")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let mut first = spawn();
    let mut second = spawn();
    std::thread::sleep(Duration::from_millis(80));
    assert!(first.try_wait().unwrap().is_none());
    assert!(second.try_wait().unwrap().is_none());
    assert!(!config::catalog_path(&h.0).exists());
    drop(guard);
    let first = first.wait_with_output().unwrap();
    let second = second.wait_with_output().unwrap();
    assert_ne!(first.status.success(), second.status.success());
    let failed = if first.status.success() {
        second
    } else {
        first
    };
    assert!(String::from_utf8_lossy(&failed.stderr).contains("Catalog already exists"));
}
#[test]
fn backup_failure_preserves_source_and_stale_skill_cleanup_is_scoped() {
    let h = Home::new();
    h.put("file", "original");
    h.put(".local/state/comandos/extensions/backups", "occupied");
    assert!(
        config::replace_config(&h.0, &h.0.join("file"), Some(b"original"), b"replacement").is_err()
    );
    assert_eq!(h.read("file"), "original");
    fs::remove_file(config::state_dir(&h.0).join("backups")).unwrap();
    h.put(".agents/skills/demo/SKILL.md", "canonical");
    skills::sync_skills(&h.0).unwrap();
    fs::remove_dir_all(h.0.join(".agents/skills/demo")).unwrap();
    std::os::unix::fs::symlink(
        "/tmp/unrelated-absent-resource",
        h.0.join(".claude/skills/unrelated"),
    )
    .unwrap();
    skills::sync_skills(&h.0).unwrap();
    assert!(!h.0.join(".claude/skills/demo").is_symlink());
    assert!(h.0.join(".claude/skills/unrelated").is_symlink());
}
#[test]
fn malformed_empty_json_and_nonfinite_keys_are_rejected() {
    let h = Home::new();
    let p = h.0.join("data.json");
    for raw in [
        "",
        "{NaN:1}",
        "{Infinity:1}",
        "{-Infinity:1}",
        "{\"v\":Infinitya}",
        "{\"v\":NaN,\"v\":1}",
    ] {
        assert!(config::Document::parse(&p, Some(raw.as_bytes())).is_err());
    }
    let d = config::Document::parse(
        &p,
        Some(br#"{"v":NaN,"other":-Infinity,"s":"NaN","collision":"__comandos_nonfinite_0__"}"#),
    )
    .unwrap();
    assert_eq!(d.data["v"].to_string(), "NaN");
    assert_eq!(d.data["s"], "NaN");
    assert_eq!(d.data["collision"], "__comandos_nonfinite_0__");
    let quoted = config::Document::parse(&p, Some(br#"{"NaN":NaN,"Infinity":Infinity}"#)).unwrap();
    assert_eq!(quoted.data["NaN"].to_string(), "NaN");
    assert_eq!(quoted.data["Infinity"].to_string(), "Infinity");
}
#[test]
fn jsonc_comments_with_unmatched_quotes_preserve_nonfinite_atoms_and_literal_strings() {
    let h = Home::new();
    let p = h.0.join("data.jsonc");
    for comment in ["/* \" */", "// \"\n", "/* \"balanced\" */"] {
        let raw = format!(
            r#"{comment}{{"nan":NaN,"positive":Infinity,"negative":-Infinity,"literal":"NaN \"Infinity\" -Infinity","marker":"__comandos_nonfinite_0__","mcpServers":{{"demo":{{"command":"echo"}}}}}}"#
        );
        let d = config::Document::parse(&p, Some(raw.as_bytes())).unwrap();
        assert_eq!(d.data["nan"].to_string(), "NaN");
        assert_eq!(d.data["positive"].to_string(), "Infinity");
        assert_eq!(d.data["negative"].to_string(), "-Infinity");
        assert_eq!(d.data["literal"], "NaN \"Infinity\" -Infinity");
        assert_eq!(d.data["marker"], "__comandos_nonfinite_0__");
    }
}
#[test]
fn import_accepts_jsonc_comments_with_unmatched_quotes_and_nonfinite_atoms() {
    for comment in ["/* \" */", "// \"\n"] {
        let h = Home::new();
        h.put(
            ".claude.json",
            &format!(r#"{comment}{{"unknown":NaN,"mcpServers":{{"demo":{{"command":"echo"}}}}}}"#),
        );
        assert_eq!(ok(&h, "import")["servers"], 1);
    }
}
#[test]
fn numeric_equality_distinguishes_adjacent_big_integers() {
    let n = |s: &str| Value::Number(serde_json::Number::from_string_unchecked(s.into()));
    assert!(!config::toml::python_equal(
        &n("9007199254740992"),
        &n("9007199254740993")
    ));
    assert!(!config::toml::python_equal(
        &n("9007199254740993"),
        &n("9007199254740992.0")
    ));
    assert!(config::toml::python_equal(&json!(true), &json!(1.0)));
    assert!(config::toml::python_equal(
        &n("100000000000000000000"),
        &n("1e20")
    ));
}
#[test]
fn import_rejects_unserializable_normalized_values() {
    for spec in [
        "args=[true,false]",
        "[mcp_servers.demo.env]\nWHEN=1979-05-27",
    ] {
        let h = Home::new();
        h.put(
            ".codex/config.toml",
            &format!("[mcp_servers.demo]\ncommand='echo'\n{spec}\n"),
        );
        let out = cli(&h.0, "import");
        assert!(!out.status.success());
        assert!(!config::catalog_path(&h.0).exists());
    }
}
#[test]
fn nonfinite_numbers_are_not_equal_to_integers() {
    for spelling in ["NaN", "Infinity", "-Infinity"] {
        let n = Value::Number(serde_json::Number::from_string_unchecked(spelling.into()));
        assert!(!config::toml::python_equal(&n, &json!(1)));
        assert!(!config::toml::python_equal(&json!(1), &n));
    }
}
#[test]
fn native_reconcile_rejects_unserializable_normalized_values() {
    let h = Home::new();
    h.put(
        ".claude.json",
        r#"{"mcpServers":{"demo":{"command":"echo"}}}"#,
    );
    ok(&h, "import");
    ok(&h, "sync");
    let original = h.read(".config/comandos/extensions/catalog.json");
    h.put(
        ".codex/config.toml",
        "[mcp_servers.demo]\ncommand='updated'\n[mcp_servers.demo.env]\nWHEN=1979-05-27\n",
    );
    assert!(!cli(&h.0, "sync").status.success());
    assert_eq!(h.read(".config/comandos/extensions/catalog.json"), original);
}
#[test]
fn malformed_native_server_is_not_overwritten() {
    let h = Home::new();
    h.put(".claude.json", r#"{"mcpServers":{"demo":17}}"#);
    let before = h.read(".claude.json");
    assert!(
        cat::sync_configs(&h.0, &fixture_catalog(), "/launcher", &mut json!({}), None).is_err()
    );
    assert_eq!(h.read(".claude.json"), before);
}
#[test]
fn initial_skill_conflict_preserves_canonical_and_private_backup() {
    let h = Home::new();
    h.put(".agents/skills/a/SKILL.md", "canonical");
    h.put(".claude/skills/a/SKILL.md", "other");
    h.put(".grok/skills/b/SKILL.md", "unique");
    skills::sync_skills(&h.0).unwrap();
    assert!(h.0.join(".claude/skills/a").is_symlink());
    assert_eq!(h.read(".claude/skills/a/SKILL.md"), "canonical");
    assert_eq!(h.read(".agents/skills/b/SKILL.md"), "unique");
    assert_eq!(h.read(".gemini/config/skills/b/SKILL.md"), "unique");
    let backups = fs::read_dir(config::state_dir(&h.0).join("backups"))
        .unwrap()
        .map(|e| e.unwrap().path().join("a/SKILL.md"))
        .filter(|p| p.is_file())
        .collect::<Vec<_>>();
    assert!(
        backups
            .iter()
            .any(|p| fs::read_to_string(p).unwrap() == "other")
    );
    assert!(skills::sync_skills(&h.0).unwrap().is_empty());
}
#[test]
fn dotted_and_inline_toml_preserves_unknown_types_and_no_final_newline_noop() {
    let h = Home::new();
    let path = h.0.join(".codex/config.toml");
    let text = "model='keep'\r\n# retained\r\nopaque.\"a.b\"=1979-05-27\r\nopaque.triple='''line1\nline2'''\r\nmcp_servers={ demo={command='/launcher',args=['serve','demo'],startup_timeout_sec=45}, node_repl={command='native',integer=9223372036854775808} }";
    h.put(".codex/config.toml", text);
    sync(&h, &fixture_catalog());
    assert_eq!(fs::read(&path).unwrap(), text.as_bytes());
    let mut c = fixture_catalog();
    c["servers"]["extra"] = json!({"command":"echo"});
    sync(&h, &c);
    let parsed = config::read_config(&path).unwrap();
    assert_eq!(parsed["opaque"]["a.b"], "1979-05-27");
    assert_eq!(parsed["opaque"]["triple"], "line1\nline2");
    assert_eq!(
        parsed["mcp_servers"]["node_repl"]["integer"].to_string(),
        "9223372036854775808"
    );
    assert!(h.read(".codex/config.toml").contains("# retained"));
}
#[test]
fn replaced_skill_backup_is_named_skill_uuid_and_private() {
    use std::os::unix::fs::PermissionsExt;
    let h = Home::new();
    h.put(".agents/skills/demo/SKILL.md", "old");
    skills::sync_skills(&h.0).unwrap();
    fs::remove_file(h.0.join(".grok/skills/demo")).unwrap();
    h.put(".grok/skills/demo/SKILL.md", "new");
    skills::sync_skills(&h.0).unwrap();
    let backups = config::state_dir(&h.0).join("backups");
    let dirs = fs::read_dir(&backups)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("skill-")
        })
        .collect::<Vec<_>>();
    assert!(!dirs.is_empty(), "{dirs:?}");
    for d in &dirs {
        let n = d.file_name().unwrap().to_string_lossy().into_owned();
        let hex = n.strip_prefix("skill-").unwrap();
        assert!(hex.len() == 32 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        assert_eq!(hex.as_bytes()[12], b'4');
        assert!(matches!(hex.as_bytes()[16], b'8' | b'9' | b'a' | b'b'));
        assert_eq!(fs::metadata(d).unwrap().permissions().mode() & 0o777, 0o700);
    }
    // Dos respaldos, como el Python: la copia canónica desplazada por la reinstalación ("old")
    // y el directorio real de `.grok/skills` sustituido por el enlace ("new"). El orden de
    // `read_dir` no está definido, así que se comparan como conjunto.
    let mut contents = dirs
        .iter()
        .filter_map(|d| fs::read_to_string(d.join("demo/SKILL.md")).ok())
        .collect::<Vec<_>>();
    contents.sort();
    assert_eq!(contents, ["new", "old"]);
}
#[test]
fn sync_and_import_stdout_use_python_json_dumps_bytes() {
    let h = Home::new();
    h.put(
        ".claude.json",
        r#"{"mcpServers":{"demo":{"command":"echo","args":[]}}}"#,
    );
    let out = cli(&h.0, "import");
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "{\"servers\": 1, \"credentials_imported\": []}\n"
    );
    let out = cli(&h.0, "sync");
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "{\"configurations_changed\": 5, \"skill_links_changed\": 0}\n"
    );
}
#[test]
fn import_stdout_escapes_non_ascii_credential_names_like_python() {
    let h = Home::new();
    h.put(
        ".claude.json",
        r#"{"mcpServers":{"a":{"command":"echo","args":[]},"b":{"command":"echo","args":[]}}}"#,
    );
    h.put(
        ".config/comandos/extensions/credentials.json",
        r#"{"gmail":{"url":"x"},"señal":{"url":"y"}}"#,
    );
    let out = cli(&h.0, "import");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "{\"servers\": 2, \"credentials_imported\": [\"gmail\", \"se\\u00f1al\"]}\n"
    );
}
/// OpenCode y agy no difieren herramientas MCP: reciben solo el hub bajo demanda. El
/// resto de clientes sigue recibiendo cada servidor como `serve NAME`.
#[test]
fn hub_clients_receive_only_the_hub_and_others_every_server() {
    let h = Home::new();
    h.put(
        ".config/opencode/opencode.jsonc",
        "// usuario\n{\"mcp\":{}}",
    );
    h.put(
        ".config/opencode/opencode.json",
        r#"{"theme":"keep","mcp":{"node_repl":{"type":"local","command":["node"]},"demo":{"type":"local","command":["/launcher","serve","demo"]}}}"#,
    );
    let c = json!({"version":1,"servers":{"demo":{"command":"echo"},"mail":{"command":"mail"},"off":{"command":"x","enabled":false}}});
    sync(&h, &c);
    let opencode = config::read_config(&h.0.join(".config/opencode/opencode.json")).unwrap();
    assert_eq!(opencode["theme"], "keep");
    assert_eq!(
        opencode["mcp"],
        json!({
            "comandos":{"type":"local","command":["/launcher","hub"],"enabled":true},
            "node_repl":{"type":"local","command":["node"]}
        })
    );
    let jsonc = config::read_config(&h.0.join(".config/opencode/opencode.jsonc")).unwrap();
    assert_eq!(
        jsonc["mcp"],
        json!({"comandos":{"type":"local","command":["/launcher","hub"],"enabled":true}})
    );
    let agy = config::read_config(&h.0.join(".gemini/config/mcp_config.json")).unwrap();
    assert_eq!(
        agy["mcpServers"],
        json!({"comandos":{"command":"/launcher","args":["hub"],"disabled":false}})
    );
    let claude = config::read_config(&h.0.join(".claude.json")).unwrap();
    assert_eq!(
        claude["mcpServers"],
        json!({
            "demo":{"command":"/launcher","args":["serve","demo"],"type":"stdio"},
            "mail":{"command":"/launcher","args":["serve","mail"],"type":"stdio"}
        })
    );
    for path in [".codex/config.toml", ".grok/config.toml"] {
        let data = config::read_config(&h.0.join(path)).unwrap();
        let names: Vec<_> = data["mcp_servers"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(names, ["demo", "mail"], "{path}");
        assert_eq!(
            data["mcp_servers"]["demo"]["args"],
            json!(["serve", "demo"])
        );
    }
    // Sin servidores habilitados no se exporta nada; `node_repl` sigue intacto.
    let none = json!({"version":1,"servers":{"off":{"command":"x","enabled":false}}});
    sync(&h, &none);
    let opencode = config::read_config(&h.0.join(".config/opencode/opencode.json")).unwrap();
    assert_eq!(
        opencode["mcp"],
        json!({"node_repl":{"type":"local","command":["node"]}})
    );
    let agy = config::read_config(&h.0.join(".gemini/config/mcp_config.json")).unwrap();
    assert_eq!(agy["mcpServers"], json!({}));
}

/// Pasar a hub no desactiva servidores en el catálogo: ni en la sincronización que hace
/// el cambio, ni si se cortó antes de guardar la foto, ni en las siguientes. Un servidor
/// propio que el usuario añade a mano en OpenCode sí entra al catálogo.
#[test]
fn hub_transition_never_disables_catalog_servers() {
    let h = Home::new();
    let c = json!({"version":1,"servers":{"demo":{"command":"echo","enabled":true},"mail":{"command":"mail","enabled":true}}});
    // Estado previo: OpenCode y agy con cada servidor, y una foto que los recuerda.
    h.put(
        ".config/opencode/opencode.json",
        r#"{"mcp":{"demo":{"type":"local","command":["/launcher","serve","demo"],"enabled":true},"mail":{"type":"local","command":["/launcher","serve","mail"],"enabled":true}}}"#,
    );
    h.put(
        ".gemini/config/mcp_config.json",
        r#"{"mcpServers":{"demo":{"command":"/launcher","args":["serve","demo"],"disabled":false},"mail":{"command":"/launcher","args":["serve","mail"],"disabled":false}}}"#,
    );
    cat::save_snapshot(&h.0, &c, None).unwrap();
    assert_eq!(
        cat::reconcile(&h.0, &c, "/launcher", &mut json!({})).unwrap(),
        c
    );
    // La exportación cambia a hub pero «se corta» antes de guardar la foto.
    sync(&h, &c);
    assert_eq!(
        cat::reconcile(&h.0, &c, "/launcher", &mut json!({})).unwrap(),
        c,
        "ausencias en un cliente con hub no desactivan servidores"
    );
    let observed = sync(&h, &c);
    cat::save_snapshot(&h.0, &c, Some(&observed)).unwrap();
    assert_eq!(
        cat::reconcile(&h.0, &c, "/launcher", &mut json!({})).unwrap(),
        c
    );
    // Desactivar el hub a mano tampoco crea un servidor «comandos» en el catálogo.
    h.put(
        ".config/opencode/opencode.json",
        r#"{"mcp":{"comandos":{"type":"local","command":["/elsewhere","hub"],"enabled":false},"mine":{"type":"local","command":["my-mcp","--flag"]}}}"#,
    );
    let revised = cat::reconcile(&h.0, &c, "/launcher", &mut json!({})).unwrap();
    assert!(revised["servers"].get("comandos").is_none());
    assert_eq!(revised["servers"]["demo"], c["servers"]["demo"]);
    assert_eq!(revised["servers"]["mine"]["command"], "my-mcp");
    assert_eq!(revised["servers"]["mine"]["args"], json!(["--flag"]));
}

#[test]
fn import_ignores_the_hub_entry_of_hub_clients() {
    let h = Home::new();
    h.put(
        ".config/opencode/opencode.json",
        r#"{"mcp":{"comandos":{"type":"local","command":["/launcher","hub"],"enabled":true},"demo":{"type":"local","command":["echo"]}}}"#,
    );
    h.put(
        ".gemini/config/mcp_config.json",
        r#"{"mcpServers":{"comandos":{"command":"/launcher","args":["hub"]}}}"#,
    );
    let c = cat::import_catalog(&h.0).unwrap();
    assert_eq!(
        c["servers"].as_object().unwrap().keys().collect::<Vec<_>>(),
        ["demo"]
    );
}
