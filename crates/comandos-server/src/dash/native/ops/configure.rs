//! Las entradas de las operaciones de sesión (`bin/cc-dash`):
//! `session_configure` (3453), `session_recover` (3504),
//! `refresh_session_confirmation` (3417), `motor_queue_resume` (3639) y
//! `record_runtime_config` (185) de las confirmaciones.
//!
//! La operación corre en un hilo de sistema propio (O1), como el
//! `threading.Thread` del Python: el runtime de tokio nunca espera sus
//! minutos. Antes del `claim` todo lo que el port no reproduce con certeza
//! declina (el Python heredado atiende la petición entera); después, el
//! adaptador falla cerrado (`session_configuration`).
//!
//! Un `motor-results.json` que no se carga con certeza declina antes de
//! reclamar la operación. Las extensiones usan el mismo diario durable.
use super::{
    super::{Fault, Native, NativeOptions, operations, py, target},
    results::MotorResults,
};
use crate::HandlerError;
use comandos_core::json::{python_eq, truthy};
use comandos_runtime::{
    Unsure, dialogs,
    pane_typing::TmuxResult,
    providers,
    session_configuration::{
        self as sc, Env, Kind, ObserveCaches, SessionConfiguration, TmuxSync, UNCERTAIN_ROLLBACK,
    },
    session_operations::{
        self as ops, Adapter, Journal, OperationStore, open_journal, run_operation,
    },
};
use http::StatusCode;
use rusqlite::Connection;
use serde_json::{Map, Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex, OnceLock},
};
use tokio::runtime::Handle;

const TERMINAL: [&str; 4] = ["confirmed", "failed", "rolled_back", "recovery_required"];

fn failure() -> Fault {
    Fault::Error(HandlerError::Failure)
}

fn decline<T>(_: T) -> Fault {
    Fault::Decline
}

/// Un trabajo de disco o de tmux síncrono; su pánico es una excepción sin
/// capturar (500).
async fn blocking<T, F>(job: F) -> Result<T, Fault>
where
    F: FnOnce() -> Result<T, Fault> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|_| failure())?
}

/// `str(data.get(key) or '')` de un valor que debe ser texto: otro tipo
/// verdadero declina (el Python seguiría con un `str()` que luego no casa).
fn text_field(data: &Map<String, Value>, key: &str) -> Result<String, Fault> {
    match data.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(v) if truthy(v) => Err(Fault::Decline),
        _ => Ok(String::new()),
    }
}

