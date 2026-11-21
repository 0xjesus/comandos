//! Lecturas de `comandos-usage.sqlite` que las rutas de uso del frente necesitan, con
//! el mismo texto SQL de `bin/cc_usage.py` y `lib/session_profiles.py` (mismo plan de
//! índices y mismos empates de orden), más las dos escrituras ligeras (`record_pane`,
//! `record_quota_snapshots`). La conexión es la del carril: ya abierta, con la puerta
//! de esquema pasada y `init_db` hecho, así que aquí no se migra nada.
//!
//! Las filas salen como objetos JSON en el orden de columnas, con `REAL` como `float`
//! de Python, `INTEGER` como entero y `NULL` como `null`. Un BLOB o un texto que no es
//! UTF-8 es `Undecodable` (el Python falla al decodificar o al serializar: 500).
use comandos_core::json::{python_eq, truthy, workspace_loads};
use comandos_core::usage_state::{self, PyNum, UsageError};
use rusqlite::{
    Connection, Row, ToSql, Transaction, TransactionBehavior, params, params_from_iter,
    types::ValueRef,
};
use serde_json::{Map, Value};
use std::collections::HashSet;

type Object = Map<String, Value>;

#[derive(Debug)]
pub enum ReadError {
    Sql(rusqlite::Error),
    /// BLOB o texto no UTF-8 en una columna leída.
    Undecodable,
    /// Excepción que el Python no captura (`TypeError`, `ValueError`, `OverflowError`): 500.
    Raises,
    /// No reproducible con certeza: la ruta declina.
    Unsure,
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(e) => e.fmt(f),
            Self::Undecodable => f.write_str("valor de la base de uso no decodificable"),
            Self::Raises => f.write_str("el Python lanzaría una excepción"),
            Self::Unsure => f.write_str("resultado no reproducible con certeza"),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<rusqlite::Error> for ReadError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}

impl From<UsageError> for ReadError {
    fn from(e: UsageError) -> Self {
        match e {
            UsageError::Overflow | UsageError::Raises => Self::Raises,
            UsageError::Unsure => Self::Unsure,
        }
    }
}

pub type Result<T> = std::result::Result<T, ReadError>;

/// Filas de `build_usage_state` (cc_usage.py:876-886).
pub struct StateRows {
    /// Los turnos de la ventana, compactos (`StateTurns`): no se guarda cada fila.
    pub turns: usage_state::StateTurns,
    pub provider_usage: Vec<Object>,
    pub provider_costs: Vec<Object>,
}

/// `USAGE_LIMIT_KEYS` (cc_usage.py:21).
const USAGE_LIMIT_KEYS: &[&str] = &[
    "COMANDOS_CODEX_DAILY_TOKEN_LIMIT",
    "COMANDOS_CODEX_WEEKLY_TOKEN_LIMIT",
    "COMANDOS_CLAUDE_DAILY_TOKEN_LIMIT",
    "COMANDOS_CLAUDE_WEEKLY_TOKEN_LIMIT",
    "COMANDOS_DAILY_BUDGET_USD",
];

/// `TASK_TYPES` (cc_usage.py:2341).
const TASK_TYPES: &[&str] = &[
    "implementation",
    "debugging",
    "testing",
    "review",
    "architecture",
    "research",
    "documentation",
    "operations",
    "other",
    "unclassified",
];

// ---------------------------------------------------------------- filas

/// Un valor de SQLite como lo entrega `sqlite3` de Python.
fn cell(value: ValueRef<'_>) -> Result<Value> {
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => Value::from(n),
        ValueRef::Real(x) => usage_state::float_value(x),
        ValueRef::Text(t) => Value::String(
            std::str::from_utf8(t)
                .map_err(|_| ReadError::Undecodable)?
                .to_owned(),
        ),
        ValueRef::Blob(_) => return Err(ReadError::Undecodable),
    })
}

fn column(row: &Row<'_>, index: usize) -> Result<Value> {
    cell(row.get_ref(index)?)
}

/// `dict(sqlite3.Row)`: cada nombre toma el valor de la primera columna con ese nombre
/// (sin distinguir mayúsculas ASCII, como `Row.__getitem__`).
fn row_object(row: &Row<'_>) -> Result<Object> {
    let names: Vec<&str> = row.as_ref().column_names();
    let mut out = Object::new();
    for name in &names {
        let first = names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .unwrap_or_default();
        out.insert((*name).to_owned(), column(row, first)?);
    }
    Ok(out)
}

fn rows(conn: &Connection, sql: &str, args: &[&dyn ToSql]) -> Result<Vec<Object>> {
    let mut stmt = conn.prepare(sql)?;
    let mut cursor = stmt.query(args)?;
    let mut out = Vec::new();
    while let Some(row) = cursor.next()? {
        out.push(row_object(row)?);
    }
    Ok(out)
}

fn get<'a>(row: &'a Object, key: &str) -> &'a Value {
    static NULL: Value = Value::Null;
    row.get(key).unwrap_or(&NULL)
}

/// `int(value)` sin capturar (`int(row["rating"])`).
fn py_int(value: &Value) -> Result<i64> {
    match PyNum::of(value)? {
        Some(PyNum::Int(n)) => Ok(n),
        Some(PyNum::Float(x)) if x.is_finite() => {
            let t = x.trunc();
            if t.abs() < 2f64.powi(63) {
                Ok(t as i64)
            } else {
                Err(ReadError::Unsure)
            }
        }
        Some(PyNum::Float(_)) => Err(ReadError::Raises),
        None => match value {
            Value::String(s) => match comandos_core::text::int(s) {
                Ok(n) => Ok(n),
                Err(comandos_core::text::NumError::Invalid) => Err(ReadError::Raises),
                Err(comandos_core::text::NumError::Exotic) => Err(ReadError::Unsure),
            },
            _ => Err(ReadError::Raises),
        },
    }
}

/// `int(value or 0)`.
fn int_or_zero(value: &Value) -> Result<i64> {
    if truthy(value) { py_int(value) } else { Ok(0) }
}

/// Número de una celda; `Raises` si el Python operaría con un texto o `None`.
fn number(value: &Value) -> Result<PyNum> {
    PyNum::of(value)?.ok_or(ReadError::Raises)
}

/// `value or default` cuando el resultado se concatena como `str`.
fn str_or(value: &Value, default: &str) -> Result<String> {
    if !truthy(value) {
        return Ok(default.to_owned());
    }
    value.as_str().map(str::to_owned).ok_or(ReadError::Raises)
}

fn checked_add(a: i64, b: i64) -> Result<i64> {
    a.checked_add(b).ok_or(ReadError::Unsure)
}

/// División verdadera de Python entre enteros pequeños.
fn ratio(a: i64, b: i64) -> Result<f64> {
    let limit = 1i64 << 53;
    if a.abs() > limit || b.abs() > limit {
        return Err(ReadError::Unsure);
    }
    Ok(a as f64 / b as f64)
}

fn float(x: f64) -> Value {
    usage_state::float_value(x)
}

fn opt_float(x: Option<f64>) -> Value {
    x.map_or(Value::Null, float)
}

/// Clave hashable de Python para conjuntos de valores (`1 == 1.0 == True`).
fn set_key(value: &Value) -> Result<String> {
    Ok(match value {
        Value::String(s) => format!("s{s}"),
        Value::Null => "n".into(),
        Value::Bool(_) | Value::Number(_) => match number(value)? {
            PyNum::Int(n) => format!("i{n}"),
            PyNum::Float(x) if x.fract() == 0.0 && x.abs() < 2f64.powi(63) => {
                format!("i{}", x as i64)
            }
            PyNum::Float(x) => format!("f{}", x.to_bits()),
        },
        Value::Array(_) | Value::Object(_) => return Err(ReadError::Raises),
    })
}

// ---------------------------------------------------------------- lecturas

