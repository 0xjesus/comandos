use comandos_runtime::accounts::{self as a, Paths};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(comandos_runtime::fresh_id("accounts-fixture").unwrap());
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn paths(&self) -> Paths {
        Paths::new(&self.0, &self.0)
    }
    fn registry(&self) -> Value {
        let mut out = json!({"harnesses":{}});
        for p in ["claude", "codex", "grok"] {
            out["harnesses"][p] = json!({"defaultHome":self.0.join(p),"accountsRoot":self.0.join(format!("{p}-accounts")),"authFile":if p=="claude"{".credentials.json"}else{"auth.json"},"accountEnv":format!("{}_HOME",p.to_uppercase()),"capabilities":{"accounts":true}})
        }
        out
    }
    fn put(&self, relative: &str, value: &Value) {
        let p = self.0.join(relative);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, serde_json::to_vec(value).unwrap()).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn aliases_preserve_falsy_main_and_python_scalar_coercion() {
    for input in [
        json!(null),
        json!(false),
        json!(0),
        json!(""),
        json!([]),
        json!({}),
    ] {
        assert_eq!(a::validate_alias(&input).unwrap(), "main");
    }
    for (input, expected) in [
        (json!(true), "True"),
        (json!(12), "12"),
        (json!(1.5), "1.5"),
        (json!("a._-Z09"), "a._-Z09"),
    ] {
        assert_eq!(a::validate_alias(&input).unwrap(), expected);
    }
    assert!(a::validate_alias(&json!("x".repeat(64))).is_ok());
    for input in [
        json!("x".repeat(65)),
        json!("../x"),
        json!(".hidden"),
        json!("-flag"),
        json!("_flag"),
        json!("é"),
        json!("main\n"),
        json!([1]),
    ] {
        assert_eq!(
            a::validate_alias(&input).unwrap_err().0,
            "alias de cuenta invalido"
        );
    }
}
#[test]
fn registry_errors_are_exact_and_main_environment_is_empty() {
    let f = Fixture::new();
    let paths = f.paths();
    let reg = f.registry();
    assert_eq!(
        a::account_environment(&reg, "claude", &json!("main"), &paths).unwrap(),
        json!({})
    );
    assert_eq!(
        a::account_environment(&reg, "claude", &json!(null), &paths).unwrap(),
        json!({})
    );
    assert_eq!(
        a::account_environment(&reg, "codex", &json!("work"), &paths).unwrap(),
        json!({"CODEX_HOME":f.0.join("codex-accounts/work")})
    );
    assert_eq!(
        a::account_home(&reg, "unknown", &json!("main"), &paths)
            .unwrap_err()
            .0,
        "unknown: cuentas no soportadas"
    );
    let mut bad = reg.clone();
    bad["harnesses"]["codex"]["authFile"] = json!("");
    assert_eq!(
        a::list_accounts(&bad, "codex", &paths).unwrap_err().0,
        "codex: registro de cuentas incompleto"
    );
}
#[test]
fn expanduser_and_relative_resolution_use_only_supplied_paths() {
    let f = Fixture::new();
    let mut reg = f.registry();
    let mut paths = f.paths();
    paths
        .user_homes
        .insert("fixture-user".into(), f.0.join("someone"));
    reg["harnesses"]["codex"]["defaultHome"] = json!("~/.codex/../nested/missing");
    reg["harnesses"]["codex"]["accountsRoot"] = json!("~fixture-user/accounts");
    assert_eq!(
        a::account_home(&reg, "codex", &json!("main"), &paths).unwrap(),
        f.0.join("nested/missing")
    );
    assert_eq!(
        a::account_home(&reg, "codex", &json!("work"), &paths).unwrap(),
        f.0.join("someone/accounts/work")
    );
    reg["harnesses"]["codex"]["defaultHome"] = json!("relative/./missing");
    assert_eq!(
        a::account_home(&reg, "codex", &json!("main"), &paths).unwrap(),
        f.0.join("relative/missing")
    );
}
#[test]
fn existing_symlink_parents_resolve_but_named_symlinks_never_pass() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let paths = f.paths();
    let mut reg = f.registry();
    fs::create_dir(f.0.join("real")).unwrap();
    symlink(f.0.join("real"), f.0.join("parent")).unwrap();
    reg["harnesses"]["codex"]["accountsRoot"] = json!(f.0.join("parent/accounts"));
    assert_eq!(
        a::account_home(&reg, "codex", &json!("future"), &paths).unwrap(),
        f.0.join("real/accounts/future")
    );
    fs::create_dir(f.0.join("real/accounts")).unwrap();
    fs::create_dir(f.0.join("real/accounts/inside")).unwrap();
    symlink("inside", f.0.join("real/accounts/inward")).unwrap();
    symlink(&f.0, f.0.join("real/accounts/outward")).unwrap();
    assert_eq!(
        a::account_home(&reg, "codex", &json!("inward"), &paths)
            .unwrap_err()
            .0,
        "aliases de cuenta no pueden ser symlinks"
    );
    assert_eq!(
        a::account_home(&reg, "codex", &json!("outward"), &paths)
            .unwrap_err()
            .0,
        "cuenta fuera de accountsRoot"
    );
}
#[test]
fn provider_credentials_yield_only_public_status_identity_and_alias() {
    let f = Fixture::new();
    let reg = f.registry();
    let paths = f.paths();
    f.put(
        "claude/.credentials.json",
        &json!({"claudeAiOauth":{"accessToken":"secret-claude"}}),
    );
    f.put(
        ".claude.json",
        &json!({"oauthAccount":{"emailAddress":"fallback@example.test"}}),
    );
    f.put(
        "claude-accounts/z/.credentials.json",
        &json!({"claudeAiOauth":{"refreshToken":"secret-refresh"}}),
    );
    f.put(
        "claude-accounts/z/.claude.json",
        &json!({"oauthAccount":{"emailAddress":"named@example.test"}}),
    );
    f.put("codex/auth.json",&json!({"tokens":{"access_token":"secret-codex","id_token":"x.eyJlbWFpbCI6ImNvZGV4QGV4YW1wbGUudGVzdCJ9.y"}}));
    f.put("grok/auth.json",&json!({"issuer":{"refresh_token":"secret-grok","first_name":"Grok identity"},"later":{"email":"ignored"}}));
    let found = a::public_accounts(&reg, &paths).unwrap();
    assert_eq!(found["claude"][0]["identity"], "fallback@example.test");
    assert_eq!(found["claude"][1]["identity"], "named@example.test");
    assert_eq!(found["codex"][0]["identity"], "codex@example.test");
    assert_eq!(found["grok"][0]["identity"], "Grok identity");
    for provider in ["claude", "codex", "grok"] {
        assert_eq!(found[provider][0]["state"], "ready");
        assert_eq!(found[provider][0]["selectable"], true);
    }
    let public = found.to_string();
    for forbidden in [
        "secret",
        f.0.to_str().unwrap(),
        "access_token",
        "refresh_token",
        "authFile",
    ] {
        assert!(!public.contains(forbidden));
    }
}
#[test]
fn discovery_filters_lock_hidden_invalid_symlink_and_file_entries_and_sorts() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let reg = f.registry();
    let paths = f.paths();
    let root = f.0.join("codex-accounts");
    fs::create_dir(&root).unwrap();
    for name in [
        "z",
        "a",
        "work.lock",
        ".hidden",
        "-flag",
        "a b",
        "é",
        "main",
    ] {
        fs::create_dir(root.join(name)).unwrap();
    }
    fs::write(root.join("file"), "{}").unwrap();
    symlink(root.join("a"), root.join("linked")).unwrap();
    let aliases = a::list_accounts(&reg, "codex", &paths)
        .unwrap()
        .into_iter()
        .map(|a| a["alias"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(aliases, vec!["main", "a", "main", "z"]);
}
#[test]
fn malformed_pending_api_keys_and_first_grok_record_preserve_states() {
    let f = Fixture::new();
    let reg = f.registry();
    let paths = f.paths();
    f.put("codex/auth.json", &json!({"OPENAI_API_KEY":"private"}));
    fs::create_dir_all(f.0.join("codex-accounts/pending")).unwrap();
    let accounts = a::list_accounts(&reg, "codex", &paths).unwrap();
    assert_eq!(accounts[0]["state"], "unsupported_auth");
    assert_eq!(accounts[1]["state"], "login_required");
    assert_eq!(accounts[0]["authenticated"], false);
    fs::write(f.0.join("codex/auth.json"), b"{bad").unwrap();
    assert_eq!(
        a::list_accounts(&reg, "codex", &paths).unwrap()[0]["state"],
        "login_required"
    );
    f.put(
        "grok/auth.json",
        &json!({"first":{},"second":{"refresh_token":"private","email":"ignored"}}),
    );
    assert_eq!(
        a::list_accounts(&reg, "grok", &paths).unwrap()[0]["state"],
        "unsupported_auth"
    );
    let mut bad = reg.clone();
    bad["harnesses"]["codex"]["accountsRoot"] = json!("");
    bad["harnesses"]["other"] = json!({"capabilities":{"accounts":false}});
    let public = a::public_accounts(&bad, &paths).unwrap();
    assert_eq!(public["codex"], json!([]));
    assert!(public.get("other").is_none());
}
#[test]
fn malformed_jwt_never_leaks_tokens_and_email_coercion_is_python_style() {
    let f = Fixture::new();
    let reg = f.registry();
    let paths = f.paths();
    for token in [
        json!(null),
        json!("secret"),
        json!("x.!!.y"),
        json!("x.W10.y"),
    ] {
        f.put(
            "codex/auth.json",
            &json!({"tokens":{"refresh_token":"private","id_token":token}}),
        );
        assert_eq!(
            a::list_accounts(&reg, "codex", &paths).unwrap()[0]["identity"],
            ""
        );
    }
    f.put(
        "codex/auth.json",
        &json!({"tokens":{"refresh_token":"private","id_token":"x.eyJlbWFpbCI6dHJ1ZX0.y"}}),
    );
    assert_eq!(
        a::list_accounts(&reg, "codex", &paths).unwrap()[0]["identity"],
        "True"
    );
}
#[test]
fn menu_pairs_limits_orders_stably_rounds_even_and_clamps() {
    let ac = json!([{"alias":"main","identity":"a@x","selectable":true},{"alias":"work","identity":true,"selectable":1},{"alias":"../bad"}]);
    let limits = json!([{"provider":"claude","account":"work","kind":"weekly_scoped","label":" Semana Fable ","percent":100.5,"resets_at":7},{"provider":"claude","account":"work","kind":"session","percent":2.5},{"provider":"claude","account":"work","kind":"weekly_all","percent":-5},{"provider":"claude","account":"work","kind":"other","label":"","percent":"٢_٥.٥"},{"provider":"codex","account":"work","kind":"session","percent":99},{"provider":"claude","account":"main","kind":"weekly_all","percent":"bad"}]);
    assert_eq!(
        a::account_menu(&ac, &limits, "claude").unwrap(),
        vec![
            json!({"alias":"main","identity":"a@x","selectable":true,"limits":[]}),
            json!({"alias":"work","identity":"True","selectable":true,"limits":[{"label":"5 h","percent":2,"resetsAt":null},{"label":"semana","percent":0,"resetsAt":null},{"label":"Fable","percent":100,"resetsAt":7},{"label":"límite","percent":26,"resetsAt":null}]})
        ]
    );
}
#[test]
fn menu_nan_skips_infinity_errors_and_identity_is_neither_trimmed_nor_truncated() {
    let identity = format!(" {} ", "🦀".repeat(200));
    let ac = json!([{"alias":"main","identity":identity,"selectable":false}]);
    let limits = json!([{"provider":"claude","account":"main","kind":"session","percent":"NaN"},{"provider":"claude","account":"main","kind":"window","percent":false}]);
    let out = a::account_menu(&ac, &limits, "claude").unwrap();
    assert_eq!(out[0]["identity"], identity);
    assert_eq!(
        out[0]["limits"],
        json!([{"label":"semana","percent":0,"resetsAt":null}])
    );
    for value in ["Infinity", "-Infinity", "1e9999"] {
        assert!(
            a::account_menu(
                &ac,
                &json!([{"provider":"claude","account":"main","percent":value}]),
                "claude"
            )
            .is_err()
        );
    }
}

#[test]
fn menu_container_identity_and_labels_keep_python_representation() {
    let ac = json!([{"alias":1.0,"identity":{"x":[true,null,"a\u{2028}b"]},"selectable":[]}]);
    let limits = json!([{"provider":"claude","account":"1.0","kind":"other","label":["Semana a\u{a0}b"],"percent":"3_0.5"}]);
    assert_eq!(
        a::account_menu(&ac, &limits, "claude").unwrap(),
        vec![
            json!({"alias":"1.0","identity":"{'x': [True, None, 'a\\u2028b']}","selectable":false,"limits":[{"label":"['a\\xa0b']","percent":30,"resetsAt":null}]})
        ]
    );
}

#[test]
fn frozen_python_oracle_alias_menu_and_jwt_rows_match() {
    let reference: Value =
        serde_json::from_str(include_str!("fixtures/accounts_catalog_reference.json")).unwrap();
    for row in reference["aliases"].as_array().unwrap() {
        let result = a::validate_alias(&row["input"]);
        if row.get("error").is_some() {
            assert_eq!(result.unwrap_err().0, row["message"].as_str().unwrap());
        } else {
            assert_eq!(result.unwrap(), row["alias"].as_str().unwrap());
        }
    }
    let ac = json!([{"alias":"main","identity":true,"selectable":1}]);
    for row in reference["menu"].as_array().unwrap() {
        let limits =
            json!([{"provider":"claude","account":"main","kind":"session","percent":row["input"]}]);
        let result = a::account_menu(&ac, &limits, "claude");
        if row.get("error").is_some() {
            assert!(result.is_err(), "{row}");
        } else {
            assert_eq!(
                result.unwrap(),
                row["output"].as_array().unwrap().clone(),
                "{row}"
            );
        }
    }
    let edge: Value = serde_json::from_str(include_str!(
        "fixtures/accounts_catalog_edge_reference.json"
    ))
    .unwrap();
    for row in edge["menus"].as_array().unwrap() {
        assert_eq!(
            a::account_menu(&row["accounts"], &row["limits"], "claude").unwrap(),
            row["output"].as_array().unwrap().clone()
        );
    }
    let f = Fixture::new();
    let registry = f.registry();
    let paths = f.paths();
    for (index, row) in edge["jwt"].as_array().unwrap().iter().enumerate() {
        f.put(
            "codex/auth.json",
            &json!({"tokens":{"refresh_token":"fixture-only","id_token":row["token"]}}),
        );
        assert_eq!(
            a::list_accounts(&registry, "codex", &paths).unwrap()[0]["identity"],
            row["identity"],
            "fixture JWT parity row {index}"
        );
    }
}

#[test]
fn environment_rejects_non_utf8_symlink_target_without_losing_native_home() {
    use std::ffi::OsString;
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    let f = Fixture::new();
    let native = f.0.join(OsString::from_vec(b"native-root-\xff".to_vec()));
    fs::create_dir(&native).unwrap();
    symlink(&native, f.0.join("utf8-accounts-root")).unwrap();
    let mut registry = f.registry();
    registry["harnesses"]["codex"]["accountsRoot"] = json!(f.0.join("utf8-accounts-root"));
    let paths = f.paths();
    assert_eq!(
        a::account_home(&registry, "codex", &json!("work"), &paths).unwrap(),
        native.join("work")
    );
    let err = a::account_environment(&registry, "codex", &json!("work"), &paths).unwrap_err();
    assert_eq!(err.0, "ruta de cuenta no es UTF-8");
    // Quien lo recibe lo distingue por su clase, no por el texto.
    assert_eq!(err.kind(), a::ErrorKind::NonUtf8Home);
    let other = a::account_environment(&registry, "codex", &json!("../x"), &paths).unwrap_err();
    assert_eq!(other.kind(), a::ErrorKind::Account);
}
#[test]
fn claude_api_key_account_is_ready_without_leaking_the_key() {
    let f = Fixture::new();
    let reg = f.registry();
    let paths = f.paths();
    f.put(
        "claude-accounts/api/settings.json",
        &json!({"apiKeyHelper":"cat /secret-key-file"}),
    );
    f.put(
        "claude-accounts/envkey/settings.json",
        &json!({"env":{"ANTHROPIC_API_KEY":"secret-env-key"}}),
    );
    fs::create_dir_all(f.0.join("claude-accounts/pending")).unwrap();
    let found = a::list_accounts(&reg, "claude", &paths).unwrap();
    let states = found
        .iter()
        .map(|a| {
            (
                a["alias"].as_str().unwrap().to_owned(),
                a["state"].as_str().unwrap().to_owned(),
                a["selectable"] == true,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        vec![
            ("main".into(), "login_required".into(), false),
            ("api".into(), "ready".into(), true),
            ("envkey".into(), "ready".into(), true),
            ("pending".into(), "login_required".into(), false),
        ]
    );
    assert_eq!(found[1]["identity"], "API key");
    assert!(!serde_json::to_string(&found).unwrap().contains("secret"));
}
