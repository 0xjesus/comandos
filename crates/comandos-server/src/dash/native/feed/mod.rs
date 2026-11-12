//! GET `/session-feed` y POST `/session-seen` (ficha viva,
//! `design/ficha-viva-spec.md` §1). Rutas nuevas sin paridad con el Python:
//! se apoyan en las tarjetas ya cacheadas de `/state`, leen la pantalla de
//! cada pane con agente (`capture-pane -p`, solo lectura) y la cola del
//! transcript del pane elegido de cada sesión.
pub mod prompt;
pub mod screen;

use super::{
    Answer, Entry, Fault, Key, Native, NativeRoute, Verb, files, py, reply,
    states::tab_models::short_model,
};
use crate::{HandlerError, Request};
use futures_util::{StreamExt, stream};
use http::StatusCode;
use prompt::{Kind, Prompts};
use screen::Screen;
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedRoute {
    Feed,
    Seen,
}

pub const ROUTES: &[Entry] = &[
    Entry {
        verb: Verb::Get,
        key: Key::Path("/session-feed"),
        route: NativeRoute::Feed(FeedRoute::Feed),
    },
    Entry {
        verb: Verb::Post,
        key: Key::Path("/session-seen"),
        route: NativeRoute::Feed(FeedRoute::Seen),
    },
];

/// `~/.claude/hooks/session-seen.json`: `{sesión: segundos}`.
pub const SEEN_FILE: &str = "session-seen.json";
/// Capturas y transcripts se reutilizan durante este plazo.
const TTL: Duration = Duration::from_millis(1500);
/// `capture-pane` simultáneos contra el servidor de tmux.
const CAPTURES_AT_ONCE: usize = 8;
/// Sesiones que nunca salen en el feed.
const HIDDEN: [&str; 2] = ["hub", "control"];
const MAX_REPLY: usize = 1200;
/// Tope del texto de cada pane en `paneList` (una tarjeta muestra unas líneas).
const MAX_PANE_REPLY: usize = 480;
/// Marcas de «visto» que se guardan (las más recientes).
const MAX_SEEN: usize = 512;

/// Estado de un pane o de una sesión; el orden es la urgencia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    Idle,
    Work,
    Limit,
    Ask,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Work => "work",
            State::Limit => "limit",
            State::Ask => "ask",
        }
    }
}

/// Una sesión del feed sin su parte de «visto» (que cambia sin recalcular).
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub session: String,
    pub label: String,
    pub state: State,
    pub ts: f64,
    pub pane: String,
    pub panes: usize,
    pub agent: String,
    pub model: String,
    pub prompt: String,
    pub reply: String,
    pub ask: Value,
    /// Cuenta e id de conversación del pane elegido, para leer su prompt.
    pub account: String,
    pub sid: String,
    /// Every agent pane of the session, in tmux order (the desktop focus shelf).
    pub pane_list: Vec<PaneDraft>,
}

/// One agent pane of a session: what its card in the shelf shows.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneDraft {
    pub pane: String,
    pub state: State,
    pub ts: f64,
    pub agent: String,
    pub model: String,
    pub account: String,
    pub reply: String,
}

/// Estado del feed en el frente: el último cálculo y su vuelo único.
#[derive(Default)]
pub struct FeedState {
    last: Mutex<Option<(Instant, Arc<Vec<Draft>>)>>,
    flight: tokio::sync::Mutex<()>,
    prompts: Arc<Mutex<Prompts>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

impl FeedState {
    fn fresh(&self) -> Option<Arc<Vec<Draft>>> {
        lock(&self.last)
            .as_ref()
            .filter(|(at, _)| at.elapsed() < TTL)
            .map(|(_, drafts)| drafts.clone())
    }

    fn stale(&self) -> Option<Arc<Vec<Draft>>> {
        lock(&self.last).as_ref().map(|(_, drafts)| drafts.clone())
    }
}

fn text<'a>(item: &'a Map<String, Value>, key: &str) -> &'a str {
    item.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Un pane vivo con agente de una sesión visible, con id de pane válido.
