//! D. Lecturas ligeras y preferencias, rama a rama de `bin/cc-dash`.
use super::{
    Answer, Entry, Fault, Key, Native, NativeRoute, Verb,
    files::{self, DomainDocument, FileLock, Strict},
    py,
    query::Query,
    reply,
    tmux::{Tmux, TmuxError, run_program},
};
use crate::{HandlerError, Reply, Request};
use comandos_core::json::{response_dumps, truthy};
use comandos_web_view::user_themes;
use http::StatusCode;
use serde_json::{Map, Value, json};
use std::{
    collections::HashSet,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightRoute {
    Prefs,
    PrefsSet,
    Tabs,
    TabHistory,
    TabModels,
    ActiveTab,
    TmuxMouseGet,
    TmuxMouseSet,
}

const fn entry(verb: Verb, key: Key, route: LightRoute) -> Entry {
    Entry {
        verb,
        key,
        route: NativeRoute::Light(route),
    }
}

pub const ROUTES: &[Entry] = &[
    entry(Verb::Get, Key::Path("/prefs"), LightRoute::Prefs),
    entry(Verb::Get, Key::Path("/tabs"), LightRoute::Tabs),
    entry(Verb::Get, Key::Path("/tab-history"), LightRoute::TabHistory),
    entry(Verb::Get, Key::Path("/tab-models"), LightRoute::TabModels),
    entry(Verb::Get, Key::Path("/active-tab"), LightRoute::ActiveTab),
    entry(
        Verb::Get,
        Key::ExactOrQuery("/tmux-mouse"),
        LightRoute::TmuxMouseGet,
    ),
    entry(Verb::Post, Key::Raw("/prefs-set"), LightRoute::PrefsSet),
    entry(
        Verb::Post,
        Key::Raw("/tmux-mouse"),
        LightRoute::TmuxMouseSet,
    ),
];

pub async fn answer(native: &Native, route: LightRoute, request: &Request) -> Answer {
    let hooks = native.options().hooks.as_path();
    let tmux = &native.options().tmux;
    match route {
        LightRoute::Prefs => {
            let mut prefs = read_prefs_domain(&native.options().home, hooks)?;
            prefs.insert("fonts".into(), fonts(native).await);
            read_reply(&Value::Object(prefs))
        }
        LightRoute::PrefsSet => prefs_set(native, data(request)?).await,
        LightRoute::Tabs => tabs(&native.options().home, hooks, tmux).await,
        LightRoute::TabHistory => tab_history(&native.options().home, hooks, tmux).await,
        LightRoute::TabModels => tab_models(&native.options().home, hooks, &request.target),
        LightRoute::ActiveTab => active_tab(&native.options().home, hooks, tmux).await,
        LightRoute::TmuxMouseGet => {
            let query = Query::parse(&request.target)?;
            let sess = query.first("session").unwrap_or("");
            if !py::is_session(sess) {
                return error(StatusCode::BAD_REQUEST, "Nombre de sesion invalido");
            }
            match get_tmux_mouse(tmux, sess).await? {
                Ok(on) => reply(
                    StatusCode::OK,
                    &json!({"mouse": if on { "on" } else { "off" }}),
                ),
                Err(message) => mouse_error(&message),
            }
        }
        LightRoute::TmuxMouseSet => {
            let data = data(request)?;
            // `SESSION_RE.match(sess)` con un no-str es TypeError → 500.
            let sess = match data.get("session") {
                None => "",
                Some(Value::String(s)) => s.as_str(),
                Some(_) => return Err(HandlerError::Failure.into()),
            };
            if !py::is_session(sess) {
                return error(StatusCode::BAD_REQUEST, "Nombre de sesion invalido");
            }
            let enabled = data.get("enabled").is_none_or(truthy);
            match set_tmux_mouse(tmux, sess, enabled).await? {
                None => reply(
                    StatusCode::OK,
                    &json!({"ok": true, "mouse": if enabled { "on" } else { "off" }}),
                ),
                Some(message) => mouse_error(&message),
            }
        }
    }
}

/// El transporte garantiza un objeto en todo POST admitido.
pub(crate) fn data(request: &Request) -> Result<&Map<String, Value>, Fault> {
    request
        .data
        .as_ref()
        .and_then(Value::as_object)
        .ok_or(Fault::Error(HandlerError::Failure))
}

pub(crate) fn error(status: StatusCode, message: &str) -> Answer {
    reply(status, &json!({"error": message}))
}

/// 200 de una ruta de solo lectura: si el codificador portado no puede escribir
/// lo leído (el `json.dumps` del Python sí), se declina; no hubo efectos.
pub(crate) fn read_reply(value: &Value) -> Answer {
    Reply::json(StatusCode::OK, value).map_err(|_| Fault::Decline)
}

fn mouse_error(message: &str) -> Answer {
    let status = if message.starts_with("No hay sesion") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    error(status, message)
}

// ---------------------------------------------------------------- preferencias

fn prefs_defaults() -> [(&'static str, Value); 10] {
    [
        ("theme", json!("noche")),
        ("font_family", json!("Ubuntu Sans Mono")),
        ("font_size", json!(13)),
        ("cursor_shape", json!("block")),
        ("cursor_blink", json!(true)),
        ("terminal_padding", json!(8)),
        ("terminal_opacity", json!(100)),
        ("ligatures", json!(true)),
        ("button_style", json!("sutil")),
        ("tabs_layout", json!("row")),
    ]
}

/// `json.load(open(path))` dentro de un `try/except Exception` del Python.
/// `Ok(None)`: el Python cae en su `except`; `Decline`: no se sabe qué leería.
pub(crate) fn load(path: &Path) -> Result<Option<Value>, Fault> {
    match files::read_json_strict(path) {
        Strict::Value(value) => Ok(Some(value)),
        Strict::Missing | Strict::Unreadable => Ok(None),
        Strict::Unsure => Err(Fault::Decline),
    }
}

/// `read_prefs` (7630).
pub fn read_prefs(hooks: &Path) -> Result<Map<String, Value>, Fault> {
    prefs_from(load(&hooks.join("prefs.json"))?)
}
pub fn read_prefs_domain(home: &Path, hooks: &Path) -> Result<Map<String, Value>, Fault> {
    prefs_from(load_domain(home, hooks, "prefs.json")?)
}
fn prefs_from(value: Option<Value>) -> Result<Map<String, Value>, Fault> {
    let mut prefs = match value {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    for (key, value) in prefs_defaults() {
        prefs.entry(key).or_insert(value);
    }
    Ok(prefs)
}

const FONT_CATALOG: [(&str, &str, bool); 15] = [
    (
        "Ubuntu Sans Mono",
        "Ubuntu Sans Mono · la de Ubuntu (default)",
        false,
    ),
    (
        "JetBrainsMono Nerd Font Mono",
        "JetBrains Mono · con iconos (bundle)",
        false,
    ),
    (
        "Atkinson Hyperlegible Mono",
        "Atkinson Hyperlegible Mono",
        true,
    ),
    ("Intel One Mono", "Intel One Mono", true),
    ("JetBrains Mono", "JetBrains Mono", false),
    ("Monaspace Neon Var", "Monaspace Neon", false),
    ("Monaspace Argon Var", "Monaspace Argon", false),
    ("Geist Mono", "Geist Mono", false),
    ("Cascadia Code", "Cascadia Code", false),
    ("Fira Code", "Fira Code", false),
    ("Ubuntu Mono", "Ubuntu Mono · clásica 2011", false),
    ("Iosevka", "Iosevka", false),
    ("Hack", "Hack", false),
    ("Source Code Pro", "Source Code Pro", false),
    ("DejaVu Sans Mono", "DejaVu Sans Mono", false),
];

/// `_installed_font_families` (7724) + `installed_terminal_fonts` (7745).
async fn fonts(native: &Native) -> Value {
    let cached = native
        .fonts
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .filter(|(at, _)| at.elapsed() < Duration::from_secs(60))
        .map(|(_, fams)| fams.clone());
    let fams = match cached {
        Some(fams) => fams,
        None => {
            let mut fams = HashSet::new();
            // El Python lee `.stdout` aunque fc-list falle; una excepción deja el conjunto vacío.
            if let Ok(out) = run_program(
                &native.options().fc_list,
                &[":", "family"],
                Duration::from_secs(5),
            )
            .await
            {
                for line in py::splitlines(&out.stdout) {
                    for fam in line.split(',') {
                        let fam = py::strip(fam);
                        if !fam.is_empty() {
                            fams.insert(fam.to_owned());
                        }
                    }
                }
            }
            *native.fonts.lock().unwrap_or_else(|p| p.into_inner()) =
                Some((Instant::now(), fams.clone()));
            fams
        }
    };
    let mut rows: Vec<Value> = FONT_CATALOG
        .iter()
        .filter(|(family, _, _)| fams.contains(*family))
        .map(|(family, label, a11y)| json!({"family": family, "label": label, "a11y": a11y}))
        .collect();
    rows.push(json!({"family": "Monospace", "label": "Monospace del sistema", "a11y": false}));
    Value::Array(rows)
}

enum PrefsFault {
    /// `ValueError` → 400 con su texto.
    Bad(&'static str),
    /// Persistencia fallida; distinta de un error al convertir el dato.
    Storage,
    Fault(Fault),
}

impl From<Fault> for PrefsFault {
    fn from(fault: Fault) -> Self {
        PrefsFault::Fault(fault)
    }
}

/// `prefs.get("favorites", [])` iterada por el Python.
fn stored_favorites(prefs: &Map<String, Value>) -> Result<Vec<Value>, Fault> {
    match prefs.get("favorites") {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items.clone()),
        // Iterar un str da caracteres y un dict sus claves: raro, se declina.
        Some(Value::String(_) | Value::Object(_)) => Err(Fault::Decline),
        // `for x in None/5/True`: TypeError no capturado.
        Some(_) => Err(HandlerError::Failure.into()),
    }
}

fn dedupe(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

/// `float(v)` para un `int`/`float`/`bool` de JSON, como valor JSON.
fn py_float_value(value: &Value) -> Result<Option<Value>, PrefsFault> {
    let as_number = |x: f64| {
        serde_json::Number::from_f64(x)
            .map(Value::Number)
            .ok_or(PrefsFault::Fault(Fault::Decline))
    };
    match value {
        Value::Bool(b) => as_number(if *b { 1.0 } else { 0.0 }).map(Some),
        Value::Number(n) => {
            let raw = n.as_str();
            if matches!(raw, "NaN" | "Infinity" | "-Infinity") || raw.contains(['.', 'e', 'E']) {
                // Ya es float: `float(x)` es la identidad y `json.dumps` lo reescribe con su repr.
                Ok(Some(value.clone()))
            } else {
                let x: f64 = raw.parse().map_err(|_| Fault::Decline)?;
                if !x.is_finite() {
                    // `float(10**400)`: OverflowError no capturado.
                    return Err(Fault::Error(HandlerError::Failure).into());
                }
                // `float(-0)` es `0.0`: el entero `-0` no tiene signo en Python.
                as_number(x + 0.0).map(Some)
            }
        }
        _ => Ok(None),
    }
}

/// `max(lo, min(hi, int(x)))` para `isinstance(x, (int, float))`.
fn clamped(value: Option<&Value>, lo: i64, hi: i64) -> Result<Option<i64>, PrefsFault> {
    let n = match value {
        Some(Value::Bool(b)) => i64::from(*b),
        Some(Value::Number(n)) => match n.as_str() {
            "NaN" => return Err(PrefsFault::Bad("cannot convert float NaN to integer")),
            "Infinity" | "-Infinity" => return Err(Fault::Error(HandlerError::Failure).into()),
            raw if !raw.contains(['.', 'e', 'E']) => {
                raw.parse::<i64>().unwrap_or(if raw.starts_with('-') {
                    i64::MIN
                } else {
                    i64::MAX
                })
            }
            raw => {
                let x: f64 = raw.parse().map_err(|_| Fault::Decline)?;
                if x.is_infinite() {
                    return Err(Fault::Error(HandlerError::Failure).into());
                }
                let t = x.trunc();
                if t >= i64::MAX as f64 {
                    i64::MAX
                } else if t <= i64::MIN as f64 {
                    i64::MIN
                } else {
                    t as i64
                }
            }
        },
        _ => return Ok(None),
    };
    Ok(Some(n.clamp(lo, hi)))
}

fn is_one_of(value: Option<&Value>, options: &[&str]) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| options.contains(s))
        .map(str::to_owned)
}

/// `update_prefs` (7768) sin la escritura: mismas comprobaciones y mismo
/// orden de asignación (las claves nuevas se añaden al final, como en un dict).
fn update_prefs(
    mut prefs: Map<String, Value>,
    data: &Map<String, Value>,
) -> Result<Map<String, Value>, PrefsFault> {
    if let Some(patch) = data.get("favorite") {
        let session = patch.get("session").and_then(Value::as_str).filter(|s| {
            (1..=160).contains(&s.chars().count())
                && *s != "local"
                && !s.chars().any(|c| (c as u32) < 32)
        });
        let enabled = patch.get("enabled").and_then(Value::as_bool);
        let (Some(session), Some(enabled)) = (session, enabled) else {
            return Err(PrefsFault::Bad("Favorito inválido"));
        };
        let mut favorites = dedupe(
            stored_favorites(&prefs)?
                .into_iter()
                .filter_map(|x| x.as_str().filter(|s| *s != "local").map(str::to_owned)),
        );
        if enabled && !favorites.iter().any(|f| f == session) {
            if favorites.len() >= 200 {
                return Err(PrefsFault::Bad("Máximo de 200 favoritos alcanzado"));
            }
            favorites.push(session.to_owned());
        } else if !enabled {
            favorites.retain(|f| f != session);
        }
        prefs.insert("favorites".into(), json!(favorites));
    }
    if let Some(Value::Object(dismiss)) = data.get("nfDismiss") {
        let mut out = Map::new();
        for key in dismiss.keys().take(600) {
            out.insert(py::take_chars(key, 160), json!(1));
        }
        prefs.insert("nfDismiss".into(), Value::Object(out));
    }
    if let Some(Value::Object(snooze)) = data.get("nfSnooze") {
        let mut out = Map::new();
        for (key, value) in snooze.iter().take(200) {
            if let Some(number) = py_float_value(value)? {
                out.insert(py::take_chars(key, 160), number);
            }
        }
        prefs.insert("nfSnooze".into(), Value::Object(out));
    }
    if !data.contains_key("favorite")
        && let Some(Value::Array(list)) = data.get("favorites")
    {
        let kept = dedupe(
            list.iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.is_empty() && *s != "local")
                .map(|s| py::take_chars(s, 160)),
        );
        prefs.insert(
            "favorites".into(),
            json!(kept.into_iter().take(200).collect::<Vec<_>>()),
        );
    }
    const THEMES: [&str; 9] = [
        "noche",
        "dia",
        "calido",
        "termius",
        "bruno",
        "superglass",
        "neon",
        "contraste",
        "ubuntu",
    ];
    // Temas propios: guardar/reemplazar uno, o borrarlo. Si se borra el tema
    // activo, queda puesto el de fábrica del que salió.
    if let Some(theme) = data.get("user_theme") {
        let stored = prefs.get("user_themes").cloned().unwrap_or(Value::Null);
        let list = user_themes::upsert(&stored, theme).map_err(PrefsFault::Bad)?;
        prefs.insert("user_themes".into(), list);
    }
    if let Some(id) = data.get("user_theme_delete").and_then(Value::as_str) {
        let current = Value::Object(prefs.clone());
        let base = user_themes::find(&current, id).and_then(|t| t.get("base").cloned());
        let stored = prefs.get("user_themes").cloned().unwrap_or(Value::Null);
        prefs.insert("user_themes".into(), user_themes::remove(&stored, id));
        if prefs.get("theme").and_then(Value::as_str) == Some(id) {
            prefs.insert("theme".into(), base.unwrap_or_else(|| json!("noche")));
        }
    }
    if let Some(theme) = is_one_of(data.get("theme"), &THEMES) {
        prefs.insert("theme".into(), json!(theme));
    } else if let Some(id) = data.get("theme").and_then(Value::as_str)
        && user_themes::is_user_id(id)
        && user_themes::find(&Value::Object(prefs.clone()), id).is_some()
    {
        prefs.insert("theme".into(), json!(id));
    }
    if let Some(style) = is_one_of(
        data.get("button_style"),
        &["sutil", "arcade", "tecla", "pixel", "consola"],
    ) {
        prefs.insert("button_style".into(), json!(style));
    }
    if let Some(layout) = is_one_of(data.get("tabs_layout"), &["row", "rows"]) {
        prefs.insert("tabs_layout".into(), json!(layout));
    }
    if let Some(Value::String(family)) = data.get("font_family") {
        let family = py::take_chars(family, 120);
        let family = py::strip(&family);
        let family = if family.is_empty() {
            "Ubuntu Sans Mono"
        } else {
            family
        };
        prefs.insert("font_family".into(), json!(family));
    }
    if let Some(size) = clamped(data.get("font_size"), 8, 28)? {
        prefs.insert("font_size".into(), json!(size));
    }
    if let Some(shape) = is_one_of(data.get("cursor_shape"), &["block", "ibeam", "underline"]) {
        prefs.insert("cursor_shape".into(), json!(shape));
    }
    if let Some(Value::Bool(blink)) = data.get("cursor_blink") {
        prefs.insert("cursor_blink".into(), json!(blink));
    }
    if let Some(padding) = clamped(data.get("terminal_padding"), 0, 40)? {
        prefs.insert("terminal_padding".into(), json!(padding));
    }
    if let Some(opacity) = clamped(data.get("terminal_opacity"), 30, 100)? {
        prefs.insert("terminal_opacity".into(), json!(opacity));
    }
    if let Some(Value::Bool(ligatures)) = data.get("ligatures") {
        prefs.insert("ligatures".into(), json!(ligatures));
    }
    if let Some(pos) = is_one_of(data.get("notif_pos"), &["tl", "tr", "bl", "br", "free"]) {
        prefs.insert("notif_pos".into(), json!(pos));
    }
    Ok(prefs)
}

async fn prefs_set(native: &Native, data: &Map<String, Value>) -> Answer {
    let _guard = native.prefs_lock.lock().await;
    let doc = DomainDocument::new(
        &native.options().home,
        &native.options().hooks,
        "prefs.json",
    )
    .map_err(|_| Fault::Decline)?;
    let data = data.clone();
    let now = (native.options().clock)();
    let written = tokio::task::spawn_blocking(move || -> Result<Value, PrefsFault> {
        let access = doc.access().map_err(|_| Fault::Decline)?;
        let _lock = if access.mode() == comandos_store::unified::Mode::Sealed {
            None
        } else {
            Some(FileLock::acquire(&doc.file).map_err(|_| PrefsFault::Storage)?)
        };
        access
            .with_write_transaction(|| {
                let prefs = Value::Object(update_prefs(
                    prefs_from(value_from(doc.strict_under(&access))?)?,
                    &data,
                )?);
                let text = response_dumps(&prefs).map_err(|_| Fault::Decline)?;
                let favorites = prefs.get("favorites").cloned().unwrap_or_else(|| json!([]));
                doc.write_under(&access, text.as_bytes(), now)
                    .map_err(|_| PrefsFault::Storage)?;
                Ok(favorites)
            })
            .map_err(|_| PrefsFault::Storage)?
    })
    .await
    .map_err(|_| HandlerError::Failure)?;
    match written {
        Ok(favorites) => reply(StatusCode::OK, &json!({"ok": true, "favorites": favorites})),
        Err(PrefsFault::Bad(message)) => error(StatusCode::BAD_REQUEST, message),
        Err(PrefsFault::Storage) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "No se pudieron guardar las preferencias",
        ),
        Err(PrefsFault::Fault(fault)) => Err(fault),
    }
}