/// `re.fullmatch(r'[A-Za-z0-9_-]{8,100}', request_id)`.
fn request_id_ok(id: &str) -> bool {
    (8..=100).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn owner() -> i64 {
    i64::from(std::process::id())
}

/// `session_operation_store()` y su `recover_abandoned()`: el barrido es
/// idempotente (GET `/model/status` también lo hace en cada lectura), así que
/// hacerlo en cada entrada equivale a la primera vez del Python. Lo que el
/// barrido no decide con certeza declina sin escribir.
pub(super) fn open_store(opts: &NativeOptions) -> Result<Connection, Fault> {
    let conn = open_journal(&opts.journal_db).map_err(decline)?;
    match operations::recovery_is_certain(&conn) {
        Ok(true) => {}
        _ => return Err(Fault::Decline),
    }
    let clock = opts.clock_seconds.clone();
    let now = move || clock();
    let me = owner;
    let store = OperationStore::new(&conn, &me, &now).map_err(decline)?;
    store
        .recover_abandoned(operations::alive)
        .map_err(decline)?;
    Ok(conn)
}

fn with_store<T>(
    conn: &Connection,
    opts: &NativeOptions,
    run: impl FnOnce(&OperationStore<'_>) -> ops::Result<T>,
) -> ops::Result<T> {
    let clock = opts.clock_seconds.clone();
    let now = move || clock();
    let me = owner;
    let store = OperationStore::new(conn, &me, &now)?;
    run(&store)
}

// ------------------------------------------------------------------ Env

/// Las cachés de `observe_pane` de las operaciones (por `HOOKS`): propias,
/// nunca las del escaneo de GET `/state`.
fn observe_caches(hooks: &Path) -> Arc<Mutex<ObserveCaches>> {
    static CACHES: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<ObserveCaches>>>>> = OnceLock::new();
    CACHES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(hooks.to_owned())
        .or_default()
        .clone()
}

/// `dialog_patterns()` con su caché por `mtime`, una por checkout.
fn dialog_cache(repo: &Path) -> Arc<dialogs::DialogCache> {
    static CACHES: OnceLock<Mutex<HashMap<PathBuf, Arc<dialogs::DialogCache>>>> = OnceLock::new();
    CACHES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(repo.to_owned())
        .or_insert_with(|| Arc::new(dialogs::DialogCache::new(repo)))
        .clone()
}

/// `os.environ` para `wrap_command`: el de los hijos del frente o el del
/// proceso (sin las entradas que no son UTF-8).
fn environ(opts: &NativeOptions) -> HashMap<String, String> {
    match &opts.child_env {
        Some(pairs) => pairs
            .iter()
            .filter_map(|(k, v)| Some((k.to_str()?.to_owned(), v.to_str()?.to_owned())))
            .collect(),
        None => std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .collect(),
    }
}

/// Lo que el adaptador necesita del frente, fijado antes del `claim`.
pub(super) async fn build_env(native: &Arc<Native>) -> Result<Env, Fault> {
    let opts = native.options();
    let repo = opts.repo_root.clone().ok_or(Fault::Decline)?;
    let (registry, matrix) = super::super::usage::providers::registry_and_matrix(native).await?;
    let proxy_port = {
        let repo = repo.clone();
        blocking(move || providers::proxy_port(&repo).map_err(decline)).await?
    };
    let handle = Handle::current();
    let tmux: TmuxSync = {
        let tmux = opts.tmux.clone();
        let handle = handle.clone();
        Arc::new(move |args: &[&str]| {
            let out = tmux.run_blocking(&handle, args).map_err(|e| {
                e.python_message()
                    .unwrap_or_else(|| "tmux no respondió".to_owned())
            })?;
            Ok(TmuxResult {
                returncode: i32::from(!out.ok),
                stdout: out.stdout,
                stderr: out.stderr,
            })
        })
    };
    let working: sc::WorkingProbe = {
        let native = Arc::clone(native);
        let handle = handle.clone();
        // `grok_pane_busy`: el Python responde «libre» si `read_states_cached`
        // falla; aquí una duda no deja cerrar un agente que quizá trabaja.
        Arc::new(move |sess: &str, pane: &str| {
            let states = handle
                .block_on(native.states_cached())
                .map_err(|_| Unsure)?;
            Ok(states.items.iter().any(|i| {
                i.get("session").and_then(Value::as_str) == Some(sess)
                    && i.get("pane").and_then(Value::as_str) == Some(pane)
                    && i.get("status").and_then(Value::as_str) == Some("working")
            }))
        })
    };
    let trust_ledger: sc::TrustLedger = {
        let native = Arc::clone(native);
        let handle = handle.clone();
        Arc::new(move |note: &str| {
            let mut event = Map::new();
            event.insert("origin".into(), json!("reparto"));
            event.insert("kind".into(), json!("trust_inherited"));
            event.insert("note".into(), json!(note));
            let at = (native.options().clock_seconds)().floor() as i64;
            let _ = handle.block_on(native.usage.with(move |b| {
                comandos_store::usage::record_change(&b.conn, &event, at).map(|_| ())
            }));
        })
    };
    let clock = opts.clock_seconds.clone();
    Ok(Env {
        tmux,
        home: opts.home.clone(),
        hooks: opts.hooks.clone(),
        proc_root: opts.proc_root.clone(),
        registry,
        matrix,
        repo_root: repo.clone(),
        cwd: opts.cwd.clone(),
        search_path: opts.search_path.clone(),
        proxy_port,
        environ: environ(opts),
        journal: opts.journal_db.clone(),
        owner: owner(),
        sleep: std::thread::sleep,
        clock: Arc::new(move || clock()),
        working,
        trust_ledger,
        caches: observe_caches(&opts.hooks),
        dialogs: dialog_cache(&repo),
    })
}

// --------------------------------------------------- record_runtime_config

/// `observed.get(k, default)` del Python: la clave presente gana aunque sea
/// `None`.
fn get_or(observed: &Map<String, Value>, key: &str, default: &str) -> Value {
    observed.get(key).cloned().unwrap_or_else(|| json!(default))
}

/// `record_runtime_config(sess, pane, observed[...], …, route_id,
/// 'switch-observed')` (185). Nunca rompe: el registro de uso no deshace una
/// confirmación ya durable.
async fn record_runtime_config(
    native: &Native,
    sess: &str,
    pane: &str,
    observed: &Map<String, Value>,
    route_id: &Value,
) {
    // `observed['harness']`, `['motor']`, `['model']`: un `KeyError` lo
    // traga el `except` de quien llama.
    let (Some(harness), Some(motor), Some(model)) = (
        observed.get("harness"),
        observed.get("motor"),
        observed.get("model"),
    ) else {
        return;
    };
    let effort = get_or(observed, "effort", "");
    let harness_account = get_or(observed, "harnessAccount", "unknown");
    let motor_account = get_or(observed, "motorAccount", "unknown");
    let source = "switch-observed";
    // El proyecto de la sesión, de `read_states_cached()`; si falla, el
    // `except` interno salta el registro del cambio.
    let project = match native.states_cached().await {
        Ok(states) => Some(
            states
                .items
                .iter()
                .find(|i| i.get("session").and_then(Value::as_str) == Some(sess))
                .and_then(|i| i.get("project").cloned())
                .filter(truthy)
                .unwrap_or_else(|| json!("")),
        ),
        Err(_) => None,
    };
    let at = (native.options().clock_seconds)().floor() as i64;
    let or = |v: &Value, d: &str| if truthy(v) { v.clone() } else { json!(d) };
    let mut config = Map::new();
    config.insert("tmux_session".into(), json!(sess));
    config.insert("tmux_pane".into(), json!(pane));
    config.insert("harness".into(), harness.clone());
    config.insert("motor".into(), motor.clone());
    config.insert("model".into(), model.clone());
    config.insert("effort".into(), effort.clone());
    config.insert("harness_account".into(), or(&harness_account, "unknown"));
    config.insert("motor_account".into(), or(&motor_account, "unknown"));
    let route = if truthy(route_id) {
        route_id.clone()
    } else {
        match (harness, motor) {
            (Value::String(h), Value::String(m)) => json!(format!("{h}:{m}")),
            _ => return,
        }
    };
    config.insert("route_id".into(), route.clone());
    config.insert("effective_at".into(), json!(at));
    config.insert("source".into(), json!(source));
    config.insert("confidence".into(), json!("exact"));
    let (sess, pane) = (sess.to_owned(), pane.to_owned());
    let (model, effort, route_id) = (model.clone(), effort, route_id.clone());
    let _ = native
        .usage
        .with(move |b| {
            if let Some(project) = project {
                let _ = (|| -> Result<(), ()> {
                    let prev = comandos_store::usage_import::latest_session_config(
                        &b.conn,
                        &json!(sess),
                        &json!(pane),
                    )
                    .map_err(|_| ())?;
                    let field = |k: &str| prev.get(k).cloned().unwrap_or(Value::Null);
                    if !prev.is_empty()
                        && (!python_eq(&field("model"), &model)
                            || !python_eq(&field("route_id"), &route_id)
                            || !python_eq(&field("effort"), &effort))
                    {
                        let or_empty = |v: Value| if truthy(&v) { v } else { json!("") };
                        let mut event = Map::new();
                        event.insert("origin".into(), json!(source));
                        event.insert("kind".into(), json!("switch"));
                        event.insert("tmux_session".into(), json!(sess));
                        event.insert("tmux_pane".into(), json!(pane));
                        event.insert("project".into(), project);
                        event.insert("before_model".into(), or_empty(field("model")));
                        event.insert("before_effort".into(), or_empty(field("effort")));
                        event.insert("before_route".into(), or_empty(field("route_id")));
                        event.insert("after_model".into(), model);
                        event.insert("after_effort".into(), effort);
                        event.insert("after_route".into(), route_id);
                        comandos_store::usage::record_change(&b.conn, &event, at)
                            .map_err(|_| ())?;
                    }
                    Ok(())
                })();
            }
            comandos_store::usage_import::record_session_config(&b.conn, &config, at)
        })
        .await;
}

// ------------------------------------------------------------- notify

/// El adaptador con el `routeId` de su plan a la vista de `notify` (el
/// Python lee `adapter.plan['routeId']`; aquí `run_operation` tiene el
/// préstamo del adaptador).
struct Tap<'a> {
    inner: &'a mut SessionConfiguration,
    route: Rc<RefCell<Value>>,
}

impl Tap<'_> {
    fn note_plan(&self) {
        if let Some(plan) = self.inner.plan() {
            *self.route.borrow_mut() = plan.get("routeId").cloned().unwrap_or(Value::Null);
        }
    }
}