/// `read_usage_settings` (cc_usage.py:543): las claves de límites, en orden de filas.
pub fn usage_settings(conn: &Connection) -> Result<Vec<(String, Value)>> {
    let mut out: Vec<(String, Value)> = Vec::new();
    for row in rows(conn, "select key, value from usage_settings", &[])? {
        let Some(key) = get(&row, "key").as_str() else {
            continue;
        };
        if !USAGE_LIMIT_KEYS.contains(&key) {
            continue;
        }
        let value = get(&row, "value").clone();
        match out.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => out.push((key.to_owned(), value)),
        }
    }
    Ok(out)
}

/// Los tres `select` de `build_usage_state` (cc_usage.py:876-886). Los turnos de
/// 14 días (decenas de miles con la base real) se compactan según se leen: como
/// objetos JSON ocupaban cientos de MiB que la arena del carril ya no devolvía.
pub fn state_rows(conn: &Connection, since: i64) -> Result<StateRows> {
    let mut turns = usage_state::StateTurns::new();
    {
        // Las once columnas, en el orden de `TURN_TEXT` y luego las numéricas:
        // cada fila se lee por índice, sin el objeto de `dict(sqlite3.Row)`.
        let mut stmt = conn.prepare(
            "select tmux_session, tmux_pane, pane_pwd, git_root, agent, provider,\
             \x20model, confidence, cost_usd, total_tokens, turn_finished_at\
             \x20from usage_turns where turn_finished_at >= ? order by turn_finished_at desc",
        )?;
        let mut cursor = stmt.query(params![since])?;
        while let Some(row) = cursor.next()? {
            let text_cell = |i: usize| -> Result<usage_state::TurnCell<'_>> {
                Ok(match row.get_ref(i)? {
                    ValueRef::Text(t) => usage_state::TurnCell::Text(
                        std::str::from_utf8(t).map_err(|_| ReadError::Undecodable)?,
                    ),
                    other => usage_state::TurnCell::Value(cell(other)?),
                })
            };
            let text = [
                text_cell(0)?,
                text_cell(1)?,
                text_cell(2)?,
                text_cell(3)?,
                text_cell(4)?,
                text_cell(5)?,
                text_cell(6)?,
                text_cell(7)?,
            ];
            let (cost, tokens, finished) = (column(row, 8)?, column(row, 9)?, column(row, 10)?);
            turns.push_cells(text, &finished, &tokens, &cost);
        }
    }
    turns.finish();
    Ok(StateRows {
        turns,
        provider_usage: rows(
            conn,
            "select * from provider_usage_buckets order by end_time desc",
            &[],
        )?,
        provider_costs: rows(
            conn,
            "select * from provider_cost_buckets order by end_time desc",
            &[],
        )?,
    })
}

/// `list_panes` (cc_usage.py:535).
pub fn list_panes(conn: &Connection) -> Result<Vec<Object>> {
    rows(
        conn,
        "select * from usage_panes order by git_root, tmux_session, tmux_pane",
        &[],
    )
}

/// `list_alerts` (cc_usage.py:992).
pub fn list_alerts(conn: &Connection, limit: i64) -> Result<Vec<Object>> {
    rows(
        conn,
        "select * from usage_alerts order by created_at desc limit ?",
        &[&limit],
    )
}

/// `recent_interactions` (cc_usage.py:2713).
pub fn recent_interactions(conn: &Connection, limit: i64) -> Result<Vec<Object>> {
    let limit = limit.clamp(1, 100);
    rows(
        conn,
        "select i.id,i.started_at_ms,i.finished_at_ms,i.duration_ms,
          i.completion_status,i.confidence,r.rating,r.outcome,t.task_type,
          c.harness,c.motor,c.model,c.effort,c.route_id,c.harness_account,c.motor_account,
          (select count(*) from usage_tool_calls x where x.interaction_id=i.id) as tool_calls,
          (select count(*) from usage_tool_calls x where x.interaction_id=i.id and x.status='failed') as tool_errors
          from usage_interactions i
          left join usage_ratings r on r.interaction_id=i.id
          left join usage_tasks t on t.id=i.task_id
          left join usage_session_configs c on c.id=i.config_id
          where i.finished_at_ms is not null
          order by i.started_at_ms desc limit ?",
        &[&limit],
    )
}

/// Turnos y tramos de `analytics_week_payload` (`bin/cc-dash:6593-6603`). `wide` añade
/// `cost`, `model`, `session` y `agent` (la consulta viva de `sidebar`, D9).
pub fn week_rows(conn: &Connection, since: f64, wide: bool) -> Result<(Vec<Value>, Vec<Value>)> {
    let (mut turns, mut spans) = (Vec::new(), Vec::new());
    each_week_row(conn, since, wide, |kind, row| match kind {
        WeekRow::Turn => turns.push(row.clone()),
        WeekRow::Span => spans.push(row.clone()),
    })?;
    Ok((turns, spans))
}

/// De qué consulta de `week_rows` viene una fila.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeekRow {
    Turn,
    Span,
}

/// Las filas de `week_rows` (todos los turnos y después todos los tramos) una a
/// una, sin guardarlas: con la base real son decenas de miles de turnos en 17
/// días, y tenerlos todos como objetos JSON a la vez ocupaba cientos de MiB que
/// la arena del hilo del carril ya no devolvía. El objeto se reutiliza entre
/// filas; una fila no decodificable corta la lectura con el error.
pub fn each_week_row(
    conn: &Connection,
    since: f64,
    wide: bool,
    mut each: impl FnMut(WeekRow, &Value),
) -> Result<()> {
    let sql = if wide {
        "select provider, harness_account, git_root, pane_pwd, turn_started_at, turn_finished_at, total_tokens, \
         cost_usd, model, tmux_session, agent \
         from usage_turns where turn_finished_at >= ?"
    } else {
        "select provider, harness_account, git_root, pane_pwd, turn_started_at, turn_finished_at, total_tokens \
         from usage_turns where turn_finished_at >= ?"
    };
    let keys: &[&str] = if wide {
        &[
            "provider", "account", "git_root", "pane_pwd", "started", "finished", "tokens", "cost",
            "model", "session", "agent",
        ]
    } else {
        &[
            "provider", "account", "git_root", "pane_pwd", "started", "finished", "tokens",
        ]
    };
    keyed_each(conn, sql, since, keys, |row| each(WeekRow::Turn, row))?;
    keyed_each(
        conn,
        "select provider, account, git_root, started_at, finished_at from usage_spans where finished_at >= ?",
        since,
        &["provider", "account", "git_root", "started", "finished"],
        |row| each(WeekRow::Span, row),
    )
}

/// Filas leídas por posición y renombradas (`{"provider": r[0], …}`), entregadas
/// de una en una en el mismo objeto (las claves se crean una vez).
fn keyed_each(
    conn: &Connection,
    sql: &str,
    since: f64,
    keys: &[&str],
    mut each: impl FnMut(&Value),
) -> Result<()> {
    let mut stmt = conn.prepare(sql)?;
    let mut cursor = stmt.query(params![since])?;
    let mut item = Value::Object(Object::new());
    while let Some(row) = cursor.next()? {
        if let Value::Object(map) = &mut item {
            for (i, key) in keys.iter().enumerate() {
                let value = column(row, i)?;
                match map.get_mut(*key) {
                    Some(slot) => *slot = value,
                    None => {
                        map.insert((*key).to_owned(), value);
                    }
                }
            }
        }
        each(&item);
    }
    Ok(())
}

