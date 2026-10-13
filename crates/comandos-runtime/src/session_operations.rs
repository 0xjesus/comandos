//! Durable isolated operation journal; owner, clock and terminal effects belong to callers.
use comandos_core::json::{dumps, response_dumps};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug)]
pub enum Error {
    Conflict(String),
    Persistence(String),
    Callback(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict(s) | Self::Persistence(s) | Self::Callback(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Persistence(e.to_string())
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Persistence(e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;
const SCHEMA:&str="
 CREATE TABLE IF NOT EXISTS session_operations (
 id TEXT PRIMARY KEY, pane_key TEXT NOT NULL, fingerprint TEXT NOT NULL,
 request TEXT NOT NULL, state TEXT NOT NULL, owner INTEGER NOT NULL,
 snapshot TEXT, result TEXT, updated REAL NOT NULL);
 CREATE UNIQUE INDEX IF NOT EXISTS session_operations_active_pane
 ON session_operations(pane_key) WHERE state NOT IN ('confirmed','failed','rolled_back');
 CREATE INDEX IF NOT EXISTS session_operations_target_updated
 ON session_operations(json_extract(request,'$.session'), json_extract(request,'$.pane'), updated DESC);
 CREATE INDEX IF NOT EXISTS session_operations_pane_updated
 ON session_operations(pane_key, updated DESC) WHERE snapshot IS NOT NULL;
 CREATE TABLE IF NOT EXISTS session_operation_events (
 operation_id TEXT NOT NULL, stage TEXT NOT NULL, detail TEXT NOT NULL, at REAL NOT NULL);";
fn reject_transaction(conn: &Connection) -> Result<()> {
    if !conn.is_autocommit() {
        return Err(Error::Persistence(
            "el diario requiere una transacción propia y duradera".into(),
        ));
    }
    Ok(())
}
fn atomic<T>(conn: &Connection, run: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
    reject_transaction(conn)?;
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    comandos_store::migrate::move_db::admit_write(conn)
        .map_err(|e| Error::Persistence(e.to_string()))?;
    let value = run(&tx)?;
    tx.commit()?;
    Ok(value)
}
fn initialize(conn: &Connection) -> Result<()> {
    atomic(conn, |tx| {
        tx.execute_batch(SCHEMA)?;
        Ok(())
    })
}
/// `json.dumps(x, sort_keys=True, separators=(",", ":"))`: huella y claves de agrupación.
fn canonical(value: &Value) -> Result<String> {
    dumps(value, true, true).map_err(Error::Persistence)
}
/// `json.dumps(x)` del Python (`stage`, `cancel_waiting`): orden de inserción,
/// ASCII escapado y separadores por omisión. Quien lee el journal (el
/// `/model/status` de los dos frentes) responde con ese orden.
fn python(value: &Value) -> Result<String> {
    response_dumps(value).map_err(Error::Persistence)
}
fn decode(raw: Option<String>) -> Result<Value> {
    match raw {
        Some(raw) if !raw.is_empty() => {
            serde_json::from_str(&raw).map_err(|e| Error::Persistence(e.to_string()))
        }
        _ => Ok(Value::Null),
    }
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}
/// Create/open only the separate journal, with private permissions, no application schema.
pub fn open_journal(path: &Path) -> Result<Connection> {
    if let comandos_store::migrate::DbLocation::Unified(path) =
        comandos_store::migrate::resolve_configured(path)
            .map_err(|e| Error::Persistence(e.to_string()))?
    {
        return comandos_store::unified::open_unified(&path)
            .map_err(|e| Error::Persistence(e.to_string()));
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(15))?;
    initialize(&conn)?;
    Ok(conn)
}
/// A borrowed connection must be in autocommit before every journal mutation.
/// Supply owner identity and a clock explicitly. This type does not inspect processes.
pub struct OperationStore<'a> {
    pub connection: &'a Connection,
    pub owner: &'a dyn Fn() -> i64,
    pub clock: &'a dyn Fn() -> f64,
}
impl<'a> OperationStore<'a> {
    pub fn new(
        connection: &'a Connection,
        owner: &'a dyn Fn() -> i64,
        clock: &'a dyn Fn() -> f64,
    ) -> Result<Self> {
        initialize(connection)?;
        Ok(Self {
            connection,
            owner,
            clock,
        })
    }
    pub fn get(&self, id: &str) -> Result<Option<Value>> {
        let row=self.connection.query_row("SELECT id,pane_key,fingerprint,request,state,owner,snapshot,result,updated FROM session_operations WHERE id=?",[id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,i64>(5)?,row.get::<_,Option<String>>(6)?,row.get::<_,Option<String>>(7)?,row.get::<_,f64>(8)?))).optional()?;
        row.map(|(id,pane_key,fingerprint,request,state,owner,snapshot,result,updated)|Ok(json!({"id":id,"pane_key":pane_key,"fingerprint":fingerprint,"request":decode(Some(request))?,"state":state,"owner":owner,"snapshot":decode(snapshot)?,"result":decode(result)?,"updated":updated}))).transpose()
    }
    pub fn claim(&self, id: &str, pane_key: &str, request: &Value) -> Result<bool> {
        reject_transaction(self.connection)?;
        let raw = canonical(request)?;
        let fingerprint = format!("{:x}", Sha256::digest(raw.as_bytes()));
        atomic(self.connection, |tx| {
            let existing: Option<String> = tx
                .query_row(
                    "SELECT fingerprint FROM session_operations WHERE id=?",
                    [id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(existing) = existing {
                return if existing == fingerprint {
                    Ok(false)
                } else {
                    Err(Error::Conflict(
                        "requestId ya se usó con otra configuración".into(),
                    ))
                };
            }
            match tx.execute(
                "INSERT INTO session_operations VALUES (?,?,?,?,?,?,NULL,NULL,?)",
                params![
                    id,
                    pane_key,
                    fingerprint,
                    raw,
                    "validating",
                    (self.owner)(),
                    (self.clock)()
                ],
            ) {
                Ok(_) => Ok(true),
                Err(rusqlite::Error::SqliteFailure(code, _))
                    if code.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    Err(Error::Conflict(
                        "el panel tiene una operación pendiente o necesita recuperación".into(),
                    ))
                }
                Err(error) => Err(error.into()),
            }
        })
    }
    pub fn cancel_waiting(&self, id: &str) -> Result<bool> {
        atomic(self.connection, |tx| {
            let result = python(
                &json!({"ok":false,"error":"operación cancelada antes de cerrar el origen","cancelled":true}),
            )?;
            Ok(tx.execute("UPDATE session_operations SET state='failed',result=?,updated=? WHERE id=? AND state IN ('validating','waiting','snapshot')",params![result,(self.clock)(),id])?!=0)
        })
    }
    pub fn claim_recovery(&self, id: &str) -> Result<bool> {
        atomic(self.connection, |tx| {
            Ok(tx.execute("UPDATE session_operations SET state='recovering',owner=?,updated=? WHERE id=? AND state IN ('recovery_required','awaiting_confirmation')",params![(self.owner)(),(self.clock)(),id])?!=0)
        })
    }
    pub fn saved_origin(&self, pane_key: &str, harness: &str) -> Result<Option<Value>> {
        let mut statement=self.connection.prepare("SELECT snapshot FROM session_operations WHERE pane_key=? AND snapshot IS NOT NULL ORDER BY updated DESC")?;
        let rows = statement.query_map([pane_key], |row| row.get::<_, String>(0))?;
        for raw in rows {
            let snapshot = decode(Some(raw?))?;
            let origin = &snapshot["origin"];
            if origin["agent"] == harness && truthy(&origin["observed"]["conversationId"]) {
                return Ok(Some(origin.clone()));
            }
        }
        Ok(None)
    }
    /// The operation that still holds `pane_key` (the unique active-pane index).
    pub fn active_for_pane(&self, pane_key: &str) -> Result<Option<Value>> {
        let id:Option<String>=self.connection.query_row("SELECT id FROM session_operations WHERE pane_key=? AND state NOT IN ('confirmed','failed','rolled_back') ORDER BY updated DESC LIMIT 1",[pane_key],|row|row.get(0)).optional()?;
        match id {
            Some(id) => self.get(&id),
            None => Ok(None),
        }
    }
    /// Retires a parked row (`recovery_required`/`awaiting_confirmation`) the pane has
    /// already left behind; only if it is still in `state`, so a concurrent recovery wins.
    pub fn supersede(&self, id: &str, state: &str, result: &Value) -> Result<bool> {
        if !matches!(state, "recovery_required" | "awaiting_confirmation") {
            return Ok(false);
        }
        atomic(self.connection, |tx| {
            let current: Option<String> = tx
                .query_row("SELECT state FROM session_operations WHERE id=?", [id], |row| row.get(0))
                .optional()?;
            if current.as_deref() != Some(state) {
                return Ok(false);
            }
            stage_in(tx, id, "failed", None, Some(result), (self.clock)(), (self.clock)())?;
            Ok(true)
        })
    }
    pub fn latest_for_target(&self, session: &str, pane: &str) -> Result<Option<Value>> {
        let id:Option<String>=self.connection.query_row("SELECT id FROM session_operations WHERE json_extract(request,'$.session')=? AND json_extract(request,'$.pane')=? ORDER BY updated DESC LIMIT 1",params![session,pane],|row|row.get(0)).optional()?;
        match id {
            Some(id) => self.get(&id),
            None => Ok(None),
        }
    }
    pub fn recover_abandoned(&self, mut alive: impl FnMut(i64) -> Result<bool>) -> Result<()> {
        reject_transaction(self.connection)?;
        let rows=self.connection.prepare("SELECT id,owner,state FROM session_operations WHERE state NOT IN ('confirmed','failed','rolled_back','recovery_required','awaiting_confirmation')")?.query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, owner, old_state) in rows {
            if alive(owner)? {
                continue;
            }
            let state = if ["validating", "waiting", "snapshot"].contains(&old_state.as_str()) {
                "failed"
            } else {
                "recovery_required"
            };
            let result = json!({"ok":false,"error":"operación interrumpida; revisar recuperación","recoveryRequired":state=="recovery_required"});
            // Recheck after liveness callback so a concurrent completion cannot be overwritten.
            atomic(self.connection, |tx| {
                let current: Option<(i64, String)> = tx
                    .query_row(
                        "SELECT owner,state FROM session_operations WHERE id=?",
                        [&id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                if current == Some((owner, old_state.clone())) {
                    stage_in(
                        tx,
                        &id,
                        state,
                        None,
                        Some(&result),
                        (self.clock)(),
                        (self.clock)(),
                    )?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }
}
fn stage_in(
    tx: &Transaction<'_>,
    id: &str,
    state: &str,
    snapshot: Option<&Value>,
    result: Option<&Value>,
    updated: f64,
    at: f64,
) -> Result<()> {
    let current: Option<String> = tx
        .query_row(
            "SELECT state FROM session_operations WHERE id=?",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    if current.as_deref() == Some("failed") && state != "failed" {
        return Err(Error::Conflict("operación cancelada".into()));
    }
    let saved = snapshot.map(python).transpose()?;
    let result_raw = result.map(python).transpose()?;
    tx.execute("UPDATE session_operations SET state=?,snapshot=COALESCE(?,snapshot),result=COALESCE(?,result),updated=? WHERE id=?",params![state,saved,result_raw,updated,id])?;
    let detail = python(result.filter(|r| truthy(r)).unwrap_or(&json!({})))?;
    tx.execute(
        "INSERT INTO session_operation_events VALUES (?,?,?,?)",
        params![id, state, detail, at],
    )?;
    Ok(())
}
pub trait Journal {
    fn stage(
        &self,
        id: &str,
        state: &str,
        snapshot: Option<&Value>,
        result: Option<&Value>,
    ) -> Result<()>;
}
impl Journal for OperationStore<'_> {
    fn stage(
        &self,
        id: &str,
        state: &str,
        snapshot: Option<&Value>,
        result: Option<&Value>,
    ) -> Result<()> {
        atomic(self.connection, |tx| {
            stage_in(
                tx,
                id,
                state,
                snapshot,
                result,
                (self.clock)(),
                (self.clock)(),
            )
        })
    }
}
fn history_config(observed: &Value) -> Option<Value> {
    if !observed.is_object() || observed["confirmed"] != true {
        return None;
    }
    let mut config = serde_json::Map::new();
    for (target, source) in [
        ("toHarness", "harness"),
        ("motor", "motor"),
        ("model", "model"),
        ("effort", "effort"),
        ("harnessAccount", "harnessAccount"),
        ("motorAccount", "motorAccount"),
    ] {
        let value = match observed.get(source) {
            Some(value) => value.as_str()?,
            None if source == "effort" => "",
            None => return None,
        };
        if value == "unknown" {
            return None;
        }
        if value.is_empty() && source == "effort" {
            config.insert(target.into(), json!(""));
            continue;
        }
        let limit = if source == "model" { 160 } else { 80 };
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > limit
            || !bytes[0].is_ascii_alphanumeric()
            || !bytes.iter().all(|b| {
                b.is_ascii_alphanumeric()
                    || matches!(b, b'_' | b'.' | b'-')
                    || (source == "model" && matches!(b, b':' | b'/'))
            })
        {
            return None;
        }
        config.insert(target.into(), json!(value));
    }
    Some(Value::Object(config))
}
fn identity_key(identity: &Value) -> Option<String> {
    let identity = identity.as_object()?;
    Some(
        [
            "socket_path",
            "pid",
            "server_start",
            "session_id",
            "pane_id",
            "pane_pid",
        ]
        .iter()
        .map(|key| match identity.get(*key) {
            Some(Value::String(s)) => s.clone(),
            None => String::new(),
            Some(value) => match value {
                Value::Bool(true) => "True".into(),
                Value::Bool(false) => "False".into(),
                Value::Null => "None".into(),
                value => value.to_string(),
            },
        })
        .collect::<Vec<_>>()
        .join("|"),
    )
}
/// Read confirmed project metadata without creating a missing file or running recovery.
pub fn config_history(path: &Path, cwd: &str, pane_key: &str) -> Result<Value> {
    let location = comandos_store::migrate::resolve_configured(path)
        .map_err(|e| Error::Persistence(e.to_string()))?;
    let is_unified = matches!(location, comandos_store::migrate::DbLocation::Unified(_));
    let resolved = match location {
        comandos_store::migrate::DbLocation::Legacy(p)
        | comandos_store::migrate::DbLocation::Unified(p) => p,
    };
    if !resolved.exists() || cwd.is_empty() {
        return Ok(
            json!({"items":[],"previous":null,"scope":"project","provenance":"confirmed-operations"}),
        );
    }
    let home = resolved
        .parent()
        .ok_or_else(|| Error::Persistence("ruta sin padre".into()))?;
    let read = |db: &Connection| {
        comandos_store::migrate::move_db::admit_write(db)?;
        config_history_from(db, cwd, pane_key)
            .map_err(|e| comandos_store::Error::Validation(e.to_string()))
    };
    if is_unified {
        comandos_store::unified::with_readonly_unified(home, &resolved, read)
    } else {
        comandos_store::unified::with_readonly_db(home, &resolved, read)
    }
    .map_err(|e| Error::Persistence(e.to_string()))
}
fn config_history_from(db: &Connection, cwd: &str, pane_key: &str) -> Result<Value> {
    let mut response =
        json!({"items":[],"previous":null,"scope":"project","provenance":"confirmed-operations"});
    if cwd.is_empty() {
        return Ok(response);
    }

    let mut statement=db.prepare("SELECT pane_key,request,snapshot,result,updated FROM session_operations WHERE state='confirmed' ORDER BY updated DESC,id ASC")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, f64>(4)?,
        ))
    })?;
    let mut groups: BTreeMap<String, Value> = BTreeMap::new();
    let mut previous_seen = false;
    for row in rows {
        let (key, request, snapshot, result, updated) = row?;
        let (Ok(request), Ok(snapshot), Ok(result)) =
            (decode(request), decode(snapshot), decode(result))
        else {
            continue;
        };
        if !request.is_object() || !snapshot.is_object() || !result.is_object() {
            continue;
        }
        let origin = &snapshot["origin"];
        if !origin.is_object()
            || origin["cwd"] != cwd
            || truthy(&request["extensionsOnly"])
            || truthy(&result["unchanged"])
            || result["ok"] != true
        {
            continue;
        }
        let Some(config) = history_config(&result["observed"]) else {
            continue;
        };
        let config_key = canonical(&config)?;
        let item = groups
            .entry(config_key)
            .or_insert_with(|| json!({"config":config,"count":0,"lastUsed":updated}));
        item["count"] = json!(item["count"].as_u64().unwrap_or(0) + 1);
        if !previous_seen && key == pane_key {
            previous_seen = true;
            if identity_key(&origin["identity"]).as_deref() == Some(pane_key)
                && let Some(previous) = history_config(&origin["observed"])
            {
                response["previous"] = json!({"config":previous,"count":1,"lastUsed":updated});
            }
        }
    }
    let mut items = groups.into_iter().collect::<Vec<_>>();
    items.sort_by(|(ka, a), (kb, b)| {
        b["count"]
            .as_u64()
            .cmp(&a["count"].as_u64())
            .then_with(|| {
                b["lastUsed"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .total_cmp(&a["lastUsed"].as_f64().unwrap_or(0.0))
            })
            .then_with(|| ka.cmp(kb))
    });
    response["items"] = Value::Array(items.into_iter().take(20).map(|(_, v)| v).collect());
    Ok(response)
}
pub trait Adapter {
    fn prepare(&mut self) -> Result<Value>;
    fn wait_idle(&mut self) -> Result<()>;
    fn check_identity(&mut self) -> Result<()>;
    fn snapshot(&mut self, plan: &Value) -> Result<Value>;
    fn apply(&mut self, plan: &Value, snapshot: &Value) -> Result<()>;
    fn verify(&mut self, plan: &Value, snapshot: Option<&Value>) -> Result<Option<Value>>;
    fn rollback(&mut self, snapshot: &Value) -> Result<Option<Value>>;
    fn pending_confirmation(&mut self, _plan: &Value, _snapshot: &Value) -> Result<Option<Value>> {
        Ok(None)
    }
    /// El `dict` del snapshot tal como lo dejó el adaptador. En el Python es
    /// el mismo objeto que `snapshot` devolvió y que `_verify` anota después
    /// (`destinationProcess`); aquí los valores no se comparten, así que el
    /// adaptador que lo anote lo devuelve y `awaiting_confirmation` lo guarda.
    fn snapshot_record(&self) -> Option<Value> {
        None
    }
}
fn stage_notify(
    store: &impl Journal,
    id: &str,
    state: &str,
    snapshot: Option<&Value>,
    result: Option<&Value>,
    notify: &mut impl FnMut(&str, Option<&Value>) -> Result<()>,
) -> Result<()> {
    store.stage(id, state, snapshot, result)?;
    let _ = notify(state, result);
    Ok(())
}
fn continuity(result: &mut Value, plan: &Value) -> Result<()> {
    if let Some(continuity) = plan.get("continuity").filter(|v| truthy(v)) {
        let fields = continuity
            .as_object()
            .ok_or_else(|| Error::Callback("continuidad inválida".into()))?;
        for (key, value) in fields {
            result[key] = value.clone();
        }
    }
    Ok(())
}
/// Durable commits precede terminal effects; any destructive attempt recovers the exact snapshot.
pub fn run_operation(
    store: &impl Journal,
    id: &str,
    adapter: &mut impl Adapter,
    mut notify: impl FnMut(&str, Option<&Value>) -> Result<()>,
) -> Value {
    let mut snapshot = None;
    let mut destructive = false;
    let run = (|| -> Result<Value> {
        let plan = adapter.prepare()?;
        if !plan.is_object() {
            return Err(Error::Callback("plan inválido".into()));
        }
        if truthy(&plan["unchanged"]) {
            adapter.check_identity()?;
            let observed = adapter
                .verify(&plan, None)?
                .filter(truthy)
                .ok_or_else(|| Error::Callback("no se confirmó la configuración actual".into()))?;
            let result = json!({"ok":true,"observed":observed,"unchanged":true});
            stage_notify(store, id, "confirmed", None, Some(&result), &mut notify)?;
            return Ok(result);
        }
        stage_notify(store, id, "waiting", None, None, &mut notify)?;
        adapter.wait_idle()?;
        adapter.check_identity()?;
        snapshot = Some(adapter.snapshot(&plan)?);
        stage_notify(store, id, "snapshot", snapshot.as_ref(), None, &mut notify)?;
        adapter.check_identity()?;
        stage_notify(store, id, "applying", None, None, &mut notify)?;
        destructive = true;
        let saved = snapshot
            .as_ref()
            .ok_or_else(|| Error::Callback("snapshot no disponible".into()))?;
        adapter.apply(&plan, saved)?;
        stage_notify(store, id, "verifying", None, None, &mut notify)?;
        let observed = adapter.verify(&plan, Some(saved))?.filter(truthy);
        if let Some(observed) = observed {
            let mut result = json!({"ok":true,"observed":observed});
            continuity(&mut result, &plan)?;
            stage_notify(store, id, "confirmed", None, Some(&result), &mut notify)?;
            return Ok(result);
        }
        if let Some(pending) = adapter.pending_confirmation(&plan, saved)?.filter(truthy) {
            let mut result = json!({"ok":true,"pending":true,"confirmed":false,"recoveryAllowed":true,"observed":pending,"message":"el destino sigue abierto; revisa su terminal para confirmar o recuperar el origen"});
            continuity(&mut result, &plan)?;
            let record = adapter.snapshot_record();
            stage_notify(
                store,
                id,
                "awaiting_confirmation",
                Some(record.as_ref().unwrap_or(saved)),
                Some(&result),
                &mut notify,
            )?;
            return Ok(result);
        }
        Err(Error::Callback(
            "el destino no confirmó conversación y configuración".into(),
        ))
    })();
    match run {
        Ok(result) => result,
        Err(error) => {
            let mut result = json!({"ok":false,"error":error.to_string()});
            let mut recovery_stage = |state: &str, result: &mut Value| -> bool {
                match stage_notify(store, id, state, None, Some(result), &mut notify) {
                    Ok(()) => true,
                    Err(error) => {
                        result["persistenceError"] = json!(error.to_string());
                        false
                    }
                }
            };
            if destructive && snapshot.as_ref().is_some_and(truthy) {
                let _ = recovery_stage("recovering", &mut result);
                if let Some(saved) = snapshot.as_ref() {
                    match adapter.rollback(saved) {
                        Ok(observed) => {
                            result["rolledBack"] = json!(true);
                            if let Some(observed) = observed.filter(truthy) {
                                result["observed"] = observed;
                            }
                            if !recovery_stage("rolled_back", &mut result) {
                                result["recoveryRequired"] = json!(true);
                            }
                        }
                        Err(error) => {
                            result["recoveryRequired"] = json!(true);
                            result["recoveryError"] = json!(error.to_string());
                            let _ = recovery_stage("recovery_required", &mut result);
                        }
                    }
                }
            } else {
                let _ = recovery_stage("failed", &mut result);
            }
            result
        }
    }
}
