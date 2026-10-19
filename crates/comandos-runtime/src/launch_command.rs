//! Piezas del lanzamiento de un CLI con su cuenta (`bin/cc-dash`, árbol vivo
//! D8): el comando completo de un cambio de configuración
//! (`_configuration_command`), el id de modelo que se pasa al CLI
//! (`_launch_model_id`), la comparación de modelos (`_same_model`) y la
//! herencia de la aceptación de carpeta entre cuentas
//! (`inherit_trust_for_switch`, Claude y Codex).
//!
//! Todo es síncrono y sin tmux: lo llaman el adaptador de operaciones (en su
//! hilo) y POST `/session-new` (en `spawn_blocking`).
use crate::accounts::{self, ErrorKind, Paths};
use crate::agent_procs::{dirname, realpath};
use crate::{Unsure, claude_trust, extension_launch, providers};
use comandos_core::json::{response_dumps, truthy};
use comandos_core::text::{shlex_quote, strip};
use serde_json::{Map, Value, json};
use std::{
    ffi::OsString,
    fs, io,
    io::Write,
    net::{SocketAddr, TcpStream},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

/// Lo que el Python lee del proceso y del registro al construir un comando.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// `load_provider_registry()`.
    pub registry: Value,
    /// `os.path.expanduser("~")`.
    pub home: PathBuf,
    /// Directorio de trabajo del proceso (`Path.resolve()` de rutas relativas).
    pub cwd: PathBuf,
    /// `PATH` del proceso (`None` = ausente).
    pub search_path: Option<OsString>,
    /// `int(load_proxy_cfg().get("port") or 18765)`.
    pub proxy_port: u16,
    /// `REPO_ROOT` (resuelto).
    pub repo_root: PathBuf,
}

/// Lo que `_configuration_command` lanza.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// `ValueError` (o `AccountError`) con su texto: el cliente lo muestra.
    Value(String),
    /// Otra excepción o algo que el port no reproduce: quien llama declina.
    Unsure,
}

impl From<Unsure> for ConfigError {
    fn from(_: Unsure) -> Self {
        Self::Unsure
    }
}

/// Las variables que todo lanzamiento quita antes de poner las suyas.
const CLEAR: [&str; 9] = [
    "CLAUDE_CONFIG_DIR",
    "CODEX_HOME",
    "GROK_HOME",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
];

/// `motor_lock_env(motor, model)` (1834): fija cada ranura de Claude Code al
/// modelo del motor de suscripción.
pub fn motor_lock_env(motor: &str, model: &str) -> Vec<(&'static str, String)> {
    if !matches!(motor, "codex" | "grok") || model.is_empty() {
        return Vec::new();
    }
    [
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "CLAUDE_CODE_SUBAGENT_MODEL",
    ]
    .into_iter()
    .map(|key| (key, model.to_owned()))
    .collect()
}

