//! GET /analytics/week (8326, `analytics_week_query` 6616 → `analytics_week_payload`
//! 6589): la semana de Analytics por cuenta con los turnos, tramos y fotos de cuota
//! de la base de uso, los registros de Pomodoro de app-state y la caché de límites
//! del frente; con `sidebar=1`, solo `{"accounts": [...]}`: las cuotas y el
//! consumo medido de la columna (`analytics_week.sidebar_week`, lo mismo que
//! `sidebar_accounts` deja en `accounts`) sin calcular la semana entera. `demo=<nombre>` sirve los datos de ejemplo
//! del mockup (`tests/fixtures/analytics`).
//!
//! Orden de efectos: las dos lecturas (carril de uso y app-state) van primero y
//! son el punto de declinar; la política de Pomodoro es idempotente (como en
//! `/pomodoro`); el refresco de límites se lanza solo cuando ya no se declina.
use super::super::{
    Answer, Fault, Native,
    files::{self, Strict},
    light::{error, read_reply},
    py::{self, NumError},
    query::Query,
};
use crate::{HandlerError, Reply, Request};
use comandos_core::{
    analytics_week::{self, WeekInput, WeekRows},
    focus::policy_v1,
};
use comandos_store::{
    focus, pomodoro,
    usage_read::{self, ReadError, WeekRow},
};
use http::StatusCode;
use serde_json::Value;

/// `ANALYTICS_LOOKBACK_S`: 8 días de la semana anterior + 8 de esta + margen.
const LOOKBACK_S: f64 = 17.0 * 86400.0;

/// `quota_snapshots(USAGE_DB, since=now - 40 * 86400)`.
const SNAPSHOT_LOOKBACK_S: f64 = 40.0 * 86400.0;

/// `ANALYTICS_DEMO_DIR`, relativo al checkout del heredado.
const DEMO_DIR: &str = "tests/fixtures/analytics";

fn failure() -> Fault {
    Fault::Error(HandlerError::Failure)
}

pub async fn answer(native: &Native, request: &Request) -> Answer {
    // D10: con el carril de uso apagado el Python es dueño del dominio entero.
    if !native.usage.enabled() {
        return Err(Fault::Decline);
    }
    let query = Query::parse(&request.target)?;
    // `int((query.get("offset") or ["0"])[0])`; lo que el port no reproduce
    // (no ASCII, fuera de `i64`) declina.
    let offset = match py::int(query.first("offset").unwrap_or("0")) {
        Ok(n) => n,
        Err(NumError::Invalid) => return error(StatusCode::BAD_REQUEST, "offset inválido"),
        Err(_) => return Err(Fault::Decline),
    };
    if offset != 0 && offset != -1 {
        return error(
            StatusCode::BAD_REQUEST,
            "solo esta semana (0) o la anterior (-1)",
        );
    }
    // `re.sub(r"[^a-z]", "", …)`: solo quedan las minúsculas ASCII.
    let demo: String = query
        .first("demo")
        .unwrap_or("")
        .chars()
        .filter(char::is_ascii_lowercase)
        .collect();
    if !demo.is_empty() {
        return demo_answer(native, &demo, offset).await;
    }
    if query.first("sidebar") == Some("1") {
        return sidebar_payload(native, offset).await;
    }
    payload(native, offset).await
}

/// `open(ANALYTICS_DEMO_DIR/week-<demo>[-prev].json)` + `json.load`: no existe
/// → 404; JSON roto → la excepción sin capturar (500); otro `OSError` o una
/// lectura incierta → se declina.
async fn demo_answer(native: &Native, demo: &str, offset: i64) -> Answer {
    let Some(root) = native.options().repo_root.clone() else {
        return Err(Fault::Decline);
    };
    let suffix = if offset != 0 { "-prev" } else { "" };
    let path = root
        .join(DEMO_DIR)
        .join(format!("week-{demo}{suffix}.json"));
    let read = tokio::task::spawn_blocking(move || files::read_json_strict(&path))
        .await
        .map_err(|_| Fault::Decline)?;
    match read {
        Strict::Value(value) => read_reply(&value),
        Strict::Missing => error(
            StatusCode::NOT_FOUND,
            "no hay datos de ejemplo con ese nombre",
        ),
        Strict::Unreadable => Err(failure()),
        Strict::Unsure => Err(Fault::Decline),
    }
}

/// Una excepción que el Python no captura (500); lo demás (SQL, BLOB, texto no
/// UTF-8, incierto) se declina: las lecturas no tienen efectos.
fn read_fault(error: ReadError) -> Fault {
    match error {
        ReadError::Raises => failure(),
        ReadError::Sql(_) | ReadError::Undecodable | ReadError::Unsure => Fault::Decline,
    }
}

/// Lo que sale de la base de uso en un trabajo: turnos y tramos ya reducidos
/// (`WeekRows`) y las fotos de cuota.
type UsageRows = (WeekRows, Vec<Value>);

