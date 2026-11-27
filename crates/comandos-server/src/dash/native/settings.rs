//! Corte `services` (plan 2f-3, Tarea 1): ajustes del tablero de
//! `bin/cc-dash` (`access_token` 4730, `read_conf` 4819, `ui_lang` 7272,
//! `play_test` 7293, `write_conf_key` 7328, `_fs_safe` 609, `fs_dirs` 620,
//! `fs_mkdir` 658, `_open_url` 34, `desktop_popup` 475). También aloja, en
//! su tarea, `/models/latest` (`SettingsRoute::ModelsLatest`, P2 del preflight).
//!
//! - GET `/webterm-token`, `/conf`, `/fs/dirs` (prefijos, como el Python).
//! - POST `/conf-set`: reescribe `cc-notify.conf` bajo `flock(<conf>.lock)`
//!   (el mismo candado que el Python y cc-app) con `write_text_atomic`.
//! - POST `/fs/mkdir`: carpetas solo dentro de `HOME` (`os.makedirs` portado).
//! - POST `/open-path`, `/open-url`: `xdg-open` (o `wslview`) suelto.
//! - POST `/notify-popup`: con «Popups de escritorio» apagado responde
//!   `popup: false` sin red; encendido espera el resultado de cc-notifyd
//!   en una tarea registrada, con plazo de dos segundos.
//! - POST `/test`: el reproductor o la voz de una notificación real.
//! - GET `/models/latest` (prefijo, 8534): `_read_json_quiet(MODEL_WATCH_FILE)
//!   or {}` tal cual (el snapshot del vigilante de modelos, Tarea 6).
//!
//! Ligereza: cada ruta hace un solo salto al pool de bloqueo (dos en
//! `/conf-set`: la cola del candado de `FileLock::acquire_timeout` y la
//! reescritura); nada se cachea; los hijos sueltos los recoge una tarea del
//! runtime (`procs::spawn_detached`).
use super::{
    Answer, Entry, Fault, Key, Native, NativeOptions, NativeRoute, Verb,
    files::{FileLock, LOCK_WAIT, write_text_atomic},
    light::{data, error, read_reply},
    procs::{gui_env_for, spawn_detached, which_in},
    py::{int, str_scalar, take_chars},
    query::Query,
    reply,
    usage::pane_models::ui_lang_es,
};
use crate::{
    HandlerError, Request,
    dash::token::{TOKEN_FILE, access_token_at},
};
use comandos_core::{
    json::{response_dumps, truthy},
    text::{NumError, shlex_quote, splitlines, strip},
};
use comandos_runtime::{agent_procs::realpath, providers::read_conf};
use http::StatusCode;
use serde_json::{Map, Value, json};
use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::DirBuilderExt,
    },
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsRoute {
    WebtermToken,
    Conf,
    ConfSet,
    FsDirs,
    FsFind,
    FsMkdir,
    OpenPath,
    OpenUrl,
    NotifyPopup,
    Test,
    ModelsLatest,
}

pub const ROUTES: &[Entry] = &[
    Entry {
        verb: Verb::Get,
        key: Key::Prefix("/webterm-token"),
        route: NativeRoute::Settings(SettingsRoute::WebtermToken),
    },
    Entry {
        verb: Verb::Get,
        key: Key::Prefix("/fs/dirs"),
        route: NativeRoute::Settings(SettingsRoute::FsDirs),
    },
    Entry {
        verb: Verb::Get,
        key: Key::Prefix("/fs/find"),
        route: NativeRoute::Settings(SettingsRoute::FsFind),
    },
    Entry {
        verb: Verb::Get,
        key: Key::Prefix("/conf"),
        route: NativeRoute::Settings(SettingsRoute::Conf),
    },
    Entry {
        verb: Verb::Post,
        key: Key::Raw("/conf-set"),
        route: NativeRoute::Settings(SettingsRoute::ConfSet),
    },
    Entry {
        verb: Verb::Post,
        key: Key::Raw("/fs/mkdir"),
        route: NativeRoute::Settings(SettingsRoute::FsMkdir),
    },
    Entry {
        verb: Verb::Post,
        key: Key::Raw("/open-path"),
        route: NativeRoute::Settings(SettingsRoute::OpenPath),
    },
    Entry {
        verb: Verb::Post,
        key: Key::Raw("/open-url"),
        route: NativeRoute::Settings(SettingsRoute::OpenUrl),
    },
    Entry {
        verb: Verb::Post,
        key: Key::Raw("/notify-popup"),
        route: NativeRoute::Settings(SettingsRoute::NotifyPopup),
    },
    Entry {
        verb: Verb::Post,
        key: Key::Raw("/test"),
        route: NativeRoute::Settings(SettingsRoute::Test),
    },
    Entry {
        verb: Verb::Get,
        key: Key::Prefix("/models/latest"),
        route: NativeRoute::Settings(SettingsRoute::ModelsLatest),
    },
];

/// `CONF_KEYS` (7266) en el orden de `sorted(CONF_KEYS)`.
pub const CONF_KEYS: [&str; 10] = [
    "AUTO_WORKTREE",
    "CC_LANG",
    "DESKTOP_NOTIFY",
    "NOTIFY_MODEL_TIER",
    "NOTIFY_ON_ATTENTION",
    "NOTIFY_ON_DONE",
    "SOUND_ENABLED",
    "SPEAK_ATTENTION",
    "SPEAK_DONE",
    "VOLUME",
];