fn agent_pane(item: &Map<String, Value>) -> bool {
    item.get("alive") == Some(&Value::Bool(true))
        && !text(item, "agent").is_empty()
        && py::is_session(text(item, "session"))
        && !HIDDEN.contains(&text(item, "session"))
        && py::is_pane(text(item, "pane")) == Some(true)
}

/// Los panes cuya pantalla hay que leer.
pub fn agent_panes(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(Value::as_object)
        .filter(|item| agent_pane(item))
        .map(|item| text(item, "pane").to_owned())
        .collect()
}

/// El nombre de la pestaña sin el ` ⫽NN` que `/state` añade a los splits.
fn label(item: &Map<String, Value>) -> String {
    let session = text(item, "session");
    let project = item
        .get("project")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .unwrap_or(session);
    let pane = text(item, "pane");
    if item.get("split") == Some(&Value::Bool(true))
        && let Some(base) = project.strip_suffix(&format!(" ⫽{}", pane.trim_start_matches('%')))
    {
        return base.to_owned();
    }
    project.to_owned()
}

/// Texto del agente sin markdown pesado: `**`, `` ` ``, `#` de títulos y
/// tablas como texto; espacios colapsados y a lo sumo `max` caracteres.
pub fn plain(text: &str, max: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('|') {
            if t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) {
                continue;
            }
            let cells: Vec<&str> = t
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .collect();
            lines.push(cells.join(" · "));
            continue;
        }
        let heading = t.trim_start_matches('#');
        if heading.len() < t.len() && (heading.is_empty() || heading.starts_with(' ')) {
            lines.push(heading.trim().to_owned());
        } else {
            lines.push(t.to_owned());
        }
    }
    let joined = lines.join("\n").replace("**", "").replace('`', "");
    screen::clip(&prompt::collapse(&joined), max)
}

/// Lo último que dijo el agente de un pane (el detalle, si lo hay).
fn said_of<'a>(item: &'a Map<String, Value>) -> &'a str {
    let detail = text(item, "detail");
    if detail.trim().is_empty() {
        text(item, "last")
    } else {
        detail
    }
}

/// El estado de un pane: menú en pantalla > límite > trabajando (hook o
/// pantalla) > quieto.
fn pane_state(item: &Map<String, Value>, screen: Option<&Screen>) -> (State, Value) {
    if let Some(ask) = screen.and_then(|s| s.ask.clone()) {
        return (State::Ask, ask);
    }
    if screen.is_some_and(|s| s.limit) {
        return (State::Limit, Value::Null);
    }
    if text(item, "status") == "working" || screen.is_some_and(|s| s.work) {
        return (State::Work, Value::Null);
    }
    (State::Idle, Value::Null)
}

/// Un pane con agente ya leído: su tarjeta, estado, menú y `ts`.
type PaneRead<'a> = (&'a Map<String, Value>, State, Value, f64);

