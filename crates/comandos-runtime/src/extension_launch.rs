//! `lib/extension_launch.py`, el subconjunto que usan los cambios de
//! configuración de un pane: envolver el comando con el lanzador privado de
//! extensiones (`wrap_command`, `wrap_environment`), capturar el entorno de
//! OpenCode, reconstruir y verificar un lanzamiento ya en marcha
//! (`launch_from_pid`, `verify_launch`, `configuration_status`), normalizar una
//! selección y leer la configuración TOML (`_read`, `_parse_toml`).
//!
//! O5 ejecuta el lanzador nativo en `executor`; los wrappers conservan argv
//! y validaciones, con el alias instalado en lugar del script del repositorio.
//!
//! Errores: `Value(texto)` es un `ValueError` del Python con su texto; `Other`
//! cualquier otra excepción (`OSError`, `KeyError`, `TypeError`,
//! `AttributeError`); `Unsure`, lo que el port no reproduce con certeza.
//!
//! El TOML se lee como lo hace el heredado en esta máquina: `python3` es 3.10
//! (sin `tomllib`), así que `_parse_toml` lo pasa por `python3.11 -I` y
//! `json.dumps`. Por eso un documento con fechas falla (no son JSON) y uno con
//! sintaxis de TOML 1.1 (que `toml_edit` acepta y `tomllib` no) también. La
//! herencia de confianza de Codex usa `parse_trust_toml`, que sí acepta
//! fechas (desviación deliberada: ver su comentario). Un número que
//! `toml_edit` no representa (`1e400`, `0x8000000000000000`) es `Unsure`.
use crate::Unsure;
use comandos_core::json::{PythonLoads, python_eq, python_loads, response_dumps, truthy};
use comandos_core::text::{shlex_quote, splitlines};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    ffi::OsStr,
    fs, io,
    io::{Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, Value as Tv};
use toml_parser::{Source, lexer::TokenKind};

pub mod executor;

pub const MARKER: &str = "COMANDOS_EXTENSION_OPERATION_ID";
pub const MANIFEST_ENV: &str = "COMANDOS_EXTENSION_MANIFEST";
pub const DIGEST_ENV: &str = "COMANDOS_EXTENSION_MANIFEST_SHA256";
pub const RECEIPT_ENV: &str = "COMANDOS_EXTENSION_RECEIPT";
pub const RECEIPT_HASH_ENV: &str = "COMANDOS_EXTENSION_RECEIPT_SHA256";
pub const OPENCODE_ENV_KEYS: [&str; 4] = [
    "OPENCODE_CONFIG_CONTENT",
    "OPENCODE_PERMISSION",
    "OPENCODE_CONFIG",
    "OPENCODE_CONFIG_DIR",
];

/// Lo que lanza una función del módulo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    Value(String),
    Other,
    Unsure,
}

impl From<Unsure> for LaunchError {
    fn from(_: Unsure) -> Self {
        Self::Unsure
    }
}

/// Excepción interna: `Caught` es la que `verify_launch` y `launch_from_pid`
/// tragan (`OSError`, `KeyError`, `TypeError`); `Attr`, la que no
/// (`AttributeError`).
#[derive(Debug)]
enum Fail {
    Value(String),
    Caught,
    Attr,
    Unsure,
}

impl From<Unsure> for Fail {
    fn from(_: Unsure) -> Self {
        Self::Unsure
    }
}

impl From<io::Error> for Fail {
    fn from(_: io::Error) -> Self {
        Self::Caught
    }
}

impl From<Fail> for LaunchError {
    fn from(fail: Fail) -> Self {
        match fail {
            Fail::Value(text) => Self::Value(text),
            Fail::Caught | Fail::Attr => Self::Other,
            Fail::Unsure => Self::Unsure,
        }
    }
}

/// Installed alias for an explicitly injected HOME; never selects repo/bin.
pub fn helper_for_home(home: &Path) -> PathBuf {
    home.join(".local/bin/cc-extension-session")
}
/// Compatibility signature. `repo_root` no longer selects an executable.
/// HOME selects the native alias; absent HOME, an executable sibling fails closed.
pub fn helper(_repo_root: &Path) -> PathBuf {
    if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        helper_for_home(Path::new(&home))
    } else {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("cc-extension-session")))
            .unwrap_or_else(|| PathBuf::from("/nonexistent/comandos/cc-extension-session"))
    }
}
/// Refuse before stopping an agent unless the alias resolves to the installed native binary.
pub fn require_helper(home: &Path) -> Result<PathBuf, LaunchError> {
    let refused = || {
        LaunchError::Value(
            "El lanzador nativo de extensiones no está instalado; el agente sigue abierto.".into(),
        )
    };
    let alias = helper_for_home(home);
    let resolved = fs::canonicalize(&alias).map_err(|_| refused())?;
    let binary =
        fs::canonicalize(home.join(".local/share/comandos/bin/comandos")).map_err(|_| refused())?;
    let meta = fs::metadata(&resolved).map_err(|_| refused())?;
    if resolved != binary
        || !meta.is_file()
        || (meta.uid() != uid() && meta.uid() != 0)
        || meta.mode() & 0o022 != 0
        || meta.mode() & 0o111 == 0
    {
        return Err(refused());
    }
    let mut magic = [0; 4];
    let mut file = fs::File::open(&resolved).map_err(|_| refused())?;
    file.read_exact(&mut magic).map_err(|_| refused())?;
    let native = magic == *b"\x7fELF"
        || matches!(
            magic,
            [0xfe, 0xed, 0xfa, 0xce]
                | [0xce, 0xfa, 0xed, 0xfe]
                | [0xfe, 0xed, 0xfa, 0xcf]
                | [0xcf, 0xfa, 0xed, 0xfe]
                | [0xca, 0xfe, 0xba, 0xbe]
                | [0xbe, 0xba, 0xfe, 0xca]
                | [0xca, 0xfe, 0xba, 0xbf]
                | [0xbf, 0xba, 0xfe, 0xca]
        );
    if !native {
        return Err(refused());
    }
    Ok(alias)
}

// ------------------------------------------------------------------ texto

/// `bytes.decode()` estricto con los saltos universales de `read_text()`.
pub(crate) fn universal_text(bytes: Vec<u8>) -> Option<String> {
    let text = String::from_utf8(bytes).ok()?;
    Some(text.replace("\r\n", "\n").replace('\r', "\n"))
}

/// `shlex.split(s)` (POSIX, sin comentarios). `Err` = el `ValueError` con su
/// texto.
pub fn shlex_split(s: &str) -> Result<Vec<String>, String> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Space,
        Word,
        Single,
        Double,
    }
    let is_space = |c: char| matches!(c, ' ' | '\t' | '\r' | '\n');
    let mut out = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut state = State::Space;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match state {
            State::Space | State::Word => {
                if is_space(c) {
                    if state == State::Word {
                        out.push(std::mem::take(&mut token));
                        quoted = false;
                        state = State::Space;
                    }
                } else if c == '\\' {
                    let next = chars.next().ok_or("No escaped character")?;
                    token.push(next);
                    state = State::Word;
                } else if c == '\'' {
                    quoted = true;
                    state = State::Single;
                } else if c == '"' {
                    quoted = true;
                    state = State::Double;
                } else {
                    token.push(c);
                    state = State::Word;
                }
            }
            State::Single => {
                if c == '\'' {
                    state = State::Word;
                } else {
                    token.push(c);
                }
            }
            State::Double => {
                if c == '"' {
                    state = State::Word;
                } else if c == '\\' {
                    let next = chars.next().ok_or("No escaped character")?;
                    if next != '"' && next != '\\' {
                        token.push('\\');
                    }
                    token.push(next);
                } else {
                    token.push(c);
                }
            }
        }
    }
    match state {
        State::Single | State::Double => Err("No closing quotation".into()),
        State::Word => {
            out.push(token);
            Ok(out)
        }
        State::Space => {
            if quoted {
                out.push(token);
            }
            Ok(out)
        }
    }
}

fn shlex_join<S: AsRef<str>>(words: &[S]) -> String {
    words
        .iter()
        .map(|w| shlex_quote(w.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `json.loads(text)`: `Ok(None)` = `JSONDecodeError` (con su texto en
/// `Err` de la segunda capa cuando hace falta).
fn py_json(text: &str) -> Result<Result<Value, String>, Unsure> {
    match python_loads(text) {
        PythonLoads::Ok => comandos_core::json::workspace_loads(text)
            .map(Ok)
            .map_err(|_| Unsure),
        PythonLoads::Error(message) => Ok(Err(message)),
        PythonLoads::Unsure => Err(Unsure),
    }
}

/// `json.loads(raw_bytes)`: solo UTF-8 sin BOM ni nulos al principio (la
/// detección de codificación del Python no se reproduce).
fn py_json_bytes(raw: &[u8]) -> Result<Result<Value, String>, Unsure> {
    if raw.starts_with(&[0xef, 0xbb, 0xbf]) || raw.iter().take(4).any(|b| *b == 0) {
        return Err(Unsure);
    }
    match std::str::from_utf8(raw) {
        Ok(text) => py_json(text),
        // `UnicodeDecodeError` (un `ValueError`).
        Err(_) => Ok(Err(String::new())),
    }
}

// ------------------------------------------------------------------ TOML

/// ¿Usa el texto sintaxis de TOML 1.1 (que `toml_edit` acepta y el `tomllib`
/// de 3.11 rechaza)? Saltos o comentarios dentro de una tabla en línea, coma
/// final en una tabla en línea y los escapes `\e`/`\x`.
fn toml_1_1(text: &str) -> bool {
    let source = Source::new(text);
    let tokens = source.lex().into_vec();
    let mut stack = Vec::new();
    let mut after_comma = false;
    for token in &tokens {
        let kind = token.kind();
        let raw = source.get(token).map(|r| r.as_str()).unwrap_or("");
        let in_curly = stack.last() == Some(&TokenKind::LeftCurlyBracket);
        match kind {
            TokenKind::LeftCurlyBracket | TokenKind::LeftSquareBracket => stack.push(kind),
            TokenKind::RightCurlyBracket => {
                if in_curly && after_comma {
                    return true;
                }
                stack.pop();
            }
            TokenKind::RightSquareBracket => {
                stack.pop();
            }
            TokenKind::Newline | TokenKind::Comment if in_curly => return true,
            TokenKind::BasicString | TokenKind::MlBasicString => {
                let mut chars = raw.chars();
                while let Some(c) = chars.next() {
                    if c == '\\' && matches!(chars.next(), Some('e' | 'x')) {
                        return true;
                    }
                }
            }
            _ => {}
        }
        match kind {
            TokenKind::Comma => after_comma = true,
            TokenKind::Whitespace | TokenKind::Newline | TokenKind::Comment => {}
            _ => after_comma = false,
        }
    }
    false
}

/// Un literal numérico que `toml_edit` no representa y `tomllib` sí: un
/// entero fuera de `i64` (decimal, `0x`, `0o` o `0b`) o un flotante que se va
/// al infinito (`1e400`). Si `toml_edit` falla con uno así, el veredicto del
/// Python es incierto; con números que caben, el rechazo es seguro.
fn risky_number(text: &str) -> bool {
    let source = Source::new(text);
    source.lex().into_vec().iter().any(|token| {
        token.kind() == TokenKind::Atom
            && source.get(token).is_some_and(|raw| overflows(raw.as_str()))
    })
}

/// ¿Es `atom` un número TOML que no cabe en lo que `toml_edit` representa?
fn overflows(atom: &str) -> bool {
    let clean: String = atom.chars().filter(|c| *c != '_').collect();
    let (negative, body) = match clean.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, clean.strip_prefix('+').unwrap_or(&clean)),
    };
    if !body.starts_with(|c: char| c.is_ascii_digit()) {
        return false;
    }
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(digits) = body.strip_prefix(prefix) {
            let valid = !digits.is_empty() && digits.chars().all(|c| c.is_digit(radix));
            return valid
                && u128::from_str_radix(digits, radix).map_or(true, |v| v > i64::MAX as u128);
        }
    }
    if body.chars().all(|c| c.is_ascii_digit()) {
        let limit = if negative {
            i64::MAX as u128 + 1
        } else {
            i64::MAX as u128
        };
        return body.parse::<u128>().map_or(true, |v| v > limit);
    }
    let float = body.contains(['.', 'e', 'E'])
        && body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'));
    float && body.parse::<f64>().is_ok_and(f64::is_infinite)
}

/// Qué hacer con una fecha.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dates {
    /// `json.dumps` del resultado lanza: el `_parse_toml` del Python 3.10.
    Fail,
    /// Un valor opaco verdadero (su texto): el lector de la confianza.
    Opaque,
    /// json.dumps(default=str) of the inventory fallback reader.
    Inventory,
}