/// `CONF_DEFAULTS.get(k, "1")` (7269).
fn conf_default(key: &str) -> &'static str {
    match key {
        "VOLUME" => "60",
        "CC_LANG" => "auto",
        _ => "1",
    }
}

/// `CONF_PATH` (171).
const CONF_FILE: &str = "cc-notify.conf";
/// `int(str)` de CPython rechaza textos de más de 4300 dígitos (`ValueError`).
const INT_MAX_STR_DIGITS: usize = 4300;

const OUTSIDE_HOME: &str = "fuera de tu carpeta de usuario";
const MKDIR_OUTSIDE: &str = "solo puedo crear carpetas dentro de tu carpeta de usuario";
const MKDIR_FILE: &str = "ya existe un ARCHIVO con ese nombre";
const NO_VOICE: &str = "Sin voz instalada (piper + una voz, o spd-say)";
const TEST_PHRASE: &str = "Asi suena ComandOS";

pub async fn answer(native: &Arc<Native>, route: SettingsRoute, request: &Request) -> Answer {
    let opts = native.options();
    match route {
        SettingsRoute::WebtermToken => webterm_token(opts).await,
        SettingsRoute::Conf => conf(opts).await,
        SettingsRoute::ConfSet => conf_set(opts, data(request)?).await,
        SettingsRoute::FsDirs => fs_dirs_route(opts, request).await,
        SettingsRoute::FsFind => fs_find_route(opts, request).await,
        SettingsRoute::FsMkdir => fs_mkdir_route(opts, data(request)?).await,
        SettingsRoute::OpenPath => open_path(native, data(request)?).await,
        SettingsRoute::OpenUrl => open_url_route(native, data(request)?).await,
        SettingsRoute::NotifyPopup => notify_popup(native, data(request)?).await,
        SettingsRoute::Test => test(native, data(request)?).await,
        SettingsRoute::ModelsLatest => models_latest(opts).await,
    }
}

/// GET `/models/latest`: el snapshot del vigilante, o `{}` si falta, no se
/// lee o es falso. Lo que el port no lee con certeza declina.
async fn models_latest(opts: &NativeOptions) -> Answer {
    let opts = opts.clone();
    let data =
        blocking(move || super::light::load_domain(&opts.home, &opts.hooks, "model-watch.json"))
            .await??;
    let data = data
        .filter(truthy)
        .unwrap_or_else(|| Value::Object(Map::new()));
    read_reply(&data)
}

fn failure() -> Fault {
    Fault::Error(HandlerError::Failure)
}

fn ok() -> Answer {
    reply(StatusCode::OK, &json!({"ok": true}))
}

/// Un trabajo de disco o de procesos en el pool de bloqueo; si revienta, la
/// excepción sin capturar del Python (500). Los hilos del pool tienen contexto
/// de runtime: `spawn_detached` funciona dentro.
async fn blocking<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> Result<T, Fault> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|_| failure())
}

/// `str(value or default)`: un valor falso da `default`; uno verdadero, su
/// `str()` (un flotante, una lista o un objeto no se reproducen: declinar).
fn str_or(value: Option<&Value>, default: &str) -> Result<String, Fault> {
    match value {
        Some(value) if truthy(value) => str_scalar(value).ok_or(Fault::Decline),
        _ => Ok(default.to_owned()),
    }
}

/// `read_conf()` (4819) de `cc-notify.conf`: ilegible con certeza no hay
/// (`Unsure` → declinar; no hubo efectos).
fn conf_pairs(hooks: &Path) -> Result<Vec<(String, String)>, Fault> {
    read_conf(&hooks.join(CONF_FILE)).map_err(|_| Fault::Decline)
}

fn conf_get<'a>(conf: &'a [(String, String)], key: &str) -> Option<&'a str> {
    conf.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// `os.path.expanduser(path)` con el HOME del frente; `None` con `~usuario`
/// (necesitaría `pwd`): quien llama declina.
fn expand_user(path: &str, home: &str) -> Option<String> {
    let Some(rest) = path.strip_prefix('~') else {
        return Some(path.to_owned());
    };
    if !(rest.is_empty() || rest.starts_with('/')) {
        return None;
    }
    let joined = format!("{}{rest}", home.trim_end_matches('/'));
    Some(if joined.is_empty() {
        "/".to_owned()
    } else {
        joined
    })
}

fn home_str(opts: &NativeOptions) -> Result<String, Fault> {
    opts.home.to_str().map(str::to_owned).ok_or(Fault::Decline)
}

// ---------------------------------------------------------------------------
// `/webterm-token`
// ---------------------------------------------------------------------------

/// `access_token()` (4730) sobre `HOOKS/dash-token`: el mismo lector que la
/// puerta de acceso del transporte (`dash::token`). Bloquea: llamar dentro de
/// `spawn_blocking`. `InvalidData` (no UTF-8): quien llama declina; otro error
/// de E/S es la excepción sin capturar del Python.
pub fn access_token(hooks: &Path) -> io::Result<String> {
    access_token_at(&hooks.join(TOKEN_FILE))
}

