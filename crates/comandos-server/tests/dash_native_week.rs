//! GET /analytics/week: nativa, byte a byte con el cc-dash del repo sobre el mismo HOME
//! (con y sin `sidebar=1`, que responde solo las cuentas de la columna; las dos
//! semanas, los errores de la consulta y los datos de ejemplo del mockup).
mod support;
use comandos_server::{
    Request,
    dash::native::{Native, NativeRoute, Outcome, state::StateBackend, usage::UsageRoute},
};
use serde_json::Value;
use std::{path::Path, sync::Arc, time::Duration};
use support::{
    FakeLegacy, NOW_MS, TestHome, Wire, dead_port, front, get, http_golden::FrozenHttp, seed_usage,
};

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Los campos que dependen del minuto (`measuredAt`, `now`) y del segundo del
/// refresco de límites (`captured_at` de las filas medidas) se igualan.
fn masked(text: &str) -> String {
    let mut v: Value = serde_json::from_str(text).unwrap();
    if let Some(week) = v.get_mut("week") {
        week["measuredAt"] = serde_json::json!("<hora>");
        week["now"] = serde_json::json!(0);
    }
    if let Some(accounts) = v.get_mut("accounts").and_then(Value::as_array_mut) {
        for account in accounts {
            if let Some(obj) = account.as_object_mut() {
                for (_, field) in obj.iter_mut() {
                    if let Some(inner) = field.as_object_mut()
                        && inner.contains_key("captured_at")
                    {
                        inner.insert("captured_at".into(), serde_json::json!(0));
                    }
                }
            }
        }
    }
    comandos_core::json::response_dumps(&v).unwrap()
}

