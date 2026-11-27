use comandos_runtime::model_catalog::{self as m, Paths};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(comandos_runtime::fresh_id("catalog-fixture").unwrap());
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn paths(&self) -> Paths {
        m::catalog_paths(&self.0, &self.0, None, None)
    }
    fn put(&self, provider: &str, value: &Value) {
        let p = self.0.join(format!(".{provider}/models_cache.json"));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, serde_json::to_vec(value).unwrap()).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn codex(id: &str) -> Value {
    json!({"slug":id,"visibility":"list","supported_reasoning_levels":[{"effort":"high"}],"default_reasoning_level":"high"})
}

#[test]
fn supplied_home_overrides_and_signature_order_do_not_discover_environment() {
    let f = Fixture::new();
    let paths = m::catalog_paths(
        &f.0,
        &f.0,
        Some(Path::new("codex-fixture")),
        Some(Path::new("")),
    );
    assert_eq!(
        paths.codex,
        PathBuf::from("codex-fixture/models_cache.json")
    );
    assert_eq!(paths.grok, f.0.join(".grok/models_cache.json"));
    assert_eq!(
        m::catalog_signature(&paths),
        json!([
            ["codex", "codex-fixture/models_cache.json", null],
            ["grok", f.0.join(".grok/models_cache.json"), null],
            ["omp", f.0.join(".cache/comandos/models/omp.json"), null],
            ["opencode", f.0.join(".cache/comandos/models/opencode.json"), null]
        ])
    );
}
#[test]
fn stat_signature_contains_exact_nanoseconds_size_inode_and_changes_on_replacement() {
    use std::os::unix::fs::MetadataExt;
    let f = Fixture::new();
    f.put("codex", &json!({"models":[codex("gpt-6-astra")]}));
    let paths = f.paths();
    let metadata = fs::metadata(&paths.codex).unwrap();
    let sig = m::catalog_signature(&paths);
    assert_eq!(
        sig[0][2],
        json!([
            metadata.mtime() * 1_000_000_000 + metadata.mtime_nsec(),
            metadata.size(),
            metadata.ino()
        ])
    );
    fs::rename(&paths.codex, f.0.join("old-cache")).unwrap();
    f.put("codex", &json!({"models":[]}));
    assert_ne!(m::catalog_signature(&paths), sig);
}
#[test]
fn visible_provider_rows_require_explicit_efforts_without_id_inference() {
    let f = Fixture::new();
    let mut visible = codex("gpt-6-astra");
    visible["display_name"] = json!("CLI name");
    visible["context_window"] = json!(272000);
    f.put("codex",&json!({"identity":"private","fetched_at":"2026-09-25","models":[visible,{"slug":"gpt-6-sol","visibility":"list"},{"slug":"gpt-6-luna","visibility":"hide","supported_reasoning_levels":[]},42,null]}));
    f.put("grok",&json!({"api_key":"secret","models":{"first":{"secret":"token","info":{"id":"grok-4.7-fast","name":"Fast","hidden":false,"reasoning_efforts":[{"id":"high","default":true}],"reasoning_effort":"high","context_window":500000}},"second":{"info":{"id":"grok-4.8","hidden":0,"reasoning_efforts":[]}}}}));
    let out = m::catalog_models(&f.paths());
    assert_eq!(
        out["codex"],
        json!([{"id":"gpt-6-astra","name":"CLI name","efforts":["high"],"defaultEffort":"high","contextWindow":272000,"catalogSource":{"provider":"codex","kind":"cli-cache","fetchedAt":"2026-09-25"}}])
    );
    assert_eq!(out["grok"][0]["id"], "grok-4.7-fast");
    assert_eq!(out["grok"].as_array().unwrap().len(), 1);
    for secret in ["private", "secret", "api_key", "token"] {
        assert!(!out.to_string().contains(secret));
    }
}
#[test]
fn fixed_model_pattern_accepts_unicode_decimal_digits_and_rejects_other_numbers() {
    let f = Fixture::new();
    let good = ["gpt-6", "gpt-6.1-sol", "gpt-٢.٥-x0", "gpt-𝟡-astra"];
    let bad = [
        "gpt-²",
        "gpt-6\n",
        "gpt-6-UPPER",
        "gpt-6_foo",
        "gpt-6.",
        "gpt-6--x",
        "gpt-6-",
        "grok-6",
    ];
    let rows = good.into_iter().chain(bad).map(codex).collect::<Vec<_>>();
    f.put("codex", &json!({"models":rows}));
    assert_eq!(
        m::catalog_models(&f.paths())["codex"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        good
    );
}
#[test]
fn effort_deduplication_default_membership_and_context_types_are_exact() {
    let f = Fixture::new();
    let mut row = codex("gpt-6");
    row["supported_reasoning_levels"] = json!([{"effort":"high"},{"effort":"high"},{"effort":"x-low_2"},{"effort":"A"},{"effort":"h\n"},{"effort":"a".repeat(33)},true]);
    row["default_reasoning_level"] = json!("missing");
    row["context_window"] = json!(true);
    f.put("codex", &json!({"models":[row]}));
    let out = m::catalog_models(&f.paths());
    assert_eq!(out["codex"][0]["efforts"], json!(["high", "x-low_2"]));
    assert_eq!(out["codex"][0]["defaultEffort"], "");
    assert!(out["codex"][0].get("contextWindow").is_none());
    for context in [json!(0), json!(-1), json!(12.0), json!("12"), json!(null)] {
        let mut row = codex("gpt-6");
        row["context_window"] = context;
        f.put("codex", &json!({"models":[row]}));
        assert!(
            m::catalog_models(&f.paths())["codex"][0]
                .get("contextWindow")
                .is_none()
        );
    }
}
#[test]
fn display_and_fetch_strings_truncate_by_unicode_character_and_huge_context_is_retained() {
    let f = Fixture::new();
    let mut row = codex("gpt-6");
    row["display_name"] = json!("🦀é".repeat(100));
    row["context_window"] =
        serde_json::from_str("999999999999999999999999999999999999999").unwrap();
    f.put(
        "codex",
        &json!({"models":[row.clone()],"fetched_at":"🦀é".repeat(40)}),
    );
    let out = m::catalog_models(&f.paths());
    assert_eq!(
        out["codex"][0]["name"].as_str().unwrap().chars().count(),
        160
    );
    assert_eq!(
        out["codex"][0]["catalogSource"]["fetchedAt"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        64
    );
    assert_eq!(out["codex"][0]["contextWindow"], row["context_window"]);
}
#[test]
fn malformed_missing_nonobject_deep_and_oversized_caches_fall_back() {
    let f = Fixture::new();
    let paths = f.paths();
    assert_eq!(m::catalog_models(&paths), json!({"codex":[],"grok":[],"omp":[],"opencode":[]}));
    fs::create_dir(f.0.join(".codex")).unwrap();
    for bad in [
        vec![0xff],
        b"[]".to_vec(),
        b"{bad".to_vec(),
        ["[".repeat(2000), "]".repeat(2000)].concat().into_bytes(),
        vec![b' '; m::MAX_CACHE_BYTES + 1],
    ] {
        fs::write(&paths.codex, bad).unwrap();
        assert_eq!(m::catalog_models(&paths)["codex"], json!([]));
    }
}
#[test]
fn byte_json_utf8_bom_and_utf16_cache_are_accepted_like_python() {
    let f = Fixture::new();
    let paths = f.paths();
    fs::create_dir(f.0.join(".codex")).unwrap();
    let raw = json!({"models":[codex("gpt-6")]}).to_string();
    let mut bom = vec![0xef, 0xbb, 0xbf];
    bom.extend_from_slice(raw.as_bytes());
    fs::write(&paths.codex, bom).unwrap();
    assert_eq!(m::catalog_models(&paths)["codex"][0]["id"], "gpt-6");
    let mut utf16 = vec![0xff, 0xfe];
    for unit in raw.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    fs::write(&paths.codex, utf16).unwrap();
    assert_eq!(m::catalog_models(&paths)["codex"][0]["id"], "gpt-6");
}
#[test]
fn hydration_copies_registry_preserves_curated_fields_and_cleans_only_truthy_soon_tag() {
    let f = Fixture::new();
    f.put(
        "codex",
        &json!({"models":[codex("gpt-6-astra"),codex("gpt-6-sol")]}),
    );
    let registry = json!({"motors":{"codex":{"models":[{"id":"gpt-6-astra[1m]","name":"Curated","tag":"rollout","soon":true,"efforts":["low"],"contextWindow":999}]},"grok":{}},"harnesses":{"codex":{"label":"no models"},"grok":{"models":[]}},"routes":[{"id":"keep"}]});
    let before = registry.clone();
    let out = m::hydrate_registry(&registry, &f.paths()).unwrap();
    assert_eq!(registry, before);
    assert_eq!(out["motors"]["codex"]["models"][0]["id"], "gpt-6-astra[1m]");
    assert_eq!(out["motors"]["codex"]["models"][0]["name"], "Curated");
    assert_eq!(out["motors"]["codex"]["models"][0]["contextWindow"], 999);
    assert!(out["motors"]["codex"]["models"][0].get("soon").is_none());
    assert!(out["motors"]["codex"]["models"][0].get("tag").is_none());
    assert_eq!(out["motors"]["codex"]["models"][1]["id"], "gpt-6-sol");
    assert!(out["harnesses"]["codex"].get("models").is_none());
    assert_eq!(out["routes"], before["routes"]);
}
#[test]
fn falsy_soon_is_removed_but_tag_retained_and_cached_duplicates_keep_first_slot() {
    let f = Fixture::new();
    let mut second = codex("gpt-6");
    second["supported_reasoning_levels"] = json!([]);
    f.put("codex", &json!({"models":[codex("gpt-6"),second]}));
    let registry =
        json!({"motors":{"codex":{"models":[{"id":"gpt-6","soon":false,"tag":"curated"}]}}});
    let out = m::hydrate_registry(&registry, &f.paths()).unwrap();
    assert_eq!(
        out["motors"]["codex"]["models"].as_array().unwrap().len(),
        1
    );
    assert_eq!(out["motors"]["codex"]["models"][0]["tag"], "curated");
    assert!(out["motors"]["codex"]["models"][0].get("soon").is_none());
    assert_eq!(out["motors"]["codex"]["models"][0]["efforts"], json!([]));
}

#[test]
fn curated_suffix_normalization_preserves_python_regex_newline_semantics() {
    let f = Fixture::new();
    f.put("codex", &json!({"models":[codex("gpt-6")]}));
    for id in ["gpt-6[1m]\n", "gpt-6[1m]\nmore"] {
        let registry = json!({"motors":{"codex":{"models":[{"id":id,"soon":true,"tag":"keep"}]}}});
        let out = m::hydrate_registry(&registry, &f.paths()).unwrap();
        assert_eq!(
            out["motors"]["codex"]["models"].as_array().unwrap().len(),
            2
        );
        assert_eq!(out["motors"]["codex"]["models"][0]["soon"], true);
        assert_eq!(out["motors"]["codex"]["models"][0]["tag"], "keep");
    }
}

#[test]
fn frozen_python_oracle_hydration_rows_match_without_adding_missing_sections() {
    let reference: Value = serde_json::from_str(include_str!(
        "fixtures/accounts_catalog_edge_reference.json"
    ))
    .unwrap();
    let f = Fixture::new();
    for row in reference["hydration"].as_array().unwrap() {
        f.put("codex", &row["cache"]);
        assert_eq!(
            m::hydrate_registry(&row["registry"], &f.paths()).unwrap(),
            row["output"]
        );
    }
}
#[test]
fn normalized_cli_caches_feed_omp_and_opencode_newest_first() {
    let f = Fixture::new();
    let dir = f.0.join(".cache/comandos/models");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("omp.json"),
        serde_json::to_vec(&json!({"fetched_at":"2026-11-26T00:00:00Z","models":[
            {"id":"refuseless/glm-5.3-abliterated","name":"GLM 5.3","efforts":["low","high","bogus effort"],"defaultEffort":"high","contextWindow":1048576},
            {"id":"refuseless/glm-5.3-abliterated","name":"dup"},
            {"id":"has space"},
            {"id":"refuseless/kimi-k3.1-pro","tag":"nuevo"}
        ]}))
        .unwrap(),
    )
    .unwrap();
    let models = m::catalog_models(&f.paths());
    let omp = models["omp"].as_array().unwrap();
    assert_eq!(omp.len(), 2, "duplicates and ids with blanks are dropped");
    assert_eq!(omp[0]["id"], "refuseless/glm-5.3-abliterated");
    assert_eq!(omp[0]["efforts"], json!(["low", "high"]));
    assert_eq!(omp[0]["defaultEffort"], "high");
    assert_eq!(omp[0]["contextWindow"], 1048576);
    assert_eq!(omp[1]["name"], "refuseless/kimi-k3.1-pro", "a missing name falls back to the id");
    assert_eq!(omp[1]["tag"], "nuevo");
    assert_eq!(models["opencode"], json!([]), "a missing cache adds nothing");
    let registry = json!({"motors":{"omp":{"models":[{"id":"refuseless/kimi-k3.1-pro","name":"Kimi"}]}}});
    let hydrated = m::hydrate_registry(&registry, &f.paths()).unwrap();
    let ids: Vec<_> = hydrated["motors"]["omp"]["models"].as_array().unwrap().iter().map(|m| m["id"].clone()).collect();
    assert_eq!(ids, [json!("refuseless/kimi-k3.1-pro"), json!("refuseless/glm-5.3-abliterated")]);
}