async fn webterm_token(opts: &NativeOptions) -> Answer {
    let hooks = opts.hooks.clone();
    match blocking(move || access_token(&hooks)).await? {
        Ok(token) => reply(StatusCode::OK, &json!({"token": token})),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => Err(Fault::Decline),
        Err(_) => Err(failure()),
    }
}

// ---------------------------------------------------------------------------
// `/conf` y `/conf-set`
// ---------------------------------------------------------------------------

/// GET `/conf` (8477): `{k: conf.get(k, CONF_DEFAULTS.get(k, "1")) for k in
/// sorted(CONF_KEYS)}` más `"_lang": ui_lang(conf)`. `lang` es el `LANG` del
/// proceso (el de `usage_env`, D7).
pub fn read_conf_view(conf: &[(String, String)], lang: Option<&str>) -> Value {
    let mut out = Map::new();
    for key in CONF_KEYS {
        let value = conf_get(conf, key).unwrap_or(conf_default(key));
        out.insert(key.to_owned(), Value::String(value.to_owned()));
    }
    out.insert("_lang".to_owned(), json!(ui_lang(conf, lang)));
    Value::Object(out)
}

/// `ui_lang(conf)` (7272): `CC_LANG=es|en` explícito o, si no, el `LANG`.
fn ui_lang(conf: &[(String, String)], lang: Option<&str>) -> &'static str {
    if ui_lang_es(conf_get(conf, "CC_LANG"), lang) {
        "es"
    } else {
        "en"
    }
}

async fn conf(opts: &NativeOptions) -> Answer {
    let hooks = opts.hooks.clone();
    let conf = blocking(move || conf_pairs(&hooks)).await??;
    let lang = opts.usage_env.get("LANG").map(String::as_str);
    read_reply(&read_conf_view(&conf, lang))
}

/// La validación de `/conf-set` (8828): `Ok(Some((key, val)))` válida,
/// `Ok(None)` → 400, `Err` → declinar o 500. Sin efectos.
fn conf_set_args(d: &Map<String, Value>) -> Result<Option<(String, String)>, Fault> {
    // `str(data.get("value", ""))`: el `repr` de un contenedor o flotante no
    // se reproduce.
    let val = match d.get("value") {
        None => String::new(),
        Some(value) => str_scalar(value).ok_or(Fault::Decline)?,
    };
    let key = match d.get("key") {
        None => "",
        Some(Value::String(key)) => key.as_str(),
        // `key in CONF_KEYS` con una lista u objeto: `TypeError` (500).
        Some(Value::Array(_) | Value::Object(_)) => return Err(failure()),
        Some(_) => return Ok(None),
    };
    let valid = match key {
        "VOLUME" => volume_ok(&val)?,
        "CC_LANG" => matches!(val.as_str(), "auto" | "es" | "en"),
        _ => CONF_KEYS.contains(&key) && matches!(val.as_str(), "0" | "1"),
    };
    Ok(valid.then(|| (key.to_owned(), val)))
}

/// `val.isdigit() and 0 <= int(val) <= 100`. Dígitos no ASCII (`isdigit`
/// los acepta y `int` a veces no) o más de 4300 dígitos (`int` lanza): se
/// declina.
fn volume_ok(val: &str) -> Result<bool, Fault> {
    if !val.is_ascii() {
        return Err(Fault::Decline);
    }
    if val.is_empty() || !val.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(false);
    }
    if val.len() > INT_MAX_STR_DIGITS {
        return Err(Fault::Decline);
    }
    let significant = val.trim_start_matches('0');
    Ok(significant.len() <= 3 && significant.parse::<u32>().unwrap_or(0) <= 100)
}

/// El texto nuevo de `write_conf_key` (7328): toda línea no comentada cuyo
/// `split("=")[0].strip()` es la clave pasa a `key=val`; si ninguna, se añade.
fn conf_with_key(text: Option<&str>, key: &str, val: &str) -> String {
    let mut lines: Vec<String> = text
        .map(|t| splitlines(t).into_iter().map(str::to_owned).collect())
        .unwrap_or_default();
    let mut hit = false;
    for line in &mut lines {
        let s = strip(line);
        let first = s.split('=').next().unwrap_or("");
        if !s.starts_with('#') && s.contains('=') && strip(first) == key {
            *line = format!("{key}={val}");
            hit = true;
        }
    }
    if !hit {
        lines.push(format!("{key}={val}"));
    }
    format!("{}\n", lines.join("\n"))
}

/// `write_conf_key(key, val)` (7328) bajo `flock(<conf>.lock)`, el mismo que
/// toman el Python y cc-app. Espera al otro dueño sin bloquear el runtime
/// (`acquire_timeout`); vencido `LOCK_WAIT` declina (solo pudo crearse el
/// archivo del candado, que el Python crearía igual). Un `cc-notify.conf` que
/// no es UTF-8 también declina: lo que leería el Python depende de la
/// codificación del proceso, y lo único hecho es el mismo candado.
pub async fn write_conf_key(hooks: &Path, key: &str, val: &str) -> Result<(), Fault> {
    let path = hooks.join(CONF_FILE);
    let lock = match FileLock::acquire_timeout(&path, LOCK_WAIT).await {
        Ok(lock) => lock,
        Err(e) if e.kind() == io::ErrorKind::TimedOut => return Err(Fault::Decline),
        Err(_) => return Err(failure()),
    };
    let (key, val) = (key.to_owned(), val.to_owned());
    blocking(move || -> Result<(), Fault> {
        let _lock = lock;
        let text = match fs::read(&path) {
            Ok(bytes) => Some(String::from_utf8(bytes).map_err(|_| Fault::Decline)?),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(_) => return Err(failure()),
        };
        let new = conf_with_key(text.as_deref(), &key, &val);
        write_text_atomic(&path, &new).map_err(|_| failure())
    })
    .await?
}