/// `quota_snapshots` (cc_usage.py:2036).
/// Turnos y tramos desde `since` para el tablero de Consumos, con los tokens
/// por clase (la caché leída aparte) y la sesión de cada turno.
pub fn board_rows(
    conn: &Connection,
    since: f64,
) -> Result<(
    Vec<comandos_core::usage_board::Turn>,
    Vec<comandos_core::usage_board::Span>,
)> {
    use comandos_core::usage_board::{Span, Turn};
    fn text(row: &Row<'_>, i: usize) -> rusqlite::Result<String> {
        Ok(row.get::<_, Option<String>>(i)?.unwrap_or_default())
    }
    fn num(row: &Row<'_>, i: usize) -> rusqlite::Result<f64> {
        Ok(match row.get_ref(i)? {
            ValueRef::Integer(n) => n as f64,
            ValueRef::Real(f) => f,
            _ => 0.0,
        })
    }
    let mut turns = Vec::new();
    let mut stmt = conn.prepare(
        "select provider, harness_account, git_root, pane_pwd, tmux_session, turn_finished_at, \
         input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, model \
         from usage_turns where turn_finished_at >= ?",
    )?;
    let mut cursor = stmt.query(params![since])?;
    while let Some(row) = cursor.next()? {
        turns.push(Turn {
            provider: text(row, 0)?,
            account: text(row, 1)?,
            git_root: text(row, 2)?,
            pane_pwd: text(row, 3)?,
            session: text(row, 4)?,
            at: num(row, 5)?,
            input: num(row, 6)? as i64,
            output: num(row, 7)? as i64,
            cache_read: num(row, 8)? as i64,
            cache_write: num(row, 9)? as i64,
            model: text(row, 10)?,
        });
    }
    let mut spans = Vec::new();
    let mut stmt = conn.prepare(
        "select provider, account, git_root, session_id, started_at, finished_at \
         from usage_spans where finished_at >= ?",
    )?;
    let mut cursor = stmt.query(params![since])?;
    while let Some(row) = cursor.next()? {
        spans.push(Span {
            provider: text(row, 0)?,
            account: text(row, 1)?,
            git_root: text(row, 2)?,
            session: text(row, 3)?,
            started: num(row, 4)?,
            finished: num(row, 5)?,
        });
    }
    Ok((turns, spans))
}

pub fn quota_snapshots(conn: &Connection, since: i64) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "select provider, account, win, scope, resets_at, percent from usage_quota_snapshots \
         where resets_at >= ? order by resets_at",
    )?;
    let mut cursor = stmt.query(params![since])?;
    let keys = [
        "provider",
        "account",
        "window",
        "scope",
        "resets_at",
        "percent",
    ];
    let mut out = Vec::new();
    while let Some(row) = cursor.next()? {
        let mut item = Object::new();
        for (i, key) in keys.iter().enumerate() {
            item.insert((*key).to_owned(), column(row, i)?);
        }
        out.push(Value::Object(item));
    }
    Ok(out)
}

/// `_measured_usage` (cc_usage.py:1935): `day_start` es la medianoche local de `now`
/// (`LocalZone::day_start`). Sin turnos en 7 días → `None`.
pub fn measured_usage(
    conn: &Connection,
    provider: &str,
    now: i64,
    day_start: i64,
) -> Result<Option<Value>> {
    // Cada consulta se prepara una vez por llamada y se reutiliza (el `prepare_cached`
    // de rusqlite pide su rasgo `cache` y una dependencia más).
    let mut window = conn.prepare(
        "SELECT COUNT(*), COALESCE(SUM(total_tokens),0), MAX(turn_finished_at) \
         FROM usage_turns WHERE provider=? AND turn_finished_at >= ?",
    )?;
    let mut agg = |since: i64| -> Result<(i64, i64, i64)> {
        let mut cursor = window.query(params![provider, since])?;
        let row = cursor.next()?.ok_or(ReadError::Unsure)?;
        Ok((
            int_or_zero(&column(row, 0)?)?,
            int_or_zero(&column(row, 1)?)?,
            int_or_zero(&column(row, 2)?)?,
        ))
    };
    let today = agg(day_start)?;
    let week = agg(now - 7 * 86_400)?;
    let mut daily = Vec::new();
    let mut day = conn.prepare(
        "SELECT COALESCE(SUM(total_tokens),0) FROM usage_turns \
         WHERE provider=? AND turn_finished_at >= ? AND turn_finished_at < ?",
    )?;
    for d in (0..14).rev() {
        let start = day_start - d * 86_400;
        let mut cursor = day.query(params![provider, start, start + 86_400])?;
        let row = cursor.next()?.ok_or(ReadError::Unsure)?;
        let mut item = Object::new();
        item.insert("day".into(), start.into());
        item.insert("tokens".into(), int_or_zero(&column(row, 0)?)?.into());
        daily.push(Value::Object(item));
    }
    if week.0 == 0 {
        return Ok(None);
    }
    let mut out = Object::new();
    out.insert("tokens_today".into(), today.1.into());
    out.insert("turns_today".into(), today.0.into());
    out.insert("tokens_7d".into(), week.1.into());
    out.insert("turns_7d".into(), week.0.into());
    out.insert("last_at".into(), week.2.into());
    out.insert("daily".into(), Value::Array(daily));
    Ok(Some(Value::Object(out)))
}

/// `os.path.basename(path)`.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `json.loads(raw or "{}").get("path", "")` dentro del `try` de la guardia: solo una
/// ruta de texto cuenta; cualquier otra cosa se salta como el `except Exception`.
fn raw_path(raw: &Value) -> Option<String> {
    let text = match raw {
        v if !truthy(v) => "{}",
        Value::String(s) => s.as_str(),
        _ => return None,
    };
    let parsed = workspace_loads(text).ok()?;
    match parsed.as_object()?.get("path") {
        Some(Value::String(path)) => Some(path.clone()),
        _ => None,
    }
}

/// `token_guard_report` (cc_usage.py:2806). Detector local; nunca pausa nada.
pub fn token_guard_report(conn: &Connection, now: i64) -> Result<Value> {
    let since_hour = now - 3600;
    let since_ten = now - 600;
    let found = rows(
        conn,
        "select git_root,model,total_tokens,turn_finished_at,raw
          from usage_turns where source='claude_jsonl' and turn_finished_at>=?
          and lower(model) like 'claude-%'",
        &[&since_hour],
    )?;
    struct Project {
        root: Value,
        calls10m: i64,
        calls_hour: i64,
        tokens_hour: i64,
        subagents: HashSet<String>,
        models: Vec<(String, i64)>,
    }
    let mut projects: Vec<Project> = Vec::new();
    for row in &found {
        let root = match get(row, "git_root") {
            v if truthy(v) => v.clone(),
            _ => Value::String("unknown".into()),
        };
        if !root.is_string() {
            return Err(ReadError::Raises);
        }
        let index = match projects.iter().position(|p| python_eq(&p.root, &root)) {
            Some(i) => i,
            None => {
                projects.push(Project {
                    root: root.clone(),
                    calls10m: 0,
                    calls_hour: 0,
                    tokens_hour: 0,
                    subagents: HashSet::new(),
                    models: Vec::new(),
                });
                projects.len() - 1
            }
        };
        let tokens = usage_state::as_int(get(row, "total_tokens"), 0)?;
        let finished = usage_state::as_int(get(row, "turn_finished_at"), 0)?;
        let model = usage_state::text(get(row, "model"))?;
        let path = raw_path(get(row, "raw"));
        let Some(item) = projects.get_mut(index) else {
            continue;
        };
        item.calls_hour += 1;
        item.tokens_hour = checked_add(item.tokens_hour, tokens)?;
        if finished >= since_ten {
            item.calls10m += 1;
        }
        match item.models.iter_mut().find(|(m, _)| *m == model) {
            Some((_, n)) => *n += 1,
            None => item.models.push((model, 1)),
        }
        if let Some(path) = path
            && path.contains("/subagents/")
        {
            item.subagents.insert(basename(&path).to_owned());
        }
    }
    let mut output: Vec<(u8, i64, Value)> = Vec::new();
    for p in projects {
        let root = p.root.as_str().unwrap_or_default();
        let name = basename(root.trim_end_matches('/'));
        let mut top: Option<&(String, i64)> = None;
        for entry in &p.models {
            if top.is_none_or(|t| entry.1 > t.1) {
                top = Some(entry);
            }
        }
        let subagents = i64::try_from(p.subagents.len()).unwrap_or(i64::MAX);
        let critical = p.calls10m >= 300 || p.tokens_hour >= 100_000_000 || subagents >= 20;
        let warning = p.calls10m >= 120 || p.tokens_hour >= 40_000_000 || subagents >= 8;
        let (rank, level, action) = if critical {
            (
                0,
                "critical",
                "Revisar fan-out y encolar un modelo barato; no se congelará la sesión.",
            )
        } else if warning {
            (1, "warning", "Vigilar ritmo y contexto.")
        } else {
            (2, "normal", "Dentro de límites locales.")
        };
        let mut item = Object::new();
        item.insert(
            "project".into(),
            if name.is_empty() { "unknown" } else { name }.into(),
        );
        item.insert(
            "path".into(),
            if root == "unknown" { "" } else { root }.into(),
        );
        item.insert("calls10m".into(), p.calls10m.into());
        item.insert("callsHour".into(), p.calls_hour.into());
        item.insert("tokensHour".into(), p.tokens_hour.into());
        item.insert("subagentFiles".into(), subagents.into());
        item.insert(
            "topModel".into(),
            top.map(|(m, _)| m.clone()).unwrap_or_default().into(),
        );
        item.insert("level".into(), level.into());
        item.insert("action".into(), action.into());
        output.push((rank, p.tokens_hour, Value::Object(item)));
    }
    // `sort` estable por `(nivel, -tokensHour)`.
    output.sort_by_key(|(rank, tokens, _)| (*rank, std::cmp::Reverse(*tokens)));
    let critical = output.iter().filter(|(r, _, _)| *r == 0).count();
    let warning = output.iter().filter(|(r, _, _)| *r == 1).count();
    let mut policy = Object::new();
    for (key, value) in [
        ("calls10mWarning", 120i64),
        ("calls10mCritical", 300),
        ("tokensHourWarning", 40_000_000),
        ("tokensHourCritical", 100_000_000),
        ("subagentsWarning", 8),
        ("subagentsCritical", 20),
    ] {
        policy.insert(key.into(), value.into());
    }
    policy.insert("automaticFreeze".into(), false.into());
    let mut out = Object::new();
    out.insert("generatedAt".into(), now.into());
    out.insert("policy".into(), Value::Object(policy));
    out.insert(
        "projects".into(),
        Value::Array(output.into_iter().map(|(_, _, v)| v).collect()),
    );
    out.insert("critical".into(), critical.into());
    out.insert("warning".into(), warning.into());
    Ok(Value::Object(out))
}

