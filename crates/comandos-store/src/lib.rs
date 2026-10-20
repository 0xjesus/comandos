//! Native persistence adapters. Callers own connections and transactions.
pub mod domains;
pub mod extension_gate;
pub mod files;
pub mod intake;
pub mod marks;
pub mod migrate;
pub mod news;
pub mod pane_extensions;
pub mod session_profiles;
pub mod snapshot_files;
#[path = "state_db/mod.rs"]
pub mod state;
pub mod unified;
use comandos_core::event;
use rusqlite::{
    Connection, OptionalExtension, Row, params, params_from_iter,
    types::{Value as SqlValue, ValueRef},
};
use serde_json::{Map, Value};

#[derive(Debug)]
pub enum Error {
    ModeBusy,
    Validation(String),
    MissingEvent,
    Io(std::io::Error),
    Conflict(Value),
    Sql(rusqlite::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModeBusy => write!(
                f,
                "modo ocupado durante lectura: {}",
                std::fs::TryLockError::WouldBlock
            ),
            Self::Validation(s) => f.write_str(s),
            Self::Io(e) => e.fmt(f),
            Self::MissingEvent => f.write_str("evento inexistente"),
            Self::Conflict(_) => f.write_str("Revisión desactualizada"),
            Self::Sql(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;

/// How long a background read waits for a mode change to release the lock.
pub const BACKGROUND_READ_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// For worker threads and CLIs only: a writer owns the mode lock for tens of
/// milliseconds, so the read is retried until `wait` runs out instead of failing at
/// once. Request threads keep the nonblocking reads.
pub fn wait_admitted<T>(wait: std::time::Duration, mut read: impl FnMut() -> Result<T>) -> Result<T> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match read() {
            Err(Error::ModeBusy) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            result => return result,
        }
    }
}

// Joining a caller transaction must not commit or roll it back. Admission
// reads pin its SQLite snapshot: a deferred transaction cannot upgrade that
// snapshot after a mover commits (SQLITE_BUSY_SNAPSHOT). Owned
// transactions roll back on an error or unwind, and take the write lock first.
fn with_transaction<T>(conn: &Connection, run: impl FnOnce() -> Result<T>) -> Result<T> {
    if !conn.is_autocommit() {
        migrate::move_db::admit_write(conn)?;
        return run();
    }
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    migrate::move_db::admit_write(conn)?;
    let value = run()?;
    tx.commit()?;
    Ok(value)
}

const COLUMNS: &[(&str, &str)] = &[
    ("sequence", "sequence"),
    ("eventId", "event_id"),
    ("sourceEventId", "source_event_id"),
    ("source", "source"),
    ("harness", "harness"),
    ("projectKey", "project_key"),
    ("sessionKey", "session_key"),
    ("paneKey", "pane_key"),
    ("paneId", "pane_id"),
    ("processKey", "process_key"),
    ("conversationId", "conversation_id"),
    ("turnId", "turn_id"),
    ("requestId", "request_id"),
    ("kind", "kind"),
    ("evidence", "evidence"),
    ("correlation", "correlation"),
    ("occurredAtMs", "occurred_at_ms"),
    ("receivedAtMs", "received_at_ms"),
    ("title", "title"),
    ("excerpt", "excerpt"),
];

fn select() -> String {
    format!(
        "SELECT {} FROM events",
        COLUMNS
            .iter()
            .map(|(_, col)| *col)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn row_event(row: &Row<'_>) -> rusqlite::Result<Value> {
    let mut out = Map::new();
    for (index, (name, _)) in COLUMNS.iter().enumerate() {
        let value = match row.get_ref(index)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(n) => n.into(),
            ValueRef::Real(n) => Value::from(n),
            ValueRef::Text(_) => row.get::<_, String>(index)?.into(),
            ValueRef::Blob(_) => {
                return Err(rusqlite::Error::InvalidColumnType(
                    index,
                    name.to_string(),
                    rusqlite::types::Type::Blob,
                ));
            }
        };
        out.insert((*name).into(), value);
    }
    let mut out = Value::Object(out);
    out["destination"] = event::destination(&out).into();
    Ok(out)
}

fn sql_value(value: &Value) -> Result<SqlValue> {
    match value {
        Value::Null => Ok(SqlValue::Null),
        Value::String(s) => Ok(SqlValue::Text(s.clone())),
        Value::Number(n) => n
            .as_i64()
            .map(SqlValue::Integer)
            .ok_or_else(|| Error::Validation("entero fuera de rango SQLite".into())),
        _ => Err(Error::Validation("valor SQLite inválido".into())),
    }
}

/// An existing caller transaction retains BEGIN/COMMIT/ROLLBACK ownership.
/// Otherwise the event and receipt commit together under an immediate write lock.
/// Supply fresh ids from the native adapter; local ids never affect deduplication.
pub fn append_event(
    conn: &Connection,
    input: &Value,
    now_ms: u64,
    new_event_id: &str,
    receipt_id: &str,
) -> Result<Value> {
    with_transaction(conn, || {
        let normalized = Value::Object(
            event::normalize(input, now_ms, new_event_id).map_err(Error::Validation)?,
        );
        let key = event::dedupe_key(&normalized);
        let existing = if let Some(key) = &key {
            conn.query_row(
                &format!("{} WHERE dedupe_key = ?", select()),
                [key],
                row_event,
            )
            .optional()?
        } else {
            None
        };
        let duplicate = existing.is_some();
        let mut stored = if let Some(existing) = existing {
            existing
        } else {
            let columns = COLUMNS[1..]
                .iter()
                .map(|(_, col)| *col)
                .collect::<Vec<_>>()
                .join(", ");
            let placeholders = vec!["?"; COLUMNS.len() - 1].join(", ");
            let mut values = COLUMNS[1..]
                .iter()
                .map(|(name, _)| sql_value(&normalized[*name]))
                .collect::<Result<Vec<_>>>()?;
            values.push(key.map(SqlValue::Text).unwrap_or(SqlValue::Null));
            conn.execute(
                &format!("INSERT INTO events ({columns}, dedupe_key) VALUES ({placeholders}, ?)"),
                params_from_iter(values),
            )?;
            get_event(
                conn,
                normalized["eventId"].as_str().expect("normalized eventId"),
            )?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?
        };
        conn.execute(
            "INSERT INTO event_receipts VALUES (?, ?, ?, ?, ?)",
            params![
                receipt_id,
                stored["eventId"].as_str(),
                normalized["source"].as_str(),
                sql_value(&normalized["receivedAtMs"])?,
                duplicate
            ],
        )?;
        stored["duplicate"] = duplicate.into();
        Ok(stored)
    })
}

pub fn get_event(conn: &Connection, event_id: &str) -> Result<Option<Value>> {
    Ok(conn
        .query_row(
            &format!("{} WHERE event_id = ?", select()),
            [event_id],
            row_event,
        )
        .optional()?)
}

/// Pagination coercion belongs to the HTTP adapter; this API takes integers.
pub fn list_events(conn: &Connection, after: i64, limit: i64) -> Result<Vec<Value>> {
    let mut statement = conn.prepare(&format!(
        "{} WHERE sequence > ? ORDER BY sequence LIMIT ?",
        select()
    ))?;
    Ok(statement
        .query_map(params![after.max(0), limit.clamp(1, 500)], row_event)?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn latest_sequence(conn: &Connection) -> Result<i64> {
    Ok(
        conn.query_row("SELECT COALESCE(MAX(sequence), 0) FROM events", [], |row| {
            row.get(0)
        })?,
    )
}

pub fn claim_delivery(
    conn: &Connection,
    event_id: &str,
    channel: &str,
    device_id: &str,
    now_ms: i64,
) -> Result<bool> {
    with_transaction(conn, || {
        if channel.is_empty() || channel.chars().count() > 64 {
            return Err(Error::Validation("channel inválido".into()));
        }
        if device_id.chars().count() > event::MAX_ID {
            return Err(Error::Validation("deviceId inválido".into()));
        }
        if get_event(conn, event_id)?.is_none() {
            return Err(Error::MissingEvent);
        }
        Ok(conn.execute("INSERT OR IGNORE INTO deliveries (event_id, channel, device_id, state, created_at_ms, updated_at_ms) VALUES (?, ?, ?, 'claimed', ?, ?)",
        params![event_id,channel,device_id,now_ms,now_ms])?==1)
    })
}

pub mod notifications;

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub mod workspace;

pub mod focus;
mod line_scan;
pub mod pomodoro;
pub mod usage;
pub mod usage_import;
pub mod usage_read;

pub mod operator;