async fn conf_set(opts: &NativeOptions, d: &Map<String, Value>) -> Answer {
    let Some((key, val)) = conf_set_args(d)? else {
        return error(StatusCode::BAD_REQUEST, "Clave o valor invalido");
    };
    write_conf_key(&opts.hooks, &key, &val).await?;
    ok()
}

// ---------------------------------------------------------------------------
// `/fs/dirs` y `/fs/mkdir`
// ---------------------------------------------------------------------------

/// Texto de una ruta del disco (`realpath`, nombres): uno que no es UTF-8
/// llegaría al JSON del Python con escapes sustitutos: declinar.
fn utf8(bytes: Vec<u8>) -> Result<String, Fault> {
    String::from_utf8(bytes).map_err(|_| Fault::Decline)
}

/// `FS_HOME` (606): `realpath(expanduser("~"))` del HOME del frente.
fn fs_home(home: &str) -> Result<String, Fault> {
    let expanded = expand_user("~", home).ok_or(Fault::Decline)?;
    utf8(realpath(expanded.as_bytes()))
}

/// `_fs_safe(path)` (609) sobre `str(path or "~")` ya formado: `Ok(None)` si
/// sale de `HOME`. Un `\0` (el `lstat` del Python lanza `ValueError`), un
/// `~usuario` o una ruta relativa (dependería del directorio de trabajo del
/// Python) declinan.
fn fs_safe(raw: &str, home: &str, fs_home: &str) -> Result<Option<String>, Fault> {
    if raw.contains('\0') {
        return Err(Fault::Decline);
    }
    let expanded = expand_user(raw, home).ok_or(Fault::Decline)?;
    if !expanded.starts_with('/') {
        return Err(Fault::Decline);
    }
    let real = utf8(realpath(expanded.as_bytes()))?;
    let inside = real == fs_home
        || real
            .strip_prefix(fs_home)
            .is_some_and(|rest| rest.starts_with('/'));
    Ok(inside.then_some(real))
}

/// `os.path.split(p)`.
fn py_split(p: &str) -> (String, String) {
    let at = p.rfind('/').map_or(0, |i| i + 1);
    let (head, tail) = (p.get(..at).unwrap_or(""), p.get(at..).unwrap_or(""));
    let head = if !head.is_empty() && !head.bytes().all(|b| b == b'/') {
        head.trim_end_matches('/')
    } else {
        head
    };
    (head.to_owned(), tail.to_owned())
}

/// `os.path.join(a, b)` con `b` relativo.
fn py_join(a: &str, b: &str) -> String {
    if a.is_empty() || a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}

/// `str.lower()` cuando coincide con certeza con el de Rust: todo carácter no
/// ASCII con minúscula propia (tablas Unicode de versiones distintas, sigma
/// final…) declina.
fn py_lower(s: &str) -> Result<String, Fault> {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c.to_ascii_lowercase());
        } else {
            let mut lower = c.to_lowercase();
            if lower.next() != Some(c) || lower.next().is_some() {
                return Err(Fault::Decline);
            }
            out.push(c);
        }
    }
    Ok(out)
}

fn is_dir(p: &str) -> bool {
    Path::new(p).is_dir()
}

/// `fs_dirs(path)` (620): estado de la ruta y subcarpetas del ancestro
/// existente más cercano. Bloquea: llamar dentro de `spawn_blocking`.
fn fs_dirs(raw: &str, home: &str) -> Result<Value, Fault> {
    let fs_home = fs_home(home)?;
    let Some(real) = fs_safe(raw, home, &fs_home)? else {
        return Ok(json!({"error": OUTSIDE_HOME}));
    };
    let exists = is_dir(&real);
    let mut base = real.clone();
    let mut missing = String::new();
    while !base.is_empty() && !is_dir(&base) {
        let (head, tail) = py_split(&base);
        missing = if missing.is_empty() {
            tail
        } else {
            format!("{tail}/{missing}")
        };
        base = head;
    }
    let frag = if exists {
        String::new()
    } else {
        py_lower(missing.split('/').next().unwrap_or(""))?
    };
    let mut dirs: Vec<(String, String)> = Vec::new();
    // `os.scandir` con su `except OSError: pass`: un error al abrir deja la
    // lista vacía y uno a mitad de la lectura la corta donde iba.
    if let Ok(entries) = fs::read_dir(&base) {
        for entry in entries {
            let Ok(entry) = entry else { break };
            // `is_dir(follow_symlinks=False)`; `OSError` → se salta.
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_dir() {
                continue;
            }
            let name = entry.file_name();
            if name.as_bytes().starts_with(b".") && !frag.starts_with('.') {
                continue;
            }
            let name = utf8(name.into_vec())?;
            let lower = py_lower(&name)?;
            if !frag.is_empty() && !lower.starts_with(&frag) {
                continue;
            }
            dirs.push((lower, name));
        }
    }
    // `dirs.sort(key=str.lower)`: estable, por puntos de código.
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    let listed: Vec<Value> = dirs
        .into_iter()
        .take(150)
        .map(|(_, name)| {
            let path = py_join(&base, &name);
            json!({"name": name, "path": path})
        })
        .collect();
    let parent = if base != fs_home {
        py_split(&base).0
    } else {
        String::new()
    };
    Ok(json!({
        "path": real,
        "exists": exists,
        "base": base,
        "parent": parent,
        "missing": missing,
        "home": fs_home,
        "dirs": listed,
    }))
}