/// Fecha UTC ISO de `datetime.fromtimestamp(ms / 1000, timezone.utc).date()`.
fn utc_date(ms: &Value) -> Result<String> {
    let seconds = match number(ms)? {
        PyNum::Int(n) => ratio(n, 1000)?,
        PyNum::Float(x) => x / 1000.0,
    };
    if !seconds.is_finite() {
        return Err(ReadError::Raises);
    }
    // `_fromtimestamp`: parte entera y microsegundos redondeados (mitad al par).
    let mut whole = seconds.trunc();
    let us = ((seconds - whole) * 1e6).round_ties_even();
    if us >= 1e6 {
        whole += 1.0;
    } else if us < 0.0 {
        whole -= 1.0;
    }
    if whole.abs() >= 2f64.powi(62) {
        return Err(ReadError::Raises);
    }
    let date = chrono::DateTime::from_timestamp(whole as i64, 0)
        .map(|d| d.date_naive())
        .ok_or(ReadError::Raises)?;
    if !(1..=9999).contains(&chrono::Datelike::year(&date)) {
        return Err(ReadError::Raises);
    }
    Ok(date.format("%Y-%m-%d").to_string())
}

struct Variant {
    index: i64,
    fields: Object,
    solved: i64,
    failed: i64,
    ratings: Vec<i64>,
    durations: Vec<i64>,
}

struct Experiment {
    id: Value,
    label: Value,
    task_type: Value,
    /// `tasks[task_id][variant_index] = row` (la última fila gana).
    tasks: Vec<(Value, Vec<(i64, Object)>)>,
    variants: Vec<Variant>,
}

const JUDGED: &[&str] = &["solved", "failed", "partial"];

/// `_paired_experiment_analytics` (cc_usage.py:2919).
fn paired_experiment_analytics(conn: &Connection) -> Result<Vec<Value>> {
    let found = rows(
        conn,
        "select e.id as experiment_id,e.label,e.task_type,
      x.variant_index,x.harness,x.motor,x.model,x.effort,x.route_id,
      i.task_id,r.outcome,r.rating,i.duration_ms
      from usage_experiments e join usage_experiment_runs x on x.experiment_id=e.id
      join usage_interactions i on i.id=x.interaction_id
      left join usage_ratings r on r.interaction_id=i.id
      where x.status='completed' and i.finished_at_ms is not null and i.task_id!=''
      order by e.id,i.task_id,x.variant_index",
        &[],
    )?;
    let mut experiments: Vec<Experiment> = Vec::new();
    for row in found {
        let id = get(&row, "experiment_id").clone();
        let index = match experiments.iter().position(|e| python_eq(&e.id, &id)) {
            Some(i) => i,
            None => {
                experiments.push(Experiment {
                    id: id.clone(),
                    label: get(&row, "label").clone(),
                    task_type: get(&row, "task_type").clone(),
                    tasks: Vec::new(),
                    variants: Vec::new(),
                });
                experiments.len() - 1
            }
        };
        let Some(exp) = experiments.get_mut(index) else {
            continue;
        };
        let variant_index = py_int(get(&row, "variant_index"))?;
        let task_id = get(&row, "task_id").clone();
        if !exp.variants.iter().any(|v| v.index == variant_index) {
            let mut fields = Object::new();
            fields.insert("variantIndex".into(), variant_index.into());
            for (key, column) in [
                ("harness", "harness"),
                ("motor", "motor"),
                ("model", "model"),
                ("effort", "effort"),
                ("routeId", "route_id"),
            ] {
                fields.insert(key.into(), get(&row, column).clone());
            }
            exp.variants.push(Variant {
                index: variant_index,
                fields,
                solved: 0,
                failed: 0,
                ratings: Vec::new(),
                durations: Vec::new(),
            });
        }
        let task = match exp.tasks.iter().position(|(t, _)| python_eq(t, &task_id)) {
            Some(i) => i,
            None => {
                exp.tasks.push((task_id, Vec::new()));
                exp.tasks.len() - 1
            }
        };
        if let Some((_, slots)) = exp.tasks.get_mut(task) {
            match slots.iter_mut().find(|(i, _)| *i == variant_index) {
                Some((_, slot)) => *slot = row,
                None => slots.push((variant_index, row)),
            }
        }
    }
    let mut output = Vec::new();
    for mut exp in experiments {
        let mut indexes: Vec<i64> = exp.variants.iter().map(|v| v.index).collect();
        indexes.sort_unstable();
        let judged = |row: &Object| {
            get(row, "outcome")
                .as_str()
                .is_some_and(|o| JUDGED.contains(&o))
        };
        let complete: Vec<&Vec<(i64, Object)>> = exp
            .tasks
            .iter()
            .map(|(_, slots)| slots)
            .filter(|slots| {
                !indexes.is_empty()
                    && indexes.iter().all(|i| {
                        slots
                            .iter()
                            .find(|(j, _)| j == i)
                            .is_some_and(|(_, row)| judged(row))
                    })
            })
            .collect();
        for slots in &complete {
            for index in &indexes {
                let Some((_, row)) = slots.iter().find(|(j, _)| j == index) else {
                    continue;
                };
                let Some(variant) = exp.variants.iter_mut().find(|v| v.index == *index) else {
                    continue;
                };
                match get(row, "outcome").as_str() {
                    Some("solved") => variant.solved += 1,
                    Some("failed") => variant.failed += 1,
                    _ => {}
                }
                if !get(row, "rating").is_null() {
                    variant.ratings.push(py_int(get(row, "rating"))?);
                }
                if !get(row, "duration_ms").is_null() {
                    variant.durations.push(py_int(get(row, "duration_ms"))?);
                }
            }
        }
        let mut variants: Vec<(i64, f64, f64, Option<f64>, Value)> = Vec::new();
        for index in &indexes {
            let Some(variant) = exp.variants.iter_mut().find(|v| v.index == *index) else {
                continue;
            };
            let binary = variant.solved + variant.failed;
            let (lo, hi) = usage_state::wilson_interval(variant.solved, binary);
            let mut durations = std::mem::take(&mut variant.durations);
            durations.sort_unstable();
            let ratings = std::mem::take(&mut variant.ratings);
            let rate = if binary != 0 {
                Some(ratio(variant.solved, binary)?)
            } else {
                None
            };
            let mut fields = std::mem::take(&mut variant.fields);
            fields.insert("solved".into(), variant.solved.into());
            fields.insert("failed".into(), variant.failed.into());
            fields.insert("successRate".into(), opt_float(rate));
            fields.insert("successCI".into(), Value::Array(vec![float(lo), float(hi)]));
            fields.insert("ratingMean".into(), mean(&ratings)?);
            fields.insert(
                "medianDurationMs".into(),
                durations
                    .get(durations.len() / 2)
                    .map_or(Value::Null, |d| Value::from(*d)),
            );
            variants.push((variant.index, lo, hi, rate, Value::Object(fields)));
        }
        let eligible = complete.len() >= 10 && variants.len() >= 2;
        let mut winner = Value::Null;
        if eligible {
            let mut ranked: Vec<&(i64, f64, f64, Option<f64>, Value)> =
                variants.iter().filter(|v| v.3.is_some()).collect();
            // `sorted(…, reverse=True)` es estable: los empates conservan su orden.
            ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((first, rest)) = ranked.split_first()
                && !rest.is_empty()
            {
                let best_rest =
                    rest.iter()
                        .map(|v| v.2)
                        .fold(None, |acc: Option<f64>, x| match acc {
                            Some(m) if x.partial_cmp(&m) != Some(std::cmp::Ordering::Greater) => {
                                Some(m)
                            }
                            _ => Some(x),
                        });
                if best_rest.is_some_and(|m| first.1 > m) {
                    winner = Value::from(first.0);
                }
            }
        }
        let complete_pairs = complete.len();
        let mut item = Object::new();
        item.insert("id".into(), std::mem::take(&mut exp.id));
        item.insert("label".into(), std::mem::take(&mut exp.label));
        item.insert("taskType".into(), std::mem::take(&mut exp.task_type));
        item.insert("completePairs".into(), complete_pairs.into());
        item.insert("eligible".into(), eligible.into());
        item.insert("winnerVariant".into(), winner);
        item.insert(
            "variants".into(),
            Value::Array(variants.into_iter().map(|v| v.4).collect()),
        );
        output.push(Value::Object(item));
    }
    Ok(output)
}