/// Un valor de `toml_edit` como el JSON que devuelve `json.dumps(tomllib…)`;
/// `None` si el Python lanzaría.
fn toml_value(value: &Tv, dates: Dates) -> Option<Value> {
    Some(match value {
        Tv::String(s) => json!(s.value()),
        Tv::Integer(i) => json!(i.value()),
        Tv::Float(f) => {
            let n = *f.value();
            if n.is_finite() {
                json!(n)
            } else {
                // `json.dumps` escribe `NaN`/`Infinity`; `json.loads` los lee.
                let text = if n.is_nan() {
                    "NaN"
                } else if n.is_sign_positive() {
                    "Infinity"
                } else {
                    "-Infinity"
                };
                Value::Number(serde_json::Number::from_string_unchecked(text.into()))
            }
        }
        Tv::Boolean(b) => json!(b.value()),
        Tv::Datetime(dt) => {
            // Una hora sin segundos es TOML 1.1: `tomllib` 3.11 la rechaza.
            // `23:59:60`: `toml_edit` lo acepta; `tomllib` no (no es un `time`).
            let unrepresentable = dt
                .value()
                .time
                .is_some_and(|t| t.second.is_none_or(|second| second > 59));
            if dates == Dates::Fail || unrepresentable {
                return None;
            }
            if dates == Dates::Inventory {
                let dt = dt.value();
                let mut text = String::new();
                if let Some(d) = dt.date {
                    if d.year == 0
                        || chrono::NaiveDate::from_ymd_opt(
                            i32::from(d.year),
                            u32::from(d.month),
                            u32::from(d.day),
                        )
                        .is_none()
                    {
                        return None;
                    }
                    text = format!("{:04}-{:02}-{:02}", d.year, d.month, d.day);
                }
                if let Some(t) = dt.time {
                    if !text.is_empty() {
                        text.push(' ')
                    }
                    text.push_str(&format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second?));
                    let micros = t.nanosecond.unwrap_or(0) / 1000;
                    if micros > 0 {
                        text.push_str(&format!(".{micros:06}"));
                    }
                }
                if let Some(o) = dt.offset {
                    let offset = o.to_string();
                    if offset == "Z" {
                        text.push_str("+00:00");
                    } else {
                        if offset.get(1..3)?.parse::<u8>().ok()? >= 24 {
                            return None;
                        }
                        text.push_str(&offset);
                    }
                }
                json!(text)
            } else {
                json!(dt.value().to_string())
            }
        }
        Tv::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| toml_value(v, dates))
                .collect::<Option<_>>()?,
        ),
        Tv::InlineTable(table) => {
            let mut map = Map::new();
            for (key, value) in table.iter() {
                map.insert(key.to_owned(), toml_value(value, dates)?);
            }
            Value::Object(map)
        }
    })
}

fn toml_item(item: &Item, dates: Dates) -> Option<Value> {
    match item {
        Item::None => None,
        Item::Value(value) => toml_value(value, dates),
        Item::Table(table) => {
            let mut map = Map::new();
            for (key, value) in table.iter() {
                map.insert(key.to_owned(), toml_item(value, dates)?);
            }
            Some(Value::Object(map))
        }
        Item::ArrayOfTables(tables) => Some(Value::Array(
            tables
                .iter()
                .map(|t| toml_item(&Item::Table(t.clone()), dates))
                .collect::<Option<_>>()?,
        )),
    }
}

fn parse_with(text: &str, dates: Dates) -> Result<Option<Value>, Unsure> {
    if text.starts_with('\u{feff}') || toml_1_1(text) {
        return Ok(None);
    }
    match text.parse::<DocumentMut>() {
        Ok(doc) => Ok(toml_item(doc.as_item(), dates)),
        Err(_) if risky_number(text) => Err(Unsure),
        Err(_) => Ok(None),
    }
}

/// `_parse_toml(text)`: `Ok(None)` = el `ValueError` del Python.
pub fn parse_toml(text: &str) -> Result<Option<Value>, Unsure> {
    parse_with(text, Dates::Fail)
}

/// El TOML de la herencia de confianza de Codex. Como `parse_toml`, pero las
/// fechas son valores opacos verdaderos (su texto): desviación deliberada, el
/// Python solo falla con ellas por pasar el resultado por JSON, y entonces un
/// `config.toml` con una fecha nunca heredaba la confianza (el cambio de
/// cuenta se quedaba en el diálogo). Lo demás que `tomllib` 3.11 rechaza
/// (sintaxis 1.1, BOM, hora sin segundos, segundo 60) se sigue rechazando,
/// salvo el año 0 (`0000-01-01`): `tomllib` no lo admite (no cabe en un
/// `date`), pero el CLI de Codex (`toml`) lo lee, así que se acepta como las
/// demás fechas (misma desviación).
pub(crate) fn parse_trust_toml(text: &str) -> Result<Option<Value>, Unsure> {
    parse_with(text, Dates::Opaque)
}

/// Inventory reader (Python 3.10 fallback): dates serialize as strings,
/// but year zero is still invalid in tomllib.
pub(crate) fn parse_inventory_toml(text: &str) -> Result<Option<Value>, Unsure> {
    if let Ok(result) = parse_with(text, Dates::Inventory) {
        return Ok(result);
    }
    // toml_edit narrows integer literals to i64. Shield valid overflowing
    // numeric tokens, parse the complete grammar, then restore Python numbers.
    let integer=regex::Regex::new(r"^([-+]?(0|[1-9](_?[0-9])*)|0x[0-9A-Fa-f](_?[0-9A-Fa-f])*|0o[0-7](_?[0-7])*|0b[01](_?[01])*)$").map_err(|_|Unsure)?;
    let float = regex::Regex::new(
        r"^[-+]?(0|[1-9](_?[0-9])*)(\.[0-9](_?[0-9])*)?([eE][-+]?[0-9](_?[0-9])*)?$",
    )
    .map_err(|_| Unsure)?;
    use sha2::{Digest, Sha256};
    let prefix = format!(
        "__comandos_inventory_number_{:x}_",
        Sha256::digest(text.as_bytes())
    );
    let source = Source::new(text);
    let mut converted = String::new();
    let mut cursor = 0;
    let mut values = std::collections::HashMap::new();
    for token in source.lex().into_vec().iter() {
        if token.kind() != TokenKind::Atom {
            continue;
        }
        let Some(raw) = source.get(token) else {
            continue;
        };
        let raw = raw.as_str();
        if !overflows(raw) {
            continue;
        }
        let clean = raw.replace('_', "");
        let value = if integer.is_match(raw) {
            let normalized = if let Some(octal) = clean.strip_prefix("0o") {
                format!("0{octal}")
            } else {
                clean.clone()
            };
            crate::profile_yaml::integer(&normalized)
        } else if float.is_match(raw) && clean.parse::<f64>().is_ok_and(f64::is_infinite) {
            Some(Value::Number(serde_json::Number::from_string_unchecked(
                if clean.starts_with('-') {
                    "-Infinity"
                } else {
                    "Infinity"
                }
                .into(),
            )))
        } else {
            None
        };
        let Some(value) = value else { continue };
        let marker = format!("{prefix}{}", values.len());
        let span = token.span();
        converted.push_str(&text[cursor..span.start()]);
        converted.push('"');
        converted.push_str(&marker);
        converted.push('"');
        cursor = span.end();
        values.insert(marker, (raw.to_owned(), value));
    }
    if values.is_empty() {
        return Err(Unsure);
    }
    converted.push_str(&text[cursor..]);
    let Some(mut value) = parse_with(&converted, Dates::Inventory)? else {
        return Ok(None);
    };
    fn restore(value: &mut Value, values: &std::collections::HashMap<String, (String, Value)>) {
        match value {
            Value::String(s) => {
                if let Some((_, v)) = values.get(s) {
                    *value = v.clone();
                }
            }
            Value::Array(a) => {
                for v in a {
                    restore(v, values)
                }
            }
            Value::Object(m) => {
                let old = std::mem::take(m);
                for (k, mut v) in old {
                    restore(&mut v, values);
                    let key = values.get(&k).map(|(raw, _)| raw.clone()).unwrap_or(k);
                    m.insert(key, v);
                }
            }
            _ => {}
        }
    }
    restore(&mut value, &values);
    Ok(Some(value))
}

/// `_read(path)` de un `.toml` con el lector dado: `Ok(None)` =
/// `ValueError('Configuración ilegible…')`.
fn read_toml_with(path: &Path, dates: Dates) -> Result<Option<Value>, Unsure> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        // `path.exists()` es falso con estos errores.
        Err(e)
            if e.kind() == io::ErrorKind::NotFound
                || e.kind() == io::ErrorKind::NotADirectory
                || e.raw_os_error() == Some(40) =>
        {
            return Ok(Some(json!({})));
        }
        Err(_) => return Ok(None),
    };
    if meta.len() > 4_000_000 {
        return Ok(None);
    }
    let Ok(bytes) = fs::read(path) else {
        return Ok(None);
    };
    let Some(text) = universal_text(bytes) else {
        return Ok(None);
    };
    parse_with(&text, dates)
}

/// `_read(path)` para la confianza de Codex (`parse_trust_toml`).
pub(crate) fn read_trust_toml(path: &Path) -> Result<Option<Value>, Unsure> {
    read_toml_with(path, Dates::Opaque)
}

// ---------------------------------------------------------------- manifest

fn uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

fn path_of(value: &Value) -> Result<PathBuf, Fail> {
    value.as_str().map(PathBuf::from).ok_or(Fail::Caught)
}

fn private_mode(meta: &fs::Metadata) -> bool {
    meta.uid() == uid() && meta.mode() & 0o077 == 0
}

fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// `Path(a).parent == Path(b).parent`.
fn same_parent(a: &Path, b: &Path) -> bool {
    let parent = |p: &Path| {
        let parent = p.parent().map(Path::to_path_buf).unwrap_or_default();
        if parent.as_os_str().is_empty() && !p.is_absolute() {
            PathBuf::from(".")
        } else {
            parent
        }
    };
    parent(a).components().eq(parent(b).components())
}

/// `_load_manifest(path, bundle)`.
fn load_manifest(path: &Path, bundle: Option<&Value>) -> Result<Value, Fail> {
    let meta = fs::metadata(path)?;
    if is_link(path) || !private_mode(&meta) {
        return Err(Fail::Value("Manifest privado inválido.".into()));
    }
    let text = universal_text(fs::read(path)?).ok_or(Fail::Unsure)?;
    let data = py_json(&text)?.map_err(Fail::Value)?;
    let Value::Object(map) = &data else {
        return Err(Fail::Attr);
    };
    let version_ok = map.get("version").is_some_and(|v| python_eq(v, &json!(1)));
    let bundle_ok = bundle.is_none_or(|b| map.get("bundle").is_some_and(|v| python_eq(v, b)));
    if !version_ok || !bundle_ok {
        return Err(Fail::Value(
            "Manifest incompatible con el lanzamiento.".into(),
        ));
    }
    let parent = path.parent().ok_or(Fail::Unsure)?;
    if !private_mode(&fs::metadata(parent)?) {
        return Err(Fail::Value("Directorio del manifest no es privado.".into()));
    }
    let artifacts = match map.get("artifacts") {
        None => Map::new(),
        Some(Value::Object(artifacts)) => artifacts.clone(),
        Some(_) => return Err(Fail::Attr),
    };
    for (name, digest) in &artifacts {
        let artifact = Path::new(name);
        let bad = !same_parent(artifact, path)
            || is_link(artifact)
            || fs::metadata(artifact)?.mode() & 0o077 != 0;
        if bad {
            return Err(Fail::Value("Artefacto privado inválido.".into()));
        }
        if !python_eq(&json!(sha256_hex(&fs::read(artifact)?)), digest) {
            return Err(Fail::Value(
                "El artefacto de lanzamiento cambió después de prepararse.".into(),
            ));
        }
    }
    Ok(data)
}