/// `shlex.join(words)`.
pub fn shlex_join<S: AsRef<str>>(words: &[S]) -> String {
    words
        .iter()
        .map(|w| shlex_quote(w.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `proxy_alive()` (3815): conexión TCP a `127.0.0.1:<puerto>` con plazo de
/// 300 ms. Bloquea.
pub fn proxy_alive(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// `(d.get(key) or {})` de un `dict`: un valor verdadero que no es objeto
/// lanzaría `AttributeError` en el `.get` siguiente.
fn sub<'a>(value: &'a Value, key: &str) -> Result<Option<&'a Map<String, Value>>, Unsure> {
    let Value::Object(map) = value else {
        return Err(Unsure);
    };
    match map.get(key) {
        Some(v) if truthy(v) => v.as_object().map(Some).ok_or(Unsure),
        _ => Ok(None),
    }
}

/// `provider_registry.which(name)` como texto.
fn which(ctx: &Ctx, name: &str) -> Result<Option<String>, Unsure> {
    providers::which(name, ctx.search_path.as_deref(), &ctx.home)
        .map(|p| p.into_os_string().into_string().map_err(|_| Unsure))
        .transpose()
}

/// `_harness_bin(name)` (1972): `harnesses.<name>.binary` (o el nombre) por
/// `which`; si no aparece, el nombre tal cual.
pub fn harness_bin(ctx: &Ctx, name: &str) -> Result<String, Unsure> {
    let spec = sub(&ctx.registry, "harnesses")?.and_then(|h| h.get(name));
    let spec = match spec {
        Some(v) if truthy(v) => v.as_object().ok_or(Unsure)?.get("binary"),
        _ => None,
    };
    let binary = match spec {
        Some(v) if truthy(v) => v.as_str().ok_or(Unsure)?.to_owned(),
        _ => name.to_owned(),
    };
    Ok(which(ctx, &binary)?.unwrap_or(binary))
}

/// Un `AccountError` del registro es un `ValueError` con su texto; el resto
/// (rutas que no son UTF-8, lo que el port no reproduce) declina.
fn account_failure(e: &accounts::AccountError) -> ConfigError {
    if accounts::is_account_error(e) {
        ConfigError::Value(e.0.clone())
    } else {
        ConfigError::Unsure
    }
}

/// `_native_launch_model`: descarta modelos ajenos y ajusta el esfuerzo
/// al catálogo del CLI. No consulta credenciales ni lanza procesos.
pub fn native_launch_model(
    registry: &Value,
    harness: &str,
    model: &str,
    effort: &str,
) -> Result<(String, String), Unsure> {
    let motor = sub(registry, "motors")?.and_then(|m| m.get(harness));
    let native = sub(registry, "harnesses")?.and_then(|m| m.get(harness));
    let mut models = Vec::new();
    // `models` es lista, mientras los contenedores superiores son objetos.
    for spec in [motor, native].into_iter().flatten() {
        if let Some(raw) = spec.get("models").filter(|v| truthy(v)) {
            for row in raw.as_array().ok_or(Unsure)? {
                if row.is_object() && row.get("id").is_some_and(truthy) {
                    models.push(row);
                }
            }
        }
    }
    let found = models.iter().find(|row| row["id"].as_str() == Some(model));
    let pattern = motor
        .and_then(|m| m.get("modelMatch"))
        .filter(|v| truthy(v));
    let fits = !model.is_empty()
        && (found.is_some()
            || match pattern {
                Some(p) => providers::py_search(
                    &providers::py_regex(p.as_str().ok_or(Unsure)?, true)?,
                    model,
                )?,
                None => false,
            });
    if !fits {
        return Ok((String::new(), String::new()));
    }
    let mut effort = effort.to_owned();
    if let Some(spec) = found {
        let allowed = spec.get("efforts").filter(|v| truthy(v));
        let allowed = match allowed {
            Some(v) => v.as_array().ok_or(Unsure)?.as_slice(),
            None => &[],
        };
        if !effort.is_empty() && !allowed.iter().any(|v| v.as_str() == Some(&effort)) {
            effort = if allowed.is_empty() {
                String::new()
            } else {
                spec.get("defaultEffort")
                    .filter(|v| truthy(v))
                    .map(|v| v.as_str().map(str::to_owned).ok_or(Unsure))
                    .transpose()?
                    .unwrap_or_default()
            };
        }
    }
    Ok((model.to_owned(), effort))
}

/// `_acp_launch_cmd` para nuevas sesiones; la cuenta pertenece al motor.
pub fn acp_launch_command(
    registry: &Value,
    motor: &str,
    model: &str,
    effort: &str,
    account: &str,
    danger: bool,
) -> Result<String, Unsure> {
    let motor = if motor.is_empty() { "claude" } else { motor };
    let (model, effort) = native_launch_model(registry, motor, model, effort)?;
    let mut command = format!("cc-acp --agent {}", shlex_quote(motor));
    for (flag, value) in [("--model", model.as_str()), ("--effort", effort.as_str())] {
        if !value.is_empty() {
            command += &format!(" {flag} {}", shlex_quote(value));
        }
    }
    if !account.is_empty() && account != "main" {
        command += &format!(" --account {}", shlex_quote(account));
    }
    if danger {
        command += " --danger";
    }
    Ok(command)
}

/// Rama OpenCode/AGY de `_harness_launch_cmd`. Sus comandos no anteponen
/// entorno de cuenta; resuelven el binario igual que el resto del runtime.
pub fn native_harness_command(
    ctx: &Ctx,
    harness: &str,
    model: &str,
    effort: &str,
) -> Result<String, Unsure> {
    if !matches!(harness, "opencode" | "agy") {
        return Err(Unsure);
    }
    let (model, effort) = native_launch_model(&ctx.registry, harness, model, effort)?;
    let mut command = shlex_quote(&harness_bin(ctx, harness)?);
    if !model.is_empty() {
        command += &format!(" --model {}", shlex_quote(&model));
    }
    if harness == "agy" && !effort.is_empty() {
        command += &format!(" --effort {}", shlex_quote(&effort));
    }
    Ok(command)
}

/// `_configuration_command(harness, motor, model, effort, account, resume,
/// flags, preserve_model_flags=…)` (2308): un único `env -u … <asignaciones>
/// <binario> …` con todo el borrador, por `shlex.join`. `main` nunca lleva
/// `CLAUDE_CONFIG_DIR` (lo decide `account_environment`).
#[allow(clippy::too_many_arguments)]
pub fn configuration_command(
    ctx: &Ctx,
    harness: &str,
    motor: &str,
    model: &str,
    effort: &str,
    account: &str,
    resume: &str,
    flags: &[String],
    preserve_model_flags: bool,
) -> Result<String, ConfigError> {
    let binary = harness_bin(ctx, harness)?;
    if which(ctx, &binary)?.is_none() {
        return Err(ConfigError::Value(format!(
            "no encuentro el ejecutable {binary}"
        )));
    }
    let provider = if harness == "acp" { motor } else { harness };
    let spec = sub(&ctx.registry, "harnesses")?.and_then(|h| h.get(provider));
    let caps = match spec {
        Some(v) if truthy(v) => sub(v, "capabilities")?,
        _ => None,
    };
    let accounts_enabled = caps.and_then(|c| c.get("accounts")).is_some_and(truthy);
    let mut environment = Map::new();
    if accounts_enabled {
        let paths = Paths::new(&ctx.home, &ctx.cwd);
        match accounts::account_environment(&ctx.registry, provider, &json!(account), &paths) {
            Ok(Value::Object(map)) => environment = map,
            Ok(_) => return Err(ConfigError::Unsure),
            Err(e) => return Err(account_failure(&e)),
        }
    }
    if harness == "claude" && motor != "claude" {
        if !proxy_alive(ctx.proxy_port) {
            return Err(ConfigError::Value("el gateway local no responde".into()));
        }
        environment.insert(
            "ANTHROPIC_BASE_URL".into(),
            json!(format!("http://127.0.0.1:{}", ctx.proxy_port)),
        );
        for (key, value) in motor_lock_env(motor, model) {
            environment.insert(key.into(), json!(value));
        }
    }
    let mut argv: Vec<String> = vec!["env".into()];
    for name in CLEAR {
        argv.extend(["-u".into(), name.into()]);
    }
    for (key, value) in &environment {
        argv.push(format!("{key}={}", value.as_str().ok_or(Unsure)?));
    }
    argv.push(binary);
    if harness == "acp" {
        argv.extend(["--agent".into(), motor.into()]);
        if account != "main" {
            argv.extend(["--account".into(), account.into()]);
        }
    }
    if !resume.is_empty() {
        let flag = match harness {
            "codex" => "resume",
            "opencode" => "--session",
            "agy" => "--conversation",
            _ => "--resume",
        };
        argv.extend([flag.into(), resume.into()]);
    }
    if !model.is_empty() {
        let flag = if harness == "codex" { "-m" } else { "--model" };
        argv.extend([flag.into(), model.into()]);
    }
    if !effort.is_empty() {
        if harness == "codex" {
            argv.extend(["-c".into(), format!("model_reasoning_effort=\"{effort}\"")]);
        } else {
            argv.extend(["--effort".into(), effort.into()]);
        }
    }
    argv.extend(kept_flags(flags, preserve_model_flags));
    Ok(shlex_join(&argv))
}

/// `split('=', 1)[0]` de Python.
fn before_eq(s: &str) -> &str {
    s.split_once('=').map_or(s, |(head, _)| head)
}

/// El filtro de `flags` de `_configuration_command`: los permisos explícitos se
/// conservan; modelo y esfuerzo solo con `preserve_model_flags`.
fn kept_flags(flags: &[String], preserve: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(flag) = flags.get(i) {
        if !preserve && matches!(flag.as_str(), "--model" | "-m" | "--effort") {
            i += 2;
            continue;
        }
        if !preserve && matches!(before_eq(flag), "--model" | "--effort") {
            i += 1;
            continue;
        }
        if matches!(flag.as_str(), "-c" | "--config")
            && let Some(next) = flags.get(i + 1)
        {
            if !preserve && matches!(strip(before_eq(next)), "model" | "model_reasoning_effort") {
                i += 2;
                continue;
            }
            out.extend([flag.clone(), next.clone()]);
            i += 2;
            continue;
        }
        if !preserve
            && let Some(rest) = flag.strip_prefix("--config=")
            // `flag.split('=', 2)[1]`: entre el primer y el segundo `=`.
            && matches!(strip(before_eq(rest)), "model" | "model_reasoning_effort")
        {
            i += 1;
            continue;
        }
        out.push(flag.clone());
        i += 1;
    }
    out
}

/// `spec['id']` de un modelo del registro.
fn spec_id(spec: &Value) -> Result<String, Unsure> {
    spec.get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(Unsure)
}

/// `_launch_model_id(motor, model)` (2833): el id canónico del registro si lo
/// conoce; si no (CLI más nuevo que `providers.json`), el del CLI, con
/// `claude-` delante si venía abreviado (`opus-5-5`).
pub fn launch_model_id(registry: &Value, motor: &str, model: &str) -> Result<String, Unsure> {
    if model.is_empty() {
        return Ok(String::new());
    }
    if let Some(spec) = providers::model_spec(registry, motor, model, "motors")? {
        return spec_id(&spec);
    }
    if motor == "claude" && abbreviated_claude(model)? {
        return Ok(format!("claude-{model}"));
    }
    Ok(model.to_owned())
}

/// `re.match(r'(?i)^(opus|sonnet|haiku|fable)\b', model)` sobre texto ASCII
/// (con `ſ`, `K` o un carácter no ASCII tras el prefijo, el `re` Unicode
/// podría decidir otra cosa).
fn abbreviated_claude(model: &str) -> Result<bool, Unsure> {
    if !model.is_ascii() {
        return Err(Unsure);
    }
    let lower = model.to_ascii_lowercase();
    Ok(["opus", "sonnet", "haiku", "fable"].iter().any(|prefix| {
        lower.strip_prefix(prefix).is_some_and(|rest| {
            rest.chars()
                .next()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
        })
    }))
}

/// `_same_model(motor, expected, observed)` (2848): mismo modelo según el
/// registro si conoce ambos; si no, por la clave normalizada.
pub fn same_model(
    registry: &Value,
    motor: &str,
    expected: &str,
    observed: &str,
) -> Result<bool, Unsure> {
    if expected.is_empty() {
        return Ok(true);
    }
    if observed.is_empty() {
        return Ok(false);
    }
    if observed == expected {
        return Ok(true);
    }
    let wanted = providers::model_spec(registry, motor, expected, "motors")?;
    let actual = providers::model_spec(registry, motor, observed, "motors")?;
    if let (Some(wanted), Some(actual)) = (wanted, actual) {
        // `wanted['id'] == actual['id']`.
        return Ok(comandos_core::json::python_eq(
            wanted.get("id").ok_or(Unsure)?,
            actual.get("id").ok_or(Unsure)?,
        ));
    }
    Ok(providers::model_key(expected)? == providers::model_key(observed)?)
}

/// `_claude_config_dir(alias)` (2390): `None` para `main` (vive en el HOME).
fn claude_config_dir(registry: &Value, alias: &str, home: &str) -> Result<Option<String>, Unsure> {
    if alias.is_empty() || alias == "main" {
        return Ok(None);
    }
    // `(registry.get("harnesses") or {}).get("claude") or {}`: el registro ya
    // está validado; otra forma lanzaría dentro del `try` del llamador.
    let spec = &registry["harnesses"]["claude"];
    let root = &spec["accountsRoot"];
    let root = if truthy(root) {
        root.as_str().ok_or(Unsure)?
    } else {
        "~/.claude-accounts"
    };
    let root = claude_trust::expanduser(root, home)?;
    // `os.path.join(root, alias)`: un alias absoluto descarta la raíz (el
    // `from_alias` de `apply` sale del proceso observado, no se valida).
    if alias.starts_with('/') {
        return Ok(Some(alias.to_owned()));
    }
    Ok(Some(if root.is_empty() || root.ends_with('/') {
        format!("{root}{alias}")
    } else {
        format!("{root}/{alias}")
    }))
}

fn or_main(alias: &str) -> &str {
    if alias.is_empty() { "main" } else { alias }
}

/// `inherit_trust_for_switch(cwd, from_alias, to_alias, harness)` (2438): copia
/// la aceptación de la carpeta de la cuenta origen a la destino, solo si la
/// cuenta cambia y el origen ya la tenía. `Ok(false)` también para toda
/// excepción del Python (la traga). `home` es `os.path.expanduser("~")`.
///
/// Antes del `claim`, quien llama comprueba `trust_probe` (Claude) o
/// `codex_trust_probe` (Codex) y declina con `Unsure`. Después de los efectos
/// (en `apply`, con el agente original ya cerrado), un `Unsure` se trata como
/// `false`: lo mismo que la excepción que el Python traga, nunca `Decline`.
///
/// El `cc_usage.record_change(... "trust_inherited" ...)` que sigue a un
/// acierto lo escribe quien llama (la base de uso es del frente): su nota es
/// `trust_note(cwd, from_alias, to_alias)`.
pub fn inherit_trust_for_switch(
    registry: &Value,
    home: &str,
    cwd: &str,
    from_alias: &str,
    to_alias: &str,
    harness: &str,
) -> Result<bool, Unsure> {
    if !matches!(harness, "claude" | "codex")
        || cwd.is_empty()
        || or_main(to_alias) == or_main(from_alias)
    {
        return Ok(false);
    }
    if harness == "codex" {
        return inherit_codex_trust(registry, home, cwd, from_alias, to_alias);
    }
    let Some(dest) = claude_config_dir(registry, to_alias, home)? else {
        return Ok(false);
    };
    let source = claude_config_dir(registry, from_alias, home)?;
    Ok(claude_trust::inherit_cwd_trust(cwd, source.as_deref(), &dest, home)? == Some(true))
}

/// `account_home(registry, 'codex', alias) / 'config.toml'`. Una excepción del
/// registro es el `except` de `inherit_trust_for_switch` (`None`).
fn codex_config(registry: &Value, home: &str, alias: &str) -> Result<Option<PathBuf>, Unsure> {
    // `Path(os.path.expanduser(x)).resolve()`: con `~`, `~/…` o una ruta
    // absoluta no depende del directorio del proceso; con una relativa o
    // `~usuario`, sí (o de la base de usuarios): `Unsure`.
    if let Some(spec) = registry["harnesses"]["codex"].as_object() {
        for key in ["defaultHome", "accountsRoot"] {
            if let Some(raw) = spec.get(key).and_then(Value::as_str)
                && !(raw == "~" || raw.starts_with("~/") || raw.starts_with('/'))
            {
                return Err(Unsure);
            }
        }
    }
    let paths = Paths::new(Path::new(home), Path::new("/"));
    match accounts::account_home(registry, "codex", &json!(or_main(alias)), &paths) {
        Ok(dir) => Ok(Some(dir.join("config.toml"))),
        Err(e) if e.kind() == ErrorKind::Account => Ok(None),
        Err(_) => Err(Unsure),
    }
}

/// `decision(data)` de `_inherit_codex_trust`: el primer `trust_level`
/// verdadero subiendo de `cwd` a `/`. `None` = la excepción (`projects` o una
/// entrada que no es tabla).
fn codex_decision(data: &Value, cwd: &[u8]) -> Result<Option<Value>, Unsure> {
    let projects = match data.get("projects") {
        Some(v) if truthy(v) => match v.as_object() {
            Some(map) => map.clone(),
            None => return Ok(None),
        },
        _ => Map::new(),
    };
    let mut folder = cwd.to_vec();
    loop {
        let key = String::from_utf8(folder.clone()).map_err(|_| Unsure)?;
        match projects.get(&key) {
            Some(entry) if truthy(entry) => {
                let Some(entry) = entry.as_object() else {
                    return Ok(None);
                };
                if let Some(level) = entry.get("trust_level").filter(|v| truthy(v)) {
                    return Ok(Some(level.clone()));
                }
            }
            _ => {}
        }
        let parent = dirname(&folder);
        if parent == folder {
            return Ok(Some(json!("")));
        }
        folder = parent;
    }
}

/// `_inherit_codex_trust(cwd, from_alias, to_alias)` (2398, D8) dentro del
/// `try` de `inherit_trust_for_switch`: cualquier excepción es `false`.
///
/// Desviación deliberada (como la de `claude_trust`): el temporal nace con el
/// modo del `config.toml` destino si ya existía; el Python lo dejaba siempre
/// con el 0600 de `mkstemp`. Uno nuevo nace 0600 como en el Python.
fn inherit_codex_trust(
    registry: &Value,
    home: &str,
    cwd: &str,
    from_alias: &str,
    to_alias: &str,
) -> Result<bool, Unsure> {
    let (Some(source), Some(target)) = (
        codex_config(registry, home, from_alias)?,
        codex_config(registry, home, to_alias)?,
    ) else {
        return Ok(false);
    };
    let link = |p: &Path| fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink());
    if link(&source) || link(&target) {
        return Ok(false);
    }
    let cwd = realpath(cwd.as_bytes());
    let Some(data) = extension_launch::read_trust_toml(&source)? else {
        return Ok(false);
    };
    if codex_decision(&data, &cwd)? != Some(json!("trusted")) {
        return Ok(false);
    }
    codex_stamp(&target, &cwd)
}

/// Marca `cwd` como `trusted` en el `config.toml` de un `CODEX_HOME` para que
/// `codex resume` no se detenga en la pregunta de confianza. Lo usa la
/// restauración: la conversación ya corrió en esa carpeta con esa cuenta.
/// `Ok(true)` si la carpeta queda confiable (ya lo estaba o se marcó).
pub fn ensure_codex_cwd_trusted(codex_home: &Path, cwd: &str) -> Result<bool, Unsure> {
    if !cwd.starts_with('/') || !Path::new(cwd).is_dir() {
        return Ok(false);
    }
    let target = codex_home.join("config.toml");
    if fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
        return Ok(false);
    }
    let cwd = realpath(cwd.as_bytes());
    // Una decisión explícita (también `untrusted`) no se toca:
    // `codex_stamp_locked` solo escribe cuando no hay ninguna.
    if let Some(data) = extension_launch::read_trust_toml(&target)?
        && codex_decision(&data, &cwd)? == Some(json!("trusted"))
    {
        return Ok(true);
    }
    codex_stamp(&target, &cwd)
}

