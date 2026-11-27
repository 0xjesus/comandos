//! Read cached CLI catalogs without probing a provider or mutating its registry.
use comandos_core::json::truthy;
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
pub const MAX_CACHE_BYTES: usize = 4 * 1024 * 1024;
pub struct Paths {
    pub codex: PathBuf,
    pub grok: PathBuf,
    /// `~/.cache/comandos/models/omp.json`: `omp models --json` ya
    /// normalizado por `cc-model-sync` (formato [`CLI_CACHES`]).
    pub omp: PathBuf,
    /// `~/.cache/comandos/models/opencode.json`: `opencode models` con nombre
    /// y fecha, lo más nuevo primero.
    pub opencode: PathBuf,
    /// `~/.cache/comandos/models/refuseless.json`: the `/v1/models` of
    /// Refuseless (a Claude Code motor), same format.
    pub refuseless: PathBuf,
    pub cwd: PathBuf,
}
/// Catálogos que escribe `cc-model-sync` en un formato ya normalizado:
/// `{"fetched_at": "...", "models": [{"id", "name", "efforts", "defaultEffort",
/// "contextWindow"?, "tag"?}]}`.
pub const CLI_CACHES: [&str; 3] = ["omp", "opencode", "refuseless"];
pub fn catalog_paths(
    home: &Path,
    cwd: &Path,
    codex_home: Option<&Path>,
    grok_home: Option<&Path>,
) -> Paths {
    let path = |provider: &str, over: Option<&Path>| {
        over.filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(format!(".{provider}")))
            .join("models_cache.json")
    };
    let cache = |cli: &str| home.join(".cache/comandos/models").join(format!("{cli}.json"));
    Paths {
        codex: path("codex", codex_home),
        grok: path("grok", grok_home),
        omp: cache("omp"),
        opencode: cache("opencode"),
        refuseless: cache("refuseless"),
        cwd: cwd.into(),
    }
}
impl Paths {
    fn entries(&self) -> [(&'static str, &Path); 5] {
        [
            ("codex", &self.codex),
            ("grok", &self.grok),
            ("omp", &self.omp),
            ("opencode", &self.opencode),
            ("refuseless", &self.refuseless),
        ]
    }
    fn supplied(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.into()
        } else {
            self.cwd.join(path)
        }
    }
}
pub fn catalog_signature(paths: &Paths) -> Value {
    use std::os::unix::fs::MetadataExt;
    Value::Array(
        paths
            .entries()
            .iter()
            .map(|(p, path)| {
                let stamp = fs::metadata(paths.supplied(path)).ok().map(|m| {
                    json!([
                        m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128,
                        m.size(),
                        m.ino()
                    ])
                });
                json!([p, path, stamp])
            })
            .collect(),
    )
}
pub(crate) use comandos_core::json::workspace_loads_bytes as byte_json;