/// `re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*=.*', word, re.S)`.
fn assignment(word: &str) -> Option<(&str, &str)> {
    let (key, value) = word.split_once('=')?;
    let mut chars = key.chars();
    let first = chars.next()?;
    let ok = (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then_some((key, value))
}

/// `_resolve_command(data, command)` solo por sus excepciones: `wrap_command`
/// descarta lo que devuelve.
fn resolve_command(
    data: &Map<String, Value>,
    words: &[String],
    environ: &HashMap<String, String>,
) -> Result<(), Fail> {
    let mut command: Vec<String> = words.to_vec();
    let mut env = environ.clone();
    if command.first().is_some_and(|w| w == "env") {
        command.remove(0);
        while command.first().is_some_and(|w| w.starts_with('-')) {
            let flag = command.remove(0);
            if matches!(flag.as_str(), "-u" | "--unset") && !command.is_empty() {
                env.remove(&command.remove(0));
            } else if let Some(name) = flag.strip_prefix("--unset=") {
                env.remove(name);
            } else if flag == "--" {
                break;
            } else {
                return Err(Fail::Value(
                    "Opción env no soportada por el lanzamiento aislado.".into(),
                ));
            }
        }
    }
    while let Some((key, value)) = command.first().and_then(|w| assignment(w)) {
        env.insert(key.to_owned(), value.to_owned());
        command.remove(0);
    }
    if command.is_empty() {
        return Err(Fail::Value(
            "El comando intenta cambiar HOME o está vacío.".into(),
        ));
    }
    let home = data.get("home").ok_or(Fail::Caught)?;
    let ours = env.get("HOME").map_or(Value::Null, |h| json!(h));
    if !python_eq(&ours, home) {
        return Err(Fail::Value(
            "El comando intenta cambiar HOME o está vacío.".into(),
        ));
    }
    let extra: Vec<Value> = match data.get("args").ok_or(Fail::Caught)? {
        Value::Array(items) => items.clone(),
        _ => return Err(Fail::Unsure),
    };
    let harness = match data.get("bundle").ok_or(Fail::Caught)? {
        Value::Object(bundle) => bundle.get("harness").ok_or(Fail::Caught)?.clone(),
        _ => return Err(Fail::Unsure),
    };
    let env_spec = data.get("env").ok_or(Fail::Caught)?;
    if harness == json!("opencode") {
        resolve_opencode(&env, env_spec)?;
    } else if !env_spec.is_object() {
        return Err(Fail::Unsure);
    }
    if harness == json!("codex") {
        resolve_codex(data, &command, &extra)?;
    }
    if harness == json!("claude") {
        resolve_claude(&command, &extra)?;
    }
    Ok(())
}

fn resolve_opencode(env: &HashMap<String, String>, env_spec: &Value) -> Result<(), Fail> {
    let invalid = || Fail::Value("Overlay OpenCode inválido.".into());
    let content = env
        .get("OPENCODE_CONFIG_CONTENT")
        .map_or("{}", String::as_str);
    let prior = py_json(content)?.map_err(|_| invalid())?;
    if !prior.is_object() {
        return Err(Fail::Attr);
    }
    let selected = match env_spec {
        Value::Object(map) => match map.get("OPENCODE_CONFIG_CONTENT") {
            Some(Value::String(text)) => py_json(text)?.map_err(|_| invalid())?,
            Some(_) => return Err(invalid()),
            None => return Err(Fail::Caught),
        },
        // `data['env'][…]` sobre una lista o una cadena: `TypeError`.
        Value::Array(_) | Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => {
            return Err(invalid());
        }
    };
    if !selected.is_object() {
        return Err(Fail::Attr);
    }
    Ok(())
}

fn resolve_codex(
    data: &Map<String, Value>,
    command: &[String],
    extra: &[Value],
) -> Result<(), Fail> {
    let mut old = Value::Array(Vec::new());
    for (index, word) in command.iter().enumerate() {
        let value = if matches!(word.as_str(), "-c" | "--config") && index + 1 < command.len() {
            command.get(index + 1).map_or("", String::as_str)
        } else {
            word.strip_prefix("--config=").unwrap_or("")
        };
        if value.starts_with("skills.config=") {
            let invalid = || Fail::Value("Override previo de skills inválido.".into());
            let parsed = parse_toml(value)?.ok_or_else(invalid)?;
            old = parsed
                .get("skills")
                .and_then(|s| s.get("config"))
                .cloned()
                .ok_or_else(invalid)?;
        }
    }
    if !truthy(&old) {
        return Ok(());
    }
    for word in extra {
        let Value::String(word) = word else {
            return Err(Fail::Attr);
        };
        if !word.starts_with("skills.config=") {
            continue;
        }
        let parsed = parse_toml(word)?
            .ok_or_else(|| Fail::Value("Configuración TOML no soportada.".into()))?;
        let rules = parsed
            .get("skills")
            .and_then(|s| s.get("config"))
            .ok_or(Fail::Caught)?;
        let count = data.get("codexSelectedSkillCount").ok_or(Fail::Caught)?;
        if count.as_u64().is_none() {
            return Err(Fail::Unsure);
        }
        // `before + old + selected`: listas, o `TypeError`.
        if !rules.is_array() || !old.is_array() {
            return Err(Fail::Caught);
        }
    }
    Ok(())
}

fn resolve_claude(command: &[String], extra: &[Value]) -> Result<(), Fail> {
    let unreadable = || Fail::Value("Settings de Claude ilegibles.".into());
    let mut index = 0;
    while let Some(word) = command.get(index) {
        index += 1;
        if word == "--settings" || word.starts_with("--settings=") {
            let value = if word == "--settings" {
                let value = command
                    .get(index)
                    .ok_or_else(|| Fail::Value("Settings de Claude incompletos.".into()))?;
                index += 1;
                value.clone()
            } else {
                word.split_once('=').map_or("", |(_, v)| v).to_owned()
            };
            let text = if strip_start(&value).starts_with('{') {
                value
            } else {
                let bytes = fs::read(Path::new(if value.is_empty() { "." } else { &value }))
                    .map_err(|_| unreadable())?;
                universal_text(bytes).ok_or_else(unreadable)?
            };
            let old = py_json(&text)?.map_err(|_| unreadable())?;
            if !old.is_object() {
                // `_deep_merge(…, old)`: `old.items()` fuera del `except`.
                return Err(Fail::Attr);
            }
        } else if word == "--mcp-config" {
            while command.get(index).is_some_and(|w| !w.starts_with('-')) {
                index += 1;
            }
        }
    }
    let Some(at) = extra
        .iter()
        .position(|w| python_eq(w, &json!("--settings")))
    else {
        return Err(Fail::Value("'--settings' is not in list".into()));
    };
    let path = path_of(extra.get(at + 1).ok_or(Fail::Caught)?)?;
    let text = universal_text(fs::read(&path)?).ok_or(Fail::Unsure)?;
    let selected = py_json(&text)?.map_err(Fail::Value)?;
    if !selected.is_object() {
        return Err(Fail::Attr);
    }
    Ok(())
}

/// `s.lstrip()` de Python.
fn strip_start(s: &str) -> &str {
    s.trim_start_matches(comandos_core::text::is_space)
}

/// `wrap_command(command, launch)`: envuelve un comando simple, conservando
/// su argv y su prefijo de entorno. `environ` es `os.environ` del proceso.
pub fn wrap_command(
    command: &str,
    launch: &Value,
    helper: &Path,
    environ: &HashMap<String, String>,
) -> Result<String, LaunchError> {
    let words = shlex_split(command).map_err(LaunchError::Value)?;
    let compound = words
        .iter()
        .any(|w| matches!(w.as_str(), ";" | "&&" | "||" | "|" | ">" | "<" | "&"));
    if words.is_empty() || compound {
        return Err(LaunchError::Value(
            "Se requiere un comando simple para aplicar extensiones.".into(),
        ));
    }
    let Value::Object(bundle) = launch else {
        return Err(LaunchError::Other);
    };
    let manifest = bundle.get("manifest").ok_or(LaunchError::Other)?;
    let manifest = manifest.as_str().ok_or(LaunchError::Other)?;
    let data = load_manifest(Path::new(manifest), Some(launch))?;
    let Value::Object(data) = data else {
        return Err(LaunchError::Other);
    };
    resolve_command(&data, &words, environ)?;
    let helper = helper.to_str().ok_or(LaunchError::Unsure)?;
    let mut argv = vec![
        helper.to_owned(),
        "--manifest".into(),
        manifest.to_owned(),
        "--".into(),
    ];
    argv.extend(words);
    Ok(shlex_join(&argv))
}

/// `wrap_environment(command, reference)`.
pub fn wrap_environment(
    command: &str,
    reference: &Value,
    helper: &Path,
) -> Result<String, LaunchError> {
    let field = |key: &str| {
        reference
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(LaunchError::Other)
    };
    let helper = helper.to_str().ok_or(LaunchError::Unsure)?;
    let mut argv = vec![
        helper.to_owned(),
        "--environment-file".into(),
        field("path")?,
        "--environment-sha256".into(),
        field("sha256")?,
        "--".into(),
    ];
    argv.extend(shlex_split(command).map_err(LaunchError::Value)?);
    Ok(shlex_join(&argv))
}

/// `uuid.uuid4().hex`.
fn uuid4_hex() -> Result<String, Unsure> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| Unsure)?;
    if let Some(b) = bytes.get_mut(6) {
        *b = (*b & 0x0f) | 0x40;
    }
    if let Some(b) = bytes.get_mut(8) {
        *b = (*b & 0x3f) | 0x80;
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// `_private(path, data)`: creación exclusiva 0600, escritura y `fsync`.
fn write_private(path: &Path, raw: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(raw)?;
    file.flush()?;
    file.sync_all()
}

/// `capture_opencode_environment(source, runtime_dir, inventory, managed)`:
/// guarda solo los overrides admitidos de OpenCode en
/// `<runtime_dir>/environment-<uuid>.json` (0600) y devuelve `{path, sha256}`.
/// `source` es el entorno del proceso (bytes, como `/proc/<pid>/environ`).
pub fn capture_opencode_environment(
    source: &HashMap<Vec<u8>, Vec<u8>>,
    runtime_dir: &Path,
    inventory: &Value,
    managed: bool,
) -> Result<Value, LaunchError> {
    let mut values = Map::new();
    for key in OPENCODE_ENV_KEYS {
        if let Some(raw) = source.get(key.as_bytes()) {
            // `UnicodeDecodeError` sale con su propio texto.
            let text = String::from_utf8(raw.clone()).map_err(|_| LaunchError::Unsure)?;
            values.insert(key.into(), json!(text));
        }
    }
    let set = |key: &str| values.get(key).is_some_and(truthy);
    if set("OPENCODE_CONFIG") || set("OPENCODE_CONFIG_DIR") {
        return Err(LaunchError::Value(
            "OpenCode usa un catálogo alternativo no compatible; el agente sigue abierto".into(),
        ));
    }
    opencode_overrides(&values, inventory, managed)?;
    let directory = runtime_dir;
    if !directory.is_dir() {
        if let Some(parent) = directory.parent() {
            fs::create_dir_all(parent).map_err(|_| LaunchError::Other)?;
        }
        match fs::DirBuilder::new().mode(0o700).create(directory) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && directory.is_dir() => {}
            Err(_) => return Err(LaunchError::Other),
        }
    }
    let meta = fs::metadata(directory).map_err(|_| LaunchError::Other)?;
    if !private_mode(&meta) {
        return Err(LaunchError::Value(
            "Directorio de entorno no privado".into(),
        ));
    }
    let path = directory.join(format!("environment-{}.json", uuid4_hex()?));
    let body = response_dumps(&json!({"version": 1, "values": values})).map_err(|_| Unsure)?;
    write_private(&path, body.as_bytes()).map_err(|_| LaunchError::Other)?;
    let digest = sha256_hex(&fs::read(&path).map_err(|_| LaunchError::Other)?);
    let path = path.to_str().ok_or(LaunchError::Unsure)?;
    Ok(json!({"path": path, "sha256": digest}))
}