async fn fs_dirs_route(opts: &NativeOptions, request: &Request) -> Answer {
    let query = Query::parse(&request.target)?;
    let raw = take_chars(query.first("path").unwrap_or("~"), 512);
    let home = home_str(opts)?;
    let value = blocking(move || fs_dirs(&raw, &home)).await??;
    read_reply(&value)
}

/// GET `/fs/find?q=`: carpetas de todo el HOME que coinciden con lo escrito
/// (`fs_find`), para el buscador de «Nueva sesión».
async fn fs_find_route(opts: &NativeOptions, request: &Request) -> Answer {
    let query = Query::parse(&request.target)?;
    let q = take_chars(query.first("q").unwrap_or(""), 200);
    let limit = query
        .first("limit")
        .and_then(|l| l.parse::<usize>().ok())
        .unwrap_or(super::fs_find::LIMIT)
        .clamp(1, 100);
    let home = fs_home(&home_str(opts)?)?;
    let value = blocking(move || super::fs_find::answer(&home, &q, limit)).await?;
    read_reply(&value)
}

/// `repr(str)` de CPython para texto de U+0000 a U+024F (ASCII, Latin-1,
/// Latín extendido A y B: todo asignado e imprimible salvo los controles,
/// U+00A0 y U+00AD, que van como `\xNN`); `None` fuera de ese rango.
fn repr_latin(s: &str) -> Option<String> {
    if s.chars().any(|c| u32::from(c) > 0x24f) {
        return None;
    }
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        let n = u32::from(c);
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            _ if n < 0x20 || (0x7f..=0xa0).contains(&n) || n == 0xad => {
                out.push_str(&format!("\\x{n:02x}"));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    Some(out)
}

/// `str(OSError)` con nombre de archivo: `[Errno N] <strerror>: <repr>`.
/// `strerror` es el mismo texto de la libc que da `io::Error`.
fn os_error_message(e: &io::Error, filename: &str) -> Option<String> {
    let code = e.raw_os_error()?;
    let text = e.to_string();
    let strerror = text.strip_suffix(&format!(" (os error {code})"))?;
    Some(format!(
        "[Errno {code}] {strerror}: {}",
        repr_latin(filename)?
    ))
}

/// `os.makedirs(name, exist_ok=False)` (modo 0o777 menos la `umask`): el
/// error y la ruta en la que ocurrió (la de su excepción).
fn makedirs(name: &str) -> Result<(), (io::Error, String)> {
    let (mut head, mut tail) = py_split(name);
    if tail.is_empty() {
        (head, tail) = py_split(&head);
    }
    if !head.is_empty() && !tail.is_empty() && !Path::new(&head).exists() {
        match makedirs(&head) {
            // `except FileExistsError: pass`.
            Err((e, _)) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(other) => return Err(other),
            Ok(()) => {}
        }
        if tail == "." {
            return Ok(());
        }
    }
    fs::DirBuilder::new()
        .mode(0o777)
        .create(name)
        .map_err(|e| (e, name.to_owned()))
}

/// `fs_mkdir(path)` (658) con el `except (ValueError, OSError)` de la ruta:
/// `Ok(Ok(ruta))` o `Ok(Err(mensaje))` (400). Bloquea.
fn fs_mkdir(raw: &str, home: &str) -> Result<Result<String, String>, Fault> {
    let fs_home = fs_home(home)?;
    let raw = if raw.is_empty() { "~" } else { raw };
    let Some(real) = fs_safe(raw, home, &fs_home)? else {
        return Ok(Err(MKDIR_OUTSIDE.to_owned()));
    };
    if is_dir(&real) {
        return Ok(Ok(real));
    }
    if Path::new(&real).exists() {
        return Ok(Err(MKDIR_FILE.to_owned()));
    }
    // El mensaje de un fallo lleva el `repr` de una de sus rutas (todas
    // prefijos de `real`): si no se puede reproducir, declinar ANTES de crear.
    if repr_latin(&real).is_none() {
        return Err(Fault::Decline);
    }
    match makedirs(&real) {
        Ok(()) => Ok(Ok(real)),
        Err((e, path)) => os_error_message(&e, &path).map(Err).ok_or_else(failure),
    }
}

async fn fs_mkdir_route(opts: &NativeOptions, d: &Map<String, Value>) -> Answer {
    let raw = take_chars(&str_or(d.get("path"), "")?, 512);
    let home = home_str(opts)?;
    match blocking(move || fs_mkdir(&raw, &home)).await?? {
        Ok(created) => reply(StatusCode::OK, &json!({"ok": true, "path": created})),
        Err(message) => error(StatusCode::BAD_REQUEST, &message),
    }
}

// ---------------------------------------------------------------------------
// `/open-path` y `/open-url`
// ---------------------------------------------------------------------------

/// `_open_url(url, env)` (34): `open` en macOS; en WSL (`osrelease` contiene
/// `microsoft`) y con `wslview` en el `PATH`, `wslview`; si no, `xdg-open`. Se
/// lanza suelto (sesión nueva, salidas a `/dev/null`) por su ruta absoluta en
/// el `PATH` del frente; sin el programa o con un fallo al lanzarlo no pasa
/// nada (el `try/except: pass` del Python). `env` se añade al entorno de los
/// hijos (`gui_env()` en `/open-path`). Bloquea un instante (`/proc`, `PATH`):
/// llamar dentro de `spawn_blocking`.
pub fn open_url(opts: &NativeOptions, url: &OsStr, env: &[(OsString, OsString)]) {
    let search = opts.search_path.as_deref();
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        let wsl = fs::read(opts.proc_root.join("sys/kernel/osrelease"))
            .map(|b| {
                String::from_utf8_lossy(&b)
                    .to_lowercase()
                    .contains("microsoft")
            })
            .unwrap_or(false);
        if wsl && which_in(search, "wslview").is_some() {
            "wslview"
        } else {
            "xdg-open"
        }
    };
    if let Some(path) = which_in(search, opener) {
        let _ = spawn_detached(&opts.program(path), &[url.to_owned()], env);
    }
}

