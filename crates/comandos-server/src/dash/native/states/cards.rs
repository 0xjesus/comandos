//! El bucle de `read_states` (7035): anotación de los registros, tarjetas
//! vivas en el orden de `panes` con cada efecto en su sitio, históricas y el
//! orden final. Los efectos pasan por `CardEffects` para que la Tarea 5 los
//! haga reales y las pruebas los sustituyan.
use super::{
    PyFloat, StateFault, is_text, observe, observe::Observed, or_default, py_float, py_str,
    records::Record,
};
use crate::dash::native::{
    py::{is_pane, is_session, is_space, session_name},
    tmux::{Output, TmuxError},
};
use comandos_core::json::{python_eq, truthy};
use comandos_runtime::{
    agent_procs::{AgentInfo, AgentMaps, PaneRow},
    hooks::py::float_value,
};
use regex::Regex;
use serde_json::{Map, Value};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    sync::LazyLock,
    time::Duration,
};

/// `DONE_GRACE_SECS` (5930).
const DONE_GRACE_SECS: f64 = 600.0;
/// `ZOMBIE_WAITING_GRACE_SECS` (5933).
const ZOMBIE_WAITING_GRACE_SECS: f64 = 86_400.0;
/// `HIDDEN_SESSIONS` (6307).
const HIDDEN_SESSIONS: [&str; 3] = ["hub", "local", "control"];
/// Plazo de `tmux()` (5715) y de la pista de Codex (`pane_status_hint`, 6171).
const TMUX_TIMEOUT: Duration = Duration::from_secs(5);
const HINT_TIMEOUT: Duration = Duration::from_secs(2);

// Expresiones fijas con las clases del Python: `\s` incluye U+001C–U+001F.
// `\b` no es idéntico: `regex` cuenta las marcas Mn como letras de palabra y no
// los números «No» (`²`), al revés que `re`; para «Working» de Codex (texto
// ASCII de su pie) la diferencia es teórica.
pub(crate) static WORKING: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\bWorking[\s\x1c-\x1f]*\(").ok());
pub(crate) static INTERRUPT: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)esc to interrupt|ctrl\+c to interrupt|esc para interrumpir").ok()
});
pub(crate) static QUESTION: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?m)^[\s\x1c-\x1f]*(Do you want|Allow|¿Permitir|Yes, |No, )").ok()
});
pub(crate) static CONFIRM: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)❯[\s\x1c-\x1f]*1\.|\(y\)|Enter to confirm").ok());

pub(crate) fn search(re: &LazyLock<Option<Regex>>, text: &str) -> Result<bool, StateFault> {
    re.as_ref()
        .map(|re| re.is_match(text))
        .ok_or(StateFault::Failure)
}

/// Cada efecto que el bucle de `read_states` hace por pane, en su orden.
pub trait CardEffects {
    /// `tmux(*args, timeout=…)`.
    fn tmux(
        &self,
        args: &[&str],
        timeout: Duration,
    ) -> impl Future<Output = Result<Output, TmuxError>> + Send;
    /// `account_for_pid(pid, agent)`.
    fn account(
        &self,
        pid: i64,
        agent: &str,
    ) -> impl Future<Output = Result<Map<String, Value>, StateFault>> + Send;
    /// `grok_metadata_for_pid(pid)`.
    fn grok_metadata(
        &self,
        pid: i64,
    ) -> impl Future<Output = Result<Map<String, Value>, StateFault>> + Send;
    /// `observe_pane` dentro del `try` de `reconcile_card_config`.
    fn observe(
        &self,
        session: &str,
        pane: &str,
        info: &AgentInfo,
    ) -> impl Future<Output = Result<Observed, StateFault>> + Send;
    /// `cc_usage.latest_session_config(USAGE_DB, session, pane)` (`None` = `{}`).
    fn session_config(
        &self,
        session: &str,
        pane: &str,
    ) -> impl Future<Output = Result<Option<Map<String, Value>>, StateFault>> + Send;
    /// `ssh -O check <host>` con plazo 3 s: `true` si sale 0 (cualquier excepción → `false`).
    fn ssh_check(&self, host: &str) -> impl Future<Output = bool> + Send;
}