/// El bloque `try` de `capture_opencode_environment`.
fn opencode_overrides(
    values: &Map<String, Value>,
    inventory: &Value,
    managed: bool,
) -> Result<(), LaunchError> {
    let refused = || {
        LaunchError::Value(
            "OpenCode contiene overrides no compatibles; el agente sigue abierto".into(),
        )
    };
    let text = |key: &str| values.get(key).and_then(Value::as_str).unwrap_or("{}");
    let content = py_json(text("OPENCODE_CONFIG_CONTENT"))?.map_err(|_| refused())?;
    let Value::Object(content) = content else {
        return Err(refused());
    };
    // `{row['id'] for row in inventory['mcps']}`.
    let rows = match inventory.get("mcps") {
        Some(Value::Array(rows)) => rows,
        Some(_) => return Err(LaunchError::Unsure),
        None => return Err(LaunchError::Other),
    };
    let mut known = Vec::new();
    for row in rows {
        match row {
            Value::Object(row) => known.push(row.get("id").ok_or(LaunchError::Other)?.clone()),
            _ => return Err(LaunchError::Unsure),
        }
    }
    // `set(content.get('mcp', {})) - known`: cualquier fallo es el mismo texto.
    let members: Vec<Value> = match content.get("mcp") {
        None => Vec::new(),
        Some(Value::Object(map)) => map.keys().map(|k| json!(k)).collect(),
        Some(Value::Array(items)) => items.clone(),
        Some(Value::String(s)) => s.chars().map(|c| json!(c.to_string())).collect(),
        Some(_) => return Err(refused()),
    };
    if members
        .iter()
        .any(|m| !m.is_string() || !known.iter().any(|k| python_eq(k, m)))
    {
        return Err(refused());
    }
    let get = |key: &str| content.get(key).is_some_and(truthy);
    if get("plugin") || ((get("skills") || get("mcp")) && !managed) {
        return Err(refused());
    }
    let permission = py_json(text("OPENCODE_PERMISSION"))?.map_err(|_| refused())?;
    if !(permission.is_object() || permission.is_string()) {
        return Err(refused());
    }
    Ok(())
}

// ---------------------------------------------------------------- procesos

/// `_process_env(pid)`: el último valor de cada clave, en bytes.
fn process_env(pid: u32) -> io::Result<HashMap<Vec<u8>, Vec<u8>>> {
    let raw = fs::read(format!("/proc/{pid}/environ"))?;
    let mut env = HashMap::new();
    for entry in raw.split(|b| *b == 0) {
        if let Some(at) = entry.iter().position(|b| *b == b'=') {
            env.insert(
                entry.get(..at).unwrap_or_default().to_vec(),
                entry.get(at + 1..).unwrap_or_default().to_vec(),
            );
        }
    }
    Ok(env)
}

/// `env.get(key) != value` con `value` de JSON (`None` = clave ausente).
fn env_differs(env: &HashMap<Vec<u8>, Vec<u8>>, key: &str, value: &Value) -> bool {
    match (env.get(key.as_bytes()), value) {
        (None, Value::Null) => false,
        (Some(ours), Value::String(theirs)) => ours.as_slice() != theirs.as_bytes(),
        _ => true,
    }
}

/// `verify_launch(pid, launch)`: evidencia de configuración del proceso (no
/// conexiones MCP). `Err(Unsure)` = un `AttributeError` que el Python no traga.
pub fn verify_launch(pid: u32, launch: &Value) -> Result<bool, Unsure> {
    match verify(pid, launch) {
        Ok(ok) => Ok(ok),
        Err(Fail::Value(_) | Fail::Caught) => Ok(false),
        Err(Fail::Attr | Fail::Unsure) => Err(Unsure),
    }
}

fn verify(pid: u32, launch: &Value) -> Result<bool, Fail> {
    let Value::Object(bundle) = launch else {
        return Err(Fail::Caught);
    };
    let manifest_value = bundle.get("manifest").ok_or(Fail::Caught)?;
    let manifest = path_of(manifest_value)?;
    let Value::Object(data) = load_manifest(&manifest, Some(launch))? else {
        return Err(Fail::Attr);
    };
    let env = process_env(pid)?;
    let operation = bundle.get("operationId").ok_or(Fail::Caught)?;
    if env_differs(&env, MARKER, operation) || env_differs(&env, MANIFEST_ENV, manifest_value) {
        return Ok(false);
    }
    let digest = sha256_hex(&fs::read(&manifest)?);
    if let Some(artifacts) = data.get("artifacts") {
        let Value::Object(artifacts) = artifacts else {
            return Err(Fail::Attr);
        };
        for (path, hash) in artifacts {
            if !python_eq(&json!(sha256_hex(&fs::read(path)?)), hash) {
                return Ok(false);
            }
        }
    }
    let home = data.get("home").ok_or(Fail::Caught)?;
    if env_differs(&env, DIGEST_ENV, &json!(digest)) || env_differs(&env, "HOME", home) {
        return Ok(false);
    }
    let receipt_path = PathBuf::from(OsStr::from_bytes(
        env.get(RECEIPT_ENV.as_bytes())
            .map_or(&[][..], Vec::as_slice),
    ));
    let receipt_path = if receipt_path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        receipt_path
    };
    if !same_parent(&receipt_path, &manifest) {
        return Ok(false);
    }
    let meta = fs::metadata(&receipt_path)?;
    if is_link(&receipt_path) || !private_mode(&meta) {
        return Ok(false);
    }
    let receipt_raw = fs::read(&receipt_path)?;
    let receipt_hash = env.get(RECEIPT_HASH_ENV.as_bytes());
    if receipt_hash.map(Vec::as_slice) != Some(sha256_hex(&receipt_raw).as_bytes()) {
        return Ok(false);
    }
    let receipt = py_json_bytes(&receipt_raw)?.map_err(|_| Fail::Caught)?;
    let Value::Object(receipt) = receipt else {
        return Err(Fail::Attr);
    };
    if !receipt
        .get("manifestSha256")
        .is_some_and(|v| python_eq(v, &json!(digest)))
    {
        return Ok(false);
    }
    match receipt.get("env").ok_or(Fail::Caught)? {
        Value::Object(expected) => {
            if expected.iter().any(|(k, v)| env_differs(&env, k, v)) {
                return Ok(false);
            }
        }
        _ => return Err(Fail::Attr),
    }
    match receipt.get("artifacts").ok_or(Fail::Caught)? {
        Value::Object(artifacts) => {
            for (path, hash) in artifacts {
                if !python_eq(&json!(sha256_hex(&fs::read(path)?)), hash) {
                    return Ok(false);
                }
            }
        }
        _ => return Err(Fail::Attr),
    }
    let cmdline =
        String::from_utf8(fs::read(format!("/proc/{pid}/cmdline"))?).map_err(|_| Fail::Caught)?;
    let argv: Vec<&str> = cmdline.trim_end_matches('\0').split('\0').collect();
    match receipt.get("argv").ok_or(Fail::Caught)? {
        Value::Array(items) => {
            let expected = items.get(1..).unwrap_or_default();
            if !expected.is_empty() {
                let tail = argv
                    .get(argv.len().saturating_sub(expected.len())..)
                    .unwrap_or(&argv);
                let same = tail.len() == expected.len()
                    && tail
                        .iter()
                        .zip(expected)
                        .all(|(a, e)| python_eq(&json!(a), e));
                if !same {
                    return Ok(false);
                }
            }
        }
        // `receipt['argv'][1:]` de una cadena: otra cadena, nunca igual a la lista.
        Value::String(s) => {
            if s.chars().count() > 1 {
                return Ok(false);
            }
        }
        _ => return Err(Fail::Caught),
    }
    match data.get("mounts").ok_or(Fail::Caught)? {
        Value::Array(mounts) if !mounts.is_empty() => verify_mounts(pid, mounts),
        value if !truthy(value) => Ok(true),
        // Iterar un objeto, una cadena o un escalar acaba en `TypeError`.
        _ => Ok(false),
    }
}