/// `re.sub(r":\d+$", "", text)`: quita `:<dígitos>` final. `\d` del Python
/// incluye dígitos Unicode: uno no ASCII en ese sufijo declina.
fn strip_line_suffix(text: &str) -> Result<&str, Fault> {
    let Some(at) = text.rfind(':') else {
        return Ok(text);
    };
    let suffix = text.get(at + 1..).unwrap_or("");
    if suffix.chars().any(|c| !c.is_ascii() && c.is_numeric()) {
        return Err(Fault::Decline);
    }
    if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(text.get(..at).unwrap_or(text));
    }
    Ok(text)
}

/// POST `/open-path` (8790): solo abre, nunca ejecuta; exige una ruta
/// absoluta (o `~`) que exista.
async fn open_path(native: &Arc<Native>, d: &Map<String, Value>) -> Answer {
    let opts = native.options();
    let raw = take_chars(&str_or(d.get("path"), "")?, 1024);
    if raw.contains('\0') || !(raw.starts_with('/') || raw.starts_with('~')) {
        return error(StatusCode::BAD_REQUEST, "ruta invalida");
    }
    let target = strip_line_suffix(strip(&raw))?;
    let target = expand_user(target, &home_str(opts)?).ok_or(Fault::Decline)?;
    let native = Arc::clone(native);
    let opened = blocking(move || {
        let opts = native.options();
        // `os.path.exists`: sigue enlaces; un error cuenta como ausente.
        if !Path::new(&target).exists() {
            return false;
        }
        let env = gui_env_for(opts);
        open_url(opts, OsStr::new(&target), &env);
        true
    })
    .await?;
    if opened {
        ok()
    } else {
        error(StatusCode::NOT_FOUND, "esa ruta ya no existe")
    }
}

/// POST `/open-url` (9076): solo `http(s)://`, con el entorno del frente.
async fn open_url_route(native: &Arc<Native>, d: &Map<String, Value>) -> Answer {
    let url = str_or(d.get("url"), "")?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return error(StatusCode::BAD_REQUEST, "solo http(s)");
    }
    let native = Arc::clone(native);
    blocking(move || open_url(native.options(), OsStr::new(&url), &[])).await?;
    ok()
}

// ---------------------------------------------------------------------------
// `/notify-popup`
// ---------------------------------------------------------------------------

/// El cuerpo de `desktop_popup` (475) para `POST 127.0.0.1:4778/notify`:
/// `json.dumps` con el orden de claves del Python y `ensure_ascii`. Lo usará
/// la ruta a través de `NotifyPost::post_checked`.
pub fn popup_payload(title: &str, body: &str, project: &str, kind: &str) -> Option<String> {
    let body = take_chars(body, 400);
    let payload = json!({
        "title": take_chars(title, 200),
        "body": body,
        "session": "",
        "kind": if kind == "waiting" { "waiting" } else { "done" },
        "project": take_chars(project, 80),
        "options": "",
        "full": body,
    });
    response_dumps(&payload).ok()
}