/// Lo que el escaneo (Tarea 5) junta antes del bucle.
pub struct Inputs {
    /// `time.time()`; `now_ts = int(now)` para los bordes de las históricas.
    pub now: f64,
    /// `tmux_pane_inventory()`.
    pub panes: Vec<PaneRow>,
    /// `agent_pane_maps(procs, panes, ownership)`.
    pub maps: AgentMaps,
    /// Pares `(cwd, agente)` de procesos sin pane (bucle de 7131).
    pub external: HashSet<(String, String)>,
    /// `session_labels()`.
    pub labels: HashMap<String, String>,
    /// `glob(STATE/*.json)` ya leído.
    pub records: Vec<Record>,
}

type PaneKey = (String, String);

/// `annotations` y `detached` del primer bucle.
struct Annotations<'a> {
    by_pane: HashMap<PaneKey, ((bool, f64), &'a Record)>,
    detached: Vec<(String, &'a Record)>,
}

/// `str(record.get(key) or '')` cuando el texto solo se compara con un
/// patrón: un contenedor verdadero (su `repr`) nunca casa y se marca `None`.
fn compared_text(value: Option<&Value>) -> Result<Option<String>, StateFault> {
    match value.filter(|v| truthy(v)) {
        None => Ok(Some(String::new())),
        Some(Value::Array(_) | Value::Object(_)) => Ok(None),
        Some(v) => py_str(v).map(Some),
    }
}

/// `PANE_RE.fullmatch(text)`; sin `%` delante no casa aunque haya no-ASCII.
fn pane_match(text: &str) -> Result<bool, StateFault> {
    if !text.starts_with('%') {
        return Ok(false);
    }
    is_pane(text).ok_or(StateFault::Decline)
}

fn annotate(inputs: &Inputs) -> Result<Annotations<'_>, StateFault> {
    let mut agents: HashMap<(&str, &str), &AgentInfo> = HashMap::new();
    let mut by_session: HashMap<&str, Vec<&AgentInfo>> = HashMap::new();
    for (_, rows) in &inputs.maps.by_cwd {
        for info in rows {
            if agents
                .insert((info.session.as_str(), info.pane.as_str()), info)
                .is_none()
            {
                by_session
                    .entry(info.session.as_str())
                    .or_default()
                    .push(info);
            }
        }
    }
    let live: HashSet<&str> = inputs.panes.iter().map(|r| r.session.as_str()).collect();
    let mut out = Annotations {
        by_pane: HashMap::new(),
        detached: Vec::new(),
    };
    for record in &inputs.records {
        let value = &record.value;
        // `str(record.get('session') or '')`: un contenedor da texto no vacío e inválido.
        let explicit = compared_text(value.get("session"))?;
        let explicit_present = explicit.as_ref().is_none_or(|s| !s.is_empty());
        let session = match explicit.as_deref().filter(|s| is_session(s)) {
            Some(s) => s.to_owned(),
            None => match value.get("project") {
                None => String::new(),
                Some(Value::String(project)) => session_name(project),
                // `re.sub` de un no-str: `TypeError` sin capturar.
                Some(_) => return Err(StateFault::Failure),
            },
        };
        if !is_session(&session) {
            continue;
        }
        let pane = compared_text(value.get("pane"))?;
        let exact = match pane.as_deref() {
            Some(p) => pane_match(p)?,
            None => false,
        };
        let reported = or_default(value.get("agent"), "claude");
        let same_agent =
            |info: &&AgentInfo| python_eq(&Value::from(info.agent.as_str()), &reported);
        let matched: Option<&AgentInfo> = if exact {
            let pane = pane.as_deref().unwrap_or_default();
            agents
                .get(&(session.as_str(), pane))
                .copied()
                .filter(|info| same_agent(info))
        } else {
            let mut candidates: Vec<&AgentInfo> = by_session
                .get(session.as_str())
                .map(|rows| rows.iter().copied().filter(same_agent).collect())
                .unwrap_or_default();
            // Un hook heredado sin pane solo es seguro si no es ambiguo.
            if candidates.is_empty() && !explicit_present {
                candidates = match value.get("cwd").filter(|v| truthy(v)) {
                    // `by_cwd.get(lista)`: `TypeError` (no hasheable) sin capturar.
                    Some(Value::Array(_) | Value::Object(_)) => return Err(StateFault::Failure),
                    Some(Value::String(cwd)) => cwd_rows(&inputs.maps, cwd, &reported),
                    Some(_) => Vec::new(),
                    None => cwd_rows(&inputs.maps, "", &reported),
                };
            }
            match candidates.as_slice() {
                [only] => Some(*only),
                _ => None,
            }
        };
        if let Some(info) = matched {
            let key = (info.session.clone(), info.pane.clone());
            let priority = (exact, record.timestamp);
            let better = out
                .by_pane
                .get(&key)
                .is_none_or(|(previous, _)| priority_gt(priority, *previous));
            if better {
                out.by_pane.insert(key, (priority, record));
            }
        } else if !live.contains(session.as_str()) || is_text(value.get("status"), "waiting") {
            out.detached.push((session, record));
        }
    }
    Ok(out)
}