/// Una entrada por sesión con agentes, en orden de `ts` (más reciente
/// primero). El pane elegido es el más urgente y, a igualdad, el más reciente.
pub fn assemble(items: &[Value], screens: &HashMap<String, Screen>) -> Vec<Draft> {
    let mut sessions: Vec<(String, Vec<PaneRead<'_>>)> = Vec::new();
    for item in items.iter().filter_map(Value::as_object) {
        if !agent_pane(item) {
            continue;
        }
        let (state, ask) = pane_state(item, screens.get(text(item, "pane")));
        let ts = item.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
        let session = text(item, "session");
        let index = match sessions.iter().position(|(s, _)| s == session) {
            Some(index) => index,
            None => {
                sessions.push((session.to_owned(), Vec::new()));
                sessions.len() - 1
            }
        };
        sessions[index].1.push((item, state, ask, ts));
    }
    let mut drafts: Vec<Draft> = sessions
        .into_iter()
        .filter_map(|(session, panes)| {
            let count = panes.len();
            let pane_list = panes
                .iter()
                .map(|(item, state, _, ts)| PaneDraft {
                    pane: text(item, "pane").to_owned(),
                    state: *state,
                    ts: *ts,
                    agent: text(item, "agent").to_owned(),
                    model: short_model(text(item, "model")),
                    account: text(item, "account").to_owned(),
                    reply: plain(said_of(item), MAX_PANE_REPLY),
                })
                .collect();
            let (item, state, ask, ts) = panes.into_iter().max_by(|a, b| {
                a.1.cmp(&b.1)
                    .then(a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal))
            })?;
            let said = said_of(item);
            Some(Draft {
                label: label(item),
                session,
                state,
                ts,
                pane: text(item, "pane").to_owned(),
                panes: count,
                agent: text(item, "agent").to_owned(),
                model: short_model(text(item, "model")),
                prompt: String::new(),
                reply: plain(said, MAX_REPLY),
                ask,
                account: text(item, "account").to_owned(),
                sid: text(item, "agentSessionId").to_owned(),
                pane_list,
            })
        })
        .collect();
    drafts.sort_by(|a, b| b.ts.partial_cmp(&a.ts).unwrap_or(std::cmp::Ordering::Equal));
    drafts
}

/// `ask` y `limit` siempre cuentan como no leídos; si no, hay respuesta nueva
/// del agente después de la última vez que se abrió la sesión.
pub fn unread(draft: &Draft, seen: f64) -> bool {
    matches!(draft.state, State::Ask | State::Limit)
        || (draft.state != State::Work && draft.ts > seen && !draft.reply.is_empty())
}