/// POST `/notify-popup`: sin fallback después de enviar. La tarea completa
/// el envío aunque se cierre la conexión HTTP del solicitante.
async fn notify_popup(native: &Arc<Native>, d: &Map<String, Value>) -> Answer {
    let opts = native.options();
    let title = d.get("title").and_then(Value::as_str);
    let body = d.get("body").and_then(Value::as_str);
    if title.is_none_or(|t| strip(t).is_empty()) || body.is_none() {
        return error(StatusCode::BAD_REQUEST, "title y body son obligatorios");
    }
    let hooks = opts.hooks.clone();
    let conf = blocking(move || conf_pairs(&hooks)).await??;
    if conf_get(&conf, "DESKTOP_NOTIFY").unwrap_or("1") != "1" {
        return reply(StatusCode::OK, &json!({"ok": true, "popup": false}));
    }
    let project = str_or(d.get("project"), "ComandOS")?;
    let kind = str_or(d.get("kind"), "waiting")?;
    let payload = popup_payload(
        title.ok_or(Fault::Error(HandlerError::Failure))?,
        body.ok_or(Fault::Error(HandlerError::Failure))?,
        &project,
        &kind,
    )
    .ok_or(Fault::Decline)?;
    let notify = Arc::clone(&opts.notifyd);
    let job = native
        .tasks()
        .spawn_handle(async move {
            notify
                .post_checked(payload, std::time::Duration::from_secs(2))
                .await
        })
        .map_err(|_| Fault::Error(HandlerError::Failure))?;
    let shown = job.await.map_err(|_| Fault::Error(HandlerError::Failure))?;
    reply(StatusCode::OK, &json!({"ok": true, "popup": shown}))
}

// ---------------------------------------------------------------------------
// `/test`
// ---------------------------------------------------------------------------

/// `conf_volume(conf)` (7278): `max(0, min(100, int(VOLUME)))`, 60 si
/// `int()` falla; un entero que `int()` acepta por reglas Unicode o grandes
/// no se reproduce: declinar.
fn conf_volume(conf: &[(String, String)]) -> Result<i64, Fault> {
    match int(conf_get(conf, "VOLUME").unwrap_or("60")) {
        Ok(n) => Ok(n.clamp(0, 100)),
        Err(NumError::Invalid) => Ok(60),
        Err(NumError::Exotic) => Err(Fault::Decline),
    }
}

/// `play_cmd(vol)` (7285): el nombre (para el guion de la voz), su ruta en el
/// `PATH` del frente (si existe) y la opción de volumen.
struct Player {
    name: &'static str,
    path: Option<PathBuf>,
    volume: String,
}

fn play_cmd(opts: &NativeOptions, vol: i64) -> Player {
    let search = opts.search_path.as_deref();
    match which_in(search, "pw-play") {
        // `f"--volume={vol / 100:.2f}"` con `vol` entero de 0 a 100: exacto.
        Some(path) => Player {
            name: "pw-play",
            path: Some(path),
            volume: format!("--volume={}.{:02}", vol / 100, vol % 100),
        },
        None => Player {
            name: "paplay",
            path: which_in(search, "paplay"),
            volume: format!("--volume={}", vol * 655),
        },
    }
}

/// Un `subprocess.Popen(..., start_new_session=True)` de `play_test`: fuera de
/// todo `try`, un fallo al lanzar es la excepción del Python (500).
fn launch(opts: &NativeOptions, path: PathBuf, args: Vec<OsString>) -> Result<(), Fault> {
    spawn_detached(&opts.program(path), &args, &[]).map_err(|_| failure())
}

/// `play_test(kind)` (7293): `Ok(Some(error))` → 400. Bloquea.
fn play_test(opts: &NativeOptions, kind: &str) -> Result<Option<String>, Fault> {
    let conf = conf_pairs(&opts.hooks)?;
    let vol = conf_volume(&conf)?;
    let player = play_cmd(opts, vol);
    let search = opts.search_path.as_deref();
    if kind == "voice" {
        let voice = conf_get(&conf, "PIPER_VOICE").unwrap_or("es_MX-ald-medium");
        let model = expand_user(
            &format!("~/.local/share/piper-voices/{voice}.onnx"),
            &home_str(opts)?,
        )
        .ok_or(Fault::Decline)?;
        if which_in(search, "piper").is_some() && Path::new(&model).exists() {
            let script = format!(
                "w=$(mktemp --suffix=.wav); printf %s '{TEST_PHRASE}' | piper -m {} \
                 -f \"$w\" >/dev/null 2>&1 && {} {} \"$w\"; rm -f \"$w\"",
                shlex_quote(&model),
                player.name,
                player.volume,
            );
            launch(
                opts,
                PathBuf::from("/bin/sh"),
                vec!["-c".into(), script.into()],
            )?;
            return Ok(None);
        }
        if let Some(say) = which_in(search, "spd-say") {
            let args = ["-l", "es", "-i", &(vol * 2 - 100).to_string(), TEST_PHRASE];
            launch(opts, say, args.iter().map(OsString::from).collect())?;
            return Ok(None);
        }
        return Ok(Some(NO_VOICE.to_owned()));
    }
    let snd = if kind == "done" {
        conf_get(&conf, "SOUND_DONE").unwrap_or("/usr/share/sounds/freedesktop/stereo/message.oga")
    } else {
        conf_get(&conf, "SOUND_ATTENTION")
            .unwrap_or("/usr/share/sounds/freedesktop/stereo/window-attention.oga")
    };
    // Una ruta relativa se resolvería contra el directorio del Python.
    if !snd.is_empty() && !snd.starts_with('/') {
        return Err(Fault::Decline);
    }
    if !Path::new(snd).exists() {
        return Ok(Some(format!("No existe el sonido {snd}")));
    }
    // Sin `pw-play` ni `paplay`: `FileNotFoundError` sin capturar (500).
    let path = player.path.ok_or_else(failure)?;
    launch(opts, path, vec![player.volume.into(), snd.into()])?;
    Ok(None)
}