impl Adapter for Tap<'_> {
    fn prepare(&mut self) -> ops::Result<Value> {
        let plan = self.inner.prepare();
        self.note_plan();
        plan
    }
    fn wait_idle(&mut self) -> ops::Result<()> {
        self.inner.wait_idle()
    }
    fn check_identity(&mut self) -> ops::Result<()> {
        self.inner.check_identity()
    }
    fn snapshot(&mut self, plan: &Value) -> ops::Result<Value> {
        self.inner.snapshot(plan)
    }
    fn apply(&mut self, plan: &Value, snapshot: &Value) -> ops::Result<()> {
        self.inner.apply(plan, snapshot)
    }
    fn verify(&mut self, plan: &Value, snapshot: Option<&Value>) -> ops::Result<Option<Value>> {
        self.inner.verify(plan, snapshot)
    }
    fn rollback(&mut self, snapshot: &Value) -> ops::Result<Option<Value>> {
        self.inner.rollback(snapshot)
    }
    fn pending_confirmation(
        &mut self,
        plan: &Value,
        snapshot: &Value,
    ) -> ops::Result<Option<Value>> {
        self.inner.pending_confirmation(plan, snapshot)
    }
    fn snapshot_record(&self) -> Option<Value> {
        self.inner.snapshot_record()
    }
}