/// `sum(xs) / len(xs) if xs else None`.
fn mean(values: &[i64]) -> Result<Value> {
    if values.is_empty() {
        return Ok(Value::Null);
    }
    let mut total = 0i64;
    for v in values {
        total = checked_add(total, *v)?;
    }
    let len = i64::try_from(values.len()).map_err(|_| ReadError::Unsure)?;
    Ok(float(ratio(total, len)?))
}

/// `percentile` de una lista de enteros.
fn percentile_of(values: &[i64], q: f64) -> Value {
    let floats: Vec<f64> = values.iter().map(|v| *v as f64).collect();
    opt_float(usage_state::percentile(&floats, q))
}

struct Group {
    fields: Object,
    attempts: i64,
    labeled: i64,
    solved: i64,
    failed: i64,
    partial: i64,
    ratings: Vec<i64>,
    durations: Vec<i64>,
    tokens: Vec<i64>,
    cache_read: i64,
    reasoning: i64,
    tool_calls: i64,
    tool_errors: i64,
    task_ids: HashSet<String>,
    days: HashSet<String>,
}

/// El cuerpo del bucle de `experiment_analytics` para una interacción.
fn add_attempt(groups: &mut Vec<(String, Group)>, d: &Object) -> Result<()> {
    let key = format!(
        "{}|{}|{}|{}|{}",
        str_or(get(d, "route_id"), "unknown")?,
        str_or(get(d, "model"), "")?,
        str_or(get(d, "effort"), "")?,
        str_or(get(d, "harness_account"), "unknown")?,
        str_or(get(d, "motor_account"), "unknown")?
    );
    let index = match groups.iter().position(|(k, _)| *k == key) {
        Some(i) => i,
        None => {
            let value_or = |column: &str, default: &str| match get(d, column) {
                v if truthy(v) => v.clone(),
                _ => Value::String(default.into()),
            };
            let mut fields = Object::new();
            fields.insert("configKey".into(), key.clone().into());
            fields.insert("harness".into(), value_or("harness", "unknown"));
            fields.insert("motor".into(), value_or("motor", "unknown"));
            fields.insert("model".into(), value_or("model", ""));
            fields.insert("effort".into(), value_or("effort", ""));
            fields.insert(
                "harnessAccount".into(),
                value_or("harness_account", "unknown"),
            );
            fields.insert("motorAccount".into(), value_or("motor_account", "unknown"));
            groups.push((
                key,
                Group {
                    fields,
                    attempts: 0,
                    labeled: 0,
                    solved: 0,
                    failed: 0,
                    partial: 0,
                    ratings: Vec::new(),
                    durations: Vec::new(),
                    tokens: Vec::new(),
                    cache_read: 0,
                    reasoning: 0,
                    tool_calls: 0,
                    tool_errors: 0,
                    task_ids: HashSet::new(),
                    days: HashSet::new(),
                },
            ));
            groups.len() - 1
        }
    };
    let Some((_, g)) = groups.get_mut(index) else {
        return Ok(());
    };
    g.attempts += 1;
    if truthy(get(d, "task_id")) {
        g.task_ids.insert(set_key(get(d, "task_id"))?);
    }
    if truthy(get(d, "finished_at_ms")) {
        g.days.insert(utc_date(get(d, "finished_at_ms"))?);
    }
    let outcome = match get(d, "outcome") {
        v if truthy(v) => v.clone(),
        _ => Value::String("unknown".into()),
    };
    match outcome.as_str() {
        Some("solved") => {
            g.labeled += 1;
            g.solved += 1;
        }
        Some("failed") => {
            g.labeled += 1;
            g.failed += 1;
        }
        Some("partial") => {
            g.labeled += 1;
            g.partial += 1;
        }
        _ => {}
    }
    if !get(d, "rating").is_null() {
        g.ratings.push(py_int(get(d, "rating"))?);
    }
    if !get(d, "duration_ms").is_null() {
        g.durations.push(py_int(get(d, "duration_ms"))?);
    }
    if !get(d, "interaction_tokens").is_null() {
        g.tokens.push(py_int(get(d, "interaction_tokens"))?);
    }
    g.cache_read = checked_add(
        g.cache_read,
        usage_state::as_int(get(d, "cache_read_tokens"), 0)?,
    )?;
    g.reasoning = checked_add(
        g.reasoning,
        usage_state::as_int(get(d, "reasoning_tokens"), 0)?,
    )?;
    g.tool_calls = checked_add(g.tool_calls, usage_state::as_int(get(d, "tool_calls"), 0)?)?;
    g.tool_errors = checked_add(
        g.tool_errors,
        usage_state::as_int(get(d, "tool_errors"), 0)?,
    )?;
    Ok(())
}

