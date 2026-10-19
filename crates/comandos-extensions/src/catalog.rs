//! Discovery, one-time precedence, snapshot reconciliation and native exports.
use crate::{
    Result,
    config::{self, Document, fingerprint, read_config, state_dir},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
pub const CLIENT_FIELDS: [&str; 7] = [
    "tools",
    "default_tools_approval_mode",
    "enabled_tools",
    "disabled_tools",
    "required",
    "startup_timeout_sec",
    "tool_timeout_sec",
];
const MAIL: [&str; 4] = ["gmail", "gmail-signara", "qcdr-mail", "proton-mail"];
const REMOTE: [&str; 5] = [
    "playwright",
    "x-playwright",
    "lightpanda",
    "obscura",
    "screenwright",
];
/// Clientes que no saben diferir herramientas MCP: cargan todas las definiciones en cada
/// petición. Reciben solo el hub (`cc-extensions hub`), que las busca bajo demanda.
/// Desviación del Python (15-oct, Jesús: OpenCode pagaba ≈500k tokens por mensaje con
/// 1.120 herramientas): `lib/extension_catalog.py` sigue exportando cada servidor a estos
/// dos clientes; el oráculo `tests/catalog_oracle.py` ya no aplica a sus configuraciones.
pub const HUB_KINDS: [&str; 2] = ["opencode", "agy"];
pub fn uses_hub(kind: &str) -> bool {
    HUB_KINDS.contains(&kind)
}
#[derive(Debug, Clone)]
pub struct Target {
    pub kind: &'static str,
    pub path: PathBuf,
    pub key: &'static str,
    pub mail_only: bool,
}
pub fn accounts(home: &Path, kind: &str) -> Result<Vec<PathBuf>> {
    let root = home.join(format!(".{kind}-accounts"));
    let mut paths = Vec::new();
    match fs::read_dir(&root) {
        Ok(entries) => {
            for entry in entries {
                let path = entry.map_err(|_| config::err(&root))?.path();
                if path.is_dir() && !path.is_symlink() {
                    paths.push(path);
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(config::err(&root)),
    }
    paths.sort();
    Ok(paths)
}
pub fn targets(home: &Path) -> Result<Vec<Target>> {
    let mut out = Vec::new();
    for (kind, path, key) in [
        ("claude", ".claude.json", "mcpServers"),
        ("codex", ".codex/config.toml", "mcp_servers"),
        ("grok", ".grok/config.toml", "mcp_servers"),
        ("opencode", ".config/opencode/opencode.json", "mcp"),
        ("agy", ".gemini/config/mcp_config.json", "mcpServers"),
    ] {
        out.push(Target {
            kind,
            path: home.join(path),
            key,
            mail_only: false,
        });
    }
    let path = home.join(".config/opencode/opencode.jsonc");
    if path.exists() {
        out.push(Target {
            kind: "opencode",
            path,
            key: "mcp",
            mail_only: false,
        });
    }
    for (kind, file, key) in [
        ("claude", ".claude.json", "mcpServers"),
        ("codex", "config.toml", "mcp_servers"),
        ("grok", "config.toml", "mcp_servers"),
    ] {
        for root in accounts(home, kind)? {
            out.push(Target {
                kind,
                path: root.join(file),
                key,
                mail_only: false,
            });
        }
    }
    for (kind, path, key) in [
        ("gemini", ".gemini/settings.json", "mcpServers"),
        ("vscode", ".config/Code/User/mcp.json", "servers"),
        (
            "copilot",
            ".config/github-copilot/intellij/mcp.json",
            "servers",
        ),
    ] {
        let path = home.join(path);
        if path.exists() {
            out.push(Target {
                kind,
                path,
                key,
                mail_only: true,
            });
        }
    }
    Ok(out)
}
pub fn alias(name: &str) -> &str {
    match name {
        "x_suite" => "x-suite",
        "linear-server" => "linear",
        "chrome-devtools" | "chrome-devtools-current" | "chrome-current" => "chrome-bg",
        _ => name,
    }
}
fn safe(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}
fn truth(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
    }
}
fn enabled(spec: &Value) -> bool {
    spec.get("enabled").is_none_or(truth)
}
pub fn normalize(spec: &Value) -> Result<Option<Value>> {
    if !spec.is_object() {
        return Err("Invalid MCP definition".into());
    }
    let command = spec.get("command").filter(|v| truth(v));
    let endpoint = ["url", "serverUrl", "httpUrl"]
        .iter()
        .find_map(|k| spec.get(k).filter(|v| truth(v)));
    if command.is_none() && endpoint.is_none() {
        return Ok(None);
    }
    let mut out = json!({"enabled":spec["enabled"]!=false&&!truth(&spec["disabled"])});
    if let Some(command) = command {
        if let Some(command) = command.as_array() {
            out["command"] = command[0].clone();
            out["args"] = json!(&command[1..]);
        } else {
            out["command"] = command.clone();
            out["args"] = spec.get("args").cloned().unwrap_or(json!([]));
        }
        for k in ["cwd", "env", "env_vars"] {
            if let Some(v) = spec.get(k) {
                out[k] = v.clone();
            }
        }
        if let Some(v) = spec.get("environment") {
            out["env"] = v.clone();
        }
    } else {
        out["url"] = endpoint.unwrap().clone();
        out["transport"] = json!(if spec["type"] == "sse" { "sse" } else { "http" });
        out["headers"] = spec
            .get("headers")
            .filter(|v| truth(v))
            .or_else(|| spec.get("http_headers").filter(|v| truth(v)))
            .cloned()
            .unwrap_or(json!({}));
        for k in ["bearer_token_env_var", "env_http_headers"] {
            if let Some(v) = spec.get(k) {
                out[k] = v.clone();
            }
        }
    }
    if truth(&spec["disabled_tools"]) {
        out["disabled_tools"] = spec["disabled_tools"].clone();
    }
    if !spec["enabled_tools"].is_null() {
        out["enabled_tools"] = spec["enabled_tools"].clone();
    }
    Ok(Some(out))
}
fn remote_rules(home: &Path, name: &str, mut spec: Value) -> Value {
    if name == "chrome-bg" {
        spec =
            json!({"command":home.join(".local/bin/cc-browser-remote"),"args":[],"enabled":true});
    }
    if REMOTE.contains(&name) {
        spec["enabled"] = json!(false);
        spec["unavailable_reason"] = json!("Remote browser runtime has not been verified");
    }
    spec
}
fn is_local(spec: &Value) -> bool {
    spec["url"]
        .as_str()
        .and_then(|s| url::Url::parse(s).ok())
        .and_then(|u| u.host_str().map(str::to_owned))
        .is_some_and(|s| s == "localhost" || s == "127.0.0.1")
}
fn object<'a>(value: &'a Value, key: &str) -> Result<Option<&'a serde_json::Map<String, Value>>> {
    match value.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_object()
            .map(Some)
            .ok_or_else(|| format!("Invalid configuration field: {key}")),
    }
}
pub fn import_catalog(home: &Path) -> Result<Value> {
    let mut servers = json!({});
    let mut serializable = BTreeMap::new();
    for target in targets(home)? {
        let bytes = config::read_bytes(&target.path)?;
        let document = Document::parse(&target.path, bytes.as_deref())?;
        let data = &document.data;
        let mut candidates = Vec::new();
        if let Some(projects) = object(data, "projects")? {
            for (project_name, p) in projects {
                if let Some(items) = object(p, "mcpServers")? {
                    for (n, s) in items {
                        candidates.push((
                            n,
                            s,
                            false,
                            vec!["projects", project_name.as_str(), "mcpServers", n.as_str()],
                        ));
                    }
                }
            }
        }
        if let Some(items) = object(data, target.key)? {
            for (n, s) in items {
                let disabled = data["disabled_mcp_servers"]
                    .as_array()
                    .is_some_and(|a| a.contains(&json!(alias(n))));
                candidates.push((n, s, disabled, vec![target.key, n.as_str()]));
            }
        }
        for (raw, s, disabled, source) in candidates {
            if target.mail_only && !MAIL.contains(&raw.as_str()) {
                continue;
            }
            let name = alias(raw);
            if name == "node_repl" || !safe(name) || uses_hub(target.kind) && name == HUB {
                continue;
            }
            let Some(mut spec) = normalize(s)? else {
                continue;
            };
            if disabled {
                spec["enabled"] = json!(false);
            }
            let previous = &servers[name];
            if previous.is_null()
                || !enabled(previous) && enabled(&spec)
                || is_local(&spec) && !is_local(previous)
            {
                serializable.insert(name.to_owned(), document.check_normalized(&source, s));
                servers[name] = spec;
            }
        }
    }
    for (n, s) in servers.as_object_mut().unwrap() {
        if n != "chrome-bg"
            && let Some(result) = serializable.remove(n)
        {
            result?;
        }
        *s = remote_rules(home, n, s.clone());
    }
    Ok(json!({"version":1,"servers":servers}))
}
fn managed(spec: &Value, launcher: &str) -> bool {
    spec["command"] == launcher
        || spec["command"]
            .as_array()
            .is_some_and(|a| a.first() == Some(&json!(launcher)))
}
fn specs(data: &Value, key: &str) -> Result<Value> {
    let mut out = data.get(key).cloned().unwrap_or(json!({}));
    let items = out.as_object_mut().ok_or("Invalid MCP definitions")?;
    if let Some(disabled) = data["disabled_mcp_servers"].as_array() {
        for (n, s) in items {
            if !s.is_object() {
                return Err(format!("Invalid MCP definition: {n}"));
            }
            if disabled.contains(&json!(n)) {
                s["enabled"] = json!(false);
            }
        }
    }
    Ok(out)
}
pub fn reconcile(
    home: &Path,
    catalog: &Value,
    launcher: &str,
    observed: &mut Value,
) -> Result<Value> {
    let snapshot = read_config(&state_dir(home).join("snapshot.json"))?;
    let mut proposals = BTreeMap::new();
    for target in targets(home)? {
        let p = target.path.to_string_lossy();
        let raw = config::read_bytes(&target.path)?;
        let document = Document::parse(&target.path, raw.as_deref())?;
        let current = specs(&document.data, target.key)?;
        let hashes = document.native_fingerprints(target.key)?;
        observed[p.as_ref()] = hashes.clone();
        let previous = snapshot["targets"].get(p.as_ref());
        if previous.is_none() && snapshot.as_object().is_some_and(|o| o.is_empty()) {
            continue;
        }
        let mut names = BTreeSet::new();
        if let Some(prev) = previous.and_then(Value::as_object) {
            names.extend(prev.keys().cloned());
        }
        names.extend(current.as_object().unwrap().keys().cloned());
        for raw_name in names {
            let name = alias(&raw_name);
            if target.mail_only && !MAIL.contains(&name) || name == "node_repl" || !safe(name) {
                continue;
            }
            let item = current.get(&raw_name);
            // Un cliente con hub no declara cada servidor: su ausencia no desactiva nada
            // (ni tras el primer cambio a hub, ni si se cortó antes de guardar la foto).
            if uses_hub(target.kind) && (name == HUB || item.is_none()) {
                continue;
            }
            if item.is_some() && previous.and_then(|p| p.get(&raw_name)) == hashes.get(&raw_name) {
                continue;
            }
            let old = catalog["servers"].get(name);
            let mut desired = if let Some(item) = item {
                if managed(item, launcher) {
                    let Some(old) = old else { continue };
                    let mut desired = old.clone();
                    desired["enabled"] =
                        json!(item["enabled"] != false && !truth(&item["disabled"]));
                    if config::toml::python_equal(
                        &desired["enabled"],
                        &old.get("enabled").cloned().unwrap_or(json!(true)),
                    ) {
                        continue;
                    }
                    desired
                } else {
                    if name != "chrome-bg" {
                        document.check_normalized(&[target.key, &raw_name], item)?;
                    }
                    normalize(item)?.ok_or_else(|| format!("Incomplete MCP definition: {name}"))?
                }
            } else {
                let Some(old) = old else { continue };
                let mut desired = old.clone();
                desired["enabled"] = json!(false);
                desired
            };
            desired = remote_rules(home, name, desired);
            if let Some(expected) = snapshot["catalog"].get(name)
                && fingerprint(old.unwrap_or(&Value::Null))? != expected.as_str().unwrap_or("")
                && !old.is_some_and(|old| config::toml::python_equal(old, &desired))
            {
                return Err(format!("Simultaneous catalog and native edit: {name}"));
            }
            if proposals
                .get(name)
                .is_some_and(|p| !config::toml::python_equal(p, &desired))
            {
                return Err(format!("Conflicting native edits: {name}"));
            }
            proposals.insert(name.to_owned(), desired);
        }
    }
    let mut result = catalog.clone();
    for (n, s) in proposals {
        result["servers"][n] = s;
    }
    Ok(result)
}
pub fn save_snapshot(home: &Path, catalog: &Value, expected: Option<&Value>) -> Result<()> {
    let mut snapshot = json!({"catalog":{},"targets":{}});
    for (n, s) in catalog["servers"].as_object().ok_or("Invalid catalog")? {
        snapshot["catalog"][n] = json!(fingerprint(s)?);
    }
    for target in targets(home)? {
        let bytes = config::read_bytes(&target.path)?;
        let doc = Document::parse(&target.path, bytes.as_deref())?;
        snapshot["targets"][target.path.to_string_lossy().as_ref()] =
            doc.native_fingerprints(target.key)?;
    }
    if expected.is_some_and(|e| e != &snapshot["targets"]) {
        return Err("MCP configuration changed during synchronization; retry".into());
    }
    let path = state_dir(home).join("snapshot.json");
    if read_config(&path)? != snapshot {
        config::save_json(&path, &snapshot)?;
    }
    Ok(())
}
const HUB: &str = crate::hub::NAME;
/// Entrada del hub con la forma propia de cada cliente con hub.
pub fn hub_entry(kind: &str, launcher: &str) -> Value {
    if kind == "opencode" {
        return json!({"type":"local","command":[launcher,"hub"],"enabled":true});
    }
    json!({"command":launcher,"args":["hub"],"disabled":false})
}
pub fn native_entry(kind: &str, name: &str, launcher: &str, old: &Value) -> Value {
    if kind == "opencode" {
        return json!({"type":"local","command":[launcher,"serve",name],"enabled":true});
    }
    let mut entry = json!({"command":launcher,"args":["serve",name]});
    if ["claude", "vscode"].contains(&kind) {
        entry["type"] = json!("stdio");
    }
    if kind == "agy" {
        entry["disabled"] = json!(false);
    }
    if ["codex", "grok"].contains(&kind) {
        for key in CLIENT_FIELDS {
            if let Some(value) = old.get(key) {
                entry[key] = value.clone();
            }
        }
        if entry.get("startup_timeout_sec").is_none() {
            entry["startup_timeout_sec"] = json!(45);
        }
    }
    entry
}
pub fn sync_configs(
    home: &Path,
    catalog: &Value,
    launcher: &str,
    observed: &mut Value,
    expected: Option<&Value>,
) -> Result<Vec<PathBuf>> {
    let mut changed = Vec::new();
    let policy_path = state_dir(home).join("client-policies.json");
    let mut policies = read_config(&policy_path)?;
    let enabled: BTreeMap<_, _> = catalog["servers"]
        .as_object()
        .ok_or("Invalid catalog")?
        .iter()
        .filter(|(_, s)| enabled(s))
        .collect();
    for target in targets(home)? {
        let path = &target.path;
        let path_key = path.to_string_lossy();
        let before = config::read_bytes(path)?;
        let mut doc = Document::parse(path, before.as_deref())?;
        let original = doc.data.clone();
        let old = doc.data.get(target.key).cloned().unwrap_or(json!({}));
        if expected.is_some_and(|e| {
            e.get(path_key.as_ref()) != doc.native_fingerprints(target.key).ok().as_ref()
        }) {
            return Err(format!(
                "MCP configuration changed before export; retry: {}",
                path.display()
            ));
        }
        if policies.get(path_key.as_ref()).is_none() {
            policies[path_key.as_ref()] = json!({});
        }
        let saved = &mut policies[path_key.as_ref()];
        for (n, s) in old.as_object().ok_or_else(|| config::err(path))? {
            if !s.is_object() {
                return Err(format!("Invalid MCP definition: {n}"));
            }
            let mut fields = json!({});
            for key in CLIENT_FIELDS {
                if let Some(v) = s.get(key) {
                    doc.check_policy(target.key, n, key)?;
                    fields[key] = v.clone();
                }
            }
            saved[alias(n)] = fields;
        }
        if !config::toml::python_equal(&read_config(&policy_path)?, &policies) {
            config::save_json(&policy_path, &policies)?;
        }
        let saved = &policies[path_key.as_ref()];
        let mut desired = json!({});
        if target.mail_only {
            for (n, s) in old.as_object().unwrap() {
                if !MAIL.contains(&n.as_str()) {
                    desired[n] = s.clone();
                }
            }
        }
        if uses_hub(target.kind) {
            if !enabled.is_empty() {
                desired[HUB] = hub_entry(target.kind, launcher);
            }
        } else {
            for name in enabled.keys() {
                if !target.mail_only || MAIL.contains(&name.as_str()) {
                    desired[*name] = native_entry(target.kind, name, launcher, &saved[*name]);
                }
            }
        }
        if let Some(s) = old.get("node_repl") {
            desired["node_repl"] = s.clone();
        }
        if !config::toml::python_equal(&old, &desired) {
            doc.data[target.key] = desired;
        }
        if let Some(projects) = doc.data.get_mut("projects").and_then(Value::as_object_mut) {
            for project in projects.values_mut() {
                if let Some(specs) = project.get_mut("mcpServers").and_then(Value::as_object_mut) {
                    for (name, spec) in specs {
                        let canonical = alias(name);
                        if let Some(enabled) = enabled.get(&canonical.to_owned())
                            && (MAIL.contains(&canonical)
                                || normalize(spec)?
                                    .is_some_and(|spec| config::toml::python_equal(&spec, enabled)))
                        {
                            *spec = native_entry(target.kind, canonical, launcher, &json!({}));
                        }
                    }
                }
            }
        }
        if target.kind == "grok" {
            doc.data["disabled_mcp_servers"] = json!(
                doc.data["disabled_mcp_servers"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|n| n
                        .as_str()
                        .is_none_or(|n| !enabled.contains_key(&n.to_owned())))
                    .collect::<Vec<_>>()
            );
        }
        observed[path_key.as_ref()] = doc.native_fingerprints(target.key)?;
        if before.is_some() && config::toml::python_equal(&original, &doc.data) {
            continue;
        }
        if config::replace_config(home, path, before.as_deref(), &doc.render()?)? {
            changed.push(path.clone());
        }
    }
    Ok(changed)
}