fn iso(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// Turnos y tramos de dos semanas, registros de Pomodoro y archivos de límites
/// locales (rollout de Codex, log de Grok, cuota de agy) relativos a la hora real.
fn seed_week(home: &TestHome, now: i64) {
    let turn = |id: &str,
                prov: &str,
                agent: &str,
                acct: &str,
                pwd: &str,
                model: &str,
                s: i64,
                e: i64,
                tok: i64,
                cost: f64| {
        format!(
            "insert into usage_turns(id,provider,agent,tmux_session,tmux_pane,pane_pwd,git_root,model,\
             harness_account,turn_started_at,turn_finished_at,total_tokens,cost_usd,source,confidence,raw) values \
             ('{id}','{prov}','{agent}','s-{id}','%1','{pwd}','{pwd}','{model}','{acct}',{s},{e},{tok},{cost},'hook','exact','{{}}');"
        )
    };
    let sql = [
        turn("a", "claude", "claude", "main", "/r/Alfa", "claude-fable-5", now - 7200, now - 7000, 900, 0.5),
        turn("a2", "claude", "claude", "main", "/r/Alfa", "claude-fable-5", now - 6900, now - 6000, 1200, 0.25),
        turn("b", "grok", "grok", "unknown", "/r/Beta", "grok-4.5", now - 90_000, now - 89_000, 60, 0.0),
        turn("c", "codex", "codex", "main", "/r/Gamma", "gpt-5.5", now - 3 * 86_400, now - 3 * 86_400 + 600, 5000, 0.0),
        turn("d", "claude", "claude", "relotto", "/r/Delta", "claude-fable-5", now - 9 * 86_400, now - 9 * 86_400 + 1800, 7000, 1.5),
        turn("o", "opencode", "opencode", "main", "/r/Omega", "opencode/model-free", now - 4000, now - 3500, 123_456, 0.75),
        "insert into usage_spans(id,provider,account,git_root,started_at,finished_at,source) values \
         ('sp1','claude','main','/r/Alfa',".to_string()
            + &format!("{},{}", now - 5000, now - 4000)
            + ",'hook');",
    ]
    .concat();
    seed_usage(home, &sql);

    let rollout_at = now - 300;
    write(
        &home.root.join(".codex/sessions/2026/10/04/rollout-a.jsonl"),
        &format!(
            "{{\"timestamp\":\"{}\",\"payload\":{{\"rate_limits\":{{\"primary\":{{\"used_percent\":20,\"window_minutes\":300,\"resets_at\":{}}},\"secondary\":{{\"used_percent\":60.06,\"window_minutes\":10080,\"resets_at\":{}}},\"plan_type\":\"pro\"}}}}}}\n",
            iso(rollout_at),
            now + 4 * 3600,
            now + 3 * 86_400
        ),
    );
    write(
        &home.root.join(".grok/logs/unified.jsonl"),
        &format!(
            "{{\"ts\":\"{}\",\"msg\":\"billing: fetched credits config\",\"ctx\":{{\"subscriptionTier\":\"SuperGrok\",\"config\":{{\"creditUsagePercent\":37.5,\"currentPeriod\":{{\"start\":\"{}\",\"end\":\"{}\"}}}}}}}}\n",
            iso(now - 3600),
            iso(now - 2 * 86_400),
            iso(now + 5 * 86_400)
        ),
    );
    home.write(
        "agy-quota.json",
        &format!(
            r#"{{"captured_at": {}, "plan_tier": "pro", "quota": {{"gemini-weekly": {{"remaining_fraction": 0.42, "reset_time": "{}"}}}}}}"#,
            now - 60,
            iso(now + 4 * 86_400)
        ),
    );
    seed_snapshots(home, now);
    seed_records(home, now);
}

/// Fotos de cuota del ciclo anterior (`waste`, `weekUsed`).
fn seed_snapshots(home: &TestHome, now: i64) {
    seed_usage(
        home,
        &format!(
            "insert into usage_quota_snapshots(limit_id,provider,account,win,scope,resets_at,percent,captured_at) values \
             ('codex_weekly','codex','main','7d','',{r1},80.5,{c1}),\
             ('claude_weekly','claude','main','7d','',{r2},33.0,{c2});",
            r1 = now - 4 * 86_400,
            c1 = now - 4 * 86_400 - 120,
            r2 = now - 2 * 86_400,
            c2 = now - 2 * 86_400 - 300,
        ),
    );
}

fn seed_records(home: &TestHome, now: i64) {
    let backend = StateBackend::open(&home.state_db(), now as f64).unwrap();
    let ms = now * 1000;
    for (id, status, start, active, project) in [
        ("p1", "completed", ms - 3_600_000, 1_500_000, "Alfa"),
        ("p2", "cancelled", ms - 2 * 86_400_000, 600_000, ""),
        ("p3", "completed", ms - 9 * 86_400_000, 1_500_000, "Delta"),
    ] {
        backend
            .conn
            .execute(
                "INSERT INTO pomodoro_records (block_id,mode,status,target_ms,active_ms,planned_ms,\
                 started_at_ms,ended_at_ms,project,session_key,pane_key,provenance,recorded_at_ms) \
                 VALUES (?1,'focus',?2,1500000,?3,1500000,?4,?5,?6,'s','%1','measured',?5)",
                rusqlite::params![id, status, active, start, start + active, project],
            )
            .unwrap();
    }
}

/// Los dos lados sobre la misma consulta; un desfase de minuto entre las dos
/// peticiones (etiquetas de tiempo restante) se reintenta.
async fn compare(front_port: u16, py: &FrozenHttp<'_>, target: &str) {
    let mut last = (String::new(), String::new());
    for _ in 0..3 {
        let a = get(front_port, target).await;
        let b = py.get(target).await;
        assert_eq!(a.status, b.status, "{target}: {} / {}", a.text(), b.text());
        assert_eq!(
            a.header("content-type"),
            b.header("content-type"),
            "{target}"
        );
        assert_eq!(
            a.header("cache-control"),
            b.header("cache-control"),
            "{target}"
        );
        let (x, y) = if a.status == 200 && target.contains("sidebar=1") {
            // La columna responde solo `{"accounts": [...]}`: lo mismo que deja
            // el Python (`build_week` + `sidebar_accounts`) en `accounts`.
            let python: Value = serde_json::from_str(&b.text()).unwrap();
            let only = serde_json::json!({"accounts": python["accounts"]});
            (masked(&a.text()), masked(&only.to_string()))
        } else if a.status == 200 {
            (masked(&a.text()), masked(&b.text()))
        } else {
            (a.text(), b.text())
        };
        if x == y {
            return;
        }
        last = (x, y);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(last.0, last.1, "{target}");
}

/// La caché de límites de cada lado se llena con la primera petición
/// (refresco a demanda en el frente; el Python además refresca al arrancar).
async fn warm(front_port: u16, py: &mut FrozenHttp<'_>, home: &TestHome) {
    let _ = get(front_port, "/analytics/week").await;
    if home.hooks().join("agy-quota.json").exists() {
        // Limits publish before their queued SQLite write. Wait for the actual
        // native snapshot, not a variable race between that write and the oracle key.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let conn = rusqlite::Connection::open_with_flags(
                home.usage_db(),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let ready: i64 = conn
                .query_row(
                    "select count(*) from usage_quota_snapshots where captured_at >= ?1",
                    [NOW_MS / 1000 - 300],
                    |row| row.get(0),
                )
                .unwrap();
            if ready > 0 {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "native quota snapshot did not finish"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    py.prime_unchanged("/analytics/week").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
}

const TARGETS: &[&str] = &[
    "/analytics/week",
    "/analytics/week?offset=0&sidebar=1",
    "/analytics/week?offset=-1",
    "/analytics/week?offset=-1&sidebar=1",
    "/analytics/week?offset=",
    "/analytics/week?sidebar=0",
    "/analytics/week?offset=x",
    "/analytics/week?offset=2",
    "/analytics/week?offset=+0",
    "/analytics/week?offset=%20-1%20",
    "/analytics/week?offset=-0&offset=x",
    "/analytics/week?demo=normal",
    "/analytics/week?demo=normal&offset=-1",
    "/analytics/week?demo=LIMITE1limite",
    "/analytics/week?demo=Base1",
    "/analytics/week?demo=nada",
    "/analytics/week?demo=normal&offset=3",
    "/analytics/week?demo=123",
];

#[tokio::test]
async fn analytics_week_matches_python() {
    let home = TestHome::new("week");
    let now = NOW_MS / 1000;
    seed_week(&home, now);
    home.write("cc-notify.conf", "DESKTOP_NOTIFY=0\n");
    let mut py = FrozenHttp::new_rooted_with(
        &home,
        "server-http-analytics-week",
        &[
            ".claude/hooks/comandos-usage.sqlite",
            ".local/state/comandos/app-state.sqlite3",
            ".claude/hooks/agy-quota.json",
            ".claude/hooks/cc-notify.conf",
            ".codex/sessions/2026/10/04/rollout-a.jsonl",
            ".grok/logs/unified.jsonl",
        ],
        "",
    )
    .await;
    let mut opts = home.options();
    opts.clock = Arc::new(|| NOW_MS);
    let front = front(&home, dead_port(), opts).await;
    warm(front.port, &mut py, &home).await;
    for target in TARGETS {
        compare(front.port, &py, target).await;
    }
    // Hubo límites de verdad en los dos lados (no una comparación de vacíos).
    let week: Value =
        serde_json::from_str(&get(front.port, "/analytics/week").await.text()).unwrap();
    let ids: Vec<&str> = week["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert!(ids.contains(&"codex:main"), "{ids:?}");
    let side: Value =
        serde_json::from_str(&get(front.port, "/analytics/week?sidebar=1").await.text()).unwrap();
    assert!(
        side["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == "opencode:main" && a["measured"].is_object()),
        "{side}"
    );
    front.stop().await;
}

/// HOME vacío: sin base de uso (las dos la crean), sin archivos de límites.
#[tokio::test]
async fn analytics_week_empty_home_matches_python() {
    let home = TestHome::new("week-empty");
    home.write("cc-notify.conf", "DESKTOP_NOTIFY=0\n");
    let mut py = FrozenHttp::new_rooted_with(
        &home,
        "server-http-analytics-week",
        &[
            ".claude/hooks/comandos-usage.sqlite",
            ".local/state/comandos/app-state.sqlite3",
            ".claude/hooks/agy-quota.json",
            ".claude/hooks/cc-notify.conf",
            ".codex/sessions/2026/10/04/rollout-a.jsonl",
            ".grok/logs/unified.jsonl",
        ],
        "",
    )
    .await;
    let mut opts = home.options();
    opts.clock = Arc::new(|| NOW_MS);
    let front = front(&home, dead_port(), opts).await;
    warm(front.port, &mut py, &home).await;
    for target in [
        "/analytics/week",
        "/analytics/week?sidebar=1",
        "/analytics/week?offset=-1",
        "/analytics/week?offset=-1&sidebar=1",
    ] {
        compare(front.port, &py, target).await;
    }
    front.stop().await;
}

#[tokio::test]
async fn analytics_week_prefix_is_native() {
    let home = TestHome::new("week-prefix");
    let legacy = FakeLegacy::start().await;
    let front = front(&home, legacy.port, home.options()).await;
    let wire = get(front.port, "/analytics/weekly").await;
    assert_eq!(wire.status, 200);
    assert!(serde_json::from_slice::<serde_json::Value>(&wire.body).unwrap()["week"].is_object());
    assert!(legacy.requests().is_empty());
    front.stop().await;
}

/// Ejemplo con JSON roto: el `json.load` lanza fuera del `except OSError` → 500.
/// Sin checkout del heredado no se sabe dónde están los ejemplos: se reenvía.
#[tokio::test]
async fn demo_broken_json_is_500_and_unknown_repo_forwards() {
    let home = TestHome::new("week-demo");
    let repo = home.root.join("repo");
    write(
        &repo.join("tests/fixtures/analytics/week-roto.json"),
        "{\"week\": ",
    );
    write(
        &repo.join("tests/fixtures/analytics/week-bien-prev.json"),
        "{\"x\": 1.50, \"y\": [1e2, -0.0]}",
    );
    let legacy = FakeLegacy::start().await;
    let mut opts = home.options();
    opts.repo_root = Some(repo);
    let front_a = front(&home, legacy.port, opts).await;
    let wire = get(front_a.port, "/analytics/week?demo=roto").await;
    assert_eq!(
        (wire.status, wire.text()),
        (500, r#"{"error": "Error interno del tablero"}"#.to_owned())
    );
    let wire = get(front_a.port, "/analytics/week?demo=bien&offset=-1").await;
    assert_eq!(
        (wire.status, wire.text()),
        (200, r#"{"x": 1.5, "y": [100.0, -0.0]}"#.to_owned())
    );
    front_a.stop().await;

    let mut opts = home.options();
    opts.repo_root = None;
    let front_b = front(&home, legacy.port, opts).await;
    let wire = get(front_b.port, "/analytics/week?demo=normal").await;
    assert_eq!(wire.text(), r#"{"legacy": true}"#);
    // Los errores de la consulta no dependen del checkout.
    let wire: Wire = get(front_b.port, "/analytics/week?offset=x").await;
    assert_eq!(
        (wire.status, wire.text()),
        (400, r#"{"error": "offset inv\u00e1lido"}"#.to_owned())
    );
    front_b.stop().await;
    assert_eq!(
        legacy.requests(),
        ["GET /analytics/week?demo=normal HTTP/1.1"]
    );
}

/// Base de uso más nueva: el carril se apaga y la ruta entera se reenvía (D10):
/// el Python es dueño del dominio, también de los errores de la consulta.
#[tokio::test]
async fn newer_usage_db_forwards_week() {
    let home = TestHome::new("week-newer");
    let conn = rusqlite::Connection::open(home.usage_db()).unwrap();
    conn.execute_batch("pragma user_version=12").unwrap();
    drop(conn);
    let legacy = FakeLegacy::start().await;
    let front = front(&home, legacy.port, home.options()).await;
    for target in [
        "/analytics/week",
        "/analytics/week?sidebar=1",
        "/analytics/week?offset=5",
        "/analytics/week?demo=normal",
    ] {
        assert_eq!(get(front.port, target).await.text(), r#"{"legacy": true}"#);
    }
    front.stop().await;
    assert_eq!(legacy.requests().len(), 4);
    // El frente no tocó la base: sigue en la versión 12.
    let conn = rusqlite::Connection::open(home.usage_db()).unwrap();
    let version: i64 = conn
        .query_row("pragma user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 12);
}

fn week_request(target: &str) -> Request {
    Request {
        method: http::Method::GET,
        target: target.into(),
        peer: "127.0.0.1:12345".parse().unwrap(),
        headers: vec![],
        data: None,
        body: bytes::Bytes::new(),
        internal_producer: false,
    }
}

/// Una fila que el Python no podría decodificar (BLOB en `model`) declina antes de
/// cualquier efecto: el refresco de límites no se lanza. Control: la misma ruta
/// sin el BLOB sí lo lanza (vencido desde siempre).
#[tokio::test]
async fn undecodable_row_declines_without_limits_refresh() {
    let home = TestHome::new("week-blob");
    let now = NOW_MS / 1000;
    seed_usage(
        &home,
        &format!(
            "insert into usage_turns(id,provider,agent,tmux_session,tmux_pane,pane_pwd,git_root,model,\
             harness_account,turn_started_at,turn_finished_at,total_tokens,cost_usd,source,confidence,raw) values \
             ('x','claude','claude','s','%1','/r','/r',X'00ff','main',{},{},1,0.0,'hook','exact','{{}}');",
            now - 100,
            now - 50
        ),
    );
    let native = Arc::new(Native::new(home.options()));
    let route = NativeRoute::Usage(UsageRoute::Week);
    let outcome = native
        .dispatch(route, &week_request("/analytics/week"))
        .await
        .unwrap();
    assert!(matches!(outcome, Outcome::Decline));
    assert!(
        !native.limits().refreshing(),
        "declinar no lanza el refresco"
    );
    native.shutdown().await;

    let conn = rusqlite::Connection::open(home.usage_db()).unwrap();
    conn.execute("update usage_turns set model='claude-fable-5'", [])
        .unwrap();
    drop(conn);
    let native = Arc::new(Native::new(home.options()));
    let outcome = native
        .dispatch(route, &week_request("/analytics/week"))
        .await
        .unwrap();
    assert!(matches!(outcome, Outcome::Reply(_)));
    assert!(native.limits().refreshing(), "la respuesta sí lo lanza");
    native.shutdown().await;
}

/// Turnos de cuatro cuentas repartidos en 17 días (uno cada `17 * 85_000 / n` s),
/// la mitad en cadenas de menos de `GAP_S` que cruzan la medianoche y el corte.
fn seed_many_turns(home: &TestHome, now: i64, n: i64) {
    let step = 17 * 86_400 / n;
    let accounts = [
        ("claude", "main"),
        ("claude", "relotto"),
        ("codex", "main"),
        ("opencode", "main"),
    ];
    let mut sql = String::from("begin;");
    for i in 0..n {
        let (p, acct) = accounts[(i % 4) as usize];
        let end = now - 17 * 86_400 + i * step;
        sql.push_str(&format!(
            "insert into usage_turns(id,provider,agent,tmux_session,tmux_pane,pane_pwd,git_root,model,\
             harness_account,turn_started_at,turn_finished_at,total_tokens,cost_usd,source,confidence,raw) values \
             ('t{i}','{p}','{p}','s{}','%1','/r/P{}','/r/P{}','m{}','{acct}',{},{end},{},0.01,'hook','exact','{{}}');",
            i % 40,
            i % 12,
            i % 12,
            i % 3,
            end - step / 2 - (i % 7) * 60,
            1000 + i % 5000
        ));
    }
    sql.push_str("commit;");
    seed_usage(home, &sql);
}

fn reply_json(outcome: Outcome, target: &str) -> Value {
    let Outcome::Reply(reply) = outcome else {
        panic!("{target} declinó");
    };
    let comandos_server::ReplyBody::Bytes(bytes) = reply.body else {
        panic!("{target}: cuerpo en flujo");
    };
    serde_json::from_slice(&bytes).unwrap()
}

/// Lo que respondía `sidebar=1` antes del atajo: `build_week` con todas las filas
/// de 17 días y `sidebar_accounts` sobre sus cuentas.
fn sidebar_before(home: &TestHome, full: &Value, now: f64) -> Value {
    let conn = rusqlite::Connection::open(home.usage_db()).unwrap();
    let mut turns = vec![];
    comandos_store::usage_read::each_week_row(&conn, now - 17.0 * 86_400.0, true, |kind, row| {
        if matches!(kind, comandos_store::usage_read::WeekRow::Turn) {
            turns.push(row.clone())
        }
    })
    .unwrap();
    let accounts = comandos_core::analytics_week::sidebar_accounts(
        full["accounts"].as_array().unwrap(),
        &[],
        &turns,
        now,
    )
    .unwrap();
    serde_json::json!({ "accounts": accounts })
}

/// `sidebar=1` responde solo `{"accounts": [...]}` y esas cuentas son las mismas
/// que antes (semana entera + `sidebar_accounts`) con turnos que cruzan el corte.
#[tokio::test]
async fn sidebar_accounts_match_the_full_week_path() {
    let home = TestHome::new("week-sidebar-same");
    let now = NOW_MS / 1000;
    seed_many_turns(&home, now, 4_000);
    let mut opts = home.options();
    opts.clock = Arc::new(|| NOW_MS);
    let native = Arc::new(Native::new(opts));
    let route = NativeRoute::Usage(UsageRoute::Week);
    let full = reply_json(
        native
            .dispatch(route, &week_request("/analytics/week?offset=0"))
            .await
            .unwrap(),
        "full",
    );
    let side = reply_json(
        native
            .dispatch(route, &week_request("/analytics/week?offset=0&sidebar=1"))
            .await
            .unwrap(),
        "sidebar",
    );
    assert_eq!(side.as_object().unwrap().len(), 1, "{side}");
    assert_eq!(side["accounts"].as_array().unwrap().len(), 4, "{side}");
    assert_eq!(
        comandos_core::json::response_dumps(&side).unwrap(),
        comandos_core::json::response_dumps(&sidebar_before(&home, &full, now as f64)).unwrap()
    );
    native.shutdown().await;
}

/// Tiempo del manejador sobre una base del tamaño de la real (~85 mil turnos en
/// 17 días): la semana entera (lo que `sidebar=1` calculaba antes) contra el
/// atajo de la columna. `cargo test … sidebar_timing -- --ignored --nocapture`.
#[tokio::test]
#[ignore]
async fn sidebar_timing_against_the_full_week() {
    let home = TestHome::new("week-timing");
    let now = NOW_MS / 1000;
    seed_many_turns(&home, now, 85_000);
    let mut opts = home.options();
    opts.clock = Arc::new(|| NOW_MS);
    let native = Arc::new(Native::new(opts));
    let route = NativeRoute::Usage(UsageRoute::Week);
    let mut bodies = vec![];
    for target in [
        "/analytics/week?offset=0",
        "/analytics/week?offset=0&sidebar=1",
    ] {
        let _ = native.dispatch(route, &week_request(target)).await.unwrap();
        let start = std::time::Instant::now();
        let mut body = Value::Null;
        for _ in 0..5 {
            body = reply_json(
                native.dispatch(route, &week_request(target)).await.unwrap(),
                target,
            );
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / 5.0;
        println!("{target}: {ms:.1} ms, {} bytes", body.to_string().len());
        bodies.push(body);
    }
    let before = sidebar_before(&home, &bodies[0], now as f64);
    assert_eq!(
        comandos_core::json::response_dumps(&bodies[1]).unwrap(),
        comandos_core::json::response_dumps(&before).unwrap()
    );
    native.shutdown().await;
}