/// `experiment_analytics` (cc_usage.py:2975). Un `task_type` fuera de `TASK_TYPES` es
/// el `ValueError("invalid task type")` interior.
pub fn experiment_analytics(
    conn: &Connection,
    days: i64,
    task_type: &str,
    now: f64,
) -> Result<std::result::Result<Value, String>> {
    let days = days.clamp(1, 30);
    let since = (now - (days * 86_400) as f64) * 1000.0;
    if !since.is_finite() || since.abs() >= 2f64.powi(63) {
        return Err(ReadError::Raises);
    }
    let since_ms = since.trunc() as i64;
    let mut sql_where = "where i.finished_at_ms>=?".to_owned();
    let mut args: Vec<Box<dyn ToSql>> = vec![Box::new(since_ms)];
    if !task_type.is_empty() {
        if !TASK_TYPES.contains(&task_type) {
            return Ok(Err("invalid task type".into()));
        }
        sql_where.push_str(" and t.task_type=?");
        args.push(Box::new(task_type.to_owned()));
    }
    let sql = format!(
        "select i.*,r.outcome,r.rating,t.task_type,
          c.harness,c.motor,c.model,c.effort,c.route_id,c.harness_account,c.motor_account,
          (select sum(u.total_tokens) from usage_turns u where u.interaction_id=i.id) as interaction_tokens,
          (select sum(u.cache_read_tokens) from usage_turns u where u.interaction_id=i.id) as cache_read_tokens,
          (select sum(u.reasoning_tokens) from usage_turns u where u.interaction_id=i.id) as reasoning_tokens,
          (select count(*) from usage_tool_calls x where x.interaction_id=i.id) as tool_calls,
          (select count(*) from usage_tool_calls x where x.interaction_id=i.id and x.status='failed') as tool_errors
          from usage_interactions i
          left join usage_ratings r on r.interaction_id=i.id
          left join usage_tasks t on t.id=i.task_id
          left join usage_session_configs c on c.id=i.config_id
          {sql_where}"
    );
    // Las filas se agregan según se leen (no se guardan: con la base real son
    // miles de objetos de ~30 columnas, ~20 MiB que la arena no devolvía). El
    // Python lee todas antes de agregar: un error de lectura de cualquier fila
    // y los de `paired_experiment_analytics` ganan al primero de la agregación,
    // que se guarda hasta el final.
    let mut groups: Vec<(String, Group)> = Vec::new();
    let mut failed: Option<ReadError> = None;
    {
        let mut stmt = conn.prepare(&sql)?;
        let mut cursor = stmt.query(params_from_iter(args.iter()))?;
        while let Some(row) = cursor.next()? {
            let d = row_object(row)?;
            if failed.is_none()
                && let Err(e) = add_attempt(&mut groups, &d)
            {
                failed = Some(e);
            }
        }
    }
    let paired = paired_experiment_analytics(conn)?;
    if let Some(e) = failed {
        return Err(e);
    }
    let mut out: Vec<(bool, f64, i64, Value)> = Vec::new();
    for (_, g) in groups {
        let judged = g.labeled;
        let (lo, hi) = usage_state::wilson_interval(g.solved, judged);
        let rate = |n: i64| -> Result<Value> {
            if judged != 0 {
                Ok(float(ratio(n, judged)?))
            } else {
                Ok(Value::Null)
            }
        };
        let distinct = i64::try_from(g.task_ids.len()).unwrap_or(i64::MAX);
        let active = i64::try_from(g.days.len()).unwrap_or(i64::MAX);
        let eligible = g.labeled >= 12 && distinct >= 6 && active >= 3 && (hi - lo) <= 0.50;
        let mut f = g.fields;
        f.insert("attempts".into(), g.attempts.into());
        f.insert("labeled".into(), g.labeled.into());
        f.insert("solved".into(), g.solved.into());
        f.insert("failed".into(), g.failed.into());
        f.insert("partial".into(), g.partial.into());
        f.insert("cacheRead".into(), g.cache_read.into());
        f.insert("reasoningTokens".into(), g.reasoning.into());
        f.insert("toolCalls".into(), g.tool_calls.into());
        f.insert("toolErrors".into(), g.tool_errors.into());
        f.insert("successRate".into(), rate(g.solved)?);
        f.insert("partialRate".into(), rate(g.partial)?);
        f.insert("successCI".into(), Value::Array(vec![float(lo), float(hi)]));
        f.insert("ratingMean".into(), mean(&g.ratings)?);
        f.insert("ratingN".into(), g.ratings.len().into());
        let p50 = percentile_of(&g.durations, 0.5);
        f.insert("durationP50Ms".into(), p50.clone());
        f.insert("durationP90Ms".into(), percentile_of(&g.durations, 0.9));
        f.insert("medianDurationMs".into(), p50);
        let t50 = percentile_of(&g.tokens, 0.5);
        f.insert("tokensP50".into(), t50.clone());
        f.insert("tokensP90".into(), percentile_of(&g.tokens, 0.9));
        f.insert("medianTokens".into(), t50);
        let tool_rate = if g.tool_calls != 0 {
            float(ratio(g.tool_errors, g.tool_calls)?)
        } else {
            Value::Null
        };
        f.insert("toolErrorRate".into(), tool_rate);
        f.insert("distinctTasks".into(), distinct.into());
        f.insert("activeDays".into(), active.into());
        f.insert("eligible".into(), eligible.into());
        let evidence = if eligible { "eligible" } else { "insufficient" };
        f.insert("evidence".into(), evidence.into());
        out.push((eligible, lo, g.attempts, Value::Object(f)));
    }
    // `sorted(key=(not eligible, -(lo if eligible else 0), -attempts))`, estable.
    out.sort_by(|a, b| {
        let ka = (!a.0, -(if a.0 { a.1 } else { 0.0 }), -a.2);
        let kb = (!b.0, -(if b.0 { b.1 } else { 0.0 }), -b.2);
        ka.0.cmp(&kb.0)
            .then(ka.1.partial_cmp(&kb.1).unwrap_or(std::cmp::Ordering::Equal))
            .then(ka.2.cmp(&kb.2))
    });
    let mut policy = Object::new();
    for (key, value) in [
        ("observationalMinLabeled", 12i64),
        ("minDistinctTasks", 6),
        ("minActiveDays", 3),
        ("recommendationMinLabeled", 20),
        ("pairedMinComplete", 10),
    ] {
        policy.insert(key.into(), value.into());
    }
    let mut result = Object::new();
    result.insert("days".into(), days.into());
    result.insert(
        "taskType".into(),
        if task_type.is_empty() {
            "all"
        } else {
            task_type
        }
        .into(),
    );
    result.insert(
        "configurations".into(),
        Value::Array(out.into_iter().map(|(_, _, _, v)| v).collect()),
    );
    result.insert("policy".into(), Value::Object(policy));
    result.insert("mode".into(), "observational".into());
    result.insert("pairedExperiments".into(), Value::Array(paired));
    result.insert(
        "disclaimer".into(),
        "Direccional; solo experimentos pareados pueden declarar ganador.".into(),
    );
    Ok(Ok(Value::Object(result)))
}

// ---------------------------------------------------------------- uso de extensiones

/// `re.fullmatch(r'[A-Za-z0-9._-]{1,80}', s)`.
fn valid_session(s: &str) -> bool {
    (1..=80).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `_NAME = ^[A-Za-z0-9_][A-Za-z0-9_.:@/-]{0,159}$` con `fullmatch`.
fn valid_name(s: &str) -> bool {
    let mut bytes = s.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    s.len() <= 160
        && (first.is_ascii_alphanumeric() || first == b'_')
        && bytes.all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'@' | b'/' | b'-')
        })
}

/// Validaciones de `extension_usage` (`lib/session_profiles.py:450-453`), en su orden.
/// La ruta las aplica antes de convertir `days` con `int()`, como el Python.
pub fn extension_scope_error(session: &str, pane: &str) -> Option<String> {
    if !session.is_empty() && !valid_session(session) {
        return Some("sesión inválida".into());
    }
    let pane_ok = pane
        .strip_prefix('%')
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
    if !pane.is_empty() && (session.is_empty() || !pane_ok) {
        return Some("panel inválido".into());
    }
    None
}