fn cwd_rows<'a>(maps: &'a AgentMaps, cwd: &str, reported: &Value) -> Vec<&'a AgentInfo> {
    maps.by_cwd
        .iter()
        .find(|(c, _)| c == cwd)
        .map(|(_, rows)| {
            rows.iter()
                .filter(|info| python_eq(&Value::from(info.agent.as_str()), reported))
                .collect()
        })
        .unwrap_or_default()
}

/// `(exact, timestamp) > previous` con tuplas de Python.
fn priority_gt(a: (bool, f64), b: (bool, f64)) -> bool {
    match a.0.cmp(&b.0) {
        Ordering::Equal => a.1 > b.1,
        other => other == Ordering::Greater,
    }
}

async fn run_tmux<E: CardEffects>(
    effects: &E,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, StateFault> {
    effects
        .tmux(args, timeout)
        .await
        .map_err(|e| StateFault::from_tmux(&e))
}

/// `pane_status_hint(pane, agent) == 'working'` (6171).
async fn codex_working<E: CardEffects>(
    effects: &E,
    pane: &str,
    agent: Option<&str>,
) -> Result<bool, StateFault> {
    if pane.is_empty() || agent != Some("codex") {
        return Ok(false);
    }
    let out = run_tmux(
        effects,
        &["capture-pane", "-p", "-t", pane, "-S", "-12"],
        HINT_TIMEOUT,
    )
    .await?;
    Ok(out.ok && search(&WORKING, &out.stdout)?)
}

/// `bool(claude_pane_busy(pane))` (3457).
async fn claude_busy<E: CardEffects>(effects: &E, pane: &str) -> Result<bool, StateFault> {
    let out = run_tmux(
        effects,
        &["capture-pane", "-p", "-t", pane, "-S", "-14"],
        TMUX_TIMEOUT,
    )
    .await?;
    if !out.ok {
        return Ok(false);
    }
    let tail = out.stdout.trim_end_matches('\n');
    if search(&INTERRUPT, tail)? {
        return Ok(true);
    }
    if search(&QUESTION, tail)? && search(&CONFIRM, tail)? {
        return Ok(true);
    }
    Ok(!tail.contains('❯') && !tail.contains('>'))
}

/// `SSH_HOST_RE = ^[A-Za-z0-9._-]{1,60}\Z` (7341).
fn ssh_host(host: &str) -> bool {
    (1..=60).contains(&host.len())
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `ssh_state(sess)` (7708): `ssh`, `mux` o `""`.
async fn ssh_state<E: CardEffects>(effects: &E, session: &str) -> Result<&'static str, StateFault> {
    let target = format!("={session}");
    let out = run_tmux(
        effects,
        &[
            "list-panes",
            "-s",
            "-t",
            &target,
            "-F",
            "#{pane_current_command}",
        ],
        TMUX_TIMEOUT,
    )
    .await?;
    // `"ssh" in stdout.split()`: igualdad de palabra (`sshd` no cuenta).
    if out.ok && out.stdout.split(is_space).any(|word| word == "ssh") {
        return Ok("ssh");
    }
    let host = session.get(4..).unwrap_or("");
    if ssh_host(host) && effects.ssh_check(host).await {
        return Ok("mux");
    }
    Ok("")
}

fn basename_of(cwd: &str) -> &str {
    let trimmed = cwd.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

fn insert(item: &mut Map<String, Value>, key: &str, value: Value) {
    item.insert(key.to_owned(), value);
}

/// Una tarjeta viva; `None` si la sesión oculta no tiene agente.
async fn live_card<E: CardEffects>(
    inputs: &Inputs,
    annotations: &Annotations<'_>,
    counts: &HashMap<&str, usize>,
    row: &PaneRow,
    effects: &E,
) -> Result<Option<Map<String, Value>>, StateFault> {
    let (session, pane) = (row.session.as_str(), row.pane.as_str());
    let info = inputs
        .maps
        .by_cwd
        .iter()
        .flat_map(|(_, rows)| rows.iter())
        .find(|info| info.session == session && info.pane == pane);
    let agent = info.map(|i| i.agent.as_str());
    let has_agent = agent.is_some_and(|a| !a.is_empty());
    if HIDDEN_SESSIONS.contains(&session) && session != "local" && !has_agent {
        return Ok(None);
    }
    let empty = Map::new();
    let record = annotations
        .by_pane
        .get(&(row.session.clone(), row.pane.clone()))
        .map_or(&empty, |(_, record)| &record.value);
    let cwd = match info.map(|i| i.cwd.as_str()).filter(|c| !c.is_empty()) {
        Some(c) => c.to_owned(),
        None => row.cwd.clone(),
    };
    let base = if session == "local" {
        Value::from("⌂ local")
    } else if let Some(label) = inputs.labels.get(session).filter(|l| !l.is_empty()) {
        Value::from(label.as_str())
    } else if let Some(project) = record.get("project").filter(|v| truthy(v)) {
        project.clone()
    } else if !basename_of(&cwd).is_empty() {
        Value::from(basename_of(&cwd))
    } else {
        Value::from(session)
    };
    let split = counts.get(session).copied().unwrap_or(0) > 1;
    let label = if split {
        let tail: String = pane.chars().skip(1).collect();
        Value::from(format!("{} ⫽{tail}", py_str(&base)?))
    } else {
        base
    };
    let mut status = or_default(record.get("status"), "idle");
    if is_text(Some(&status), "dead") {
        status = Value::from("idle");
    }
    let mut detail = or_default(record.get("detail"), "");
    if codex_working(effects, pane, agent).await? && !is_text(Some(&status), "waiting") {
        status = Value::from("working");
        detail = Value::from("");
    }
    if record.is_empty() && agent == Some("claude") && claude_busy(effects, pane).await? {
        status = Value::from("working");
    }
    let ts = match record.get("ts").filter(|v| truthy(v)) {
        Some(ts) => ts.clone(),
        None if row.activity != 0.0 => {
            if !row.activity.is_finite() {
                return Err(StateFault::Decline);
            }
            float_value(row.activity)
        }
        None => Value::from(0),
    };
    let mut item = Map::new();
    insert(&mut item, "project", label);
    insert(&mut item, "session", Value::from(session));
    insert(&mut item, "pane", Value::from(pane));
    insert(&mut item, "agent", agent.map_or(Value::Null, Value::from));
    insert(&mut item, "status", status);
    insert(&mut item, "detail", detail);
    insert(&mut item, "cwd", Value::from(cwd));
    insert(&mut item, "options", or_default(record.get("options"), ""));
    insert(&mut item, "last", or_default(record.get("last"), ""));
    insert(&mut item, "ts", ts);
    insert(&mut item, "alive", Value::Bool(true));
    insert(&mut item, "external", Value::Bool(false));
    insert(
        &mut item,
        "tabbed",
        Value::Bool(inputs.labels.contains_key(session)),
    );
    insert(&mut item, "split", Value::Bool(split));
    insert(&mut item, "paneActive", Value::Bool(row.pane_active));
    match info.filter(|i| i.pid != 0) {
        Some(info) => {
            for (key, value) in effects.account(info.pid, &info.agent).await? {
                item.insert(key, value);
            }
            if info.agent == "grok" {
                let metadata = effects.grok_metadata(info.pid).await?;
                insert(
                    &mut item,
                    "grokTitle",
                    or_default(metadata.get("title"), ""),
                );
            }
            let observed = effects.observe(session, pane, info).await?;
            let config = effects.session_config(session, pane).await?;
            observe::reconcile(&mut item, &observed, config)?;
        }
        None => {
            insert(&mut item, "model", Value::from(""));
            insert(&mut item, "effort", Value::from(""));
            insert(&mut item, "motor", Value::from(""));
            insert(&mut item, "harnessAccount", Value::from("unknown"));
            insert(&mut item, "motorAccount", Value::from("unknown"));
            insert(&mut item, "configConfirmed", Value::Bool(false));
            insert(&mut item, "modelSource", Value::from("unconfirmed"));
        }
    }
    if session.starts_with("ssh-") {
        let state = ssh_state(effects, session).await?;
        insert(&mut item, "sshConnected", Value::Bool(!state.is_empty()));
        insert(&mut item, "sshMux", Value::Bool(state == "mux"));
    }
    Ok(Some(item))
}

/// Las tarjetas históricas: historial desacoplado y procesos externos.
fn historical(inputs: &Inputs, annotations: &Annotations<'_>) -> Result<Vec<Value>, StateFault> {
    // `now_ts = int(time.time())`.
    let now_ts = inputs.now.trunc();
    let mut order: Vec<PaneKey> = Vec::new();
    let mut cards: HashMap<PaneKey, (f64, Map<String, Value>)> = HashMap::new();
    for (session, record) in &annotations.detached {
        let value = &record.value;
        let timestamp = record.timestamp;
        let cwd = or_default(value.get("cwd"), "");
        let agent = or_default(value.get("agent"), "claude");
        // `(cwd, agent) in external_agents`: hashear un contenedor es `TypeError`.
        let external = match (&cwd, &agent) {
            (Value::Array(_) | Value::Object(_), _) | (_, Value::Array(_) | Value::Object(_)) => {
                return Err(StateFault::Failure);
            }
            (Value::String(c), Value::String(a)) => {
                inputs.external.contains(&(c.clone(), a.clone()))
            }
            _ => false,
        };
        let mut status = or_default(value.get("status"), "idle");
        let recent_done = is_text(Some(&status), "done") && now_ts - timestamp <= DONE_GRACE_SECS;
        if is_text(Some(&status), "waiting")
            && !external
            && !inputs.labels.contains_key(session)
            && now_ts - timestamp > ZOMBIE_WAITING_GRACE_SECS
        {
            status = Value::from("dead");
        }
        if !external && !is_text(Some(&status), "waiting") && !recent_done {
            status = Value::from("dead");
        }
        // `str(record.get('pane') or '')` llega a la salida (`previousPane`).
        let pane = super::str_or_empty(value.get("pane"))?;
        let key = (session.clone(), pane.clone());
        if cards.get(&key).is_some_and(|(ts, _)| *ts >= timestamp) {
            continue;
        }
        let zombie = is_text(Some(&status), "waiting") && !external;
        let mut item = Map::new();
        insert(
            &mut item,
            "project",
            or_default(value.get("project"), session),
        );
        insert(&mut item, "session", Value::from(session.as_str()));
        insert(&mut item, "agent", agent);
        insert(&mut item, "status", status);
        insert(&mut item, "detail", or_default(value.get("detail"), ""));
        insert(&mut item, "cwd", cwd);
        insert(&mut item, "options", or_default(value.get("options"), ""));
        insert(&mut item, "last", or_default(value.get("last"), ""));
        insert(&mut item, "ts", float_value(timestamp));
        insert(&mut item, "alive", Value::Bool(false));
        insert(&mut item, "external", Value::Bool(external));
        insert(
            &mut item,
            "tabbed",
            Value::Bool(inputs.labels.contains_key(session)),
        );
        insert(&mut item, "historical", Value::Bool(true));
        insert(&mut item, "operable", Value::Bool(false));
        insert(&mut item, "previousPane", Value::from(pane));
        insert(&mut item, "zombie", Value::Bool(zombie));
        // Reasignar una clave del `dict` conserva su posición.
        if !cards.contains_key(&key) {
            order.push(key.clone());
        }
        cards.insert(key, (timestamp, item));
    }
    Ok(order
        .into_iter()
        .filter_map(|key| cards.remove(&key))
        .map(|(_, item)| Value::Object(item))
        .collect())
}

/// Tarjetas vivas (en el orden de `panes`) y luego históricas, sin ordenar ni
/// sugerencias. Secuencial: el primer error es el del Python (D3).
pub async fn build<E: CardEffects + Sync>(
    inputs: &Inputs,
    effects: &E,
) -> Result<Vec<Value>, StateFault> {
    let annotations = annotate(inputs)?;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for row in &inputs.panes {
        *counts.entry(row.session.as_str()).or_default() += 1;
    }
    let mut items = Vec::new();
    for row in &inputs.panes {
        if let Some(item) = live_card(inputs, &annotations, &counts, row, effects).await? {
            items.push(Value::Object(item));
        }
    }
    items.extend(historical(inputs, &annotations)?);
    Ok(items)
}

/// `items.sort(key=lambda item: (not alive, 0 if agent else 1, -float(ts or 0)))`
/// (estable). `float()` imposible es la excepción sin capturar del Python.
pub fn sort_items(items: &mut [Value]) -> Result<(), StateFault> {
    let mut keyed = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let get = |key: &str| item.get(key).filter(|v| truthy(v));
        let ts = match get("ts") {
            None => 0.0,
            Some(v) => match py_float(v) {
                PyFloat::Value(f) if !f.is_nan() => f,
                PyFloat::Value(_) | PyFloat::Unsure => return Err(StateFault::Decline),
                PyFloat::Raises => return Err(StateFault::Failure),
            },
        };
        keyed.push((
            get("alive").is_none(),
            u8::from(get("agent").is_none()),
            -ts,
            index,
        ));
    }
    keyed.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.partial_cmp(&b.2).unwrap_or(Ordering::Equal))
    });
    let sorted: Vec<Value> = keyed
        .iter()
        .filter_map(|(_, _, _, index)| items.get(*index).cloned())
        .collect();
    for (slot, item) in items.iter_mut().zip(sorted) {
        *slot = item;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_patterns_compile_and_match_like_python() {
        assert!(search(&WORKING, "  Working (12s • esc)\n").unwrap());
        assert!(search(&WORKING, "Working(").unwrap());
        assert!(!search(&WORKING, "XWorking (").unwrap());
        assert!(search(&WORKING, "é Working\u{1f}(").unwrap());
        assert!(search(&INTERRUPT, "● x\n ESC TO INTERRUPT").unwrap());
        assert!(search(&QUESTION, "hola\n  Allow write?").unwrap());
        assert!(!search(&QUESTION, "hola Allow write?").unwrap());
        assert!(search(&CONFIRM, "❯ 1. Yes").unwrap());
        assert_eq!(basename_of("/proj/app/"), "app");
        assert_eq!(basename_of("/"), "");
        assert_eq!(basename_of("rel"), "rel");
        assert!(ssh_host("box.local"));
        assert!(!ssh_host("a b"));
    }
}