/// La parte de montajes de `verify_launch`.
fn verify_mounts(pid: u32, mounts: &[Value]) -> Result<bool, Fail> {
    if fs::read_link(format!("/proc/{pid}/ns/mnt"))? == fs::read_link("/proc/self/ns/mnt")? {
        return Ok(false);
    }
    let info =
        String::from_utf8(fs::read(format!("/proc/{pid}/mountinfo"))?).map_err(|_| Fail::Caught)?;
    for mount in mounts {
        let Value::Object(mount) = mount else {
            return Err(Fail::Caught);
        };
        let target = mount
            .get("target")
            .ok_or(Fail::Caught)?
            .as_str()
            .ok_or(Fail::Attr)?;
        let escaped = target
            .replace('\\', "\\134")
            .replace(' ', "\\040")
            .replace('\t', "\\011")
            .replace('\n', "\\012");
        let mounted = splitlines(&info)
            .iter()
            .any(|line| line.split_whitespace().nth(4) == Some(escaped.as_str()));
        if !mounted {
            return Ok(false);
        }
        let child = PathBuf::from(format!("/proc/{pid}/root")).join(target.trim_start_matches('/'));
        if mount.get("kind") == Some(&json!("directory")) {
            let source = path_of(mount.get("source").ok_or(Fail::Caught)?)?;
            let (original, inside) = (fs::metadata(source)?, fs::metadata(&child)?);
            if (original.dev(), original.ino()) != (inside.dev(), inside.ino()) {
                return Ok(false);
            }
        } else {
            let expected = mount.get("sha256").ok_or(Fail::Caught)?;
            if !python_eq(&json!(sha256_hex(&fs::read(&child)?)), expected) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// `configuration_status(pid, verified)`: un proceso externo corriente no es
/// un lanzamiento gestionado fallido.
pub fn configuration_status(pid: Option<u32>, verified: bool) -> &'static str {
    if verified {
        return "verified";
    }
    let Some(pid) = pid.filter(|p| *p != 0) else {
        return "not_started";
    };
    match process_env(pid) {
        Ok(env) => {
            let set = |key: &str| env.get(key.as_bytes()).is_some_and(|v| !v.is_empty());
            if set(MANIFEST_ENV) || set(MARKER) {
                "unverified"
            } else {
                "external"
            }
        }
        Err(_) => "unknown",
    }
}

/// `launch_from_pid(pid)`: el paquete saneado, solo si la evidencia viva lo
/// verifica.
pub fn launch_from_pid(pid: u32) -> Result<Option<Value>, Unsure> {
    let Ok(env) = process_env(pid) else {
        return Ok(None);
    };
    let Some(path) = env.get(MANIFEST_ENV.as_bytes()).filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    let Ok(path) = std::str::from_utf8(path) else {
        return Err(Unsure);
    };
    let bundle = match load_manifest(Path::new(path), None) {
        Ok(Value::Object(data)) => match data.get("bundle") {
            Some(bundle) => bundle.clone(),
            None => return Ok(None),
        },
        Ok(_) | Err(Fail::Attr | Fail::Unsure) => return Err(Unsure),
        Err(Fail::Value(_) | Fail::Caught) => return Ok(None),
    };
    Ok(verify_launch(pid, &bundle)?.then_some(bundle))
}

// ------------------------------------------------- caminos aún sin portar

/// Lo que decide, antes del `claim`, si una operación llegaría a
/// `inventory`/`_internal_inventory`/`prepare_launch` (aún sin portar).
#[derive(Debug, Clone, Copy)]
pub struct InventoryNeed<'a> {
    /// `request.extensionsOnly`: `PaneExtensionConfiguration.prepare`
    /// (`bin/cc-dash:3332`) llama a `prepare_launch` siempre.
    pub extensions_only: bool,
    /// El harness del agente original (`frm`): con `opencode`, `snapshot`
    /// (`:3075`) pide `inventory` para `capture_opencode_environment`.
    pub from: &'a str,
    /// `plan['to']`.
    pub to: &'a str,
    /// `plan['sameConversation']`.
    pub same_conversation: bool,
    /// `plan['unchanged']`: con él, `_preserve_extension_plan` (`:2712`)
    /// reutiliza el lanzamiento verificado sin inventario.
    pub unchanged: bool,
    /// `adapter.original['pid']`.
    pub original_pid: Option<u32>,
    /// `plan['returnOrigin']['extensionLaunch']`.
    pub return_origin_launch: Option<&'a Value>,
}

/// `true` si la operación pasaría por un camino que necesita
/// `inventory`/`prepare_launch`: quien llama declina ANTES del `claim`.
///
/// Reproduce la decisión de `_preserve_extension_plan`: `prior` es el
/// lanzamiento de `returnOrigin` o, con la misma conversación y un pid, el
/// que `launch_from_pid` verifica. Si el proceso tiene
/// `COMANDOS_EXTENSION_MANIFEST` pero no verifica, el Python lanza
/// `ValueError('no se pudo verificar el lanzamiento original; el agente sigue
/// abierto')` sin inventario: eso lo porta el adaptador, aquí es `false`.
/// `Unsure` = `launch_from_pid` dudoso o un `prior` sin `harness`.
pub fn needs_inventory(need: &InventoryNeed<'_>) -> Result<bool, Unsure> {
    if need.extensions_only || need.from == "opencode" {
        return Ok(true);
    }
    let mut prior = need.return_origin_launch.filter(|v| truthy(v)).cloned();
    if need.same_conversation
        && let Some(pid) = need.original_pid.filter(|p| *p != 0)
    {
        prior = launch_from_pid(pid)?;
    }
    let Some(prior) = prior.filter(truthy) else {
        return Ok(false);
    };
    let harness = prior.get("harness").ok_or(Unsure)?;
    if !python_eq(harness, &json!(need.to)) {
        return Ok(false);
    }
    Ok(!need.unchanged)
}

/// La versión prudente de `needs_inventory` para cuando el plan aún no se
/// conoce: declina con `extensionsOnly`, origen `opencode`, un
/// `returnOrigin.extensionLaunch` o un proceso original con huella de
/// lanzamiento gestionado (`COMANDOS_EXTENSION_MANIFEST` u
/// `COMANDOS_EXTENSION_OPERATION_ID`), verificado o no.
pub fn may_need_inventory(
    extensions_only: bool,
    from: &str,
    original_pid: Option<u32>,
    return_origin_launch: Option<&Value>,
) -> bool {
    extensions_only
        || from == "opencode"
        || return_origin_launch.is_some_and(truthy)
        || original_pid
            .filter(|p| *p != 0)
            .is_some_and(|pid| configuration_status(Some(pid), false) != "external")
}

// ---------------------------------------------------------------- selección

/// `_normalize(inv, selection)`: la selección completa (solo filas con estado
/// booleano), validada contra el inventario.
pub fn normalize(inventory: &Value, selection: &Value) -> Result<Value, LaunchError> {
    let Value::Object(selection) = selection else {
        return Err(LaunchError::Value(
            "Selección de extensiones inválida.".into(),
        ));
    };
    if selection.keys().any(|k| k != "mcps" && k != "skills") {
        return Err(LaunchError::Value(
            "Selección de extensiones inválida.".into(),
        ));
    }
    let unknown =
        || LaunchError::Value("Selección desconocida o estado de extensión inválido.".into());
    let mut result = Map::new();
    for kind in ["mcps", "skills"] {
        let empty = Value::Object(Map::new());
        let supplied = selection.get(kind).unwrap_or(&empty);
        let Value::Array(list) = inventory.get(kind).ok_or(LaunchError::Other)? else {
            return Err(LaunchError::Unsure);
        };
        let mut rows: Map<String, Value> = Map::new();
        for row in list {
            let id = row.get("id").ok_or(LaunchError::Other)?;
            rows.insert(
                id.as_str().ok_or(LaunchError::Unsure)?.to_owned(),
                row.clone(),
            );
        }
        let Value::Object(supplied) = supplied else {
            return Err(unknown());
        };
        if supplied.keys().any(|k| !rows.contains_key(k))
            || supplied.values().any(|v| !v.is_boolean())
        {
            return Err(unknown());
        }
        for (ident, value) in supplied {
            let row = rows.get(ident).ok_or(LaunchError::Other)?;
            let enabled = row.get("enabled").ok_or(LaunchError::Other)?;
            let toggleable = row.get("toggleable").ok_or(LaunchError::Other)?;
            if !truthy(toggleable) && !python_eq(value, enabled) {
                let name = row.get("name").ok_or(LaunchError::Other)?;
                let name = name.as_str().ok_or(LaunchError::Other)?;
                return Err(LaunchError::Value(format!(
                    "Esta extensión no admite cambios aislados: {name}"
                )));
            }
        }
        let mut chosen = Map::new();
        for (ident, row) in &rows {
            let enabled = row.get("enabled").ok_or(LaunchError::Other)?;
            if enabled.is_boolean() {
                chosen.insert(
                    ident.clone(),
                    supplied.get(ident).unwrap_or(enabled).clone(),
                );
            }
        }
        result.insert(kind.into(), Value::Object(chosen));
    }
    Ok(Value::Object(result))
}

fn discovery_fault(e: crate::capabilities::Fault) -> LaunchError {
    match e {
        crate::capabilities::Fault::Invalid(s) => LaunchError::Value(s),
        crate::capabilities::Fault::Uncertain(_) => LaunchError::Unsure,
        crate::capabilities::Fault::Io(_) => LaunchError::Other,
    }
}
/// The launch reader uses the full object, unlike capabilities' public TOML projection.
fn read_configuration(path: &Path) -> Result<Value, LaunchError> {
    let error = || {
        LaunchError::Value(
            "Configuración ilegible; no se puede preparar el lanzamiento aislado.".into(),
        )
    };
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if matches!(e.raw_os_error(), Some(2 | 20 | 40)) => return Ok(json!({})),
        Err(_) => return Err(error()),
    };
    if !meta.is_file() || meta.len() > 4_000_000 {
        return Err(error());
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| error())?;
    if !file.metadata().map_err(|_| error())?.is_file() {
        return Err(error());
    }
    let mut bytes = Vec::new();
    std::io::Read::take(file, 4_000_001)
        .read_to_end(&mut bytes)
        .map_err(|_| error())?;
    if bytes.len() > 4_000_000 {
        return Err(error());
    }
    let text = universal_text(bytes).ok_or_else(error)?;
    let value = if path.extension() == Some(OsStr::new("toml")) {
        parse_toml(&text).map_err(|_| LaunchError::Unsure)?
    } else {
        crate::capabilities::jsonc(&text)
    };
    value.filter(Value::is_object).ok_or_else(error)
}
/// Unsanitized inventory retained privately for isolated launch preparation.
pub fn internal_inventory(
    registry: &Value,
    harness: &str,
    account: &str,
    cwd: &Path,
    paths: &crate::capabilities::Paths,
) -> Result<(crate::capabilities::Context, Value), LaunchError> {
    use crate::{capabilities as c, session_profiles as p};
    if !["claude", "codex", "grok", "opencode", "agy"].contains(&harness) {
        return Err(LaunchError::Value(
            "CLI sin adaptador de extensiones por proceso.".into(),
        ));
    }
    let mut ctx =
        c::configuration(registry, harness, account, cwd, paths).map_err(discovery_fault)?;
    let base = c::from_context(&mut ctx, paths);
    let mut skills = p::skills(&mut ctx, &json!({"skills":{"supported":true}}), paths);
    if ctx.uncertain {
        return Err(LaunchError::Unsure);
    }
    let mut mcps = c::list(&base["mcps"]);
    for row in &mut mcps {
        row["id"] = row["name"].clone();
    }
    let catalog = read_configuration(&paths.home.join(".config/comandos/extensions/catalog.json"))?;
    let shared = c::object(&catalog["servers"]);
    // Desviación del Python (15-oct): OpenCode y agy reciben solo el hub bajo demanda
    // (`cc-extensions hub`, declarado como `comandos`). Con el hub configurado, cada
    // servidor del catálogo sigue en el selector pero apagado por omisión: el hub ya lo
    // alcanza sin cargar sus herramientas, y encenderlo en un panel lo carga completo.
    let hub = matches!(harness, "opencode" | "agy")
        && mcps
            .iter()
            .any(|r| r["name"] == comandos_extensions::hub::NAME);
    for (name, spec) in &shared {
        if !mcps.iter().any(|r| r["name"] == *name)
            && !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            if !spec.is_object() {
                return Err(LaunchError::Other);
            }
            mcps.push(json!({"id":name,"name":name,"enabled":spec["enabled"]!=false&&!hub,"scope":"shared","source":"shared-catalog","synthetic":true}));
        }
    }
    if harness == "grok" {
        for row in &mut mcps {
            if c::list(&ctx.settings["disabled_mcp_servers"]).contains(&row["name"]) {
                row["enabled"] = false.into();
            }
        }
    }
    if harness == "agy" {
        let home = ctx.home.as_ref().ok_or(LaunchError::Other)?;
        let cfg = read_configuration(&home.join("config/skills.json"))?;
        for row in &mut skills {
            if c::list(&cfg["exclude"]).contains(&row["name"]) {
                row["enabled"] = false.into();
            }
        }
    }
    let shared_root = c::resolved(&paths.home.join(".agents/skills"));
    for row in &mut skills {
        let name = row["name"].as_str().ok_or(LaunchError::Other)?.to_owned();
        let plugin = row["plugin"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if harness == "claude" && plugin.is_none() && ctx.settings["skillOverrides"][&name] == "off"
        {
            row["enabled"] = false.into();
        }
        if harness == "opencode" {
            let permissions = &ctx.settings["permission"]["skill"];
            if permissions == "deny"
                || permissions
                    .get(&name)
                    .or_else(|| permissions.get("*"))
                    .is_some_and(|v| v == "deny")
            {
                row["enabled"] = false.into();
            }
        }
        let path = Path::new(row["path"].as_str().ok_or(LaunchError::Other)?);
        let id = if path.starts_with(&shared_root) && plugin.is_none() {
            format!("shared:{name}")
        } else if let Some(plugin) = &plugin {
            format!("plugin-skill:{plugin}:{name}")
        } else {
            format!("skill:{}", row["id"].as_str().ok_or(LaunchError::Other)?)
        };
        row["id"] = id.into();
        row["toggleable"] = (row["status"] != "shadowed").into();
        row["reason"] = "".into();
        if plugin.is_some() && harness == "claude" {
            row["toggleable"] = false.into();
            row["reason"] = "Se controla mediante el grupo del plugin completo.".into();
        } else if plugin.is_some() && row["enabled"] != true {
            row["toggleable"] = false.into();
            row["reason"] = "El plugin está desactivado o su carga no está confirmada.".into();
        }
    }
    if harness == "claude" {
        for plugin in &ctx.plugins {
            skills.push(json!({"id":format!("plugin:{}",plugin.key),"name":plugin.name,"plugin":plugin.key,"group":true,"scope":plugin.scope,"source":plugin.source,"enabled":plugin.enabled,"toggleable":plugin.enabled.is_boolean(),"reason":"Activa o desactiva todas las skills y hooks del plugin."}));
        }
    }
    for row in &mut mcps {
        let name = row["name"].as_str().ok_or(LaunchError::Other)?.to_owned();
        if matches!(harness, "codex" | "claude")
            && !truthy(&row["plugin"])
            && (harness != "codex" || !name.contains('.'))
            && shared
                .get(&name)
                .is_some_and(|spec| spec["enabled"] != false)
        {
            // The shared catalog owns provider configuration. A stale CLI
            // disable is not a per-pane exclusion, and launches must use the
            // catalog proxy rather than retain an old private command.
            row["enabled"] = true.into();
            row["synthetic"] = true.into();
        }
        let toggle = !truthy(&row["plugin"]) && !truthy(&row["unavailable"]);
        row["toggleable"] = toggle.into();
        row["reason"] = if toggle {
            ""
        } else {
            "Declaración no controlable individualmente por este adaptador."
        }
        .into();
        if harness == "codex" && name.contains('.') {
            row["toggleable"] = false.into();
            row["reason"] = "Codex no admite componentes con puntos en sus overrides CLI.".into();
        }
        if shared.get(&name).is_some_and(|s| s["enabled"] == false) {
            row["enabled"] = false.into();
            row["toggleable"] = false.into();
            row["enforceDisabled"] = true.into();
            row["reason"] =
                "Desactivado en el catálogo compartido; no se activa por sesión.".into();
        }
    }
    for row in mcps.iter_mut().chain(skills.iter_mut()) {
        if !row["enabled"].is_boolean() {
            row["toggleable"] = false.into();
            row["reason"] = "Estado configurado desconocido; se conserva sin cambios.".into();
        }
        if !ctx.errors.is_empty() {
            row["toggleable"] = false.into();
            row["reason"] =
                "Inventario incompleto; corrige la configuración antes de cambiar extensiones."
                    .into();
        }
    }
    let mut limitations = ctx.limitations.clone();
    limitations.extend([json!("El catálogo previo puede permanecer en el historial al reanudar."),json!("La verificación comprueba configuración del proceso, no conexiones MCP ni disponibilidad del proveedor.")]);
    let inv = json!({"harness":harness,"account":account,"mcps":mcps,"skills":skills,"limitations":limitations,"status":if ctx.errors.is_empty(){"configured"}else{"incomplete"}});
    Ok((ctx, inv))
}
/// Sanitized catalog; no configuration contents or filesystem paths escape.
pub fn inventory(
    registry: &Value,
    harness: &str,
    account: &str,
    cwd: &Path,
    paths: &crate::capabilities::Paths,
) -> Result<Value, LaunchError> {
    use crate::capabilities as c;
    use comandos_extensions::metadata as m;
    let (ctx, mut inv) = internal_inventory(registry, harness, account, cwd, paths)?;
    let rows = c::list(&inv["skills"]);
    let cache_path = paths
        .env
        .get("TIKTOKEN_CACHE_DIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.home.join(".cache/comandos/tiktoken"));
    let counter = |texts: Vec<String>| {
        std::future::ready(
            comandos_extensions::tokenizer::offline_counts_at(&cache_path, &texts)
                .unwrap_or_else(|| vec![None; texts.len()]),
        )
    };
    // Retain only bounded file identities and counts, never tokenizer tables.
    type MetadataCaches = std::collections::BTreeMap<(PathBuf, PathBuf), m::SkillMetadataCache>;
    static CACHES: std::sync::OnceLock<std::sync::Mutex<MetadataCaches>> =
        std::sync::OnceLock::new();
    let mut caches = CACHES
        .get_or_init(|| std::sync::Mutex::new(MetadataCaches::new()))
        .lock()
        .map_err(|_| LaunchError::Unsure)?;
    let cache_key = (paths.home.clone(), cache_path.clone());
    if !caches.contains_key(&cache_key)
        && caches.len() >= 8
        && let Some(oldest) = caches.keys().next().cloned()
    {
        caches.remove(&oldest);
    }
    let cache = caches.entry(cache_key).or_default();
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let at = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64();
    let mut future = Box::pin(cache.measure(&rows, &paths.home, at, &counter));
    let mut task = std::task::Context::from_waker(std::task::Waker::noop());
    let details = match std::future::Future::poll(future.as_mut(), &mut task) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => return Err(LaunchError::Unsure),
    };
    drop(future);
    drop(caches);
    for row in inv["skills"].as_array_mut().ok_or(LaunchError::Other)? {
        let id = row["id"].as_str().ok_or(LaunchError::Other)?.to_owned();
        for (k, v) in c::object(&details[&id]) {
            row[k] = v;
        }
    }
    let catalog = read_configuration(&paths.home.join(".config/comandos/extensions/catalog.json"))?;
    let shared = c::object(&catalog["servers"]);
    let mut native = Map::new();
    let key = if ["codex", "grok"].contains(&harness) {
        "mcp_servers"
    } else if harness == "opencode" {
        "mcp"
    } else {
        "mcpServers"
    };
    for layer in &ctx.layers {
        if let Ok(data) = read_configuration(&layer.path) {
            native.extend(c::object(&data[key]));
        }
    }
    for row in inv["mcps"].as_array_mut().ok_or(LaunchError::Other)? {
        let name = row["name"].as_str().ok_or(LaunchError::Other)?.to_owned();
        let scope = row["scope"].as_str().unwrap_or("unknown");
        let source = row["source"].as_str().unwrap_or("unknown");
        let label = match scope {
            "shared" => "Catálogo compartido".to_owned(),
            "project" => "Configuración del proyecto".to_owned(),
            "user" => "Configuración de la cuenta".to_owned(),
            "local" => "Configuración local".to_owned(),
            _ => format!("Configuración: {source}"),
        };
        let origin = if truthy(&row["plugin"]) {
            json!({"id":format!("plugin:{}",row["plugin"].as_str().ok_or(LaunchError::Other)?),"label":row["plugin"],"kind":"plugin"})
        } else {
            json!({"id":format!("config:{scope}:{source}"),"label":label,"kind":"configuration"})
        };
        row["origin"] = origin;
        row["size"] = m::unknown_size("tool-definitions");
        let spec = native
            .get(&name)
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let argv = if spec["command"].is_array() {
            c::list(&spec["command"])
        } else {
            let mut a = vec![spec["command"].clone()];
            a.extend(c::list(&spec["args"]));
            a
        };
        if (truthy(&row["synthetic"])
            || argv
                == vec![
                    json!(paths.home.join(".local/bin/cc-extensions")),
                    json!("serve"),
                    json!(name),
                ])
            && !truthy(&row["plugin"])
            && let Some(spec) = shared.get(&name)
        {
            row["origin"] =
                json!({"id":"config:shared","label":"Catálogo compartido","kind":"configuration"});
            row["size"] = m::mcp_size(&paths.home, &name, spec, m::now());
        }
    }
    let fields = [
        "id",
        "name",
        "enabled",
        "toggleable",
        "reason",
        "scope",
        "source",
        "plugin",
        "group",
        "origin",
        "size",
    ];
    for kind in ["mcps", "skills"] {
        for row in inv[kind].as_array_mut().ok_or(LaunchError::Other)? {
            let mut clean = Map::new();
            for key in fields {
                if let Some(v) = row.get(key) {
                    clean.insert(key.into(), v.clone());
                }
            }
            *row = Value::Object(clean);
        }
    }
    Ok(inv)
}