struct Extension {
    kind: &'static str,
    name: String,
    count: i64,
    last_seen: Option<PyNum>,
    duration: Option<PyNum>,
    observed: i64,
    failures: i64,
    sources: Vec<String>,
    confidence: Vec<String>,
}

/// `sorted(set(prev + (text or '').split(',')) - {''})`.
fn merge_csv(prev: &mut Vec<String>, value: &Value) -> Result<()> {
    let text = match value {
        v if !truthy(v) => "",
        Value::String(s) => s.as_str(),
        _ => return Err(ReadError::Raises),
    };
    let mut set: HashSet<String> = prev.drain(..).collect();
    set.extend(text.split(',').map(str::to_owned));
    set.remove("");
    let mut merged: Vec<String> = set.into_iter().collect();
    merged.sort();
    *prev = merged;
    Ok(())
}

fn number_or_zero(value: &Value) -> Result<PyNum> {
    if truthy(value) {
        number(value)
    } else {
        Ok(PyNum::Int(0))
    }
}

/// El resultado de `extension_usage` sin eventos (`lib/session_profiles.py:455-460`): lo
/// que el Python devuelve cuando la base no existe (`Path(db).is_file()` falso) o le
/// faltan las tablas. `days` es el `int(days)` de la ruta, aún sin acotar.
pub fn extension_usage_empty(session: &str, pane: &str, days: i64) -> Value {
    Value::Object(extension_empty(session, pane, days))
}

fn extension_empty(session: &str, pane: &str, days: i64) -> Object {
    let mut scope = Object::new();
    scope.insert("session".into(), session.into());
    scope.insert("pane".into(), pane.into());
    let mut result = Object::new();
    result.insert("scope".into(), Value::Object(scope));
    result.insert("days".into(), days.clamp(1, 90).into());
    result.insert(
        "provenance".into(),
        "usage_tool_calls + usage_interactions; observed hook events".into(),
    );
    result.insert("extensions".into(), Value::Array(Vec::new()));
    result.insert("unattributedSkillCalls".into(), 0.into());
    result.insert("tokens".into(), Value::Null);
    result.insert("costUsd".into(), Value::Null);
    result.insert(
        "coverage".into(),
        "Solo eventos capturados; ausencia de eventos no demuestra ausencia de uso.".into(),
    );
    result.insert("status".into(), "empty".into());
    result
}

/// `extension_usage` (`lib/session_profiles.py:449`) con `days` ya convertido por la
/// ruta. La comprobación de que el archivo existe (`Path(db).is_file()`) también es de
/// la ruta (D8): aquí la base ya está abierta.
pub fn extension_usage(
    conn: &Connection,
    session: &str,
    pane: &str,
    days: i64,
    now: f64,
) -> Result<std::result::Result<Value, String>> {
    if let Some(message) = extension_scope_error(session, pane) {
        return Ok(Err(message));
    }
    let mut result = extension_empty(session, pane, days);
    let days = days.clamp(1, 90);
    let mut tables = HashSet::new();
    {
        let mut stmt = conn.prepare("select name from sqlite_master where type='table'")?;
        let mut cursor = stmt.query([])?;
        while let Some(row) = cursor.next()? {
            if let Value::String(name) = column(row, 0)? {
                tables.insert(name);
            }
        }
    }
    if !tables.contains("usage_tool_calls") || !tables.contains("usage_interactions") {
        return Ok(Ok(Value::Object(result)));
    }
    let mut has_skill = false;
    {
        let mut stmt = conn.prepare("pragma table_info(usage_tool_calls)")?;
        let mut cursor = stmt.query([])?;
        while let Some(row) = cursor.next()? {
            if column(row, 1)?.as_str() == Some("skill_name") {
                has_skill = true;
            }
        }
    }
    let skill_col = if has_skill { "t.skill_name" } else { "''" };
    let since = (now - (days * 86_400) as f64) * 1000.0;
    if !since.is_finite() || since.abs() >= 2f64.powi(63) {
        return Err(ReadError::Raises);
    }
    let mut args: Vec<Box<dyn ToSql>> = vec![Box::new(since.trunc() as i64)];
    let mut sql_where = "coalesce(t.finished_at_ms,t.started_at_ms)>=?".to_owned();
    if !session.is_empty() {
        sql_where.push_str(" and i.tmux_session=?");
        args.push(Box::new(session.to_owned()));
    }
    if !pane.is_empty() {
        sql_where.push_str(" and i.tmux_pane=?");
        args.push(Box::new(pane.to_owned()));
    }
    let sql = format!(
        "select t.tool_name,{skill_col} as skill_name,count(*) as n,
            max(coalesce(t.finished_at_ms,t.started_at_ms)) as last_seen,
            sum(t.duration_ms) as duration, count(t.duration_ms) as measured,
            sum(case when t.status='failed' then 1 else 0 end) as failures,
            group_concat(distinct i.source) as sources, group_concat(distinct t.confidence) as confidence
            from usage_tool_calls t join usage_interactions i on i.id=t.interaction_id
            where {sql_where} group by t.tool_name,{skill_col} limit 2000"
    );
    let found = {
        let mut stmt = conn.prepare(&sql)?;
        let mut cursor = stmt.query(params_from_iter(args.iter()))?;
        let mut out = Vec::new();
        while let Some(row) = cursor.next()? {
            out.push(row_object(row)?);
        }
        out
    };
    let mut unattributed = PyNum::Int(0);
    let mut grouped: Vec<Extension> = Vec::new();
    for row in &found {
        let tool = get(row, "tool_name").as_str().ok_or(ReadError::Raises)?;
        let (kind, name) = if tool.to_lowercase() == "skill" {
            let skill = get(row, "skill_name");
            if !truthy(skill) {
                unattributed = unattributed.plus(number(get(row, "n"))?)?;
                continue;
            }
            ("skill", skill.as_str().ok_or(ReadError::Raises)?.to_owned())
        } else if tool.starts_with("mcp__") && tool.split("__").count() >= 3 {
            let name = tool.split("__").nth(1).unwrap_or_default();
            ("mcp", name.to_owned())
        } else {
            continue;
        };
        if !valid_name(&name) {
            continue;
        }
        let index = match grouped
            .iter()
            .position(|e| e.kind == kind && e.name == name)
        {
            Some(i) => i,
            None => {
                grouped.push(Extension {
                    kind,
                    name,
                    count: 0,
                    last_seen: None,
                    duration: None,
                    observed: 0,
                    failures: 0,
                    sources: Vec::new(),
                    confidence: Vec::new(),
                });
                grouped.len() - 1
            }
        };
        let Some(entry) = grouped.get_mut(index) else {
            continue;
        };
        let n = py_int(get(row, "n"))?;
        entry.count = checked_add(entry.count, n)?;
        // `max(lastSeen or 0, (last_seen or 0) / 1000)`: `max` devuelve el primero en empate.
        let seen = match number_or_zero(get(row, "last_seen"))? {
            PyNum::Int(v) => PyNum::Float(ratio(v, 1000)?),
            PyNum::Float(x) => PyNum::Float(x / 1000.0),
        };
        let current = entry
            .last_seen
            .filter(|v| v.truthy())
            .unwrap_or(PyNum::Int(0));
        entry.last_seen = Some(if seen.gt(current)? { seen } else { current });
        entry.failures = checked_add(entry.failures, int_or_zero(get(row, "failures"))?)?;
        let measured = number(get(row, "measured"))?;
        if measured.truthy() {
            let base = entry
                .duration
                .filter(|v| v.truthy())
                .unwrap_or(PyNum::Int(0));
            entry.duration = Some(base.plus(number(get(row, "duration"))?)?);
            entry.observed = checked_add(entry.observed, py_int(get(row, "measured"))?)?;
        }
        merge_csv(&mut entry.sources, get(row, "sources"))?;
        merge_csv(&mut entry.confidence, get(row, "confidence"))?;
    }
    let observed = !grouped.is_empty() || unattributed.truthy();
    // `sorted(key=(-count, name))`, estable.
    grouped.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    let extensions = grouped
        .into_iter()
        .map(|e| {
            let mut item = Object::new();
            item.insert("kind".into(), e.kind.into());
            item.insert("name".into(), e.name.into());
            item.insert("count".into(), e.count.into());
            item.insert(
                "lastSeen".into(),
                e.last_seen.map_or(Value::Null, PyNum::to_value),
            );
            item.insert(
                "durationMs".into(),
                e.duration.map_or(Value::Null, PyNum::to_value),
            );
            item.insert("durationObservedCalls".into(), e.observed.into());
            item.insert("failures".into(), e.failures.into());
            item.insert("tokens".into(), Value::Null);
            item.insert("costUsd".into(), Value::Null);
            item.insert("provenance".into(), "observed_tool_events".into());
            item.insert(
                "sources".into(),
                Value::Array(e.sources.into_iter().map(Value::String).collect()),
            );
            item.insert(
                "confidence".into(),
                Value::Array(e.confidence.into_iter().map(Value::String).collect()),
            );
            Value::Object(item)
        })
        .collect();
    result.insert("extensions".into(), Value::Array(extensions));
    result.insert("unattributedSkillCalls".into(), unattributed.to_value());
    result.insert(
        "status".into(),
        if observed { "observed" } else { "empty" }.into(),
    );
    result.insert("truncated".into(), (found.len() == 2000).into());
    Ok(Ok(Value::Object(result)))
}