/// El cuerpo de GET `/session-feed`.
pub fn feed_json(now: f64, drafts: &[Draft], seen: &HashMap<String, f64>) -> Value {
    let sessions: Vec<Value> = drafts
        .iter()
        .map(|d| {
            let seen_at = seen.get(&d.session).copied().unwrap_or(0.0);
            json!({
                "session": d.session,
                "label": d.label,
                "state": d.state.name(),
                "unread": unread(d, seen_at),
                "ts": d.ts,
                "seen": seen_at,
                "pane": d.pane,
                "panes": d.panes,
                "agent": d.agent,
                "model": d.model,
                "prompt": d.prompt,
                "reply": d.reply,
                "ask": d.ask,
                "paneList": d.pane_list.iter().map(|p| json!({
                    "pane": p.pane,
                    "state": p.state.name(),
                    "ts": p.ts,
                    "agent": p.agent,
                    "model": p.model,
                    "account": p.account,
                    "reply": p.reply,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({"now": now, "sessions": sessions})
}

/// Las marcas guardadas; un archivo ausente o roto es «nunca visto».
pub fn read_seen(path: &Path) -> HashMap<String, f64> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .map(|map| {
            map.into_iter()
                .filter_map(|(session, at)| at.as_f64().map(|at| (session, at)))
                .collect()
        })
        .unwrap_or_default()
}

/// `seen[session] = now`, conservando las `MAX_SEEN` más recientes. Con el
/// candado de `<archivo>.lock` ya tomado; escritura atómica.
pub fn mark_seen(path: &Path, session: &str, now: f64) -> std::io::Result<()> {
    let mut marks: Vec<(String, f64)> = read_seen(path).into_iter().collect();
    marks.retain(|(s, _)| s != session);
    marks.push((session.to_owned(), now));
    marks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    marks.truncate(MAX_SEEN);
    marks.sort_by(|a, b| a.0.cmp(&b.0));
    let map: Map<String, Value> = marks
        .into_iter()
        .map(|(s, at)| (s, Value::from(at)))
        .collect();
    files::write_json_atomic(path, &Value::Object(map))
}

pub async fn answer(native: &Arc<Native>, route: FeedRoute, request: &Request) -> Answer {
    match route {
        FeedRoute::Feed => feed(native).await,
        FeedRoute::Seen => seen(native, request).await,
    }
}

/// Lee las pantallas de los panes (solo lectura): todas en un lote de tmux
/// y, las que el lote no trajo, una a una de a `CAPTURES_AT_ONCE`.
async fn screens(native: &Native, panes: Vec<String>) -> HashMap<String, Screen> {
    let tmux = native.options().tmux.clone();
    let commands: Vec<Vec<String>> = panes
        .iter()
        .map(|pane| {
            ["capture-pane", "-p", "-t", pane, "-S", "-14"]
                .map(str::to_owned)
                .to_vec()
        })
        .collect();
    let batch = tmux.run_many(&commands).await.unwrap_or_default();
    let mut found = HashMap::new();
    let mut missing = Vec::new();
    for (i, pane) in panes.into_iter().enumerate() {
        match batch.get(i).and_then(Option::as_ref) {
            Some(out) => {
                found.insert(pane, screen::read(&out.stdout));
            }
            None => missing.push(pane),
        }
    }
    let rest: Vec<(String, Screen)> = stream::iter(missing)
        .map(|pane| {
            let tmux = tmux.clone();
            async move {
                let out = tmux
                    .run(&["capture-pane", "-p", "-t", &pane, "-S", "-14"])
                    .await;
                match out {
                    Ok(out) if out.ok => Some((pane, screen::read(&out.stdout))),
                    _ => None,
                }
            }
        })
        .buffer_unordered(CAPTURES_AT_ONCE)
        .filter_map(|found| async move { found })
        .collect()
        .await;
    found.extend(rest);
    found
}

/// El feed sin «visto», recalculado como mucho cada `TTL`.
async fn drafts(native: &Native) -> Result<Arc<Vec<Draft>>, Fault> {
    let feed = &native.feed;
    if let Some(hit) = feed.fresh() {
        return Ok(hit);
    }
    let _flight = feed.flight.lock().await;
    if let Some(hit) = feed.fresh() {
        return Ok(hit);
    }
    let states = match native.states_cached().await {
        Ok(states) => states,
        // Sin tarjetas nuevas, el último feed vale más que un error.
        Err(fault) => return feed.stale().ok_or(fault),
    };
    let items = states.items.clone();
    let screens = screens(native, agent_panes(&items)).await;
    let mut drafts = assemble(&items, &screens);
    let prompts = feed.prompts.clone();
    let home = native.options().home.clone();
    let codex_home = native.options().codex_home.clone();
    let drafts = tokio::task::spawn_blocking(move || {
        let mut prompts = lock(&prompts);
        for draft in &mut drafts {
            let Some(kind) = Kind::of(&draft.agent) else {
                continue;
            };
            if draft.sid.is_empty() {
                continue;
            }
            if let Some(root) =
                prompt::account_root(&home, codex_home.as_deref(), kind, &draft.account)
            {
                draft.prompt = screen::clip(
                    &prompts.prompt(&home, kind, &root, &draft.sid),
                    prompt::MAX_PROMPT,
                );
            }
        }
        drafts
    })
    .await
    .map_err(|_| Fault::Error(HandlerError::Failure))?;
    let drafts = Arc::new(drafts);
    *lock(&feed.last) = Some((Instant::now(), drafts.clone()));
    Ok(drafts)
}

async fn feed(native: &Native) -> Answer {
    let drafts = match drafts(native).await {
        Ok(drafts) => drafts,
        // No hay heredado que responda esta ruta: declinar sería un 404.
        Err(Fault::Decline) => {
            return reply(
                StatusCode::SERVICE_UNAVAILABLE,
                &json!({"error": "Estado de sesiones no disponible"}),
            );
        }
        Err(fault) => return Err(fault),
    };
    let path = native.options().hooks.join(SEEN_FILE);
    let seen = tokio::task::spawn_blocking(move || read_seen(&path))
        .await
        .unwrap_or_default();
    let now = (native.options().clock_seconds)();
    reply(StatusCode::OK, &feed_json(now, &drafts, &seen))
}

async fn seen(native: &Native, request: &Request) -> Answer {
    let session = request
        .data
        .as_ref()
        .and_then(|data| data.get("session"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    if !py::is_session(&session) {
        return reply(
            StatusCode::BAD_REQUEST,
            &json!({"error": "Sesión inválida"}),
        );
    }
    let path = native.options().hooks.join(SEEN_FILE);
    let Ok(guard) = files::FileLock::acquire_timeout(&path, files::LOCK_WAIT).await else {
        return reply(
            StatusCode::SERVICE_UNAVAILABLE,
            &json!({"error": "Marcas de sesión ocupadas"}),
        );
    };
    let now = (native.options().clock_seconds)();
    let written = tokio::task::spawn_blocking(move || {
        let _guard = guard;
        mark_seen(&path, &session, now)
    })
    .await;
    match written {
        Ok(Ok(())) => reply(StatusCode::OK, &json!({"ok": true})),
        _ => reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            &json!({"error": "No se pudo guardar la marca"}),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(session: &str, pane: &str, status: &str, ts: f64) -> Value {
        json!({
            "project": "Proyecto ⫽".to_owned() + pane.trim_start_matches('%'),
            "session": session,
            "pane": pane,
            "agent": "claude",
            "status": status,
            "detail": "",
            "last": format!("**Listo** en `{pane}`"),
            "ts": ts,
            "alive": true,
            "split": true,
            "model": "claude-opus-5-5",
            "account": "main",
            "agentSessionId": format!("sid{}", pane.trim_start_matches('%')),
        })
    }

    fn ask_screen() -> Screen {
        screen::read("Do you want to proceed?\n❯ 1. Yes\n  2. No\n")
    }

    #[test]
    fn session_takes_its_most_urgent_pane() {
        let items = vec![
            card("term-a", "%1", "idle", 300.0),
            card("term-a", "%2", "idle", 100.0),
            card("term-a", "%3", "working", 200.0),
            card("term-b", "%4", "working", 50.0),
            card("term-b", "%5", "done", 60.0),
            card("term-c", "%6", "done", 10.0),
            card("term-c", "%7", "waiting", 20.0),
        ];
        let mut screens = HashMap::new();
        screens.insert("%2".to_owned(), ask_screen());
        screens.insert(
            "%6".to_owned(),
            screen::read("You've hit your usage limit. Try again at 3pm"),
        );
        let drafts = assemble(&items, &screens);
        let by: HashMap<&str, &Draft> = drafts.iter().map(|d| (d.session.as_str(), d)).collect();
        // Pregunta > trabajando > más reciente.
        assert_eq!(by["term-a"].pane, "%2");
        assert_eq!(by["term-a"].state, State::Ask);
        assert_eq!(by["term-a"].panes, 3);
        assert_eq!(by["term-a"].ask["options"][1]["label"], "No");
        assert_eq!(by["term-a"].label, "Proyecto");
        assert_eq!(by["term-a"].model, "opus-5");
        assert_eq!(by["term-a"].reply, "Listo en %2");
        assert_eq!(by["term-a"].sid, "sid2");
        // Trabajando gana a un `done` más reciente.
        assert_eq!(
            (by["term-b"].pane.as_str(), by["term-b"].state),
            ("%4", State::Work)
        );
        // El límite en pantalla gana a un `waiting` del hook sin menú.
        assert_eq!(
            (by["term-c"].pane.as_str(), by["term-c"].state),
            ("%6", State::Limit)
        );
        assert_eq!(by["term-c"].ask, Value::Null);
        // Orden de `ts`, más reciente primero.
        let order: Vec<&str> = drafts.iter().map(|d| d.session.as_str()).collect();
        assert_eq!(order, ["term-a", "term-b", "term-c"]);
    }

    #[test]
    fn hidden_dead_and_agentless_panes_are_left_out() {
        let mut dead = card("term-x", "%9", "idle", 1.0);
        dead["alive"] = json!(false);
        let mut shell = card("term-y", "%10", "idle", 1.0);
        shell["agent"] = Value::Null;
        let mut local = card("local", "%11", "idle", 1.0);
        local["project"] = json!("⌂ local");
        local["split"] = json!(false);
        let items = vec![
            card("hub", "%8", "idle", 1.0),
            card("control", "%12", "idle", 1.0),
            dead,
            shell,
            local,
        ];
        let drafts = assemble(&items, &HashMap::new());
        assert_eq!(drafts.len(), 1);
        assert_eq!(
            (drafts[0].session.as_str(), drafts[0].label.as_str()),
            ("local", "⌂ local")
        );
        assert_eq!(agent_panes(&items), vec!["%11".to_owned()]);
    }

    #[test]
    fn reply_prefers_detail_and_drops_heavy_markdown() {
        let mut item = card("term-a", "%1", "waiting", 5.0);
        item["detail"] =
            json!("## Resumen\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n**Ojo**: usa `cargo`\u{1f}fin");
        let drafts = assemble(&[item], &HashMap::new());
        assert_eq!(drafts[0].reply, "Resumen a · b 1 · 2 Ojo: usa cargo fin");
        assert_eq!(
            plain(&"x".repeat(2000), MAX_REPLY).chars().count(),
            MAX_REPLY
        );
        assert_eq!(plain("#hashtag no es título", 50), "#hashtag no es título");
    }

    #[test]
    fn unread_and_seen() {
        let mut items = vec![card("term-a", "%1", "idle", 100.0)];
        let drafts = assemble(&items, &HashMap::new());
        let d = &drafts[0];
        assert!(unread(d, 0.0));
        assert!(unread(d, 99.0));
        assert!(!unread(d, 100.0), "visto justo en la respuesta");
        assert!(!unread(d, 150.0));
        // Trabajando nunca es «sin leer»; sin respuesta tampoco.
        items[0]["status"] = json!("working");
        assert!(!unread(&assemble(&items, &HashMap::new())[0], 0.0));
        items[0]["status"] = json!("idle");
        items[0]["last"] = json!("");
        assert!(!unread(&assemble(&items, &HashMap::new())[0], 0.0));
        // Te espera: siempre cuenta.
        let mut screens = HashMap::new();
        screens.insert("%1".to_owned(), ask_screen());
        assert!(unread(&assemble(&items, &screens)[0], 1e12));
        let body = feed_json(
            7.5,
            &assemble(&items, &screens),
            &HashMap::from([("term-a".to_owned(), 3.0)]),
        );
        let entry = &body["sessions"][0];
        assert_eq!(body["now"], json!(7.5));
        assert_eq!(entry["seen"], json!(3.0));
        assert_eq!(entry["state"], "ask");
        assert_eq!(entry["unread"], true);
        let keys: Vec<&str> = entry
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "session", "label", "state", "unread", "ts", "seen", "pane", "panes", "agent",
                "model", "prompt", "reply", "ask", "paneList"
            ]
        );
        let panes = entry["paneList"].as_array().unwrap();
        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0]["pane"], "%1");
        assert_eq!(panes[0]["state"], "ask");
    }

    #[test]
    fn seen_marks_are_stored_atomically_and_bounded() {
        let dir = std::env::temp_dir().join(format!("cmd-feed-seen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(SEEN_FILE);
        assert!(read_seen(&path).is_empty());
        mark_seen(&path, "term-a", 10.0).unwrap();
        mark_seen(&path, "term-b", 20.5).unwrap();
        mark_seen(&path, "term-a", 30.0).unwrap();
        let seen = read_seen(&path);
        assert_eq!(seen.get("term-a"), Some(&30.0));
        assert_eq!(seen.get("term-b"), Some(&20.5));
        for n in 0..MAX_SEEN + 5 {
            mark_seen(&path, &format!("s{n}"), 100.0 + n as f64).unwrap();
        }
        let seen = read_seen(&path);
        assert_eq!(seen.len(), MAX_SEEN);
        assert!(!seen.contains_key("term-a"), "las más viejas salen primero");
        std::fs::write(&path, b"{roto").unwrap();
        assert!(read_seen(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