pub fn value_from(read: Strict) -> Result<Option<Value>, Fault> {
    match read {
        Strict::Value(value) => Ok(Some(value)),
        Strict::Missing | Strict::Unreadable => Ok(None),
        Strict::Unsure => Err(Fault::Decline),
    }
}
pub fn load_domain(home: &Path, hooks: &Path, name: &str) -> Result<Option<Value>, Fault> {
    let doc = DomainDocument::new(home, hooks, name).map_err(|_| Fault::Decline)?;
    value_from(doc.strict())
}

// These entrypoints are for the state/workspace blocking workers only.
fn load_domain_in_worker(home: &Path, hooks: &Path, name: &str) -> Result<Option<Value>, Fault> {
    let doc = DomainDocument::new(home, hooks, name).map_err(|_| Fault::Decline)?;
    value_from(doc.strict_in_worker())
}
pub(crate) fn read_prefs_in_worker(home: &Path, hooks: &Path) -> Result<Map<String, Value>, Fault> {
    prefs_from(load_domain_in_worker(home, hooks, "prefs.json")?)
}
pub(crate) fn tab_labels_in_worker(
    home: &Path,
    hooks: &Path,
) -> Result<Vec<(String, String)>, Fault> {
    tab_labels_from(load_domain_in_worker(home, hooks, "app-tabs.json")?)
}
pub(crate) fn tab_history_in_worker(
    home: &Path,
    hooks: &Path,
) -> Result<Vec<Map<String, Value>>, Fault> {
    tab_history_from(load_domain_in_worker(home, hooks, "app-tabs-history.json")?)
}