fn compact_response(value: &Value) -> Result<String, LaunchError> {
    let text = response_dumps(value).map_err(|_| LaunchError::Unsure)?;
    let mut out = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for ch in text.chars() {
        if quoted {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
        } else if ch == '"' {
            quoted = true;
            out.push(ch);
        } else if ch != ' ' {
            out.push(ch);
        }
    }
    Ok(out)
}
fn launch_toml(value: &Value) -> Result<String, LaunchError> {
    match value {
        Value::Object(map) => Ok(format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| Ok(format!(
                    "{}={}",
                    response_dumps(&json!(k)).map_err(|_| LaunchError::Unsure)?,
                    launch_toml(v)?
                )))
                .collect::<Result<Vec<_>, LaunchError>>()?
                .join(",")
        )),
        Value::Array(items) => Ok(format!(
            "[{}]",
            items
                .iter()
                .map(launch_toml)
                .collect::<Result<Vec<_>, _>>()?
                .join(",")
        )),
        Value::Bool(b) => Ok(b.to_string()),
        Value::String(_) | Value::Number(_) => {
            response_dumps(value).map_err(|_| LaunchError::Unsure)
        }
        _ => Err(LaunchError::Value("Override TOML no soportado.".into())),
    }
}
fn proxy_entry(harness: &str, name: &str, home: &Path) -> Value {
    let launcher = home.join(".local/bin/cc-extensions");
    if harness == "opencode" {
        return json!({"type":"local","command":[launcher,"serve",name],"enabled":true});
    }
    let mut result = json!({"command":launcher,"args":["serve",name]});
    match harness {
        "claude" => result["type"] = "stdio".into(),
        "agy" => result["disabled"] = false.into(),
        "codex" | "grok" => result["startup_timeout_sec"] = 45.into(),
        _ => {}
    }
    result
}
const MCP_SESSION_BINDING: &str = "COMANDOS_MCP_SESSION_BINDING";

fn managed_proxy(spec: &Value) -> bool {
    let command = spec["command"].as_str().unwrap_or_default();
    let executable = Path::new(command).file_name().and_then(OsStr::to_str);
    let args = &spec["args"];
    match executable {
        Some("cc-extensions" | "comandos-extensions") => args[0] == "serve",
        Some("comandos") => args[0] == "ext" && args[1] == "serve",
        _ => false,
    }
}