// ---------------------------------------------------------------- escrituras ligeras

/// Un valor JSON enlazado como lo enlaza `sqlite3` de Python.
fn bind(value: &Value) -> Result<rusqlite::types::Value> {
    use rusqlite::types::Value as Sql;
    Ok(match value {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::String(s) => Sql::Text(s.clone()),
        Value::Number(_) => match number(value)? {
            PyNum::Int(n) => Sql::Integer(n),
            PyNum::Float(x) => Sql::Real(x),
        },
        // `sqlite3.InterfaceError`: tipo no admitido como parámetro.
        Value::Array(_) | Value::Object(_) => return Err(ReadError::Raises),
    })
}

/// `record_pane` (cc_usage.py:495): el `insert … on conflict` de 506-531.
pub fn record_pane(conn: &Connection, pane: &Object) -> Result<()> {
    const FIELDS: &[&str] = &[
        "tmux_session",
        "tmux_pane",
        "pane_pwd",
        "git_root",
        "tab_label",
        "agent",
        "provider",
        "agent_pid",
        "model",
        "reasoning_effort",
        "started_at",
        "last_seen_at",
        "raw",
    ];
    let mut values = Vec::with_capacity(FIELDS.len());
    for field in FIELDS {
        let default = if *field == "agent_pid" {
            Value::from(0)
        } else {
            Value::from("")
        };
        values.push(bind(pane.get(*field).unwrap_or(&default))?);
    }
    conn.execute(
        "
            insert into usage_panes (
              tmux_session, tmux_pane, pane_pwd, git_root, tab_label,
              agent, provider, agent_pid, model, reasoning_effort,
              started_at, last_seen_at, raw
            ) values (
              ?1, ?2, ?3, ?4, ?5,
              ?6, ?7, ?8, ?9, ?10,
              ?11, ?12, ?13
            )
            on conflict(tmux_session, tmux_pane) do update set
              pane_pwd=excluded.pane_pwd,
              git_root=excluded.git_root,
              tab_label=excluded.tab_label,
              agent=excluded.agent,
              provider=excluded.provider,
              agent_pid=excluded.agent_pid,
              model=excluded.model,
              reasoning_effort=excluded.reasoning_effort,
              last_seen_at=excluded.last_seen_at,
              raw=excluded.raw
            ",
        params_from_iter(values),
    )?;
    Ok(())
}

/// `record_pane` de cada pane de `usage_live_panes` (cc-dash:7253), en una
/// sola transacción: el error de un pane se ignora (su `except Exception:
/// pass`) y los demás se registran. Devuelve cuántos se registraron; un fallo
/// al abrir o confirmar la transacción es `Err` (que el que llama ignora).
pub fn record_panes(conn: &Connection, panes: &[Object]) -> Result<usize> {
    if panes.is_empty() {
        return Ok(0);
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    crate::migrate::move_db::admit_write(conn).map_err(|_| ReadError::Unsure)?;
    let recorded = panes
        .iter()
        .filter(|pane| record_pane(&tx, pane).is_ok())
        .count();
    tx.commit()?;
    Ok(recorded)
}

/// `round(x)` de Python para `float`: entero, mitad al par.
fn py_round(x: f64) -> Result<i64> {
    if x.is_nan() {
        return Err(ReadError::Raises);
    }
    let r = x.round_ties_even();
    if !r.is_finite() || r.abs() >= 2f64.powi(63) {
        return Err(if r.is_finite() {
            ReadError::Unsure
        } else {
            ReadError::Raises
        });
    }
    Ok(r as i64)
}

/// `float(value)` sin capturar.
fn py_float(value: &Value) -> Result<f64> {
    match PyNum::of(value)? {
        Some(n) => Ok(n.as_f64()),
        None => match value {
            Value::String(s) => match comandos_core::text::float(s) {
                Ok(x) => Ok(x),
                Err(comandos_core::text::NumError::Invalid) => Err(ReadError::Raises),
                Err(comandos_core::text::NumError::Exotic) => Err(ReadError::Unsure),
            },
            _ => Err(ReadError::Raises),
        },
    }
}

/// `record_quota_snapshots` (cc_usage.py:2012): última foto de cada ciclo (5 h / 7 d),
/// con el reset redondeado a la hora. Devuelve cuántas filas se enviaron.
pub fn record_quota_snapshots(conn: &Connection, rows: &[Value], now: i64) -> Result<usize> {
    let mut keep: Vec<[rusqlite::types::Value; 8]> = Vec::new();
    for r in rows {
        let Some(r) = r.as_object() else {
            return Err(ReadError::Raises);
        };
        let window = get(r, "window");
        if get(r, "percent").is_null()
            || !truthy(get(r, "resets_at"))
            || !matches!(window.as_str(), Some("5h" | "7d"))
        {
            continue;
        }
        let account = match get(r, "account") {
            v if truthy(v) => v.clone(),
            _ => Value::from("main"),
        };
        let resets = usage_state::as_int(get(r, "resets_at"), 0)?;
        let hour = py_round(ratio(resets, 3600)?)?
            .checked_mul(3600)
            .ok_or(ReadError::Unsure)?;
        let percent = py_float(get(r, "percent"))?;
        let captured = match usage_state::as_int(get(r, "captured_at"), 0)? {
            0 => now,
            n => n,
        };
        use rusqlite::types::Value as Sql;
        keep.push([
            Sql::Text(usage_state::text(get(r, "id"))?),
            Sql::Text(usage_state::text(get(r, "provider"))?),
            Sql::Text(usage_state::text(&account)?),
            Sql::Text(usage_state::text(window)?),
            Sql::Text(usage_state::text(get(r, "scope"))?),
            Sql::Integer(hour),
            Sql::Real(percent),
            Sql::Integer(captured),
        ]);
    }
    if keep.is_empty() {
        return Ok(0);
    }
    // `executemany` dentro de `with connect(...)`: una sola transacción.
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    crate::migrate::move_db::admit_write(conn).map_err(|_| ReadError::Unsure)?;
    let mut stmt = tx.prepare(
        "insert into usage_quota_snapshots (limit_id, provider, account, win, scope, resets_at, percent, captured_at) \
         values (?,?,?,?,?,?,?,?) on conflict(limit_id, resets_at) do update set \
         percent=excluded.percent, captured_at=excluded.captured_at \
         where excluded.captured_at >= usage_quota_snapshots.captured_at",
    )?;
    for row in &keep {
        stmt.execute(params_from_iter(row.iter()))?;
    }
    drop(stmt);
    tx.commit()?;
    Ok(keep.len())
}
