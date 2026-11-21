//! G. POST /pane/type (9590): teclea texto literal en un pane exacto, letra a
//! letra y sin Enter, en los panes vivos del usuario.
//!
//! La caché de 256 respuestas por `requestId` y los candados por pane son del
//! frente. Para que un reintento nunca teclee dos veces:
//! - la caché se consulta antes que la resolución de sesión (el Python la mira
//!   después; con la misma petición el resultado es el mismo);
//! - un `requestId` que el frente declinó queda marcado y se declina siempre
//!   después, para que el reintento lo responda la caché del Python;
//! - el tecleo, el candado y la caché viven en el hilo de bloqueo: si la
//!   petición se suelta a medias, el tecleo termina y su respuesta se guarda;
//! - un `requestId` que el frente tecleó se responde de su caché aunque el
//!   conjunto nativo se apague después (`retry_reply`);
//! - la respuesta entra en la caché ANTES de soltar el candado, y quien toma
//!   el candado vuelve a mirar caché y marcas: dos peticiones con el mismo
//!   `requestId` nunca teclean las dos.
//!
//! Límites conocidos:
//! - un reinicio del frente pierde la caché y las marcas: el reintento de un
//!   `requestId` que tecleó el Python lo teclearía Rust si el estado de hooks
//!   ya no nombra esa sesión (raro; el único llamador, la barra de comandos,
//!   no reintenta);
//! - los candados por pane de Rust y del Python son independientes: una
//!   petición declinada y otra nativa al mismo pane no se esperan entre sí.
use super::{
    Answer, Entry, Fault, Key, Native, NativeRoute, Verb,
    light::{data, error, tmux_sessions},
    py, reply,
    tmux::Tmux,
};
use crate::{HandlerError, Reply, Request};
use comandos_core::json::truthy;
use comandos_runtime::pane_typing::{self, PaneTypingLocks, TmuxResult, TypingOptions};
use http::StatusCode;
use serde_json::{Map, Value, json};
use std::{
    collections::{HashSet, VecDeque},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::runtime::Handle;

pub const ROUTES: &[Entry] = &[Entry {
    verb: Verb::Post,
    key: Key::Raw("/pane/type"),
    route: NativeRoute::PaneType,
}];

/// `_PANE_TYPING_RESULTS` del Python: 256 entradas, FIFO.
const RESULTS: usize = 256;

/// Marcas de `requestId` declinados. Mucho mayor que la caché del Python a
/// propósito: esa caché solo crece con lo que el frente le reenvía, así que
/// una marca nunca debe caducar antes de que el Python olvide su respuesta.
const DECLINED: usize = 16_384;

const BUSY: &str = "Ya se está escribiendo en ese pane; espera a que termine.";

#[derive(Default)]
pub struct TypingState {
    /// También los toma `/session/message`: nunca escriben los dos a la vez.
    pub(super) locks: PaneTypingLocks,
    /// requestId → (status, cuerpo), en orden de llegada.
    results: Mutex<VecDeque<(String, u16, Value)>>,
    /// requestId que atendió el Python.
    declined: Mutex<Marks>,
}

/// Conjunto con desalojo FIFO: búsqueda en el `HashSet`, orden en la cola.
#[derive(Default)]
struct Marks {
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl TypingState {
    fn cached(&self, rid: &str) -> Option<(u16, Value)> {
        let results = self.results.lock().unwrap_or_else(|p| p.into_inner());
        results
            .iter()
            .find(|(key, _, _)| key == rid)
            .map(|(_, status, body)| (*status, body.clone()))
    }

    /// `results[rid] = res` + `popitem(last=False)` mientras pase de 256.
    /// Reasignar una clave de un `OrderedDict` no la mueve al final.
    fn remember(&self, rid: &str, status: u16, body: &Value) {
        let mut results = self.results.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(slot) = results.iter_mut().find(|(key, _, _)| key == rid) {
            slot.1 = status;
            slot.2 = body.clone();
        } else {
            results.push_back((rid.to_owned(), status, body.clone()));
        }
        while results.len() > RESULTS {
            results.pop_front();
        }
    }

    fn was_declined(&self, rid: &str) -> bool {
        let declined = self.declined.lock().unwrap_or_else(|p| p.into_inner());
        declined.set.contains(rid)
    }

    /// Declina y, si hay `requestId`, lo marca para siempre.
    fn decline(&self, rid: &str) -> Fault {
        if !rid.is_empty() {
            let mut declined = self.declined.lock().unwrap_or_else(|p| p.into_inner());
            if declined.set.insert(rid.to_owned()) {
                declined.order.push_back(rid.to_owned());
            }
            while declined.order.len() > DECLINED {
                if let Some(old) = declined.order.pop_front() {
                    declined.set.remove(&old);
                }
            }
        }
        Fault::Decline
    }
}

/// Suelta el candado del pane pase lo que pase (también si el hilo se pierde).
struct PaneGuard {
    state: Arc<TypingState>,
    pane: String,
}

impl Drop for PaneGuard {
    fn drop(&mut self) {
        self.state.locks.release(&self.pane);
    }
}

fn failure() -> Fault {
    Fault::Error(HandlerError::Failure)
}

/// `str(x or "")`; lo que no tiene un `str()` seguro se declina.
fn text_or_empty(value: Option<&Value>) -> Result<String, Fault> {
    match value {
        Some(v) if truthy(v) => py::str_scalar(v).ok_or(Fault::Decline),
        _ => Ok(String::new()),
    }
}

/// `sess = data.get("session", "")` + `SESSION_RE.match(sess)`: un no-texto
/// es un `TypeError` (500); `Ok(None)` es el 400 de nombre inválido.
fn session_of(data: &Map<String, Value>) -> Result<Option<&str>, Fault> {
    match data.get("session") {
        None => Ok(None),
        Some(Value::String(s)) if py::is_session(s) => Ok(Some(s.as_str())),
        Some(Value::String(_)) => Ok(None),
        Some(_) => Err(failure()),
    }
}

/// `str(data.get("requestId") or "")[:64]`.
fn request_id(data: &Map<String, Value>) -> Result<String, Fault> {
    Ok(py::take_chars(&text_or_empty(data.get("requestId"))?, 64))
}

fn replay(status: u16, body: &Value) -> Answer {
    reply(StatusCode::from_u16(status).map_err(|_| failure())?, body)
}

/// Reintento de un `requestId` que el frente ya respondió, con el conjunto
/// nativo apagado: se responde de la caché en vez de reenviarlo (el Python
/// no lo conoce y volvería a teclear). `None`: se reenvía como siempre.
pub fn retry_reply(state: &TypingState, request: &Request) -> Option<Reply> {
    let data = data(request).ok()?;
    session_of(data).ok().flatten()?;
    let rid = request_id(data).ok()?;
    if rid.is_empty() || state.was_declined(&rid) {
        return None;
    }
    let (status, body) = state.cached(&rid)?;
    replay(status, &body).ok()
}

/// `resolve_project_session` (6183) hasta saber si ALGÚN estado nombra esta
/// sesión: si sí, el Python sigue con procesos (`agent_procs`) → se declina.
/// El bucle de registros es el de `target::first_state_record` (una sola
/// implementación); la resolución completa (`target::resolve_project_session`)
/// no se usa aquí para no cambiar la ruta viva: lo hará la tarea que porte
/// las rutas de sesión con su oráculo.
pub fn project_session_matches(state: &Path, sess: &str) -> Result<bool, Fault> {
    Ok(super::target::first_state_record(state, sess)?.is_some())
}

pub async fn answer(native: &Native, request: &Request) -> Answer {
    let data = data(request)?;
    let tmux = &native.options().tmux;
    let state = &native.typing;
    let Some(sess) = session_of(data)? else {
        return error(StatusCode::BAD_REQUEST, "Nombre de sesion invalido");
    };
    let rid = request_id(data)?;
    if !rid.is_empty() {
        if state.was_declined(&rid) {
            return Err(Fault::Decline);
        }
        if let Some((status, body)) = state.cached(&rid) {
            return replay(status, &body);
        }
    }
    // `resolve_project_session`: `tmux_sessions()` sin capturar, luego los estados.
    tmux_sessions(tmux).await?;
    let dir = native.options().hooks.join("state");
    let owned = sess.to_owned();
    let matches = tokio::task::spawn_blocking(move || project_session_matches(&dir, &owned))
        .await
        .map_err(|_| failure())?;
    match matches {
        Ok(false) => {}
        Ok(true) | Err(Fault::Decline) => return Err(state.decline(&rid)),
        Err(fault) => return Err(fault),
    }
    let want = match text_or_empty(data.get("pane")) {
        Ok(want) => want,
        Err(_) => return Err(state.decline(&rid)),
    };
    let mut pane = format!("={sess}:");
    match py::is_pane(&want) {
        // `\d` de Python acepta dígitos Unicode.
        None => return Err(state.decline(&rid)),
        Some(true) => {
            let out = tmux
                .run(&["display-message", "-p", "-t", &want, "#{pane_id}"])
                .await
                .map_err(|e| Fault::Error(e.uncaught()))?;
            if py::strip(&out.stdout) == want {
                pane = want.clone();
            }
        }
        Some(false) => {}
    }
    let has = tmux
        .run(&["has-session", "-t", &format!("={sess}")])
        .await
        .map_err(|e| Fault::Error(e.uncaught()))?;
    if !has.ok {
        return error(
            StatusCode::NOT_FOUND,
            &format!("No hay sesion tmux '{sess}'. Levantala primero."),
        );
    }
    if py::is_pane(&want) != Some(true) {
        return reply(
            StatusCode::BAD_REQUEST,
            &json!({"error": "Falta un pane exacto (%N)", "code": "invalid"}),
        );
    }
    if pane != want {
        return reply(
            StatusCode::NOT_FOUND,
            &json!({"error": format!("El pane {want} ya no existe"), "code": "pane_gone"}),
        );
    }
    // `validate` del Python: un no-texto es «Texto vacío». La comprobación de
    // vacío de la librería (`is_whitespace` + U+001C–U+001F) es `py::is_space`.
    let text = match data.get("text") {
        Some(Value::String(t)) => t.clone(),
        _ => {
            return reply(
                StatusCode::BAD_REQUEST,
                &json!({"error": "Texto vacío", "code": "invalid"}),
            );
        }
    };
    if let Err(e) = pane_typing::validate(&text) {
        return reply(
            StatusCode::BAD_REQUEST,
            &json!({"error": e.message, "code": "invalid"}),
        );
    }
    if !state.locks.acquire(&pane) {
        return reply(
            StatusCode::CONFLICT,
            &json!({"error": BUSY, "code": "typing_in_progress"}),
        );
    }
    let guard = PaneGuard {
        state: state.clone(),
        pane,
    };
    // Otra petición con el mismo `requestId` pudo terminar (o declinar)
    // mientras esta resolvía la sesión: con el candado ya tomado, su respuesta
    // está en la caché o su marca puesta. Se usa esa y no se teclea.
    if !rid.is_empty() {
        if state.was_declined(&rid) {
            drop(guard);
            return Err(Fault::Decline);
        }
        if let Some((status, body)) = state.cached(&rid) {
            drop(guard);
            return replay(status, &body);
        }
    }
    let job = Job {
        tmux: tmux.clone(),
        handle: Handle::current(),
        text,
        rid: rid.clone(),
    };
    match tokio::task::spawn_blocking(move || job.run(guard)).await {
        Ok(Ok((status, body))) => replay(status, &body),
        Ok(Err(error)) => Err(Fault::Error(error)),
        // No llegó a correr (runtime parándose): nada se tecleó.
        Err(join) if join.is_cancelled() => Err(state.decline(&rid)),
        Err(_) => Err(failure()),
    }
}

/// El tecleo y lo que le sigue, en el hilo de bloqueo.
struct Job {
    tmux: Tmux,
    handle: Handle,
    text: String,
    rid: String,
}

impl Job {
    /// `type_literal` + respuesta + caché. `Err`: la excepción que el Python no
    /// captura (`TimeoutExpired` → 504, el resto → 500), sin guardar nada.
    fn run(self, guard: PaneGuard) -> Result<(u16, Value), HandlerError> {
        let started = Instant::now();
        let mut uncaught = None;
        let result = pane_typing::type_literal(
            |args| match self.tmux.run_blocking(&self.handle, args) {
                Ok(out) => TmuxResult {
                    returncode: if out.ok { 0 } else { 1 },
                    stdout: out.stdout,
                    stderr: out.stderr,
                },
                // La excepción corta el bucle: un código ≠ 0 detiene la librería.
                Err(e) => {
                    uncaught.get_or_insert(e.uncaught());
                    TmuxResult {
                        returncode: 1,
                        ..TmuxResult::default()
                    }
                }
            },
            &guard.pane,
            &self.text,
            |seconds| std::thread::sleep(Duration::from_secs_f64(seconds)),
            TypingOptions::default(),
        );
        let elapsed = started.elapsed().as_millis();
        if let Some(error) = uncaught {
            // `finally`: el candado se suelta (al soltar `guard`) sin guardar.
            return Err(error);
        }
        let (status, body) = match result {
            Ok(out) => (
                200u16,
                json!({
                    "ok": true,
                    "typed": out.typed,
                    "durationMs": u64::try_from(elapsed).unwrap_or(u64::MAX),
                    "requestId": self.rid,
                }),
            ),
            Err(e) => (
                502u16,
                json!({"error": e.message, "code": e.code, "typed": e.typed, "requestId": self.rid}),
            ),
        };
        // Primero la caché y después el candado (el Python lo hace al revés):
        // quien tome el pane ya encuentra la respuesta guardada.
        if !self.rid.is_empty() {
            guard.state.remember(&self.rid, status, &body);
        }
        drop(guard);
        Ok((status, body))
    }
}