async fn test(native: &Arc<Native>, d: &Map<String, Value>) -> Answer {
    let kind = match d.get("kind").and_then(Value::as_str) {
        Some(kind @ ("voice" | "chime" | "done")) => kind.to_owned(),
        _ => return error(StatusCode::BAD_REQUEST, "kind debe ser voice, chime o done"),
    };
    let native = Arc::clone(native);
    match blocking(move || play_test(native.options(), &kind)).await?? {
        None => ok(),
        Some(message) => error(StatusCode::BAD_REQUEST, &message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conf_keys_are_sorted() {
        let mut sorted = CONF_KEYS;
        sorted.sort_unstable();
        assert_eq!(sorted, CONF_KEYS);
    }

    #[test]
    fn base64_matches_python_token_urlsafe() {
        use crate::dash::token::base64_urlsafe;
        // base64.urlsafe_b64encode(bytes(range(32))).rstrip(b"=")
        let bytes: Vec<u8> = (0u8..32).collect();
        assert_eq!(
            base64_urlsafe(&bytes),
            "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
        );
        assert_eq!(base64_urlsafe(&[0xfb, 0xff]), "-_8");
        assert_eq!(base64_urlsafe(&[]), "");
    }

    #[test]
    fn conf_rewrite_like_python() {
        let text = "# VOLUME=1\nVOLUME = 70\nOTRA=1\r\nVOLUME=2\u{2028}x";
        assert_eq!(
            conf_with_key(Some(text), "VOLUME", "55"),
            "# VOLUME=1\nVOLUME=55\nOTRA=1\nVOLUME=55\nx\n"
        );
        assert_eq!(conf_with_key(None, "CC_LANG", "en"), "CC_LANG=en\n");
        assert_eq!(conf_with_key(Some(""), "A", "1"), "A=1\n");
    }

    #[test]
    fn volume_validation() {
        assert!(matches!(volume_ok("55"), Ok(true)));
        assert!(matches!(volume_ok("007"), Ok(true)));
        assert!(matches!(volume_ok("100"), Ok(true)));
        assert!(matches!(volume_ok("101"), Ok(false)));
        assert!(matches!(volume_ok("0001000"), Ok(false)));
        assert!(matches!(volume_ok(""), Ok(false)));
        assert!(matches!(volume_ok("-1"), Ok(false)));
        assert!(matches!(volume_ok("٥"), Err(Fault::Decline)));
    }

    #[test]
    fn os_paths_like_python() {
        assert_eq!(py_split("/a/b"), ("/a".into(), "b".into()));
        assert_eq!(py_split("/a"), ("/".into(), "a".into()));
        assert_eq!(py_split("/"), ("/".into(), String::new()));
        assert_eq!(py_join("/", "x"), "/x");
        assert_eq!(py_join("/a", "x"), "/a/x");
        assert_eq!(expand_user("~", "/h/"), Some("/h".into()));
        assert_eq!(expand_user("~/x", "/h"), Some("/h/x".into()));
        assert_eq!(expand_user("~otro/x", "/h"), None);
        assert_eq!(expand_user("/a", "/h"), Some("/a".into()));
    }

    #[test]
    fn line_suffix_like_re_sub() {
        assert!(matches!(strip_line_suffix("/a/b.rs:12"), Ok("/a/b.rs")));
        assert!(matches!(strip_line_suffix("/a:1:2"), Ok("/a:1")));
        assert!(matches!(strip_line_suffix("/a:"), Ok("/a:")));
        assert!(matches!(strip_line_suffix("/a:x1"), Ok("/a:x1")));
        assert!(matches!(strip_line_suffix("/a:１"), Err(Fault::Decline)));
    }

    #[test]
    fn repr_and_errno_like_python() {
        assert_eq!(repr_latin("/h/Música"), Some("'/h/Música'".into()));
        assert_eq!(repr_latin("a'b"), Some("\"a'b\"".into()));
        assert_eq!(
            repr_latin("a\u{a0}\u{ad}\u{80}"),
            Some("'a\\xa0\\xad\\x80'".into())
        );
        assert_eq!(repr_latin("中"), None);
        let e = io::Error::from_raw_os_error(20);
        assert_eq!(
            os_error_message(&e, "/h/f/x").as_deref(),
            Some("[Errno 20] Not a directory: '/h/f/x'")
        );
    }

    #[test]
    fn lower_declines_unicode_case() {
        assert!(matches!(py_lower("AbC_ñ").as_deref(), Ok("abc_ñ")));
        assert!(py_lower("Ñ").is_err());
        assert!(py_lower("Σ").is_err());
    }

    #[test]
    fn conf_view_defaults_and_lang() {
        let conf = vec![("VOLUME".to_owned(), "70".to_owned())];
        let view = read_conf_view(&conf, Some("es_MX.UTF-8"));
        assert_eq!(view["VOLUME"], "70");
        assert_eq!(view["CC_LANG"], "auto");
        assert_eq!(view["DESKTOP_NOTIFY"], "1");
        assert_eq!(view["_lang"], "es");
        let conf = vec![("CC_LANG".to_owned(), "en".to_owned())];
        assert_eq!(read_conf_view(&conf, Some("es"))["_lang"], "en");
    }
}
