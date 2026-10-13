use comandos_runtime::session_operations::{
    self as ops, Adapter, Error, Journal, OperationStore, Result,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Fixture {
    path: PathBuf,
    conn: Connection,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "comandos-operation-{}-{}.db",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let conn = ops::open_journal(&path).unwrap();
        Self { path, conn }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
fn clock() -> f64 {
    100.0
}
fn owner() -> i64 {
    777
}
fn store(conn: &Connection) -> OperationStore<'_> {
    OperationStore::new(conn, &owner, &clock).unwrap()
}
fn claimed(conn: &Connection) -> OperationStore<'_> {
    let s = store(conn);
    assert!(
        s.claim(
            "request-1234",
            "generation|%2",
            &json!({"session":"test","pane":"%2"})
        )
        .unwrap()
    );
    s
}
#[test]
fn atomic_claim_digest_idempotence_conflict_and_pane_lock() {
    let f = Fixture::new();
    let s = store(&f.conn);
    let draft = json!({"é":"😊","pane":"%2","session":"test"});
    assert!(s.claim("one", "pane", &draft).unwrap());
    assert!(!s.claim("one", "other-pane", &draft).unwrap());
    assert!(matches!(
        s.claim("one", "pane", &json!({})),
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        s.claim("two", "pane", &draft),
        Err(Error::Conflict(_))
    ));
    assert!(s.claim("two", "other-pane", &draft).unwrap());
    let row = s.get("one").unwrap().unwrap();
    let raw = r#"{"pane":"%2","session":"test","\u00e9":"\ud83d\ude0a"}"#;
    assert_eq!(
        row["fingerprint"],
        format!("{:x}", Sha256::digest(raw.as_bytes()))
    );
    assert_eq!(row["request"], draft);
    assert_eq!(row["owner"], 777);
    assert_eq!(row["state"], "validating");
}
#[test]
fn concurrent_same_pane_claims_have_one_winner() {
    let f = Fixture::new();
    let outcomes = std::thread::scope(|scope| {
        let joins = (0..8)
            .map(|n| {
                let path = &f.path;
                scope.spawn(move || {
                    let c = ops::open_journal(path).unwrap();
                    store(&c)
                        .claim(&format!("request-{n}"), "pane", &json!({"n":n}))
                        .unwrap_or(false)
                })
            })
            .collect::<Vec<_>>();
        joins
            .into_iter()
            .map(|j| j.join().unwrap())
            .filter(|winner| *winner)
            .count()
    });
    assert_eq!(outcomes, 1);
}
#[test]
fn stage_preserves_snapshot_result_events_and_failed_cannot_resurrect() {
    let f = Fixture::new();
    let s = claimed(&f.conn);
    let snap = json!({"origin":{"id":"exact"}});
    let result = json!({"ok":true});
    s.stage("request-1234", "snapshot", Some(&snap), Some(&result))
        .unwrap();
    s.stage("request-1234", "applying", None, None).unwrap();
    let row = s.get("request-1234").unwrap().unwrap();
    assert_eq!(row["snapshot"], snap);
    assert_eq!(row["result"], result);
    let count: i64 = f
        .conn
        .query_row("SELECT count(*) FROM session_operation_events", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 2);
    s.stage("request-1234", "failed", None, None).unwrap();
    assert!(matches!(
        s.stage("request-1234", "applying", None, None),
        Err(Error::Conflict(_))
    ));
    assert!(s.claim("next", "generation|%2", &json!({})).unwrap());
}
#[test]
fn cancellation_only_before_applying_and_recovery_claim_only_allowed_states() {
    for state in [
        "validating",
        "waiting",
        "snapshot",
        "applying",
        "verifying",
        "confirmed",
        "awaiting_confirmation",
        "recovery_required",
        "recovering",
    ] {
        let f = Fixture::new();
        let s = claimed(&f.conn);
        s.stage("request-1234", state, None, None).unwrap();
        assert_eq!(
            s.cancel_waiting("request-1234").unwrap(),
            matches!(state, "validating" | "waiting" | "snapshot")
        );
        if matches!(state, "awaiting_confirmation" | "recovery_required") {
            assert!(s.claim_recovery("request-1234").unwrap());
            assert!(!s.claim_recovery("request-1234").unwrap());
        } else {
            assert!(!s.claim_recovery("request-1234").unwrap());
        }
    }
}
#[test]
fn journal_mutators_reject_owned_transaction_without_touching_or_committing_it() {
    let f = Fixture::new();
    let s = claimed(&f.conn);
    f.conn.execute_batch("BEGIN; CREATE TABLE caller_data(value TEXT); INSERT INTO caller_data VALUES ('pending');").unwrap();
    assert!(s.claim("two", "other", &json!({})).is_err());
    assert!(s.stage("request-1234", "applying", None, None).is_err());
    assert!(s.cancel_waiting("request-1234").is_err());
    assert!(s.claim_recovery("request-1234").is_err());
    assert!(s.recover_abandoned(|_| Ok(false)).is_err());
    assert!(OperationStore::new(&f.conn, &owner, &clock).is_err());
    assert!(!f.conn.is_autocommit());
    assert_eq!(
        s.get("request-1234").unwrap().unwrap()["state"],
        "validating"
    );
    f.conn.execute_batch("ROLLBACK").unwrap();
}
#[test]
fn journal_open_is_private_and_does_not_install_other_schemas() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    assert_eq!(
        std::fs::metadata(&f.path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::set_permissions(&f.path, std::fs::Permissions::from_mode(0o644)).unwrap();
    drop(ops::open_journal(&f.path).unwrap());
    assert_eq!(
        std::fs::metadata(&f.path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let tables: Vec<String> = f
        .conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        tables,
        vec!["session_operation_events", "session_operations"]
    );
}
#[test]
fn crash_sweep_uses_liveness_without_terminal_replay_and_retains_snapshot() {
    for state in [
        "validating",
        "waiting",
        "snapshot",
        "applying",
        "verifying",
        "recovering",
        "awaiting_confirmation",
        "recovery_required",
        "confirmed",
        "rolled_back",
        "failed",
    ] {
        let f = Fixture::new();
        let s = claimed(&f.conn);
        let snap = json!({"origin":{"id":"exact-original"}});
        s.stage("request-1234", state, Some(&snap), None).unwrap();
        let owners = RefCell::new(Vec::new());
        s.recover_abandoned(|owner| {
            owners.borrow_mut().push(owner);
            Ok(false)
        })
        .unwrap();
        let expected = match state {
            "validating" | "waiting" | "snapshot" => "failed",
            "applying" | "verifying" | "recovering" => "recovery_required",
            other => other,
        };
        let row = s.get("request-1234").unwrap().unwrap();
        assert_eq!(row["state"], expected);
        assert_eq!(row["snapshot"], snap);
        assert_eq!(
            owners.borrow().len(),
            usize::from(matches!(
                state,
                "validating" | "waiting" | "snapshot" | "applying" | "verifying" | "recovering"
            ))
        );
    }
    let f = Fixture::new();
    let s = claimed(&f.conn);
    s.recover_abandoned(|_| Ok(true)).unwrap();
    assert_eq!(
        s.get("request-1234").unwrap().unwrap()["state"],
        "validating"
    );
}
#[test]
fn saved_exact_origin_and_target_queries_use_matching_indexes() {
    let f = Fixture::new();
    let s = claimed(&f.conn);
    let origin = json!({"agent":"claude","observed":{"conversationId":"exact-original"},"resume_command":"claude --resume exact-original"});
    s.stage(
        "request-1234",
        "confirmed",
        Some(&json!({"origin":origin})),
        None,
    )
    .unwrap();
    assert_eq!(
        s.saved_origin("generation|%2", "claude").unwrap(),
        Some(origin)
    );
    assert_eq!(
        s.saved_origin("other-generation|%2", "claude").unwrap(),
        None
    );
    assert_eq!(
        s.latest_for_target("test", "%2").unwrap().unwrap()["id"],
        "request-1234"
    );
    assert!(s.latest_for_target("test", "%99").unwrap().is_none());
    for (sql, index) in [
        (
            "EXPLAIN QUERY PLAN SELECT id FROM session_operations WHERE json_extract(request,'$.session')='test' AND json_extract(request,'$.pane')='%2' ORDER BY updated DESC LIMIT 1",
            "session_operations_target_updated",
        ),
        (
            "EXPLAIN QUERY PLAN SELECT snapshot FROM session_operations WHERE pane_key='generation|%2' AND snapshot IS NOT NULL ORDER BY updated DESC",
            "session_operations_pane_updated",
        ),
    ] {
        let plans: Vec<String> = f
            .conn
            .prepare(sql)
            .unwrap()
            .query_map([], |r| r.get(3))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(plans.iter().any(|p| p.contains(index)));
    }
}
struct FakeAdapter<'a> {
    path: &'a Path,
    calls: Vec<String>,
    failure: Option<&'static str>,
    unchanged: bool,
    pending: bool,
    cancel: bool,
    identity_checks: usize,
    fail_rollback: bool,
}
impl<'a> FakeAdapter<'a> {
    fn new(path: &'a Path) -> Self {
        Self {
            path,
            calls: Vec::new(),
            failure: None,
            unchanged: false,
            pending: false,
            cancel: false,
            identity_checks: 0,
            fail_rollback: false,
        }
    }
    fn step(&mut self, name: &str) -> Result<()> {
        self.calls.push(name.into());
        if self.failure == Some(name) {
            Err(Error::Callback(format!("simulated {name}")))
        } else {
            Ok(())
        }
    }
}
impl Adapter for FakeAdapter<'_> {
    fn prepare(&mut self) -> Result<Value> {
        self.step("prepare")?;
        Ok(
            json!({"unchanged":self.unchanged,"continuity":{"continuity":"resumed","handoffRequired":false}}),
        )
    }
    fn wait_idle(&mut self) -> Result<()> {
        self.step("wait")
    }
    fn check_identity(&mut self) -> Result<()> {
        self.step("identity")?;
        self.identity_checks += 1;
        if self.cancel && self.identity_checks == 2 {
            let c = ops::open_journal(self.path)?;
            assert!(store(&c).cancel_waiting("request-1234")?);
        }
        Ok(())
    }
    fn snapshot(&mut self, _plan: &Value) -> Result<Value> {
        self.step("snapshot")?;
        Ok(
            json!({"origin":{"resume_id":"exact-original","resume_command":"claude --resume exact-original"},"layout":{"windows":["untouched split"]}}),
        )
    }
    fn apply(&mut self, _plan: &Value, snapshot: &Value) -> Result<()> {
        let c = ops::open_journal(self.path)?;
        let row = store(&c).get("request-1234")?.unwrap();
        assert_eq!(row["state"], "applying");
        assert_eq!(&row["snapshot"], snapshot);
        self.step("apply")
    }
    fn verify(&mut self, _plan: &Value, _snapshot: Option<&Value>) -> Result<Option<Value>> {
        self.step("verify")?;
        if self.pending {
            Ok(None)
        } else {
            Ok(Some(
                json!({"harness":"codex","model":"gpt-6-astra","effort":"ultra"}),
            ))
        }
    }
    fn rollback(&mut self, snapshot: &Value) -> Result<Option<Value>> {
        assert_eq!(snapshot["origin"]["resume_id"], "exact-original");
        self.step("rollback")?;
        if self.fail_rollback {
            Err(Error::Callback("origin also failed".into()))
        } else {
            Ok(Some(json!({"conversationId":"exact-original"})))
        }
    }
    fn pending_confirmation(&mut self, _plan: &Value, _snapshot: &Value) -> Result<Option<Value>> {
        Ok(self
            .pending
            .then(|| json!({"confirmed":false,"conversationId":"destination"})))
    }
}
#[test]
fn adapter_order_and_reopen_prove_durable_snapshot_before_apply() {
    let f = Fixture::new();
    let s = claimed(&f.conn);
    let mut a = FakeAdapter::new(&f.path);
    let result = ops::run_operation(&s, "request-1234", &mut a, |_, _| Ok(()));
    assert_eq!(result["ok"], true);
    assert_eq!(
        a.calls,
        vec![
            "prepare", "wait", "identity", "snapshot", "identity", "apply", "verify"
        ]
    );
    let c = ops::open_journal(&f.path).unwrap();
    let row = store(&c).get("request-1234").unwrap().unwrap();
    assert_eq!(row["state"], "confirmed");
    assert_eq!(row["snapshot"]["origin"]["resume_id"], "exact-original");
    assert_eq!(result["continuity"], "resumed");
    let stages: Vec<String> = c
        .prepare("SELECT stage FROM session_operation_events ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        stages,
        vec!["waiting", "snapshot", "applying", "verifying", "confirmed"]
    );
}
#[test]
fn preflight_failures_and_waiting_cancellation_leave_origin_untouched() {
    for failure in ["prepare", "wait", "snapshot", "identity"] {
        let f = Fixture::new();
        let s = claimed(&f.conn);
        let mut a = FakeAdapter::new(&f.path);
        a.failure = Some(failure);
        let result = ops::run_operation(&s, "request-1234", &mut a, |_, _| Ok(()));
        assert_eq!(result["ok"], false);
        assert!(!a.calls.contains(&"apply".into()));
        assert!(!a.calls.contains(&"rollback".into()));
        assert_eq!(s.get("request-1234").unwrap().unwrap()["state"], "failed");
    }
    let f = Fixture::new();
    let s = claimed(&f.conn);
    let mut a = FakeAdapter::new(&f.path);
    a.cancel = true;
    assert_eq!(
        ops::run_operation(&s, "request-1234", &mut a, |_, _| Ok(()))["ok"],
        false
    );
    assert!(!a.calls.contains(&"apply".into()));
}
#[test]
fn unchanged_verifies_without_snapshot_or_restart_and_notify_failure_cannot_rollback_success() {
    for unchanged in [false, true] {
        let f = Fixture::new();
        let s = claimed(&f.conn);
        let mut a = FakeAdapter::new(&f.path);
        a.unchanged = unchanged;
        let result = ops::run_operation(&s, "request-1234", &mut a, |_, _| {
            Err(Error::Callback("broken status transport".into()))
        });
        assert_eq!(result["ok"], true);
        assert!(!a.calls.contains(&"rollback".into()));
        assert_eq!(
            s.get("request-1234").unwrap().unwrap()["state"],
            "confirmed"
        );
        if unchanged {
            assert_eq!(a.calls, vec!["prepare", "identity", "verify"]);
            assert_eq!(result["unchanged"], true);
        }
    }
}
#[test]
fn destructive_failure_rolls_back_exact_origin_and_failed_rollback_keeps_lock() {
    for failure in ["apply", "verify"] {
        for fail_rollback in [false, true] {
            let f = Fixture::new();
            let s = claimed(&f.conn);
            let mut a = FakeAdapter::new(&f.path);
            a.failure = Some(failure);
            a.fail_rollback = fail_rollback;
            let result = ops::run_operation(&s, "request-1234", &mut a, |_, _| Ok(()));
            assert_eq!(result["ok"], false);
            let row = s.get("request-1234").unwrap().unwrap();
            assert_eq!(row["snapshot"]["origin"]["resume_id"], "exact-original");
            if fail_rollback {
                assert_eq!(result["recoveryRequired"], true);
                assert_eq!(result["recoveryError"], "origin also failed");
                assert_eq!(row["state"], "recovery_required");
                assert!(s.claim("next", "generation|%2", &json!({})).is_err());
            } else {
                assert_eq!(result["rolledBack"], true);
                assert_eq!(row["state"], "rolled_back");
            }
        }
    }
}
struct FailingJournal<'a> {
    inner: OperationStore<'a>,
    failed: Vec<&'static str>,
}
impl Journal for FailingJournal<'_> {
    fn stage(
        &self,
        id: &str,
        state: &str,
        snapshot: Option<&Value>,
        result: Option<&Value>,
    ) -> Result<()> {
        if self.failed.contains(&state) {
            Err(Error::Persistence(format!("disk unavailable at {state}")))
        } else {
            self.inner.stage(id, state, snapshot, result)
        }
    }
}
#[test]
fn persistence_failure_after_apply_never_prevents_exact_rollback() {
    for failed in [
        vec!["verifying"],
        vec!["confirmed"],
        vec!["recovering"],
        vec!["verifying", "recovering", "rolled_back"],
    ] {
        let f = Fixture::new();
        let inner = claimed(&f.conn);
        let s = FailingJournal {
            inner,
            failed: failed.clone(),
        };
        let mut a = FakeAdapter::new(&f.path);
        if failed == vec!["recovering"] {
            a.failure = Some("apply");
        }
        let result = ops::run_operation(&s, "request-1234", &mut a, |_, _| Ok(()));
        assert_eq!(result["ok"], false);
        assert_eq!(result["rolledBack"], true);
        assert!(a.calls.contains(&"rollback".into()));
        if failed.contains(&"rolled_back") {
            assert_eq!(result["recoveryRequired"], true);
            assert!(
                result["persistenceError"]
                    .as_str()
                    .unwrap()
                    .contains("rolled_back")
            );
            assert!(s.inner.claim("next", "generation|%2", &json!({})).is_err());
        }
    }
}
#[test]
fn persistence_failure_before_apply_has_no_terminal_effect() {
    for state in ["waiting", "snapshot", "applying"] {
        let f = Fixture::new();
        let s = FailingJournal {
            inner: claimed(&f.conn),
            failed: vec![state],
        };
        let mut a = FakeAdapter::new(&f.path);
        assert_eq!(
            ops::run_operation(&s, "request-1234", &mut a, |_, _| Ok(()))["ok"],
            false
        );
        assert!(!a.calls.contains(&"apply".into()));
        assert!(!a.calls.contains(&"rollback".into()));
    }
}
#[test]
fn pending_confirmation_retains_recovery_lock_snapshot_and_continuity() {
    let f = Fixture::new();
    let s = claimed(&f.conn);
    let mut a = FakeAdapter::new(&f.path);
    a.pending = true;
    let result = ops::run_operation(&s, "request-1234", &mut a, |_, _| Ok(()));
    assert_eq!(result["ok"], true);
    assert_eq!(result["pending"], true);
    assert_eq!(result["confirmed"], false);
    assert_eq!(result["recoveryAllowed"], true);
    assert_eq!(result["continuity"], "resumed");
    let row = s.get("request-1234").unwrap().unwrap();
    assert_eq!(row["state"], "awaiting_confirmation");
    assert!(row["snapshot"].is_object());
    assert!(s.claim("next", "generation|%2", &json!({})).is_err());
    assert!(s.claim_recovery("request-1234").unwrap());
}
fn identity() -> Value {
    json!({"socket_path":"/tmp/private","pid":"12","server_start":"100","session_id":"$1","pane_id":"%2","pane_pid":"123"})
}
fn key() -> &'static str {
    "/tmp/private|12|100|$1|%2|123"
}
fn observed(model: &str) -> Value {
    json!({"harness":"codex","motor":"codex","model":model,"effort":"high","harnessAccount":"main","motorAccount":"main","confirmed":true,"conversationId":"private-conversation","transcriptPath":"/private/transcript"})
}
fn record(
    s: &OperationStore<'_>,
    id: &str,
    at: f64,
    request: Value,
    result: Value,
    cwd: &str,
    origin_observed: Value,
) {
    s.claim(id, key(), &request).unwrap();
    s.stage(
        id,
        "confirmed",
        Some(&json!({"origin":{"cwd":cwd,"identity":identity(),"observed":origin_observed}})),
        Some(&result),
    )
    .unwrap();
    s.connection
        .execute(
            "UPDATE session_operations SET updated=? WHERE id=?",
            params![at, id],
        )
        .unwrap();
}
#[test]
fn readonly_history_groups_project_only_and_returns_bounded_metadata() {
    let f = Fixture::new();
    let s = store(&f.conn);
    for (id, model, at, cwd) in [
        ("a", "new-model", 10.0, "/private/project"),
        ("b", "new-model", 20.0, "/private/project"),
        ("c", "other-model", 30.0, "/private/project"),
        ("foreign", "new-model", 40.0, "/other/project"),
    ] {
        record(
            &s,
            id,
            at,
            json!({}),
            json!({"ok":true,"observed":observed(model)}),
            cwd,
            observed("old-model"),
        );
    }
    let before = std::fs::read(&f.path).unwrap();
    let result = ops::config_history(&f.path, "/private/project", key()).unwrap();
    assert_eq!(result["items"][0]["count"], 2);
    assert_eq!(result["items"][0]["lastUsed"], 20.0);
    assert_eq!(result["items"][1]["config"]["model"], "other-model");
    assert_eq!(result["previous"]["config"]["model"], "old-model");
    assert_eq!(result["previous"]["lastUsed"], 30.0);
    assert!(!result.to_string().contains("private"));
    assert_eq!(before, std::fs::read(&f.path).unwrap());
    assert!(
        ops::config_history(&f.path, "/private/project", "other-key").unwrap()["previous"]
            .is_null()
    );
}
#[test]
fn history_excludes_non_config_unconfirmed_unknown_and_malformed_records() {
    for mode in 0..12 {
        let f = Fixture::new();
        let s = store(&f.conn);
        let mut request = json!({});
        let mut result = json!({"ok":true,"observed":observed("model")});
        let mut state = "confirmed";
        match mode {
            0 => state = "failed",
            1 => state = "rolled_back",
            2 => state = "awaiting_confirmation",
            3 => request["extensionsOnly"] = json!(true),
            4 => result["unchanged"] = json!(true),
            5 => result["ok"] = json!(false),
            6 => result["observed"]["confirmed"] = json!(false),
            7 => result["observed"]["motorAccount"] = json!("unknown"),
            8 => result["observed"]["model"] = json!({"secret":"value"}),
            9 => result["observed"] = json!([]),
            10 => result["observed"]["harnessAccount"] = json!("/secret/auth.json"),
            _ => result["observed"]["model"] = json!("model\nsecret"),
        };
        record(
            &s,
            "one",
            100.0,
            request,
            result,
            "/private/project",
            observed("old"),
        );
        s.connection
            .execute("UPDATE session_operations SET state=?", [state])
            .unwrap();
        assert!(
            ops::config_history(&f.path, "/private/project", key()).unwrap()["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    for raw in ["{broken", "[]", "null"] {
        let f = Fixture::new();
        let s = store(&f.conn);
        for id in ["valid", "broken"] {
            record(
                &s,
                id,
                100.0,
                json!({}),
                json!({"ok":true,"observed":observed("model")}),
                "/private/project",
                observed("old"),
            );
        }
        s.connection
            .execute(
                "UPDATE session_operations SET result=? WHERE id='broken'",
                [raw],
            )
            .unwrap();
        assert_eq!(
            ops::config_history(&f.path, "/private/project", key()).unwrap()["items"][0]["count"],
            1
        );
    }
}
#[test]
fn history_previous_checks_exact_identity_and_does_not_fallback_from_invalid_latest() {
    let f = Fixture::new();
    let s = store(&f.conn);
    record(
        &s,
        "a",
        100.0,
        json!({}),
        json!({"ok":true,"observed":observed("model")}),
        "/private/project",
        observed("old"),
    );
    record(
        &s,
        "b",
        200.0,
        json!({}),
        json!({"ok":true,"observed":observed("model")}),
        "/private/project",
        json!({}),
    );
    assert!(ops::config_history(&f.path, "/private/project", key()).unwrap()["previous"].is_null());
    s.connection.execute("UPDATE session_operations SET snapshot=json_set(snapshot,'$.origin.identity.server_start','other')",[]).unwrap();
    assert!(ops::config_history(&f.path, "/private/project", key()).unwrap()["previous"].is_null());
}
#[test]
fn history_orders_ties_limits_twenty_and_accepts_missing_effort() {
    let f = Fixture::new();
    let s = store(&f.conn);
    for n in (0..25).rev() {
        let model = format!("model-{n:02}");
        let mut obs = observed(&model);
        obs.as_object_mut().unwrap().remove("effort");
        record(
            &s,
            &n.to_string(),
            50.0,
            json!({}),
            json!({"ok":true,"observed":obs}),
            "/private/project",
            observed("old"),
        );
    }
    let items = ops::config_history(&f.path, "/private/project", key()).unwrap()["items"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(items.len(), 20);
    for (n, item) in items.iter().enumerate() {
        assert_eq!(item["config"]["model"], format!("model-{n:02}"));
        assert_eq!(item["config"]["effort"], "");
    }
}
#[test]
fn missing_history_database_is_empty_without_creation() {
    let path = std::env::temp_dir().join("missing-operation-history-db");
    assert!(!path.exists());
    assert_eq!(
        ops::config_history(&path, "/private/project", key()).unwrap()["items"],
        json!([])
    );
    assert!(!path.exists());
}
#[test]
fn unchanged_unconfirmed_observation_fails_without_restart() {
    let f = Fixture::new();
    let s = claimed(&f.conn);
    let mut a = FakeAdapter::new(&f.path);
    a.unchanged = true;
    a.pending = true;
    let result = ops::run_operation(&s, "request-1234", &mut a, |_, _| Ok(()));
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"], "no se confirmó la configuración actual");
    assert_eq!(a.calls, vec!["prepare", "identity", "verify"]);
    assert_eq!(s.get("request-1234").unwrap().unwrap()["state"], "failed");
}
#[test]
fn history_identifier_bounds_are_enforced_before_any_metadata_leaves_store() {
    for (field, value) in [
        ("model", "m".repeat(161)),
        ("effort", "e".repeat(81)),
        ("motorAccount", "a".repeat(81)),
        ("effort", "high token=secret".into()),
        ("harnessAccount", "é".into()),
    ] {
        let f = Fixture::new();
        let s = store(&f.conn);
        let mut obs = observed("model");
        obs[field] = json!(value);
        record(
            &s,
            "one",
            100.0,
            json!({}),
            json!({"ok":true,"observed":obs}),
            "/private/project",
            observed("old"),
        );
        assert!(
            ops::config_history(&f.path, "/private/project", key()).unwrap()["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    let f = Fixture::new();
    let s = store(&f.conn);
    let mut obs = observed(&"m".repeat(160));
    obs["effort"] = json!("e".repeat(80));
    record(
        &s,
        "one",
        100.0,
        json!({}),
        json!({"ok":true,"observed":obs}),
        "/private/project",
        observed("old"),
    );
    assert_eq!(
        ops::config_history(&f.path, "/private/project", key()).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn owner_and_clock_are_evaluated_at_each_durable_write() {
    let f = Fixture::new();
    let owner = Cell::new(777);
    let time = Cell::new(100.0);
    let owner_callback = || owner.get();
    let clock_callback = || time.get();
    let s = OperationStore::new(&f.conn, &owner_callback, &clock_callback).unwrap();
    s.claim("one", "pane", &json!({})).unwrap();
    assert_eq!(s.get("one").unwrap().unwrap()["owner"], 777);
    assert_eq!(s.get("one").unwrap().unwrap()["updated"], 100.0);
    time.set(200.0);
    s.stage("one", "recovery_required", None, None).unwrap();
    owner.set(888);
    time.set(300.0);
    assert!(s.claim_recovery("one").unwrap());
    let row = s.get("one").unwrap().unwrap();
    assert_eq!(row["owner"], 888);
    assert_eq!(row["updated"], 300.0);
}
/// `stage`/`cancel_waiting` guardan `json.dumps(x)` del Python: orden de
/// inserción, ASCII escapado y separadores por omisión (el `/model/status`
/// del Python y del Rust lee ese orden tal cual).
#[test]
fn durable_writes_keep_python_insertion_order() {
    let raw = |conn: &Connection, column: &str| -> String {
        conn.query_row(
            &format!("SELECT {column} FROM session_operations WHERE id='request-1234'"),
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    let detail = |conn: &Connection| -> String {
        conn.query_row(
            "SELECT detail FROM session_operation_events ORDER BY rowid DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    let f = Fixture::new();
    let s = claimed(&f.conn);
    s.stage("request-1234", "applying", None, None).unwrap();
    s.recover_abandoned(|_| Ok(false)).unwrap();
    let expected = ascii(
        r#"{"ok": false, "error": "operación interrumpida; revisar recuperación", "recoveryRequired": true}"#,
    );
    assert_eq!(raw(&f.conn, "result"), expected);
    assert_eq!(detail(&f.conn), expected);
    let f = Fixture::new();
    let s = claimed(&f.conn);
    assert!(s.cancel_waiting("request-1234").unwrap());
    assert_eq!(
        raw(&f.conn, "result"),
        ascii(
            r#"{"ok": false, "error": "operación cancelada antes de cerrar el origen", "cancelled": true}"#
        )
    );
    let f = Fixture::new();
    let s = claimed(&f.conn);
    s.stage(
        "request-1234",
        "snapshot",
        Some(&json!({"z": 1, "a": {"y": 2, "b": 3}})),
        None,
    )
    .unwrap();
    assert_eq!(
        raw(&f.conn, "snapshot"),
        r#"{"z": 1, "a": {"y": 2, "b": 3}}"#
    );
    assert_eq!(detail(&f.conn), "{}");
}

/// `ensure_ascii` del Python: cada carácter no ASCII como escape UTF-16.
fn ascii(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            for unit in c.encode_utf16(&mut [0; 2]) {
                out.push('\\');
                out.push_str(&format!("u{unit:04x}"));
            }
        }
    }
    out
}

#[test]
fn a_parked_operation_left_behind_releases_its_pane() {
    // 13-oct: rows stuck in recovery_required/awaiting_confirmation blocked every
    // later account switch on the pane with «operación pendiente».
    let f = Fixture::new();
    let s = store(&f.conn);
    assert!(s.claim("parked", "pane", &json!({"pane":"%2"})).unwrap());
    s.stage("parked", "recovery_required", None, None).unwrap();
    assert!(matches!(
        s.claim("next", "pane", &json!({"pane":"%2","alias":"relotto"})),
        Err(Error::Conflict(_))
    ));
    assert_eq!(s.active_for_pane("pane").unwrap().unwrap()["id"], "parked");
    // Only from the state it was read in: a concurrent recovery keeps the row.
    assert!(!s.supersede("parked", "awaiting_confirmation", &json!({})).unwrap());
    let result = json!({"ok":false,"superseded":true});
    assert!(s.supersede("parked", "recovery_required", &result).unwrap());
    assert_eq!(s.get("parked").unwrap().unwrap()["state"], "failed");
    assert!(s.active_for_pane("pane").unwrap().is_none());
    assert!(s.claim("next", "pane", &json!({"pane":"%2","alias":"relotto"})).unwrap());
    // Running operations are never superseded.
    assert!(!s.supersede("next", "validating", &result).unwrap());
}