// ---------------------------------------------------------------- pestañas

pub const HIDDEN_SESSIONS: [&str; 3] = ["hub", "local", "control"];

/// `tab_labels` (6430): pares `(sesión, etiqueta)` en el orden del archivo.
pub fn tab_labels(hooks: &Path) -> Result<Vec<(String, String)>, Fault> {
    tab_labels_from(load(&hooks.join("app-tabs.json"))?)
}
pub fn tab_labels_domain(home: &Path, hooks: &Path) -> Result<Vec<(String, String)>, Fault> {
    tab_labels_from(load_domain(home, hooks, "app-tabs.json")?)
}
fn tab_labels_from(value: Option<Value>) -> Result<Vec<(String, String)>, Fault> {
    Ok(match value {
        Some(Value::Object(map)) => map
            .into_iter()
            .filter_map(|(k, v)| match v {
                Value::String(s) if !s.is_empty() => Some((k, s)),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    })
}

/// `set(favorites)` tal como lo usa `ordered_tab_keys`.
pub fn favorites_set(prefs: &Map<String, Value>) -> Result<HashSet<String>, Fault> {
    let mut set = HashSet::new();
    for item in stored_favorites(prefs)? {
        match item {
            Value::String(s) => {
                set.insert(s);
            }
            // Listas y objetos no son hashables: TypeError no capturado.
            Value::Array(_) | Value::Object(_) => return Err(HandlerError::Failure.into()),
            // Números, booleanos y None nunca igualan a un nombre de sesión.
            _ => {}
        }
    }
    Ok(set)
}

/// `lib/session_tabs.ordered_tab_keys`: orden estable local → favoritas → resto.
pub fn ordered_tab_keys<'a>(
    keys: &'a [(String, String)],
    favorites: &HashSet<String>,
) -> Vec<&'a (String, String)> {
    let mut out: Vec<_> = keys.iter().collect();
    out.sort_by_key(|(key, _)| {
        if key == "local" {
            0
        } else if favorites.contains(key) {
            1
        } else {
            2
        }
    });
    out
}

