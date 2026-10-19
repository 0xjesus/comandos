//! Read-only subscription account metadata with explicitly supplied path roots.
use base64::{
    Engine,
    engine::{GeneralPurpose, GeneralPurposeConfig},
};
use comandos_core::{
    focus,
    json::{truthy, workspace_dumps, workspace_loads},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

/// Lo que quien recibe un `AccountError` necesita distinguir sin comparar
/// su texto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Un `AccountError` de `lib/account_registry.py`, con su mensaje.
    Account,
    /// La ruta de la cuenta no es UTF-8: el Python la escribiría con
    /// sustitutos, así que quien la recibe no reproduce su respuesta.
    NonUtf8Home,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountError(pub String, pub ErrorKind);
impl AccountError {
    pub fn kind(&self) -> ErrorKind {
        self.1
    }
}
impl std::fmt::Display for AccountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for AccountError {}
pub type Result<T> = std::result::Result<T, AccountError>;

/// No process environment or password database is consulted for expansion.
pub struct Paths {
    pub home: PathBuf,
    pub cwd: PathBuf,
    pub user_homes: BTreeMap<String, PathBuf>,
}
impl Paths {
    pub fn new(home: &Path, cwd: &Path) -> Self {
        Self {
            home: home.into(),
            cwd: cwd.into(),
            user_homes: BTreeMap::new(),
        }
    }
}
fn error(message: impl Into<String>) -> AccountError {
    AccountError(message.into(), ErrorKind::Account)
}
fn account_error(e: &AccountError) -> bool {
    e.0.ends_with(": cuentas no soportadas")
        || e.0.ends_with(": registro de cuentas incompleto")
        || matches!(
            e.0.as_str(),
            "alias de cuenta invalido"
                | "cuenta fuera de accountsRoot"
                | "aliases de cuenta no pueden ser symlinks"
        )
}
/// ¿Es un `AccountError` de `lib/accounts.py` (con ese mensaje)? Los demás
/// errores de este módulo son otras excepciones del Python (o lo que el port
/// no reproduce): quien llama no puede responder con su texto.
pub fn is_account_error(e: &AccountError) -> bool {
    e.kind() == ErrorKind::Account && account_error(e)
}
fn object<'a>(value: &'a Value, what: &str) -> Result<&'a serde_json::Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| error(format!("{what}: se esperaba un objeto")))
}
fn spec<'a>(registry: &'a Value, provider: &str) -> Result<&'a Value> {
    let harnesses = &registry["harnesses"];
    let item = if truthy(harnesses) {
        object(harnesses, "harnesses")?
            .get(provider)
            .unwrap_or(&Value::Null)
    } else {
        &Value::Null
    };
    if !item.is_object() {
        return Err(error(format!("{provider}: cuentas no soportadas")));
    }
    let capabilities = &item["capabilities"];
    if truthy(capabilities) {
        object(capabilities, "capabilities")?;
    }
    if !truthy(&capabilities["accounts"]) {
        return Err(error(format!("{provider}: cuentas no soportadas")));
    }
    for key in ["defaultHome", "accountsRoot", "authFile", "accountEnv"] {
        if !truthy(&item[key]) {
            return Err(error(format!("{provider}: registro de cuentas incompleto")));
        }
    }
    Ok(item)
}
fn alias_valid(alias: &str) -> bool {
    let b = alias.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(c))
}
pub fn validate_alias(alias: &Value) -> Result<String> {
    let alias = if truthy(alias) {
        python_string(alias)
    } else {
        "main".into()
    };
    if alias != "main" && !alias_valid(&alias) {
        return Err(error("alias de cuenta invalido"));
    }
    Ok(alias)
}
fn expand(path: &Value, paths: &Paths) -> Result<PathBuf> {
    let raw = path
        .as_str()
        .ok_or_else(|| error("ruta de cuenta inválida"))?;
    if !raw.starts_with('~') {
        return Ok(raw.into());
    }
    let (user, rest) = raw[1..].split_once('/').unwrap_or((&raw[1..], ""));
    let home = if user.is_empty() {
        Some(&paths.home)
    } else {
        paths.user_homes.get(user)
    };
    Ok(home.map(|h| h.join(rest)).unwrap_or_else(|| raw.into()))
}
/// pathlib.resolve(strict=False): expand existing links before applying `..`,
/// retain missing components, and reject actual link loops.
fn resolve(path: &Path, cwd: &Path) -> Result<PathBuf> {
    fn walk(path: &Path, result: &mut PathBuf, active: &mut HashSet<PathBuf>) -> Result<()> {
        for component in path.components() {
            match component {
                Component::RootDir => *result = PathBuf::from("/"),
                Component::CurDir => {}
                Component::ParentDir => {
                    result.pop();
                }
                Component::Normal(name) => {
                    result.push(name);
                    match fs::read_link(&result) {
                        Ok(target) => {
                            let link = result.clone();
                            if !active.insert(link.clone()) {
                                return Err(error("bucle de symlink en cuenta"));
                            }
                            result.pop();
                            walk(&target, result, active)?;
                            active.remove(&link);
                        }
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::NotFound
                                    | std::io::ErrorKind::InvalidInput
                                    | std::io::ErrorKind::NotADirectory
                            ) => {}
                        Err(e) => return Err(error(format!("resolución de cuenta: {}", e.kind()))),
                    }
                }
                Component::Prefix(_) => return Err(error("ruta de cuenta inválida")),
            }
        }
        Ok(())
    }
    if !cwd.is_absolute() {
        return Err(error("cwd debe ser absoluto"));
    }
    let mut resolved = if path.is_absolute() {
        PathBuf::from("/")
    } else {
        cwd.into()
    };
    walk(path, &mut resolved, &mut HashSet::new())?;
    Ok(resolved)
}
pub fn account_home(
    registry: &Value,
    provider: &str,
    alias: &Value,
    paths: &Paths,
) -> Result<PathBuf> {
    let spec = spec(registry, provider)?;
    let alias = validate_alias(alias)?;
    if alias == "main" {
        return resolve(&expand(&spec["defaultHome"], paths)?, &paths.cwd);
    }
    let root = resolve(&expand(&spec["accountsRoot"], paths)?, &paths.cwd)?;
    let candidate = root.join(alias);
    let resolved = resolve(&candidate, &paths.cwd)?;
    if !resolved.starts_with(&root) {
        return Err(error("cuenta fuera de accountsRoot"));
    }
    if fs::symlink_metadata(candidate).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(error("aliases de cuenta no pueden ser symlinks"));
    }
    Ok(resolved)
}
pub fn account_environment(
    registry: &Value,
    provider: &str,
    alias: &Value,
    paths: &Paths,
) -> Result<Value> {
    let spec = spec(registry, provider)?;
    if validate_alias(alias)? == "main" {
        return Ok(json!({}));
    }
    // Native homes retain raw Unix bytes; JSON launch environments require
    // Unicode. Reject this boundary rather than changing the selected path.
    let home = account_home(registry, provider, alias, paths)?;
    let home = home.to_str().ok_or_else(|| {
        AccountError(
            "ruta de cuenta no es UTF-8".to_owned(),
            ErrorKind::NonUtf8Home,
        )
    })?;
    let mut out = serde_json::Map::new();
    out.insert(
        python_string(&spec["accountEnv"]),
        Value::String(home.into()),
    );
    Ok(Value::Object(out))
}
fn read_json(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| workspace_loads(&s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}
fn jwt_email(token: &Value) -> String {
    let Some(token) = token
        .as_str()
        .filter(|s| s.bytes().filter(|b| *b == b'.').count() >= 2)
    else {
        return String::new();
    };
    let part = token.split('.').nth(1).unwrap_or("");
    if !part.is_ascii() {
        return String::new();
    }
    // Python ignores nonalphabet bytes and stray padding; terminal padding
    // ends decoding even if more bytes follow. Keep padding calculated from
    // the original segment length, then canonicalize that permissive envelope.
    let mut encoded = Vec::with_capacity(part.len() + 3);
    let mut padding = 0;
    let input = part
        .bytes()
        .chain(std::iter::repeat_n(b'=', (4 - part.len() % 4) % 4));
    let mut terminated = false;
    for byte in input {
        let byte = match byte {
            b'-' => b'+',
            b'_' => b'/',
            b => b,
        };
        if byte.is_ascii_alphanumeric() || b"+/".contains(&byte) {
            encoded.push(byte);
            padding = 0;
        } else if byte == b'=' && encoded.len() % 4 >= 2 {
            padding += 1;
            let required = 4 - encoded.len() % 4;
            if padding >= required {
                encoded.extend(std::iter::repeat_n(b'=', required));
                terminated = true;
                break;
            }
        }
    }
    if !terminated && !encoded.len().is_multiple_of(4) {
        return String::new();
    }
    let engine = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
    );
    let Ok(raw) = engine.decode(encoded) else {
        return String::new();
    };
    let Some(data) = super::model_catalog::byte_json(&raw).filter(Value::is_object) else {
        return String::new();
    };
    if truthy(&data["email"]) {
        python_string(&data["email"])
    } else {
        String::new()
    }
}
fn auth(provider: &str, home: &Path, auth_file: &str) -> (bool, String, &'static str) {
    let data = read_json(&home.join(auth_file));
    let mut identity = String::new();
    let mut subscription = false;
    match provider {
        "claude" => {
            let oauth = &data["claudeAiOauth"];
            subscription = truthy(&oauth["accessToken"]) || truthy(&oauth["refreshToken"]);
            let mut metadata = read_json(&home.join(".claude.json"));
            if !truthy(&metadata)
                && let Some(parent) = home.parent()
            {
                metadata = read_json(&parent.join(".claude.json"));
            }
            let email = &metadata["oauthAccount"]["emailAddress"];
            if truthy(email) {
                identity = python_string(email);
            }
        }
        "codex" => {
            let tokens = &data["tokens"];
            subscription = truthy(&tokens["access_token"]) || truthy(&tokens["refresh_token"]);
            identity = jwt_email(&tokens["id_token"]);
        }
        "grok" => {
            if let Some(record) = data
                .as_object()
                .and_then(|o| o.values().find(|v| v.is_object()))
            {
                subscription = truthy(&record["refresh_token"]);
                if let Some(value) = ["email", "first_name", "user_id", "principal_id"]
                    .iter()
                    .map(|k| &record[*k])
                    .find(|v| truthy(v))
                {
                    identity = python_string(value);
                }
            }
        }
        _ => {}
    }
    if subscription {
        (true, identity, "ready")
    } else if provider == "claude" && claude_api_key(home) {
        if identity.is_empty() {
            identity = "API key".to_owned();
        }
        (true, identity, "ready")
    } else if truthy(&data) {
        (false, identity, "unsupported_auth")
    } else {
        (false, String::new(), "login_required")
    }
}
/// Una cuenta Claude por API key (consola de Anthropic) no tiene OAuth: su
/// `settings.json` declara `apiKeyHelper` o `env.ANTHROPIC_API_KEY`, y el CLI la usa
/// tal cual al lanzarse con `CLAUDE_CONFIG_DIR` apuntando a ella.
fn claude_api_key(home: &Path) -> bool {
    let settings = read_json(&home.join("settings.json"));
    truthy(&settings["apiKeyHelper"]) || truthy(&settings["env"]["ANTHROPIC_API_KEY"])
}
pub fn list_accounts(registry: &Value, provider: &str, paths: &Paths) -> Result<Vec<Value>> {
    let spec = spec(registry, provider)?;
    let mut aliases = vec!["main".to_owned()];
    let expanded = expand(&spec["accountsRoot"], paths)?;
    let directory = if expanded.is_absolute() {
        expanded
    } else {
        paths.cwd.join(expanded)
    };
    if let Ok(entries) = fs::read_dir(directory) {
        let mut named = Vec::new();
        let mut failed = false;
        for item in entries {
            let Ok(item) = item else {
                failed = true;
                break;
            };
            let name = item.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if item
                .file_type()
                .is_ok_and(|t| t.is_dir() && !t.is_symlink())
                && alias_valid(name)
                && !name.ends_with(".lock")
            {
                named.push(name.to_owned());
            }
        }
        if !failed {
            named.sort();
            aliases.extend(named);
        }
    }
    let mut out = Vec::new();
    let auth_file = python_string(&spec["authFile"]);
    for alias in aliases {
        let home = match account_home(registry, provider, &json!(alias), paths) {
            Ok(h) => h,
            Err(e) if account_error(&e) => continue,
            Err(e) => return Err(e),
        };
        let (authenticated, identity, state) = auth(provider, &home, &auth_file);
        out.push(json!({"provider":provider,"alias":alias,"identity":identity,"authenticated":authenticated,"state":state,"selectable":authenticated}));
    }
    Ok(out)
}
pub fn public_accounts(registry: &Value, paths: &Paths) -> Result<Value> {
    let mut out = serde_json::Map::new();
    let harnesses = &registry["harnesses"];
    if !truthy(harnesses) {
        return Ok(Value::Object(out));
    }
    for (provider, spec) in object(harnesses, "harnesses")? {
        let spec = object(spec, "provider")?;
        let caps = spec.get("capabilities").unwrap_or(&Value::Null);
        if truthy(caps) {
            object(caps, "capabilities")?;
        }
        if truthy(&caps["accounts"]) {
            let accounts = match list_accounts(registry, provider, paths) {
                Ok(a) => a,
                Err(e) if account_error(&e) => vec![],
                Err(e) => return Err(e),
            };
            out.insert(provider.clone(), Value::Array(accounts));
        }
    }
    Ok(Value::Object(out))
}
fn sequence(value: &Value) -> Result<&[Value]> {
    if !truthy(value) {
        return Ok(&[]);
    }
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| error("se esperaba una lista"))
}
pub fn account_menu(accounts: &Value, limits: &Value, provider: &str) -> Result<Vec<Value>> {
    const ORDER: [&str; 4] = ["session", "weekly_all", "weekly_scoped", "window"];
    let mut out = Vec::new();
    for account in sequence(accounts)? {
        object(account, "cuenta")?;
        let alias = if truthy(&account["alias"]) {
            python_string(&account["alias"])
        } else {
            String::new()
        };
        if !alias_valid(&alias) {
            continue;
        }
        let mut mine = Vec::new();
        for limit in sequence(limits)? {
            object(limit, "límite")?;
            if limit["provider"] == provider && limit["account"] == alias {
                mine.push(limit);
            }
        }
        mine.sort_by_key(|l| {
            ORDER
                .iter()
                .position(|k| l["kind"] == *k)
                .unwrap_or(ORDER.len())
        });
        let mut rows = Vec::new();
        for limit in mine {
            if limit["kind"].is_array() || limit["kind"].is_object() {
                return Err(error("tipo de límite no hashable"));
            }
            let label: String = match limit["kind"].as_str() {
                Some("session") => "5 h".into(),
                Some("weekly_all" | "window") => "semana".into(),
                _ => {
                    let text = if truthy(&limit["label"]) {
                        python_string(&limit["label"])
                    } else {
                        String::new()
                    };
                    let text = text.replace("Semana ", "");
                    let text = text.trim_matches(|c: char| {
                        c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}')
                    });
                    if text.is_empty() {
                        "límite".into()
                    } else {
                        text.into()
                    }
                }
            };
            let value = &limit["percent"];
            if value
                .as_str()
                .is_some_and(|s| s.chars().any(|c| matches!(c, '\u{1c}'..='\u{1f}')))
            {
                continue;
            }
            let percent = if !truthy(value) {
                0.0
            } else {
                match focus::float(value) {
                    Ok(n) => n,
                    Err(_)
                        if value.as_number().is_some_and(|n| {
                            !n.as_str().contains(['.', 'e', 'E'])
                                && n.as_str().parse::<f64>().is_ok_and(|n| !n.is_finite())
                        }) =>
                    {
                        return Err(error("int too large to convert to float"));
                    }
                    Err(_) => continue,
                }
            };
            if percent.is_nan() {
                continue;
            }
            if !percent.is_finite() {
                return Err(error("cannot convert float infinity to integer"));
            }
            rows.push(json!({"label":label,"percent":percent.round_ties_even().clamp(0.0,100.0) as u64,"resetsAt":limit["resets_at"]}));
        }
        out.push(json!({"alias":alias,"identity":if truthy(&account["identity"]){python_string(&account["identity"])}else{String::new()},"selectable":truthy(&account["selectable"]),"limits":rows}));
    }
    Ok(out)
}
/// Untrimmed Python str/repr. The existing core single-character repr supplies
/// its pinned Unicode printability policy without copying a large range table.
fn python_string(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(s) => s.clone(),
        Value::Number(_) => match workspace_dumps(value)
            .unwrap_or_else(|_| value.to_string())
            .as_str()
        {
            "NaN" => "nan".into(),
            "Infinity" => "inf".into(),
            "-Infinity" => "-inf".into(),
            s => s.into(),
        },
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(python_repr).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(o) => format!(
            "{{{}}}",
            o.iter()
                .map(|(k, v)| format!("{}: {}", python_repr(&json!(k)), python_repr(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
fn python_repr(value: &Value) -> String {
    let Some(text) = value.as_str() else {
        return python_string(value);
    };
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for c in text.chars() {
        if c == quote {
            out.push('\\');
            out.push(c);
        } else if c == '\'' || c == '"' {
            out.push(c);
        } else {
            let wrapped = comandos_core::pomodoro::text(&json!([c.to_string()]));
            out.push_str(&wrapped[2..wrapped.len() - 2]);
        }
    }
    out.push(quote);
    out
}