async fn payload(native: &Native, offset: i64) -> Answer {
    let now = (native.options().clock)() as f64 / 1000.0;
    let since = now - LOOKBACK_S;
    // `int(since)` de `quota_snapshots`: trunca hacia cero.
    let snapshots_since = (now - SNAPSHOT_LOOKBACK_S) as i64;
    // El Python vivo lee siempre las columnas anchas (`cost_usd, model,
    // tmux_session, agent`), con o sin `sidebar`: mismo texto SQL. Las filas se
    // reducen al leerlas (no se guardan: la base real tiene decenas de miles de
    // turnos en la ventana); un error del cálculo queda guardado para `build`,
    // tras las lecturas, como en el Python.
    let rows = native
        .usage
        .with(move |u| -> Result<UsageRows, ReadError> {
            let mut rows = WeekRows::new(now);
            usage_read::each_week_row(&u.conn, since, true, |kind, row| match kind {
                WeekRow::Turn => rows.push_turn(row),
                WeekRow::Span => rows.push_span(row),
            })?;
            let snapshots = usage_read::quota_snapshots(&u.conn, snapshots_since)?;
            Ok((rows, snapshots))
        })
        .await?;
    let (rows, snapshots) = rows.map_err(read_fault)?;
    // `pomodoro.records(pomodoro_store().conn, int(since * 1000), None)`.
    let from_ms = (since * 1000.0) as i64;
    let clock = native.options().clock.clone();
    let records = native
        .with_state(move |b| -> Result<Vec<Value>, Fault> {
            // `pomodoro_store()`: la política se activa una vez (idempotente).
            if !b.pomodoro_policy {
                focus::ensure_policy(&b.conn, &policy_v1(), clock()).map_err(|_| Fault::Decline)?;
                b.pomodoro_policy = true;
            }
            pomodoro::records(&b.conn, Some(from_ms), None, None).map_err(|_| Fault::Decline)
        })
        .await??;
    // Las filas de `usage_provider_limits()`; su refresco se lanza abajo.
    let limits: Vec<Value> = native
        .limits
        .current()
        .rows
        .into_iter()
        .map(Value::Object)
        .collect();
    // Cálculo y serialización fuera del hilo del runtime (semanas de turnos).
    let built = tokio::task::spawn_blocking(move || {
        build(&WeekInput {
            now,
            offset,
            limits: &limits,
            rows: &rows,
            snapshots: &snapshots,
            records: &records,
            tz_name: analytics_week::TZ,
        })
    })
    .await
    // Un pánico del hilo de cálculo: todavía no hubo efectos (ni el refresco
    // de límites), así que se declina en vez de responder 500.
    .map_err(|_| Fault::Decline)?;
    if let Err(Fault::Decline) = built {
        return Err(Fault::Decline);
    }
    // Ya no se declina: `usage_provider_limits()` lanza el refresco si venció
    // (también cuando `build_week` lanza, como en el Python, que lo llama antes).
    let _ = native.limits.get(&native.refresh_deps());
    built
}

/// `sidebar=1`: la columna de Consumos lee solo `accounts` (cada 60 s mientras
/// está a la vista), así que se responde `{"accounts": [...]}` con lo mismo que
/// `build_week` + `sidebar_accounts` dejan ahí, sin calcular la semana entera:
/// los turnos desde `sidebar_since` (no los 17 días), sin Pomodoro ni sesiones
/// a la vista, y las fotos de cuota solo para una semana pasada (`weekUsed`).
async fn sidebar_payload(native: &Native, offset: i64) -> Answer {
    let now = (native.options().clock)() as f64 / 1000.0;
    let since = match analytics_week::sidebar_since(now, offset, analytics_week::TZ) {
        // Nunca antes que la ventana de la semana entera: mismas filas o menos.
        Ok(at) => at.max(now - LOOKBACK_S),
        Err(_) => return Err(failure()),
    };
    let snapshots_since = (now - SNAPSHOT_LOOKBACK_S) as i64;
    let past = offset < 0;
    let rows = native
        .usage
        .with(move |u| -> Result<UsageRows, ReadError> {
            let mut rows = WeekRows::new(now);
            usage_read::each_week_row(&u.conn, since, true, |kind, row| match kind {
                WeekRow::Turn => rows.push_turn(row),
                WeekRow::Span => rows.push_span(row),
            })?;
            let snapshots = if past {
                usage_read::quota_snapshots(&u.conn, snapshots_since)?
            } else {
                Vec::new()
            };
            Ok((rows, snapshots))
        })
        .await?;
    let (rows, snapshots) = rows.map_err(read_fault)?;
    let limits: Vec<Value> = native
        .limits
        .current()
        .rows
        .into_iter()
        .map(Value::Object)
        .collect();
    let built = tokio::task::spawn_blocking(move || {
        let accounts = analytics_week::sidebar_week(&WeekInput {
            now,
            offset,
            limits: &limits,
            rows: &rows,
            snapshots: &snapshots,
            records: &[],
            tz_name: analytics_week::TZ,
        })
        .map_err(|_| failure())?;
        let mut body = serde_json::Map::new();
        body.insert("accounts".into(), Value::Array(accounts));
        Reply::json(StatusCode::OK, &Value::Object(body)).map_err(|_| Fault::Decline)
    })
    .await
    .map_err(|_| Fault::Decline)?;
    if let Err(Fault::Decline) = built {
        return Err(Fault::Decline);
    }
    let _ = native.limits.get(&native.refresh_deps());
    built
}

/// `analytics_week.build_week(...)`. Una excepción del port es la del Python
/// (500); lo que el codificador portado no puede escribir se declina (no hubo
/// efectos todavía).
fn build(input: &WeekInput<'_>) -> Answer {
    let result = analytics_week::build_week(input).map_err(|_| failure())?;
    Reply::json(StatusCode::OK, &result).map_err(|_| Fault::Decline)
}