/// Candado `<target>.lock` y `codex_stamp_locked`.
fn codex_stamp(target: &Path, cwd: &[u8]) -> Result<bool, Unsure> {
    let Some(parent) = target.parent() else {
        return Ok(false);
    };
    if fs::create_dir_all(parent).is_err() {
        return Ok(false);
    }
    // `file_lock(str(target))`: `<target>.lock` 0600 con `flock` exclusivo.
    let mut lock_path = target.as_os_str().to_owned();
    lock_path.push(".lock");
    let Ok(lock) = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)
    else {
        return Ok(false);
    };
    if lock.lock().is_err() {
        return Ok(false);
    }
    let result = codex_stamp_locked(target, parent, cwd);
    let _ = lock.unlock();
    result
}

/// Lo que `_inherit_codex_trust` hace con el candado tomado.
fn codex_stamp_locked(target: &Path, parent: &Path, cwd: &[u8]) -> Result<bool, Unsure> {
    let Some(data) = extension_launch::read_trust_toml(target)? else {
        return Ok(false);
    };
    match codex_decision(&data, cwd)? {
        Some(level) if !truthy(&level) => {}
        _ => return Ok(false),
    }
    // `target.read_text() if target.exists() else ''` (saltos universales).
    let before = match fs::read(target) {
        Ok(bytes) => match extension_launch::universal_text(bytes) {
            Some(text) => text,
            None => return Ok(false),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(_) => return Ok(false),
    };
    let cwd = String::from_utf8(cwd.to_vec()).map_err(|_| Unsure)?;
    let quoted = toml_key(&cwd)?;
    let after = format!("{before}\n[projects.{quoted}]\ntrust_level = \"trusted\"\n");
    if extension_launch::parse_trust_toml(&after)?.is_none() {
        return Ok(false);
    }
    let original = fs::metadata(target)
        .ok()
        .map(|meta| meta.permissions().mode() & 0o7777);
    Ok(write_replacing(target, parent, &after, original).is_ok())
}

/// `json.dumps(cwd)` como clave TOML. Desviación deliberada: un carácter
/// fuera del plano básico (un emoji) sale como `\UXXXXXXXX`; `json.dumps` lo
/// partía en sustitutos `\ud83d\ude00`, que TOML rechaza, y la carpeta nunca
/// heredaba la confianza. Sin esos caracteres, el texto es el del Python.
fn toml_key(cwd: &str) -> Result<String, Unsure> {
    if cwd.chars().all(|c| u32::from(c) <= 0xFFFF) {
        return response_dumps(&json!(cwd)).map_err(|_| Unsure);
    }
    let mut out = String::from("\"");
    for c in cwd.chars() {
        if u32::from(c) > 0xFFFF {
            out.push_str(&format!("\\U{:08X}", u32::from(c)));
        } else {
            let one = response_dumps(&json!(c.to_string())).map_err(|_| Unsure)?;
            out.push_str(one.get(1..one.len() - 1).ok_or(Unsure)?);
        }
    }
    out.push('"');
    Ok(out)
}

/// Antes del `claim`: ¿leería la herencia Codex algo que el port no reproduce?
/// Lee (sin escribir) los `config.toml` de origen y destino con el mismo
/// lector y comprueba la carpeta. Sin herencia posible no lee nada.
pub fn codex_trust_probe(
    registry: &Value,
    home: &str,
    cwd: &str,
    from_alias: &str,
    to_alias: &str,
) -> Result<(), Unsure> {
    if cwd.is_empty() || or_main(to_alias) == or_main(from_alias) {
        return Ok(());
    }
    let (Some(source), Some(target)) = (
        codex_config(registry, home, from_alias)?,
        codex_config(registry, home, to_alias)?,
    ) else {
        return Ok(());
    };
    // Como la herencia: un `config.toml` enlazado no hereda, no se lee.
    let link = |p: &Path| fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink());
    if link(&source) || link(&target) {
        return Ok(());
    }
    let cwd = String::from_utf8(realpath(cwd.as_bytes())).map_err(|_| Unsure)?;
    toml_key(&cwd)?;
    extension_launch::read_trust_toml(&source)?;
    extension_launch::read_trust_toml(&target)?;
    Ok(())
}