fn read(path: &Path) -> Value {
    let mut raw = Vec::new();
    let value = fs::File::open(path)
        .ok()
        .and_then(|f| {
            f.take(MAX_CACHE_BYTES as u64 + 1)
                .read_to_end(&mut raw)
                .ok()
        })
        .filter(|_| raw.len() <= MAX_CACHE_BYTES)
        .and_then(|_| byte_json(&raw));
    value.filter(Value::is_object).unwrap_or_else(|| json!({}))
}
fn decimal(c: char) -> bool {
    c.is_ascii_digit()
        || (!c.is_ascii()
            && comandos_core::focus::float(&json!(c.to_string()))
                .is_ok_and(|n| (0.0..=9.0).contains(&n) && n.fract() == 0.0))
}
fn model_id(id: &str, provider: &str) -> bool {
    let Some(rest) = id.strip_prefix(if provider == "codex" { "gpt-" } else { "grok-" }) else {
        return false;
    };
    let mut chars = rest.chars().peekable();
    let mut digits = 0;
    while chars.peek().is_some_and(|c| decimal(*c)) {
        chars.next();
        digits += 1;
    }
    if digits == 0 {
        return false;
    }
    if chars.peek() == Some(&'.') {
        chars.next();
        let mut digits = 0;
        while chars.peek().is_some_and(|c| decimal(*c)) {
            chars.next();
            digits += 1;
        }
        if digits == 0 {
            return false;
        }
    }
    while let Some(c) = chars.next() {
        if c != '-' {
            return false;
        }
        let mut n = 0;
        while chars
            .peek()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            chars.next();
            n += 1;
        }
        if n == 0 {
            return false;
        }
    }
    true
}
fn effort(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(c))
}
pub fn catalog_models(paths: &Paths) -> Value {
    let mut result = serde_json::Map::new();
    for (provider, path) in paths.entries() {
        let data = read(&paths.supplied(path));
        if CLI_CACHES.contains(&provider) {
            result.insert(provider.into(), Value::Array(cli_cache_models(provider, &data)));
            continue;
        }
        let rows = &data["models"];
        let rows: Vec<&Value> = if provider == "codex" {
            rows.as_array()
                .map(|a| a.iter().collect())
                .unwrap_or_default()
        } else {
            rows.as_object()
                .map(|o| {
                    o.values()
                        .filter(|r| r.is_object())
                        .map(|r| &r["info"])
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut models = Vec::new();
        for row in rows {
            if !row.is_object()
                || (provider == "codex" && row["visibility"] != "list")
                || (provider == "grok" && row["hidden"] != Value::Bool(false))
            {
                continue;
            }
            let Some(id) = row[if provider == "codex" { "slug" } else { "id" }]
                .as_str()
                .filter(|id| model_id(id, provider))
            else {
                continue;
            };
            let Some(levels) = row[if provider == "codex" {
                "supported_reasoning_levels"
            } else {
                "reasoning_efforts"
            }]
            .as_array() else {
                continue;
            };
            let field = if provider == "codex" { "effort" } else { "id" };
            let mut efforts = Vec::new();
            for level in levels {
                if !level.is_object() {
                    continue;
                }
                if let Some(id) = level[field].as_str().filter(|id| effort(id))
                    && !efforts.contains(&id)
                {
                    efforts.push(id);
                }
            }
            let default = row[if provider == "codex" {
                "default_reasoning_level"
            } else {
                "reasoning_effort"
            }]
            .as_str()
            .filter(|d| efforts.contains(d))
            .unwrap_or("");
            let name = row[if provider == "codex" {
                "display_name"
            } else {
                "name"
            }]
            .as_str()
            .map(|s| s.chars().take(160).collect::<String>())
            .unwrap_or_else(|| id.into());
            let mut model = json!({"id":id,"name":name,"efforts":efforts,"defaultEffort":default,"catalogSource":{"provider":provider,"kind":"cli-cache","fetchedAt":data["fetched_at"].as_str().map(|s|s.chars().take(64).collect::<String>())}});
            if row["context_window"].as_number().is_some_and(|n| {
                !n.as_str().starts_with('-')
                    && !n.as_str().contains(['.', 'e', 'E'])
                    && n.as_str().bytes().any(|b| b != b'0')
            }) {
                model["contextWindow"] = row["context_window"].clone();
            }
            models.push(model);
        }
        result.insert(provider.into(), Value::Array(models));
    }
    Value::Object(result)
}
/// Un id de modelo de un catálogo ya normalizado: texto visible, sin blancos.
fn cli_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id.bytes().all(|b| b.is_ascii_graphic())
}
/// Las filas de un catálogo de [`CLI_CACHES`], en su orden (lo más nuevo
/// primero), sin repetir ids.
fn cli_cache_models(provider: &str, data: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for row in data["models"].as_array().into_iter().flatten() {
        let Some(id) = row["id"].as_str().filter(|id| cli_model_id(id)) else {
            continue;
        };
        if out.iter().any(|m| m["id"] == id) {
            continue;
        }
        let mut efforts: Vec<&str> = Vec::new();
        for e in row["efforts"].as_array().into_iter().flatten() {
            if let Some(e) = e.as_str().filter(|e| effort(e))
                && !efforts.contains(&e)
            {
                efforts.push(e);
            }
        }
        let default = row["defaultEffort"]
            .as_str()
            .filter(|d| efforts.contains(d))
            .unwrap_or("");
        let name = row["name"]
            .as_str()
            .filter(|n| !n.trim().is_empty())
            .map(|s| s.chars().take(160).collect::<String>())
            .unwrap_or_else(|| id.into());
        let mut model = json!({"id":id,"name":name,"efforts":efforts,"defaultEffort":default,"catalogSource":{"provider":provider,"kind":"cli-cache","fetchedAt":data["fetched_at"].as_str().map(|s|s.chars().take(64).collect::<String>())}});
        if let Some(n) = row["contextWindow"].as_u64().filter(|n| *n > 0) {
            model["contextWindow"] = n.into();
        }
        if let Some(tag) = row["tag"].as_str().filter(|t| !t.is_empty() && t.chars().count() <= 40) {
            model["tag"] = tag.into();
        }
        out.push(model);
    }
    out
}
fn normalized(id: &str) -> String {
    // Python dot does not cross LF and `$` can match before its final LF.
    let trailing = id.ends_with('\n');
    let body = if trailing { &id[..id.len() - 1] } else { id };
    let start = body.rfind('\n').map_or(0, |n| n + 1);
    let Some(pos) = body[start..].find('[').map(|n| start + n) else {
        return id.into();
    };
    format!("{}{}", &body[..pos], if trailing { "\n" } else { "" })
}
pub fn hydrate_registry(registry: &Value, paths: &Paths) -> std::result::Result<Value, String> {
    if !registry.is_object() {
        return Err("registry debe ser un objeto".into());
    }
    let mut result = registry.clone();
    for (provider, models) in catalog_models(paths).as_object().expect("catalog object") {
        for section in ["motors", "harnesses"] {
            let Some(owners) = result.get_mut(section) else {
                continue;
            };
            if !truthy(owners) {
                continue;
            }
            let owners = owners
                .as_object_mut()
                .ok_or("registry section debe ser un objeto")?;
            let Some(owner) = owners.get_mut(provider).filter(|o| truthy(o)) else {
                continue;
            };
            let owner = owner
                .as_object_mut()
                .ok_or("registry owner debe ser un objeto")?;
            if section == "harnesses" && !owner.contains_key("models") {
                continue;
            }
            let target = owner
                .entry("models")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .ok_or("models debe ser una lista")?;
            for observed in models.as_array().expect("catalog array") {
                let key = normalized(observed["id"].as_str().expect("validated id"));
                let mut position = None;
                for (index, item) in target.iter().enumerate() {
                    if !item.is_object() {
                        return Err("model debe ser un objeto".into());
                    }
                    let id = item.get("id").unwrap_or(&Value::Null);
                    let id = if id.is_null() && item.get("id").is_none() {
                        ""
                    } else {
                        id.as_str().ok_or("model id debe ser texto")?
                    };
                    if normalized(id) == key {
                        position = Some(index);
                        break;
                    }
                }
                if let Some(position) = position {
                    let existing = target[position]
                        .as_object_mut()
                        .ok_or("model debe ser un objeto")?;
                    if existing.remove("soon").is_some_and(|v| truthy(&v)) {
                        existing.remove("tag");
                    }
                    for field in ["efforts", "defaultEffort", "contextWindow", "catalogSource", "tag"] {
                        if let Some(value) = observed.get(field) {
                            existing.insert(field.into(), value.clone());
                        }
                    }
                } else {
                    target.push(observed.clone());
                }
            }
        }
    }
    Ok(result)
}