/// `notify(stage, result)` de `session_configure`: el resultado durable en
/// los estados finales (y el registro de uso si confirmó), el progreso en
/// los demás.
#[allow(clippy::too_many_arguments)]
fn notify(
    native: &Native,
    handle: &Handle,
    results: &MotorResults,
    opkey: &str,
    request_id: &str,
    route: &Value,
    stage: &str,
    result: Option<&Value>,
) {
    let id = json!(request_id);
    if !TERMINAL.contains(&stage) {
        let _ = results.stage(opkey, stage, stage, &[("operationId", id)]);
        return;
    }
    let empty = Map::new();
    let result = match result {
        Some(Value::Object(map)) => map,
        Some(v) if truthy(v) => return,
        _ => &empty,
    };
    let observed = match result.get("observed") {
        Some(Value::Object(map)) => map.clone(),
        Some(v) if truthy(v) => return,
        _ => Map::new(),
    };
    let field = |k: &str| observed.get(k).cloned().unwrap_or(Value::Null);
    let detail = match result.get("error") {
        Some(Value::String(e)) if !e.is_empty() => e.clone(),
        Some(v) if truthy(v) => return,
        _ => "configuración confirmada".to_owned(),
    };
    let ok = result.get("ok").is_some_and(truthy);
    let set = results.set(
        opkey,
        ok,
        &detail,
        &[
            ("operationId", id),
            ("state", json!(stage)),
            ("harness", field("harness")),
            ("motor", field("motor")),
            ("model", field("model")),
            ("effort", field("effort")),
            (
                "rolledBack",
                result.get("rolledBack").cloned().unwrap_or(json!(false)),
            ),
            (
                "recoveryRequired",
                result
                    .get("recoveryRequired")
                    .cloned()
                    .unwrap_or(json!(false)),
            ),
        ],
    );
    // Un fallo de `motor_result_set` sube antes de `record_runtime_config`.
    if set.is_err() {
        return;
    }
    if ok && observed.get("confirmed").is_some_and(truthy) {
        let (sess, pane) = opkey.split_once('|').unwrap_or((opkey, ""));
        handle.block_on(record_runtime_config(native, sess, pane, &observed, route));
    }
}

// ---------------------------------------------------- session_configure