/// `mkstemp(prefix='.trust-', dir=…)` + escritura + `fsync` + `os.replace`; el
/// temporal se borra si algo falla.
fn write_replacing(
    target: &Path,
    parent: &Path,
    text: &str,
    original: Option<u32>,
) -> io::Result<()> {
    let name = crate::fresh_id(".trust").map_err(|e| io::Error::other(e.to_string()))?;
    let temporary = parent.join(name);
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(original.unwrap_or(0o600))
            .open(&temporary)?;
        if let Some(mode) = original {
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        file.write_all(text.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        fs::rename(&temporary, target)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// La nota del registro de cambios tras heredar la confianza.
pub fn trust_note(cwd: &str, from_alias: &str, to_alias: &str) -> String {
    format!(
        "trust heredado {} -> {} en {cwd}",
        or_main(from_alias),
        or_main(to_alias)
    )
}

/// `claude_trust::probe` de lo que leería `inherit_trust_for_switch` (Claude,
/// de `from_alias` a `to_alias`): `Unsure` si algún archivo no se reproduce.
/// Sin herencia posible (misma cuenta, destino `main`) no lee nada.
pub fn trust_probe(
    registry: &Value,
    home: &str,
    from_alias: &str,
    to_alias: &str,
) -> Result<(), Unsure> {
    if or_main(to_alias) == or_main(from_alias) {
        return Ok(());
    }
    let Some(dest) = claude_config_dir(registry, to_alias, home)? else {
        return Ok(());
    };
    let source = claude_config_dir(registry, from_alias, home)?;
    claude_trust::probe(source.as_deref(), &dest, home)
}

#[cfg(test)]
mod tests {
    use super::{abbreviated_claude, kept_flags};

    #[test]
    fn abbreviated_prefix_needs_a_boundary() {
        assert_eq!(abbreviated_claude("opus-5-5"), Ok(true));
        assert_eq!(abbreviated_claude("Sonnet"), Ok(true));
        assert_eq!(abbreviated_claude("opusx"), Ok(false));
        assert_eq!(abbreviated_claude("opus_5"), Ok(false));
        assert!(abbreviated_claude("opusé").is_err());
    }

    #[test]
    fn flags_keep_permissions_only() {
        let flags: Vec<String> = ["--model", "x", "--yolo", "-c", "model = \"y\"", "-c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(kept_flags(&flags, false), vec!["--yolo", "-c"]);
        assert_eq!(kept_flags(&flags, true), flags);
    }
}
