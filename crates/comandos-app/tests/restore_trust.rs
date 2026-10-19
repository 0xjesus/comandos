//! A restored conversation must not stop at the folder-trust question.
use comandos_app::restore::trust_restored_cwd;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("comandos-restore-trust-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("work")).unwrap();
    fs::canonicalize(dir).unwrap()
}

fn trusted(file: &PathBuf, cwd: &str) -> Option<bool> {
    let data: Value = serde_json::from_str(&fs::read_to_string(file).ok()?).ok()?;
    data["projects"][cwd]["hasTrustDialogAccepted"].as_bool()
}

#[test]
fn claude_trust_lands_in_home_and_in_the_account_config() {
    let home = scratch("claude");
    let cwd = home.join("work").to_string_lossy().into_owned();
    let account = home.join(".claude-accounts/one");
    fs::create_dir_all(&account).unwrap();
    // The account a pane runs under, plus CLAUDE_CONFIG_DIR=~/.claude itself:
    // that one reads ~/.claude/.claude.json, not ~/.claude.json.
    for config in [json!(account.to_string_lossy()), json!("~/.claude")] {
        trust_restored_cwd(&json!({"agent":"claude","claude_config_dir":config}), &cwd, &home);
    }
    assert_eq!(trusted(&home.join(".claude.json"), &cwd), Some(true));
    assert_eq!(trusted(&account.join(".claude.json"), &cwd), Some(true));
    assert_eq!(trusted(&home.join(".claude/.claude.json"), &cwd), Some(true));
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn codex_trust_is_added_but_an_explicit_decision_is_kept() {
    let home = scratch("codex");
    let cwd = home.join("work").to_string_lossy().into_owned();
    let codex = home.join(".codex");
    fs::create_dir_all(&codex).unwrap();
    fs::write(codex.join("config.toml"), "model = \"x\"\n").unwrap();
    trust_restored_cwd(&json!({"agent":"codex"}), &cwd, &home);
    trust_restored_cwd(&json!({"agent":"codex"}), &cwd, &home);
    let text = fs::read_to_string(codex.join("config.toml")).unwrap();
    assert!(text.starts_with("model = \"x\"\n"), "{text}");
    assert_eq!(text.matches("trust_level = \"trusted\"").count(), 1, "{text}");

    let other = home.join(".codex-other");
    fs::create_dir_all(&other).unwrap();
    let refused = format!("[projects.\"{cwd}\"]\ntrust_level = \"untrusted\"\n");
    fs::write(other.join("config.toml"), &refused).unwrap();
    trust_restored_cwd(&json!({"agent":"codex","codex_home":other.to_string_lossy()}), &cwd, &home);
    assert_eq!(fs::read_to_string(other.join("config.toml")).unwrap(), refused);
    let _ = fs::remove_dir_all(&home);
}