/// La respuesta de una operación que ya existía (`store.get` primero: una
/// petición repetida es idempotente aunque el pane o tmux ya no existan).
fn replay(previous: &Value, opkey: &str, request_id: &str) -> Result<(StatusCode, Value), Fault> {
    let state = previous
        .get("state")
        .and_then(Value::as_str)
        .ok_or(Fault::Decline)?;
    let mut body = match previous.get("result") {
        Some(Value::Object(map)) => map.clone(),
        Some(v) if truthy(v) => return Err(Fault::Decline),
        _ => Map::new(),
    };
    let terminal = TERMINAL.contains(&state);
    let ok = body.get("ok").cloned().unwrap_or(json!(true));
    body.insert("ok".into(), ok);
    body.insert("pending".into(), json!(!terminal));
    body.insert("operationKey".into(), json!(opkey));
    body.insert("operationId".into(), json!(request_id));
    body.insert("state".into(), json!(state));
    let code = if terminal {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    Ok((code, Value::Object(body)))
}

fn conflict(error: &str, opkey: Option<&str>) -> (StatusCode, Value) {
    let mut body = Map::new();
    body.insert("ok".into(), json!(false));
    body.insert("error".into(), json!(error));
    if let Some(opkey) = opkey {
        body.insert("operationKey".into(), json!(opkey));
    }
    (StatusCode::CONFLICT, Value::Object(body))
}

/// `session_configure(data)` (3453).
pub async fn session_configure(
    native: &Arc<Native>,
    data: Value,
) -> Result<(StatusCode, Value), Fault> {
    let Value::Object(mut data) = data else {
        return Err(Fault::Decline);
    };
    let sess = text_field(&data, "session")?;
    let pane = text_field(&data, "pane")?;
    if !py::is_session(&sess) || !py::is_pane(&pane).ok_or(Fault::Decline)? {
        let body = json!({"ok": false, "error": "sesión y panel exactos requeridos"});
        return Ok((StatusCode::BAD_REQUEST, body));
    }
    let mut request_id = text_field(&data, "requestId")?;
    if request_id.is_empty() {
        request_id = super::super::background::pomodoro::token_hex(16).map_err(|_| failure())?;
    }
    if !request_id_ok(&request_id) {
        let body = json!({"ok": false, "error": "requestId inválido"});
        return Ok((StatusCode::BAD_REQUEST, body));
    }
    data.insert("requestId".into(), json!(request_id));
    let results = MotorResults::shared(native.options());
    let opkey = format!("{sess}|{pane}");
    let request = Value::Object(data);
    // `store.get` primero.
    let previous = {
        let native = Arc::clone(native);
        let results = Arc::clone(&results);
        let (id, req, key) = (request_id.clone(), request.clone(), opkey.clone());
        blocking(move || {
            // Un `motor-results.json` que no se lee con certeza: el heredado.
            if !results.certain() {
                return Err(Fault::Decline);
            }
            let opts = native.options();
            let conn = open_store(opts)?;
            let Some(previous) = with_store(&conn, opts, |s| s.get(&id)).map_err(decline)? else {
                return Ok(None);
            };
            let pane_key = previous
                .get("pane_key")
                .and_then(Value::as_str)
                .ok_or(Fault::Decline)?
                .to_owned();
            match with_store(&conn, opts, |s| s.claim(&id, &pane_key, &req)) {
                Ok(_) => Ok(Some(replay(&previous, &key, &id)?)),
                Err(ops::Error::Conflict(text)) => Ok(Some(conflict(&text, Some(&key)))),
                Err(_) => Err(Fault::Decline),
            }
        })
        .await?
    };
    if let Some(answer) = previous {
        return Ok(answer);
    }
    let identity = match target::pane_identity(native, &sess, &pane).await {
        Ok(identity) => identity,
        Err(target::TargetError::Value(text)) => return Ok(conflict(&text, Some(&opkey))),
        Err(target::TargetError::Fault(fault)) => return Err(fault),
    };
    let env = build_env(native).await?;
    let claimed = {
        let native = Arc::clone(native);
        let (id, req, key) = (request_id.clone(), request.clone(), opkey.clone());
        blocking(move || {
            // `__init__` lee el agente del pane; sus excepciones (tmux) son 500.
            let kind = if req.get("extensionsOnly").is_some_and(truthy) { Kind::Extensions } else { Kind::Session };
            let check = env.clone();
            let adapter =
                SessionConfiguration::new(kind, req.clone(), identity.clone(), env)
                    .map_err(decline)?;
            // Antes del `claim`: lo que el port no reproduce declina.
            adapter.probe().map_err(decline)?;
            let opts = native.options();
            let conn = open_store(opts)?;
            let pane_key = sc::identity_key(&identity);
            let pane = req.get("pane").and_then(Value::as_str).unwrap_or_default();
            if let Ok(Some(parked)) = with_store(&conn, opts, |s| s.active_for_pane(&pane_key))
                && sc::operation_left_behind(&check, pane, &parked)
            {
                let state = parked["state"].as_str().unwrap_or_default();
                let result = json!({"ok": false, "superseded": true, "recoveryRequired": false,
                    "error": "el panel ya corre otro agente; la operación quedó atrás"});
                let _ = with_store(&conn, opts, |s| {
                    s.supersede(parked["id"].as_str().unwrap_or_default(), state, &result)
                });
            }
            match with_store(&conn, opts, |s| s.claim(&id, &pane_key, &req)) {
                // La conexión va al hilo: abrirla allí podría fallar con la fila
                // ya reclamada.
                Ok(true) => {
                    if kind == Kind::Extensions {
                        use comandos_runtime::pane_extensions::{ExtensionStore, Fault as DraftFault};
                        let clock = || (opts.clock_seconds)();
                        let checked = ExtensionStore::new(&conn, &clock).and_then(|s| {
                            s.require_revision(req["extensionDraftKey"].as_str().unwrap_or(""), &req["revision"])
                        });
                        if let Err(e) = checked {
                            let result = json!({"ok":false,"error":"la selección cambió; vuelve a cargarla"});
                            let _ = with_store(&conn, opts, |s|s.stage(&id,"failed",None,Some(&result)));
                            return match e {
                                DraftFault::Invalid(text)|DraftFault::Conflict(text) => Ok(Err(conflict(&text, Some(&key)))),
                                DraftFault::Persistence(_) => Err(failure()),
                            };
                        }
                    }
                    Ok(Ok((adapter, conn)))
                }
                Ok(false) => Ok(Err((
                    StatusCode::ACCEPTED,
                    json!({"ok": true, "pending": true, "operationKey": key, "operationId": id}),
                ))),
                Err(ops::Error::Conflict(text)) => Ok(Err(conflict(&text, Some(&key)))),
                Err(_) => Err(Fault::Decline),
            }
        })
        .await?
    };
    let (adapter, conn) = match claimed {
        Ok(claimed) => claimed,
        Err(answer) => return Ok(answer),
    };
    // Reclamada: desde aquí el Python responde 202 y la operación sigue sola.
    {
        let results = Arc::clone(&results);
        let (key, id) = (opkey.clone(), request_id.clone());
        let _ = blocking(move || {
            let _ = results.stage(
                &key,
                "validando configuración",
                "validating",
                &[("operationId", json!(id))],
            );
            Ok(())
        })
        .await;
    }
    spawn_operation(
        native,
        conn,
        adapter,
        request_id.clone(),
        opkey.clone(),
        results,
    )?;
    Ok((
        StatusCode::ACCEPTED,
        json!({"ok": true, "pending": true, "queued": true, "operationKey": opkey, "operationId": request_id}),
    ))
}

/// Una fila reclamada cuyo hilo no puede correr no queda huérfana: `failed`
/// con un error genérico (por el `store` si se puede, si no a mano).
fn fail_claimed(conn: &Connection, opts: &NativeOptions, request_id: &str) {
    let result =
        json!({"ok": false, "error": "no se pudo iniciar la operación; el agente sigue abierto"});
    let staged = with_store(conn, opts, |s| {
        s.stage(request_id, "failed", None, Some(&result))
    });
    if staged.is_err()
        && let Ok(tx) =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
        && comandos_store::migrate::move_db::admit_write(conn).is_ok()
        && conn
            .execute(
                "UPDATE session_operations SET state='failed',result=?,updated=? WHERE id=?",
                rusqlite::params![result.to_string(), (opts.clock_seconds)(), request_id],
            )
            .is_ok()
    {
        let _ = tx.commit();
    }
    eprintln!("comandos dash: la operación {request_id} no pudo iniciarse");
}

/// O1: `threading.Thread(target=run_operation, …)`, con la conexión al
/// journal abierta antes del `claim`.
fn spawn_operation(
    native: &Arc<Native>,
    conn: Connection,
    mut adapter: SessionConfiguration,
    request_id: String,
    opkey: String,
    results: Arc<MotorResults>,
) -> Result<(), Fault> {
    let thread_native = Arc::clone(native);
    let handle = Handle::current();
    let id = request_id.clone();
    let spawned = std::thread::Builder::new()
        .name("comandos-op".into())
        .spawn(move || {
            let native = thread_native;
            let opts = native.options();
            let clock = opts.clock_seconds.clone();
            let now = move || clock();
            let me = owner;
            let Ok(store) = OperationStore::new(&conn, &me, &now) else {
                fail_claimed(&conn, opts, &request_id);
                return;
            };
            let route = Rc::new(RefCell::new(Value::Null));
            let mut tap = Tap {
                inner: &mut adapter,
                route: Rc::clone(&route),
            };
            let notify = |stage: &str, result: Option<&Value>| -> ops::Result<()> {
                let route = route.borrow().clone();
                notify(
                    &native,
                    &handle,
                    &results,
                    &opkey,
                    &request_id,
                    &route,
                    stage,
                    result,
                );
                Ok(())
            };
            let _ = run_operation(&store, &request_id, &mut tap, notify);
        });
    if spawned.is_err() {
        // El cierre (con la conexión) ya se soltó: otra conexión.
        let opts = native.options();
        if let Ok(conn) = open_journal(&opts.journal_db) {
            fail_claimed(&conn, opts, &id);
        }
        return Err(failure());
    }
    Ok(())
}

// ------------------------------------------------------- session_recover

/// `session_recover(data)` (3504).
pub async fn session_recover(
    native: &Arc<Native>,
    data: Value,
) -> Result<(StatusCode, Value), Fault> {
    let Value::Object(data) = data else {
        return Err(Fault::Decline);
    };
    let operation_id = text_field(&data, "operationId")?;
    let results = MotorResults::shared(native.options());
    let row = {
        let native = Arc::clone(native);
        let results = Arc::clone(&results);
        let id = operation_id.clone();
        blocking(move || {
            if !results.certain() {
                return Err(Fault::Decline);
            }
            let opts = native.options();
            let conn = open_store(opts)?;
            with_store(&conn, opts, |s| s.get(&id)).map_err(decline)
        })
        .await?
    };
    let pending = row.as_ref().filter(|row| {
        let state = row.get("state").and_then(Value::as_str).unwrap_or("");
        matches!(state, "recovery_required" | "awaiting_confirmation")
            && row.get("snapshot").is_some_and(truthy)
    });
    let Some(row) = pending.cloned() else {
        return Ok(conflict(
            "la operación no tiene una recuperación pendiente",
            None,
        ));
    };
    let snapshot = row.get("snapshot").cloned().ok_or(Fault::Decline)?;
    let identity = snapshot
        .get("origin")
        .and_then(|o| o.get("identity"))
        .and_then(Value::as_object)
        .cloned()
        .ok_or(Fault::Decline)?;
    let request = row.get("request").cloned().ok_or(Fault::Decline)?;
    let field = |k: &str| request.get(k).and_then(Value::as_str).map(str::to_owned);
    let (Some(sess), Some(pane)) = (field("session"), field("pane")) else {
        return Err(Fault::Decline);
    };
    let opkey = format!("{sess}|{pane}");
    let current = match target::pane_identity(native, &sess, &pane).await {
        Ok(identity) => identity,
        Err(target::TargetError::Value(text)) => return Ok(conflict(&text, None)),
        Err(target::TargetError::Fault(fault)) => return Err(fault),
    };
    if target::identity_key(&current) != sc::identity_key(&identity) {
        return Ok(conflict(
            "el panel original cambió; el snapshot se conserva para recuperación manual",
            None,
        ));
    }
    let env = build_env(native).await?;
    {
        let dialogs = Arc::clone(&env.dialogs);
        blocking(move || dialogs.probe().map_err(decline)).await?;
    }
    let claimed = {
        let native = Arc::clone(native);
        let id = operation_id.clone();
        blocking(move || {
            let opts = native.options();
            let conn = open_store(opts)?;
            let claimed = with_store(&conn, opts, |s| s.claim_recovery(&id)).map_err(decline)?;
            // La conexión va al hilo: abrirla allí podría fallar con la fila ya
            // en `recovering`.
            Ok(claimed.then_some(conn))
        })
        .await?
    };
    let Some(conn) = claimed else {
        return Ok(conflict("la recuperación ya está en curso", None));
    };
    {
        let results = Arc::clone(&results);
        let (key, id) = (opkey.clone(), operation_id.clone());
        let _ = blocking(move || {
            let _ = results.stage(
                &key,
                "recuperando conversación original",
                "recovering",
                &[("operationId", json!(id))],
            );
            Ok(())
        })
        .await;
    }
    let allow_pending = row.get("state").and_then(Value::as_str) == Some("awaiting_confirmation");
    let (key, id) = (opkey.clone(), operation_id.clone());
    let native_thread = Arc::clone(native);
    let spawned = std::thread::Builder::new()
        .name("comandos-op".into())
        .spawn(move || {
            recover(
                &native_thread,
                &conn,
                env,
                request,
                identity,
                &snapshot,
                allow_pending,
                &key,
                &id,
                &results,
            );
        });
    if spawned.is_err() {
        let opts = native.options();
        if let Ok(conn) = open_journal(&opts.journal_db) {
            let result = failed(sc::Fail::Py(
                "no se pudo iniciar la recuperación; el snapshot se conserva".into(),
            ));
            let _ = with_store(&conn, opts, |s| {
                s.stage(&operation_id, "recovery_required", None, Some(&result))
            });
        }
        return Err(failure());
    }
    Ok((
        StatusCode::ACCEPTED,
        json!({"ok": true, "pending": true, "operationKey": opkey, "operationId": operation_id}),
    ))
}

/// El `recover()` del hilo de `session_recover`.
#[allow(clippy::too_many_arguments)]
fn recover(
    native: &Native,
    conn: &Connection,
    env: Env,
    request: Value,
    identity: Map<String, Value>,
    snapshot: &Value,
    allow_pending: bool,
    opkey: &str,
    operation_id: &str,
    results: &MotorResults,
) {
    let opts = native.options();
    // El `try` del Python: adaptador, `rollback` y `stage('rolled_back')`.
    let kind = if request.get("extensionsOnly").is_some_and(truthy) {
        Kind::Extensions
    } else {
        Kind::Session
    };
    let attempt =
        SessionConfiguration::for_recovery(kind, request, identity, env, snapshot, allow_pending)
            .and_then(|mut adapter| adapter.recover(snapshot))
            .and_then(|observed| {
                let result = json!({
                    "ok": false, "rolledBack": true, "recoveryRequired": false,
                    "observed": observed.map_or(Value::Null, Value::Object),
                    "error": "conversación original recuperada; no se aplicó el destino",
                });
                with_store(conn, opts, |s| {
                    s.stage(operation_id, "rolled_back", None, Some(&result))
                })
                .map_err(|e| sc::Fail::Py(e.to_string()))?;
                Ok(result)
            });
    let result = match attempt {
        Ok(result) => result,
        Err(fail) => {
            let result = failed(fail);
            // Si este `stage` falla, la excepción del `except` mata el hilo
            // antes de `motor_result_set`.
            if with_store(conn, opts, |s| {
                s.stage(operation_id, "recovery_required", None, Some(&result))
            })
            .is_err()
            {
                return;
            }
            result
        }
    };
    let error = result["error"].as_str().unwrap_or("").to_owned();
    let _ = results.set(
        opkey,
        false,
        &error,
        &[
            ("operationId", json!(operation_id)),
            (
                "rolledBack",
                result.get("rolledBack").cloned().unwrap_or(json!(false)),
            ),
            (
                "recoveryRequired",
                result
                    .get("recoveryRequired")
                    .cloned()
                    .unwrap_or(json!(false)),
            ),
        ],
    );
}

fn failed(fail: sc::Fail) -> Value {
    let text = match fail {
        sc::Fail::Py(text) => text,
        sc::Fail::Unsure => UNCERTAIN_ROLLBACK.to_owned(),
    };
    json!({"ok": false, "recoveryRequired": true, "error": text})
}

// ------------------------------------------ refresh_session_confirmation

/// `refresh_session_confirmation(store, row)` (3417): una lectura del
/// destino fijado de una fila `awaiting_confirmation`; nunca teclea nada.
/// Devuelve la fila (actualizada si esta lectura cambió algo) y, si
/// confirmó, registra la configuración observada.
pub async fn refresh_session_confirmation(
    native: &Arc<Native>,
    row: Value,
) -> Result<Value, Fault> {
    if row.get("state").and_then(Value::as_str) != Some("awaiting_confirmation")
        || !row.get("snapshot").is_some_and(truthy)
    {
        return Ok(row);
    }
    let env = build_env(native).await?;
    let refreshed = blocking(move || Ok(sc::refresh_confirmation(&env, &row))).await?;
    if let Some((observed, route, sess, pane)) = &refreshed.confirmed {
        record_runtime_config(native, sess, pane, observed, &json!(route)).await;
    }
    Ok(refreshed.row)
}

// ------------------------------------------------------ motor_queue_resume

/// `motor_queue_resume()` (3639): conserva los registros de la cola vieja
/// para inspección, sin repetir nada en ningún terminal. Bloquea.
pub fn motor_queue_resume(opts: &NativeOptions) {
    if open_store(opts).is_err() {
        return;
    }
    let Ok(doc) =
        super::super::files::DomainDocument::new(&opts.home, &opts.hooks, "motor-queue.json")
    else {
        return;
    };
    // `except (OSError, ValueError): return`; lo incierto, igual.
    let queued = match doc.strict() {
        super::super::files::Strict::Value(v) => v,
        _ => return,
    };
    let keys: Vec<String> = match queued {
        Value::Object(map) => map.keys().cloned().collect(),
        Value::Array(list) => {
            let keys: Option<Vec<String>> =
                list.iter().map(|k| k.as_str().map(str::to_owned)).collect();
            match keys {
                Some(keys) => keys,
                None => return,
            }
        }
        // `for opkey in "texto"`: cada carácter.
        Value::String(text) => text.chars().map(String::from).collect(),
        _ => return,
    };
    let results = MotorResults::shared(opts);
    for opkey in keys {
        if results
            .set(
                &opkey,
                false,
                "cambio antiguo sin snapshot verificable; vuelve a solicitarlo",
                &[],
            )
            .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod sqlite_writer_tests {
    use super::*;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn failed_operation_fallback_cannot_write_cached_moved_journal() {
        let home =
            std::env::temp_dir().join(comandos_runtime::fresh_id("s4-fail-claimed").unwrap());
        std::fs::create_dir(&home).unwrap();
        let _cleanup = Cleanup(home.clone());
        let spec = comandos_store::migrate::spec_for(&home, "db-operations").unwrap();
        let cached = comandos_runtime::session_operations::open_journal(&spec.legacy).unwrap();
        cached.execute_batch("INSERT INTO session_operations VALUES('private-op','pane','hash','{}','confirmed',1,'{}','{}',1)").unwrap();
        comandos_store::migrate::move_db(
            &spec,
            &home.join(".local/share/comandos/comandos.sqlite3"),
            &home.join(".local/share/comandos/backups/test"),
            4000,
        )
        .unwrap();
        let opts = NativeOptions::for_home(&home, home.join("private-app-state.sqlite3"));
        fail_claimed(&cached, &opts, "private-op");
        let state: String = cached
            .query_row(
                "SELECT state FROM session_operations WHERE id='private-op'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "confirmed");
        assert!(cached.is_autocommit());
        drop(cached);
        std::fs::remove_dir_all(home).unwrap();
    }
}