fn bind_proxy(spec: &mut Value, binding: Option<&str>) -> Result<(), LaunchError> {
    if let Some(binding) = binding.filter(|_| managed_proxy(spec)) {
        object_mut(spec, "env")?.insert(MCP_SESSION_BINDING.into(), binding.into());
    }
    Ok(())
}
struct LaunchFiles {
    private: PathBuf,
    artifacts: Map<String, Value>,
    mounts: Vec<Value>,
}
impl LaunchFiles {
    fn file(&mut self, name: &str, data: &Value) -> Result<String, LaunchError> {
        let bytes = response_dumps(data).map_err(|_| LaunchError::Unsure)?;
        self.bytes(name, bytes.as_bytes())
    }
    fn bytes(&mut self, name: &str, bytes: &[u8]) -> Result<String, LaunchError> {
        let path = self.private.join(name);
        write_private(&path, bytes).map_err(|_| LaunchError::Other)?;
        let key = path.to_str().ok_or(LaunchError::Unsure)?.to_owned();
        self.artifacts.insert(key.clone(), sha256_hex(bytes).into());
        Ok(key)
    }
    fn mount(&mut self, target: &Path, bytes: &[u8]) -> Result<(), LaunchError> {
        if !target.is_file() || is_link(target) {
            return Err(LaunchError::Value(
                "Falta un archivo de configuración montable; no se modifica el archivo global."
                    .into(),
            ));
        }
        let suffix = target
            .extension()
            .and_then(OsStr::to_str)
            .map(|s| format!(".{s}"))
            .unwrap_or_default();
        let source = self.bytes(&format!("mount-{}{suffix}", self.mounts.len()), bytes)?;
        let target = target.canonicalize().map_err(|_| LaunchError::Other)?;
        self.mounts
            .push(json!({"source":source,"target":target,"sha256":sha256_hex(bytes)}));
        Ok(())
    }
    fn mount_json(&mut self, target: &Path, data: &Value) -> Result<(), LaunchError> {
        let text = response_dumps(data).map_err(|_| LaunchError::Unsure)?;
        self.mount(target, text.as_bytes())
    }
}
fn object_mut<'a>(
    value: &'a mut Value,
    key: &str,
) -> Result<&'a mut Map<String, Value>, LaunchError> {
    if value.get(key).is_none() {
        value[key] = json!({});
    }
    value
        .get_mut(key)
        .and_then(Value::as_object_mut)
        .ok_or(LaunchError::Other)
}
fn sorted_selection(previous: &Value, selected: &Map<String, Value>) -> Result<Value, LaunchError> {
    let mut names = std::collections::BTreeSet::new();
    if let Some(items) = previous.as_array() {
        for item in items {
            names.insert(item.as_str().ok_or(LaunchError::Other)?.to_owned());
        }
    }
    for (name, enabled) in selected {
        names.remove(name);
        if enabled == false {
            names.insert(name.clone());
        }
    }
    Ok(json!(names.into_iter().collect::<Vec<_>>()))
}
fn copy_skill_tree(
    source: &Path,
    destination: &Path,
    seen: &mut std::collections::HashSet<PathBuf>,
    budget: &mut usize,
) -> Result<(), LaunchError> {
    use std::os::unix::fs::PermissionsExt;
    if *budget == 0 {
        return Err(LaunchError::Unsure);
    }
    *budget -= 1;
    let source = source.canonicalize().map_err(|_| LaunchError::Other)?;
    if source.is_dir() {
        if !seen.insert(source.clone()) {
            return Err(LaunchError::Unsure);
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(destination)
            .map_err(|_| LaunchError::Other)?;
        for entry in fs::read_dir(&source).map_err(|_| LaunchError::Other)? {
            let entry = entry.map_err(|_| LaunchError::Other)?;
            copy_skill_tree(
                &entry.path(),
                &destination.join(entry.file_name()),
                seen,
                budget,
            )?;
        }
        seen.remove(&source);
    } else if source.is_file() {
        fs::copy(source, destination).map_err(|_| LaunchError::Other)?;
        fs::set_permissions(destination, fs::Permissions::from_mode(0o600))
            .map_err(|_| LaunchError::Other)?;
    } else {
        return Err(LaunchError::Other);
    }
    Ok(())
}
/// Prepare private launch configuration. No shared configuration or live process is changed.
#[allow(clippy::too_many_arguments)]
pub fn prepare_launch(
    registry: &Value,
    harness: &str,
    account: &str,
    cwd: &Path,
    selection: &Value,
    runtime_dir: &Path,
    operation_id: &str,
    paths: &crate::capabilities::Paths,
) -> Result<Value, LaunchError> {
    prepare_launch_with_binding(
        registry,
        harness,
        account,
        cwd,
        selection,
        runtime_dir,
        operation_id,
        paths,
        None,
    )
}

/// Bind managed MCP proxies to a pane even when the CLI launches them through a daemon.
#[allow(clippy::too_many_arguments)]
pub fn prepare_launch_with_binding(
    registry: &Value,
    harness: &str,
    account: &str,
    cwd: &Path,
    selection: &Value,
    runtime_dir: &Path,
    operation_id: &str,
    paths: &crate::capabilities::Paths,
    session_binding: Option<&str>,
) -> Result<Value, LaunchError> {
    use crate::{capabilities as c, session_profiles as p};
    use std::os::unix::fs::PermissionsExt;
    if operation_id.is_empty()
        || operation_id.len() > 101
        || !operation_id.as_bytes()[0].is_ascii_alphanumeric()
        || !operation_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err(LaunchError::Value(
            "Identificador de operación inválido.".into(),
        ));
    }
    require_helper(&paths.home)?;
    let (mut ctx, inv) = internal_inventory(registry, harness, account, cwd, paths)?;
    let chosen = normalize(&inv, selection)?;
    if !ctx.errors.is_empty() {
        return Err(LaunchError::Value(
            "Inventario incompleto; no se puede garantizar el lanzamiento aislado.".into(),
        ));
    }
    // Check the supplied path before resolving: resolution would erase a symlink
    // and chmod/write into its external target (the legacy check came too late).
    if is_link(runtime_dir) {
        return Err(LaunchError::Value("Directorio privado inválido.".into()));
    }
    let root = c::resolved(runtime_dir);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&root)
        .map_err(|_| LaunchError::Other)?;
    if is_link(&root) || fs::metadata(&root).map_err(|_| LaunchError::Other)?.uid() != uid() {
        return Err(LaunchError::Value("Directorio privado inválido.".into()));
    }
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
        .map_err(|_| LaunchError::Other)?;
    let private = root.join(format!("extensions-{operation_id}-{}", uuid4_hex()?));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&private)
        .map_err(|_| LaunchError::Other)?;
    let mut files = LaunchFiles {
        private: private.clone(),
        artifacts: Map::new(),
        mounts: Vec::new(),
    };
    let mut args: Vec<String> = vec![];
    let mut env = Map::new();
    let home = ctx.home.clone().ok_or(LaunchError::Other)?;
    let mut selected_mcps = Map::new();
    let mut selected_skills = Vec::new();
    let mut synthetic = Map::new();
    let live_binding = session_binding.filter(|_| matches!(harness, "codex" | "claude"));
    for row in c::list(&inv["mcps"]) {
        let name = row["name"].as_str().ok_or(LaunchError::Other)?.to_owned();
        let id = row["id"].as_str().ok_or(LaunchError::Other)?;
        if truthy(&row["toggleable"])
            || (truthy(&row["enforceDisabled"]) && !truthy(&row["synthetic"]))
        {
            selected_mcps.insert(name.clone(), chosen["mcps"][id].clone());
        }
        let keep_connected =
            live_binding.is_some() && truthy(&row["synthetic"]) && !truthy(&row["enforceDisabled"]);
        if truthy(&row["synthetic"]) && (chosen["mcps"][id] == true || keep_connected) {
            let mut spec = proxy_entry(harness, &name, &paths.home);
            if matches!(harness, "codex" | "claude") {
                bind_proxy(&mut spec, session_binding)?;
            }
            synthetic.insert(name.clone(), spec);
        }
        if keep_connected {
            selected_mcps.insert(name, true.into());
        }
    }
    for row in c::list(&inv["skills"]) {
        if truthy(&row["toggleable"]) {
            let id = row["id"].as_str().ok_or(LaunchError::Other)?;
            let enabled = chosen["skills"][id].clone();
            selected_skills.push((row, enabled));
        }
    }
    match harness {
        "codex" => {
            for (name, spec) in &synthetic {
                args.extend([
                    "-c".into(),
                    format!("mcp_servers.{name}={}", launch_toml(spec)?),
                ]);
            }
            if let Some(binding) = session_binding {
                // The public inventory intentionally removes command/args from TOML.
                // Read private declarations to recognize existing managed proxies.
                let mut native = Map::new();
                for layer in &ctx.layers {
                    let data = read_configuration(&layer.path)?;
                    for (name, spec) in c::object(&data["mcp_servers"]) {
                        let target = native.entry(name).or_insert_with(|| json!({}));
                        target
                            .as_object_mut()
                            .ok_or(LaunchError::Other)?
                            .extend(c::object(&spec));
                    }
                }
                for (name, spec) in native {
                    if !synthetic.contains_key(&name) && managed_proxy(&spec) {
                        args.extend([
                            "-c".into(),
                            format!(
                                "mcp_servers.{name}.env.{MCP_SESSION_BINDING}={}",
                                launch_toml(&json!(binding))?
                            ),
                        ]);
                        if !c::list(&inv["mcps"])
                            .iter()
                            .any(|row| row["name"] == name && truthy(&row["enforceDisabled"]))
                        {
                            selected_mcps.insert(name, true.into());
                        }
                    }
                }
            }
            let configs = p::codex_configs(&home).map_err(discovery_fault)?;
            let mut entries = p::overrides(&configs);
            entries.extend(
                selected_skills
                    .iter()
                    .map(|(row, enabled)| json!({"path":row["path"],"enabled":enabled})),
            );
            args.extend([
                "-c".into(),
                format!("skills.config={}", launch_toml(&json!(entries))?),
            ]);
            for (name, enabled) in &selected_mcps {
                args.extend([
                    "-c".into(),
                    format!("mcp_servers.{name}.enabled={}", launch_toml(enabled)?),
                ]);
            }
        }
        "claude" => {
            let mut servers = Map::new();
            for layer in &ctx.layers {
                servers.extend(c::object(&layer.data["mcpServers"]));
            }
            let project = ctx.project.to_str().ok_or(LaunchError::Unsure)?;
            servers.extend(c::object(&ctx.user_data["projects"][project]["mcpServers"]));
            servers.extend(synthetic);
            if c::list(&inv["mcps"]).iter().any(|r| {
                truthy(&r["plugin"])
                    && r["enabled"] != false
                    && r["plugin"]
                        .as_str()
                        .is_some_and(|p| chosen["skills"][format!("plugin:{p}")] != false)
            }) {
                return Err(LaunchError::Value("Claude strict no puede conservar este MCP de plugin; requiere un adaptador del plugin.".into()));
            }
            servers.retain(|n, spec| {
                chosen["mcps"][n] != false
                    || (live_binding.is_some()
                        && managed_proxy(spec)
                        && !c::list(&inv["mcps"])
                            .iter()
                            .any(|row| row["name"] == *n && truthy(&row["enforceDisabled"])))
            });
            for spec in servers.values_mut() {
                bind_proxy(spec, session_binding)?;
            }
            if response_dumps(&json!(servers))
                .map_err(|_| LaunchError::Unsure)?
                .contains("${")
                || servers.values().any(|s| {
                    truthy(&s["cwd"])
                        && !s["cwd"]
                            .as_str()
                            .is_some_and(|p| Path::new(p).is_absolute())
                })
            {
                return Err(LaunchError::Value(
                    "MCP con expansión o cwd relativo no admite traslado aislado.".into(),
                ));
            }
            let mut settings = json!({"skillOverrides":{},"enabledPlugins":{}});
            for (row, enabled) in &selected_skills {
                if truthy(&row["group"]) {
                    settings["enabledPlugins"]
                        [row["plugin"].as_str().ok_or(LaunchError::Other)?] = enabled.clone();
                } else {
                    settings["skillOverrides"][row["name"].as_str().ok_or(LaunchError::Other)?] =
                        if enabled == true { "on" } else { "off" }.into();
                }
            }
            args.extend([
                "--settings".into(),
                files.file("settings.json", &settings)?,
                "--strict-mcp-config".into(),
                "--mcp-config".into(),
                files.file("mcp.json", &json!({"mcpServers":servers}))?,
            ]);
        }
        "opencode" => {
            let mut mcps = Map::new();
            for (name, enabled) in &selected_mcps {
                mcps.insert(name.clone(), json!({"enabled":enabled}));
            }
            mcps.extend(synthetic);
            let mut skills = Map::new();
            for (row, enabled) in &selected_skills {
                skills.insert(
                    row["name"].as_str().ok_or(LaunchError::Other)?.into(),
                    if enabled == true { "allow" } else { "deny" }.into(),
                );
            }
            let overlay = json!({"mcp":mcps,"permission":{"skill":skills}});
            env.insert(
                "OPENCODE_CONFIG_CONTENT".into(),
                compact_response(&overlay)?.into(),
            );
        }
        "grok" => {
            for layer in &ctx.layers {
                if !layer.path.exists() {
                    continue;
                }
                let mut data = read_configuration(&layer.path)?;
                object_mut(&mut data, "mcp_servers")?.extend(synthetic.clone());
                let disabled = sorted_selection(&data["disabled_mcp_servers"], &selected_mcps)?;
                data["disabled_mcp_servers"] = disabled;
                for (name, enabled) in &selected_mcps {
                    if let Some(spec) = data["mcp_servers"].get_mut(name) {
                        spec.as_object_mut()
                            .ok_or(LaunchError::Other)?
                            .insert("enabled".into(), enabled.clone());
                    }
                }
                let names = selected_skills
                    .iter()
                    .map(|(r, e)| {
                        Ok((
                            r["name"].as_str().ok_or(LaunchError::Other)?.to_owned(),
                            e.clone(),
                        ))
                    })
                    .collect::<Result<Map<String, Value>, LaunchError>>()?;
                let skill_cfg = object_mut(&mut data, "skills")?;
                let disabled =
                    sorted_selection(skill_cfg.get("disabled").unwrap_or(&Value::Null), &names)?;
                skill_cfg.insert("disabled".into(), disabled);
                let lines = data
                    .as_object()
                    .ok_or(LaunchError::Other)?
                    .iter()
                    .map(|(k, v)| {
                        Ok(format!(
                            "{}={}",
                            response_dumps(&json!(k)).map_err(|_| LaunchError::Unsure)?,
                            launch_toml(v)?
                        ))
                    })
                    .collect::<Result<Vec<_>, LaunchError>>()?
                    .join("\n")
                    + "\n";
                files.mount(&layer.path, lines.as_bytes())?;
            }
            if files.mounts.is_empty() {
                return Err(LaunchError::Value(
                    "Falta config.toml para el montaje aislado de Grok.".into(),
                ));
            }
            args.extend([
                "--leader-socket".into(),
                private
                    .join("leader.sock")
                    .to_str()
                    .ok_or(LaunchError::Unsure)?
                    .into(),
            ]);
        }
        "agy" => {
            let mut data = read_configuration(&home.join("config/mcp_config.json"))?;
            object_mut(&mut data, "mcpServers")?.extend(synthetic);
            object_mut(&mut data, "mcpServers")?.retain(|n, _| chosen["mcps"][n] != false);
            for (name, enabled) in &selected_mcps {
                if let Some(spec) = data["mcpServers"].get_mut(name) {
                    spec.as_object_mut()
                        .ok_or(LaunchError::Other)?
                        .insert("disabled".into(), json!(enabled != true));
                }
            }
            files.mount_json(&home.join("config/mcp_config.json"), &data)?;
            for layer in &ctx.layers {
                if layer.scope == "project" && layer.path.exists() {
                    let mut data = read_configuration(&layer.path)?;
                    object_mut(&mut data, "mcpServers")?.retain(|n, _| chosen["mcps"][n] != false);
                    for (name, enabled) in &selected_mcps {
                        if let Some(spec) = data["mcpServers"].get_mut(name) {
                            spec.as_object_mut()
                                .ok_or(LaunchError::Other)?
                                .insert("disabled".into(), json!(enabled != true));
                        }
                    }
                    files.mount_json(&layer.path, &data)?;
                }
            }
            let mut data = read_configuration(&home.join("config/skills.json"))?;
            let names = selected_skills
                .iter()
                .map(|(r, e)| {
                    Ok((
                        r["name"].as_str().ok_or(LaunchError::Other)?.to_owned(),
                        e.clone(),
                    ))
                })
                .collect::<Result<Map<String, Value>, LaunchError>>()?;
            data["exclude"] = sorted_selection(&data["exclude"], &names)?;
            if home.join("config/skills.json").exists() {
                files.mount_json(&home.join("config/skills.json"), &data)?;
            } else {
                let roots = p::skill_roots(&mut ctx, paths)
                    .into_iter()
                    .filter_map(|(root, _, _, _)| root.is_dir().then(|| c::resolved(&root)))
                    .collect::<std::collections::BTreeSet<_>>();
                for target in roots {
                    let source = private.join(format!("skills-{}", files.mounts.len()));
                    fs::DirBuilder::new()
                        .mode(0o700)
                        .create(&source)
                        .map_err(|_| LaunchError::Other)?;
                    for (row, enabled) in &selected_skills {
                        let path = Path::new(row["path"].as_str().ok_or(LaunchError::Other)?);
                        if enabled == true && path.starts_with(&target) {
                            let parent = path.parent().ok_or(LaunchError::Other)?;
                            let destination = source.join(
                                parent
                                    .strip_prefix(&target)
                                    .map_err(|_| LaunchError::Other)?,
                            );
                            copy_skill_tree(
                                parent,
                                &destination,
                                &mut std::collections::HashSet::new(),
                                &mut 10000,
                            )?;
                        }
                    }
                    files
                        .mounts
                        .push(json!({"source":source,"target":target,"kind":"directory"}));
                }
            }
        }
        _ => return Err(LaunchError::Unsure),
    }
    if !files.mounts.is_empty() {
        namespace_preflight(&private, &paths.home)?;
    }
    if let Some(binding) = live_binding {
        let initial = crate::pane_extensions::gate_selection(&inv, &chosen, &json!({}));
        comandos_store::extension_gate::initialize(&paths.home, binding, &initial).map_err(
            |_| {
                LaunchError::Value(
                    "No se pudo preparar el permiso MCP del panel; el agente sigue abierto.".into(),
                )
            },
        )?;
    }
    let manifest = private.join("manifest.json");
    let bundle = json!({"manifest":manifest,"selection":chosen,"operationId":operation_id,"harness":harness,"method":if files.mounts.is_empty(){"native"}else{"namespace"},"verification":{"marker":MARKER,"evidence":"manifest+process-configuration"}});
    let data = json!({"version":1,"bundle":bundle,"args":args,"env":env,"mounts":files.mounts,"codexSelectedSkillCount":selected_skills.len(),"artifacts":files.artifacts,"cwd":ctx.project,"home":paths.home,"account":account});
    write_private(
        &manifest,
        response_dumps(&data)
            .map_err(|_| LaunchError::Unsure)?
            .as_bytes(),
    )
    .map_err(|_| LaunchError::Other)?;
    Ok(bundle)
}