/// `tmux_sessions` (5876): las excepciones no se capturan.
pub async fn tmux_sessions(tmux: &Tmux) -> Result<HashSet<String>, Fault> {
    let out = tmux
        .run(&["list-sessions", "-F", "#{session_name}"])
        .await
        .map_err(|e| Fault::Error(e.uncaught()))?;
    if !out.ok {
        return Ok(HashSet::new());
    }
    Ok(py::splitlines(&out.stdout)
        .into_iter()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

async fn tabs(home: &Path, hooks: &Path, tmux: &Tmux) -> Answer {
    let live = tmux_sessions(tmux).await?;
    let mut out = Vec::new();
    if live.contains("local") {
        out.push(json!({"session": "local", "label": "⌂ local", "closable": false}));
    }
    let labels = tab_labels_domain(home, hooks)?;
    let favorites = favorites_set(&read_prefs_domain(home, hooks)?)?;
    for (sess, label) in ordered_tab_keys(&labels, &favorites) {
        if live.contains(sess) && !HIDDEN_SESSIONS.contains(&sess.as_str()) {
            out.push(json!({"session": sess, "label": label}));
        }
    }
    read_reply(&Value::Array(out))
}

/// `str(it.get(key) or default)[:n]`.
fn text_or(item: &Map<String, Value>, key: &str, default: &str, n: usize) -> Result<String, Fault> {
    let text = match item.get(key).filter(|v| truthy(v)) {
        None => default.to_owned(),
        Some(v) => py::str_scalar(v).ok_or(Fault::Decline)?,
    };
    Ok(py::take_chars(&text, n))
}

/// `read_tab_history` (5318).
pub fn read_tab_history(hooks: &Path) -> Result<Vec<Map<String, Value>>, Fault> {
    tab_history_from(load(&hooks.join("app-tabs-history.json"))?)
}
pub(crate) fn read_tab_history_domain(
    home: &Path,
    hooks: &Path,
) -> Result<Vec<Map<String, Value>>, Fault> {
    tab_history_from(load_domain(home, hooks, "app-tabs-history.json")?)
}
pub(crate) fn tab_history_from(value: Option<Value>) -> Result<Vec<Map<String, Value>>, Fault> {
    let Some(Value::Array(items)) = value else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in items.iter().take(80) {
        let Value::Object(item) = item else { continue };
        let session = match item.get("session") {
            None => String::new(),
            Some(v) => match py::str_scalar(v) {
                Some(s) => s,
                // repr de lista/objeto lleva corchetes o llaves: nunca casa SESSION_RE.
                None if v.is_array() || v.is_object() => continue,
                None => return Err(Fault::Decline),
            },
        };
        if !py::is_session(&session) {
            continue;
        }
        let cwd = match item.get("cwd") {
            Some(Value::String(s)) if s.starts_with('/') => s.clone(),
            _ => String::new(),
        };
        let ts = match item.get("ts").filter(|v| truthy(v)) {
            None => 0,
            Some(v) => match py::int_of(v) {
                Ok(n) => n,
                Err(py::Conversion::Exotic) => return Err(Fault::Decline),
                Err(_) => return Err(HandlerError::Failure.into()),
            },
        };
        let mut row = Map::new();
        row.insert("label".into(), json!(text_or(item, "label", &session, 80)?));
        row.insert("cwd".into(), json!(cwd));
        row.insert("agent".into(), json!(text_or(item, "agent", "claude", 16)?));
        row.insert("ts".into(), json!(ts));
        row.insert(
            "reason".into(),
            json!(text_or(item, "reason", "closed", 32)?),
        );
        let mut full = Map::new();
        full.insert("session".into(), json!(session));
        full.extend(row);
        out.push(full);
    }
    Ok(out)
}

async fn tab_history(home: &Path, hooks: &Path, tmux: &Tmux) -> Answer {
    // Se lee antes que tmux solo para poder declinar pronto; un 500 del
    // historial se devuelve después de tmux, en el orden del Python.
    let history = read_tab_history_domain(home, hooks);
    if matches!(history, Err(Fault::Decline)) {
        return Err(Fault::Decline);
    }
    let labels: HashSet<String> = tab_labels_domain(home, hooks)?
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    let live = tmux_sessions(tmux).await?;
    let mut out = Vec::new();
    for mut item in history? {
        let session = item
            .get("session")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if labels.contains(&session) {
            continue;
        }
        item.insert("alive".into(), json!(live.contains(&session)));
        out.push(Value::Object(item));
    }
    out.truncate(40);
    read_reply(&Value::Array(out))
}

fn tab_models(home: &Path, hooks: &Path, target: &str) -> Answer {
    let query = Query::parse(target)?;
    let sess = query.first("session").unwrap_or("");
    let models = load_domain(home, hooks, "app-tab-models.json")?.unwrap_or_else(|| json!({}));
    let entry = models.as_object().and_then(|m| m.get(sess));
    let panes = match entry {
        None => json!([]),
        Some(e) if !truthy(e) => json!([]),
        Some(Value::Object(o)) => o
            .get("panes")
            .filter(|p| truthy(p))
            .cloned()
            .unwrap_or_else(|| json!([])),
        // `(entry or {}).get(...)` sobre lista/str/número: AttributeError.
        Some(_) => return Err(HandlerError::Failure.into()),
    };
    read_reply(&json!({"session": sess, "panes": panes}))
}

async fn active_tab(home: &Path, hooks: &Path, tmux: &Tmux) -> Answer {
    let mut active = load_domain(home, hooks, "app-tab-active.json")?.unwrap_or_else(|| json!({}));
    let Value::Object(map) = &mut active else {
        return Err(HandlerError::Failure.into());
    };
    let sess = match map.get("session").filter(|v| truthy(v)) {
        None => String::new(),
        Some(v) => py::str_scalar(v).ok_or(Fault::Decline)?,
    };
    if py::is_session(&sess) {
        let uncaught = |e: TmuxError| Fault::Error(e.uncaught());
        let target = format!("={sess}");
        if tmux
            .run(&["has-session", "-t", &target])
            .await
            .map_err(uncaught)?
            .ok
        {
            let window = format!("={sess}:");
            let rp = tmux
                .run(&["display-message", "-p", "-t", &window, "#{pane_id}"])
                .await
                .map_err(uncaught)?;
            let pane = if rp.ok {
                py::strip(&rp.stdout).to_owned()
            } else {
                String::new()
            };
            // Solo lecturas de tmux hasta aquí: declinar sigue siendo seguro.
            if py::is_pane(&pane).ok_or(Fault::Decline)? {
                map.insert("pane".into(), json!(pane));
            }
        }
    }
    read_reply(&active)
}

/// `str(exc).strip() or "tmux fallo"`; sin texto seguro, declinar.
fn caught(error: &TmuxError) -> Result<String, Fault> {
    let message = error.python_message().ok_or(Fault::Decline)?;
    let message = py::strip(&message);
    Ok(if message.is_empty() {
        "tmux fallo".into()
    } else {
        message.to_owned()
    })
}

fn stderr_or(stderr: &str) -> String {
    let text = py::strip(stderr);
    if text.is_empty() {
        "tmux fallo".into()
    } else {
        text.to_owned()
    }
}

/// `get_tmux_mouse` (5883).
async fn get_tmux_mouse(tmux: &Tmux, sess: &str) -> Result<Result<bool, String>, Fault> {
    let exists = match tmux.run(&["has-session", "-t", &format!("={sess}")]).await {
        Ok(out) => out,
        Err(e) => return Ok(Err(caught(&e)?)),
    };
    if !exists.ok {
        return Ok(Err(format!("No hay sesion tmux '{sess}'")));
    }
    let shown = match tmux
        .run(&["show-options", "-A", "-v", "-t", sess, "mouse"])
        .await
    {
        Ok(out) => out,
        Err(e) => return Ok(Err(caught(&e)?)),
    };
    if !shown.ok {
        return Ok(Err(stderr_or(&shown.stderr)));
    }
    Ok(Ok(py::strip(&shown.stdout) == "on"))
}

/// `set_tmux_mouse` (5905): `None` si fue bien.
async fn set_tmux_mouse(tmux: &Tmux, sess: &str, enabled: bool) -> Result<Option<String>, Fault> {
    let exists = match tmux.run(&["has-session", "-t", &format!("={sess}")]).await {
        Ok(out) => out,
        Err(e) => return Ok(Some(caught(&e)?)),
    };
    if !exists.ok {
        return Ok(Some(format!("No hay sesion tmux '{sess}'")));
    }
    let value = if enabled { "on" } else { "off" };
    let set = match tmux.run(&["set-option", "-t", sess, "mouse", value]).await {
        Ok(out) => out,
        // tmux ya corrió y su salida no es UTF-8: el Python da el texto del
        // UnicodeDecodeError, que no se reproduce; tras un efecto no se declina.
        Err(TmuxError::Decode) => return Err(HandlerError::Failure.into()),
        // Un plazo vencido siempre tiene texto; solo un fallo de arranque (sin
        // efecto) puede declinar.
        Err(e) => return Ok(Some(caught(&e)?)),
    };
    Ok((!set.ok).then(|| stderr_or(&set.stderr)))
}
