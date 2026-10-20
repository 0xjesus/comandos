//! Loss-checked private configuration IO. The sync lock coordinates our writers;
//! comparisons cannot provide an atomic CAS against unrelated external writers.
pub mod toml;
use crate::Result;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
pub fn state_dir(home: &Path) -> PathBuf {
    home.join(".local/state/comandos/extensions")
}
pub fn catalog_path(home: &Path) -> PathBuf {
    home.join(".config/comandos/extensions/catalog.json")
}
pub fn err(path: &Path) -> String {
    format!("Invalid configuration: {}", path.display())
}
pub fn private_dir(path: &Path) -> Result<()> {
    if path.symlink_metadata().is_err() {
        crate::mutations::before(crate::mutations::Mutation::Directory { path, mode: 0o700 })?;
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|_| err(path))?;
    crate::mutations::after(path)
}
pub fn temporary(parent: &Path, prefix: &str) -> PathBuf {
    parent.join(format!(
        ".{prefix}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}
pub struct SyncLock(File);
impl SyncLock {
    pub fn new(home: &Path) -> Result<Self> {
        let root = state_dir(home);
        private_dir(&root)?;
        let p = root.join("sync.lock");
        crate::mutations::before(crate::mutations::Mutation::Control { path: &p })?;
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(&p)
            .map_err(|_| err(&p))?;
        f.lock().map_err(|_| err(&p))?;
        Ok(Self(f))
    }
}
impl Drop for SyncLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
pub fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    if let Some((home, name)) = state_document(path) {
        let doc = comandos_store::domains::DomainStore { home: &home }.document(&name, "extensions", path.to_owned());
        let bytes = comandos_store::wait_admitted(comandos_store::BACKGROUND_READ_WAIT, || doc.read_readonly())
            .map_err(|e| e.to_string())?;
        if bytes.as_ref().is_some_and(|b| b.len() > 16 * 1024 * 1024) {
            return Err(err(path));
        }
        return Ok(bytes);
    }
    legacy_read_bytes(path)
}
fn legacy_read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(err(path)),
    };
    let mut b = Vec::new();
    f.take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut b)
        .map_err(|_| err(path))?;
    if b.len() > 16 * 1024 * 1024 {
        return Err(err(path));
    }
    Ok(Some(b))
}
pub fn json_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut b = serde_json::to_vec_pretty(value).map_err(|_| "Invalid configuration value")?;
    b.push(b'\n');
    Ok(b)
}
pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some((home, name)) = state_document(path) {
        return write_state_with(&home, &name, path, bytes, || {
            legacy_private_write(path, bytes)
        });
    }
    legacy_private_write(path, bytes)
}
/// Sólo documentos de estado catalogados; secretos, respaldos y configuración quedan archivos.
pub(crate) fn state_document(path: &Path) -> Option<(PathBuf, String)> {
    let components = path.components().collect::<Vec<_>>();
    let marker = [".local", "state", "comandos", "extensions"];
    let at = components.windows(4).position(|parts| {
        parts
            .iter()
            .zip(marker)
            .all(|(part, text)| part.as_os_str() == text)
    })?;
    let home: PathBuf = components.iter().take(at).collect();
    if !home.is_absolute() {
        return None;
    }
    let rel = path.strip_prefix(state_dir(&home)).ok()?;
    let normal = matches!(
        rel.to_str(),
        Some("snapshot.json" | "skills.json" | "client-policies.json" | "last-check.json")
    );
    let size =
        rel.parent() == Some(Path::new("sizes")) && rel.file_name()?.to_str()?.ends_with(".json");
    if !normal && !size {
        return None;
    }
    Some((home, format!("state/extensions/{}", rel.to_str()?)))
}
pub(crate) fn write_state_with(
    home: &Path,
    name: &str,
    path: &Path,
    bytes: &[u8],
    legacy: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let mut lock = path.as_os_str().to_owned();
    lock.push(".lock");
    crate::mutations::before(crate::mutations::Mutation::Control {
        path: Path::new(&lock),
    })?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    comandos_store::domains::caller::write(
        home,
        "extensions",
        Path::new(&lock),
        || legacy().map_err(comandos_store::Error::Validation),
        |db, origin| {
            let now = i64::try_from(now).unwrap_or(i64::MAX);
            crate::mutations::document(home, db, name, bytes, origin, now)
                .map_err(comandos_store::Error::Validation)?;
            comandos_store::unified::doc_put(db, name, "extensions", bytes, origin, now).map(|_| ())
        },
    )
    .map_err(|e| e.to_string())?;
    crate::mutations::document_committed(name)
}
fn legacy_private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::mutations::before(crate::mutations::Mutation::File {
        path,
        bytes,
        mode: 0o600,
    })?;
    let parent = path.parent().ok_or_else(|| err(path))?;
    private_dir(parent)?;
    let (tmp, mut file) = loop {
        let tmp = temporary(parent, "comandos");
        if tmp.symlink_metadata().is_err() {
            crate::mutations::before(crate::mutations::Mutation::File {
                path: &tmp,
                bytes,
                mode: 0o600,
            })?;
        }
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
        {
            Ok(f) => break (tmp, f),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(err(path)),
        }
    };
    let result = (|| {
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| err(path))?;
        crate::mutations::after(&tmp)?;
        crate::mutations::before(crate::mutations::Mutation::Move {
            source: &tmp,
            target: path,
        })?;
        fs::rename(&tmp, path).map_err(|_| err(path))?;
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|_| err(path))
    })();
    let _ = fs::remove_file(tmp);
    result?;
    crate::mutations::after(path)
}
pub fn save_json(path: &Path, value: &Value) -> Result<()> {
    private_write(path, &json_bytes(value)?)
}
pub fn replace_config(
    home: &Path,
    path: &Path,
    before: Option<&[u8]>,
    after: &[u8],
) -> Result<bool> {
    if path.is_symlink() {
        return Err(format!(
            "Configuration symlink requires an explicit target: {}",
            path.display()
        ));
    }
    if read_bytes(path)?.as_deref() != before {
        return Err(format!(
            "Configuration changed concurrently: {}",
            path.display()
        ));
    }
    if before == Some(after) {
        return Ok(false);
    }
    if let Some(before) = before {
        let root = state_dir(home).join("backups");
        private_dir(&root)?;
        let name = path
            .strip_prefix(home)
            .map_err(|_| err(path))?
            .to_string_lossy()
            .replace('/', "__");
        let backup = root.join(format!(
            "{name}.{}",
            &format!("{:x}", Sha256::digest(before))[..16]
        ));
        if backup.symlink_metadata().is_err() {
            crate::mutations::before(crate::mutations::Mutation::File {
                path: &backup,
                bytes: before,
                mode: 0o600,
            })?;
        }
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&backup)
        {
            Ok(mut f) => {
                f.write_all(before)
                    .and_then(|_| f.sync_all())
                    .map_err(|_| err(&backup))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if backup.is_symlink()
                    || !backup.is_file()
                    || read_bytes(&backup)?.as_deref() != Some(before)
                {
                    return Err(format!("Existing backup is invalid: {}", backup.display()));
                }
            }
            Err(_) => return Err(err(&backup)),
        }
    }
    if path.is_symlink() || read_bytes(path)?.as_deref() != before {
        return Err(format!(
            "Configuration changed during backup: {}",
            path.display()
        ));
    }
    private_write(path, after)?;
    Ok(true)
}
pub struct Document {
    pub data: Value,
    native: Option<toml::Document>,
}
impl Document {
    pub fn parse(path: &Path, raw: Option<&[u8]>) -> Result<Self> {
        let missing = raw.is_none();
        let raw = raw.unwrap_or(b"");
        if path.extension().is_some_and(|s| s == "toml") {
            let native = toml::Document::parse(std::str::from_utf8(raw).map_err(|_| err(path))?)
                .map_err(|_| err(path))?;
            let data = native.value()?;
            Ok(Self {
                data,
                native: Some(native),
            })
        } else {
            let data = if missing {
                json!({})
            } else {
                parse_json(raw).map_err(|_| err(path))?
            };
            Ok(Self { data, native: None })
        }
    }
    pub fn render(&self) -> Result<Vec<u8>> {
        match &self.native {
            Some(doc) => Ok(doc.render(&self.data)?.into_bytes()),
            None => json_bytes(&self.data),
        }
    }
    pub fn native_fingerprints(&self, key: &str) -> Result<Value> {
        let data = match &self.native {
            Some(doc) => doc.fingerprint_value(&self.data)?,
            None => self.data.clone(),
        };
        let mut result = json!({});
        if let Some(items) = data.get(key).and_then(Value::as_object) {
            for (n, s) in items {
                let mut s = s.clone();
                if data["disabled_mcp_servers"]
                    .as_array()
                    .is_some_and(|a| a.contains(&json!(n)))
                {
                    s["enabled"] = json!(false);
                }
                result[n] = json!(fingerprint(&s)?);
            }
        }
        Ok(result)
    }
    pub fn check_normalized(&self, path: &[&str], spec: &Value) -> Result<()> {
        if let Some(doc) = &self.native {
            let truth = |v: &&Value| match v {
                Value::Null => false,
                Value::Bool(v) => *v,
                Value::String(s) => !s.is_empty(),
                Value::Array(v) => !v.is_empty(),
                Value::Object(v) => !v.is_empty(),
                Value::Number(n) => n.as_f64() != Some(0.0),
            };
            let mut fields = Vec::new();
            if spec.get("command").filter(truth).is_some() {
                fields.extend(["command", "args", "cwd", "env_vars"]);
                fields.push(if spec.get("environment").is_some() {
                    "environment"
                } else {
                    "env"
                });
            } else {
                if let Some(key) = ["url", "serverUrl", "httpUrl"]
                    .into_iter()
                    .find(|key| spec.get(*key).filter(truth).is_some())
                {
                    fields.push(key);
                }
                fields.push(if spec.get("headers").filter(truth).is_some() {
                    "headers"
                } else {
                    "http_headers"
                });
                fields.extend(["bearer_token_env_var", "env_http_headers"]);
            }
            fields.extend(["disabled_tools", "enabled_tools"]);
            doc.check_fields(path, &fields)?;
        }
        Ok(())
    }
    pub fn check_policy(&self, key: &str, name: &str, field: &str) -> Result<()> {
        if let Some(doc) = &self.native {
            doc.check_policy(key, name, field)?;
        }
        Ok(())
    }
}
pub fn read_config(path: &Path) -> Result<Value> {
    let raw = read_bytes(path)?;
    Ok(Document::parse(path, raw.as_deref())?.data)
}
pub fn fingerprint(value: &Value) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(crate::python_json::dumps(value, true, false)?.as_bytes())
    ))
}