/// Darwin has no Linux user/mount namespace. Refuse before creating the
/// probe directory or starting any helper; no process has been stopped here.
#[cfg(target_os = "macos")]
fn namespace_preflight(_runtime_dir: &Path, _home: &Path) -> Result<(), LaunchError> {
    Err(LaunchError::Value(
        "El aislamiento user/mount namespace no está disponible; no se detuvo el agente.".into(),
    ))
}

/// Real child proof: user/mount isolation, original HOME and zero execution capabilities.
#[cfg(not(target_os = "macos"))]
fn namespace_preflight(runtime_dir: &Path, home: &Path) -> Result<(), LaunchError> {
    use std::os::unix::process::CommandExt;
    let refused = || {
        LaunchError::Value(
            "El aislamiento user/mount namespace no está disponible; no se detuvo el agente."
                .into(),
        )
    };
    let helper = require_helper(home)?;
    let directory = runtime_dir.join(format!("namespace-proof-{}", uuid4_hex()?));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|_| LaunchError::Other)?;
    struct ProbeDirectory(PathBuf);
    impl Drop for ProbeDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = ProbeDirectory(directory.clone());
    let target = directory.join("target");
    let source = directory.join("source");
    write_private(&target, b"parent").map_err(|_| LaunchError::Other)?;
    write_private(&source, b"child").map_err(|_| LaunchError::Other)?;
    let proof = directory.join("proof.json");
    let data = json!({"mounts":[{"source":source,"target":target}],"target":target,"home":home});
    write_private(
        &proof,
        response_dumps(&data)
            .map_err(|_| LaunchError::Other)?
            .as_bytes(),
    )
    .map_err(|_| LaunchError::Other)?;
    let output = directory.join("output");
    let stdout = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&output)
        .map_err(|_| LaunchError::Other)?;
    let mut child = match std::process::Command::new(helper)
        .arg("--namespace-proof")
        .arg(&proof)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .stdin(std::process::Stdio::null())
        .stdout(stdout)
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return Err(refused()),
    };
    let success = wait_namespace_probe(
        &mut child,
        std::time::Duration::from_secs(10),
        &mut |child| {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(child.id() as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        },
    );
    let verified = success
        && fs::read(&output).is_ok_and(|b| b == b"namespace-proof-ok\n")
        && fs::read(&target).is_ok_and(|b| b == b"parent");
    if verified { Ok(()) } else { Err(refused()) }
}

#[cfg(not(target_os = "macos"))]
fn wait_namespace_probe(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
    terminate: &mut dyn FnMut(&std::process::Child),
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    let exited = loop {
        match crate::procs::child_exited_unreaped(child) {
            Ok(true) => break true,
            Err(_) => break false,
            Ok(false) => {
                if std::time::Instant::now() >= deadline {
                    break false;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    };
    // Also terminate/reap descendants if proof was interrupted or failed.
    // The unreaped child still reserves its PID/group. An ECHILD/error never
    // authorizes a signal through an ID whose ownership is no longer proven.
    if crate::procs::child_exited_unreaped(child).is_ok() {
        terminate(child);
    }
    let success = child.wait().is_ok_and(|status| status.success());
    exited && success
}

#[cfg(test)]
mod tests {
    use super::{parse_toml, parse_trust_toml, shlex_split};
    use serde_json::json;

    #[test]
    #[cfg(target_os = "linux")]
    fn probe_leader_remains_owned_until_group_cleanup_and_timeout_is_reaped() {
        use std::os::unix::process::CommandExt;
        for (program, args, timeout, success) in [
            (
                "/usr/bin/true",
                vec![],
                std::time::Duration::from_secs(2),
                true,
            ),
            (
                "/usr/bin/sleep",
                vec!["60"],
                std::time::Duration::from_millis(30),
                false,
            ),
        ] {
            let mut child = std::process::Command::new(program)
                .args(args)
                .process_group(0)
                .spawn()
                .unwrap();
            let pid = child.id();
            let mut called = false;
            let outcome = super::wait_namespace_probe(&mut child, timeout, &mut |leader| {
                called = true;
                // Independent OS evidence: the leader's PID and process group
                // still exist at the moment cleanup receives signal authority.
                let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
                let state = stat
                    .rsplit_once(')')
                    .unwrap()
                    .1
                    .split_whitespace()
                    .next()
                    .unwrap();
                if success {
                    assert_eq!(state, "Z");
                }
                let owned = nix::unistd::Pid::from_raw(leader.id() as i32);
                assert_eq!(nix::unistd::getpgid(Some(owned)).unwrap(), owned);
                let _ = nix::sys::signal::killpg(owned, nix::sys::signal::Signal::SIGKILL);
            });
            assert!(called);
            assert_eq!(outcome, success);
            assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
        }
    }

    #[test]
    fn shlex_split_matches_posix_shlex() {
        assert_eq!(
            shlex_split(r#"env 'A=b c' "x\"y\$" z\ w ''"#).unwrap(),
            vec!["env", "A=b c", "x\"y\\$", "z w", ""]
        );
        assert_eq!(shlex_split("  ").unwrap(), Vec::<String>::new());
        assert_eq!(shlex_split("a 'b").unwrap_err(), "No closing quotation");
        assert_eq!(shlex_split("a \\").unwrap_err(), "No escaped character");
    }

    #[test]
    fn toml_rejects_what_tomllib_3_11_rejects() {
        assert_eq!(parse_toml("a = 1").unwrap(), Some(json!({"a": 1})));
        for text in [
            "a = {b=1,\n c=2}",
            "a = {b=1,}",
            "a = \"\\e\"",
            "a = \"\\x41\"",
            "a = 1979-05-27",
            "\u{feff}a = 1",
            "a = 1\na = 2",
        ] {
            assert_eq!(parse_toml(text).unwrap(), None, "{text:?}");
        }
        assert_eq!(
            parse_toml("a = [\n1,\n]\nb = {c = [1,\n2]}").unwrap(),
            Some(json!({"a": [1], "b": {"c": [1, 2]}}))
        );
        assert!(parse_toml("a = 99999999999999999999").is_err());
    }

    /// Lo que `toml_edit` no representa y `tomllib` sí: `Unsure`, nunca un
    /// rechazo.
    #[test]
    fn overflowing_numbers_are_unsure() {
        for text in [
            "a = 1e400",
            "a = -1e400",
            "a = 1.8e308",
            "a = 0x8000000000000000",
            "a = 0o1777777777777777777777",
            "a = 0b1000000000000000000000000000000000000000000000000000000000000000",
        ] {
            assert!(parse_toml(text).is_err(), "{text}");
            assert!(parse_trust_toml(text).is_err(), "{text}");
        }
        // Un documento roto con números que SÍ caben es un rechazo seguro.
        for text in [
            "a = 1e10\nb = [",
            "a = 0x7fffffffffffffff\n[",
            "a = 9223372036854775807\n[",
            "a = 1.5\nb = =",
            "1e5x = 1\n[",
        ] {
            assert_eq!(parse_toml(text), Ok(None), "{text}");
        }
    }

    /// Lector de la confianza: las fechas son valores opacos (desviación: el
    /// Python 3.10 falla al pasarlas por JSON); lo que `tomllib` 3.11 rechaza
    /// se sigue rechazando.
    #[test]
    fn trust_reader_accepts_dates() {
        assert_eq!(
            parse_trust_toml("a = 1979-05-27\nb = 1979-05-27T07:32:00Z\nc = 07:32:00").unwrap(),
            Some(json!({"a": "1979-05-27", "b": "1979-05-27T07:32:00Z", "c": "07:32:00"}))
        );
        assert_eq!(parse_toml("a = 1979-05-27").unwrap(), None);
        // `0000-01-01`: `tomllib` lo rechaza (no es un `date`), el CLI de
        // Codex (`toml`) lo lee; se acepta como las demás fechas.
        assert_eq!(
            parse_trust_toml("a = 0000-01-01").unwrap(),
            Some(json!({"a": "0000-01-01"}))
        );
        for text in [
            "a = 07:32",
            "a = 1979-05-27T07:32",
            "a = 23:59:60",
            "a = 1979-05-27T23:59:60Z",
            "a = {b=1,}",
            "a = \"\\e\"",
            "\u{feff}a=1",
        ] {
            assert_eq!(parse_trust_toml(text).unwrap(), None, "{text:?}");
        }
    }
}