// CPython accepts these non-finite constants in JSON; keep them as numeric
// atoms without feeding serde's private map tokens through generic Value serde.
pub fn parse_json(raw: &[u8]) -> Result<Value> {
    if let Ok(value) = crate::auth::parse_config_bytes(raw) {
        return Ok(value);
    }
    let clean = crate::auth::clean_config_bytes(raw)?;
    let text = std::str::from_utf8(&clean).map_err(|_| "Invalid JSON")?;
    let tokens =
        regex::Regex::new(r#""(?:\\.|[^"\\])*"|-?Infinity|NaN"#).map_err(|_| "Invalid JSON")?;
    let strings = tokens
        .find_iter(text)
        .filter_map(|m| serde_json::from_str::<String>(m.as_str()).ok())
        .collect::<Vec<_>>();
    let mut markers = std::collections::BTreeMap::new();
    let replaced = tokens.replace_all(text, |caps: &regex::Captures<'_>| {
        let token = &caps[0];
        if token.starts_with('"') {
            return token.to_owned();
        }
        let mut marker = format!("__comandos_nonfinite_{}__", markers.len());
        while strings.contains(&marker) || markers.contains_key(&marker) {
            marker.push('_');
        }
        markers.insert(marker.clone(), token.to_owned());
        format!("\"{marker}\"")
    });
    let mut value = crate::auth::parse_clean_config_bytes(replaced.as_bytes())?;
    fn restore(
        value: &mut Value,
        markers: &std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        match value {
            Value::String(s) => {
                if let Some(n) = markers.get(s) {
                    *value = Value::Number(serde_json::Number::from_string_unchecked(n.clone()));
                }
            }
            Value::Array(a) => {
                for v in a {
                    restore(v, markers)?;
                }
            }
            Value::Object(o) => {
                for (k, v) in o {
                    if markers.contains_key(k) {
                        return Err("Invalid JSON key".into());
                    }
                    restore(v, markers)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    restore(&mut value, &markers)?;
    Ok(value)
}
/// Historical per-session defaults. Retained for callers describing the migration;
/// global sharing now covers these services too.
pub const DEDICATED: &[&str] = &[
    "chrome-bg",
    "claude-in-chrome",
    "playwright",
    "x-playwright",
    "lightpanda",
    "obscura",
    "screenwright",
    "teams",
    "claude-codex",
    "x-suite",
];
/// Every connector shares the broker by default. An explicit catalog override remains
/// available for installations that intentionally require a dedicated connection.
pub fn is_shared(spec: &Value, _name: &str) -> bool {
    spec.get("shared").and_then(Value::as_bool).unwrap_or(true)
}

#[cfg(test)]
mod shared_tests {
    use super::is_shared;
    use serde_json::json;

    #[test]
    fn all_services_share_by_default_unless_the_catalog_explicitly_opts_out() {
        let plain = json!({"command": "x"});
        for name in ["x-suite", "chrome-bg", "claude-codex"] {
            assert!(is_shared(&plain, name), "{name}");
            assert!(
                is_shared(&json!({"command": "x", "shared": true}), name),
                "{name}"
            );
        }
        assert!(is_shared(&plain, "mobbin"));
        assert!(!is_shared(
            &json!({"command": "x", "shared": false}),
            "mobbin"
        ));
    }
}
