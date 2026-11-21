//! POST/GET `/session/message`: un mensaje para la sesión de un agente que no se
//! pierde ni se duplica. Ruta solo nativa (sin rama en el Python): nunca se
//! reenvía al heredado; si el conjunto nativo no puede atenderla responde 503.
//!
//! Contrato (`clientMessageId` lo genera el cliente, 1–64 `[A-Za-z0-9_-]`):
//! - POST `{"clientMessageId","session","pane"?,"text","submit"?}` pega el texto
//!   con `load-buffer` + `paste-buffer -p -d` (pegado entre corchetes, como
//!   `/paste`) y, con `submit` (por omisión `true`), ~200 ms después envía
//!   `send-keys Enter`.
//! - El recibo queda registrado como `delivering` ANTES del primer efecto de
//!   tmux (en memoria y en `~/.claude/hooks/session-messages.json`, escrito de
//!   forma atómica): un reintento con el mismo id nunca vuelve a pegar.
//!   - `delivered` → 200 con `"duplicate": true`, sin teclear nada.
//!   - `delivering` → 202 mientras el primero sigue en curso.
//!   - `uncertain` (se pegó y falló el Enter, o tmux no respondió, o el frente
//!     se reinició a mitad) → se devuelve el mismo recibo incierto (500) y
//!     nunca se reintenta solo: lo decide la persona mirando la sesión.
//! - Un rechazo (sesión o pane inexistentes, pane ocupado, `load-buffer` o
//!   `paste-buffer` que fallan sin pegar) no deja recibo: el mismo id puede
//!   volver a intentarse.
//! - Un pane pedido que ya no existe es 404 `pane_gone`: nunca se cae al pane
//!   activo de la sesión. Sin pane se usa el del agente (o el activo), como
//!   `/send`, y el recibo dice cuál.
//! - El candado por pane es el de `/pane/type`: los dos nunca escriben a la vez.
//! - GET `?clientMessageId=…` devuelve 200 con el recibo guardado (cualquier
//!   estado) o 404 `{"ok":false,"status":"unknown"}`.
use super::{Answer, Entry, Fault, Key, Native, NativeRoute, Verb, light, procs, py, query, reply, target};
use crate::{HandlerError, Reply, Request, dash::router::path_of};
use http::{Method, StatusCode};
use serde_json::{Map, Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

pub const PATH: &str = "/session/message";

pub const ROUTES: &[Entry] = &[
    Entry {
        verb: Verb::Post,
        key: Key::Path(PATH),
        route: NativeRoute::SessionMessage,
    },
    Entry {
        verb: Verb::Get,
        key: Key::Path(PATH),
        route: NativeRoute::SessionMessage,
    },
];

/// `~/.claude/hooks/session-messages.json`.
pub const FILE: &str = "session-messages.json";
/// Recibos que se conservan (los más recientes).
pub const KEEP: usize = 512;
/// Tope del texto, en caracteres (el de `/paste`).
pub const MAX_CHARS: usize = 20_000;
/// Pausa entre el pegado y el Enter: la TUI termina de procesar el pegado.
const ENTER_PAUSE: Duration = Duration::from_millis(200);

/// ¿Es esta petición de la ruta? (también con el conjunto nativo apagado).
pub fn claims(method: &Method, target: &str) -> bool {
    (*method == Method::POST || *method == Method::GET) && path_of(target) == PATH
}

#[derive(Clone)]
struct Receipt {
    id: String,
    /// Huella del texto: el mismo id con otro texto es un error del cliente.
    print: String,
    /// `delivering` | `delivered` | `uncertain`.
    status: String,
    body: Value,
}

#[derive(Default)]
struct Book {
    loaded: bool,
    list: VecDeque<Receipt>,
}

#[derive(Default)]
pub struct MessageState {
    book: Mutex<Book>,
    /// Serializa las escrituras del archivo: cada una lleva la foto más nueva.
    disk: Mutex<()>,
}

impl MessageState {
    fn book(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Carga el archivo la primera vez (bloquea: llamar en `spawn_blocking`).
    /// Un recibo que quedó `delivering` es de un frente que se paró a mitad:
    /// pasa a incierto y nunca se vuelve a pegar.
    fn load(&self, path: &Path) {
        if self.book().loaded {
            return;
        }
        let stored = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|v| v.get("receipts").and_then(Value::as_array).cloned())
            .unwrap_or_default();
        let mut book = self.book();
        if book.loaded {
            return;
        }
        book.loaded = true;
        for row in stored {
            let text = |k: &str| row.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
            let (id, print, mut status) = (text("id"), text("print"), text("status"));
            let Some(mut body) = row.get("body").and_then(Value::as_object).cloned() else {
                continue;
            };
            if id.is_empty() || book.list.iter().any(|r| r.id == id) {
                continue;
            }
            if status == "delivering" {
                status = "uncertain".into();
                body = uncertain_body(
                    &id,
                    "interrupted",
                    "El servidor se reinició mientras entregaba el mensaje; revisa la sesión antes de reenviarlo.",
                    body.get("pane").and_then(Value::as_str).unwrap_or(""),
                );
            }
            if !matches!(status.as_str(), "delivered" | "uncertain") {
                continue;
            }
            book.list.push_back(Receipt { id, print, status, body: Value::Object(body) });
        }
        while book.list.len() > KEEP {
            book.list.pop_front();
        }
    }

    fn find(&self, id: &str) -> Option<Receipt> {
        self.book().list.iter().find(|r| r.id == id).cloned()
    }

    /// Registra el id si nadie lo tiene; si ya está, devuelve el existente.
    fn claim(&self, receipt: Receipt) -> Option<Receipt> {
        let mut book = self.book();
        if let Some(old) = book.list.iter().find(|r| r.id == receipt.id) {
            return Some(old.clone());
        }
        book.list.push_back(receipt);
        while book.list.len() > KEEP {
            book.list.pop_front();
        }
        None
    }

    fn settle(&self, id: &str, status: &str, body: &Value) {
        let mut book = self.book();
        if let Some(slot) = book.list.iter_mut().find(|r| r.id == id) {
            slot.status = status.to_owned();
            slot.body = body.clone();
        }
    }

    fn forget(&self, id: &str) {
        self.book().list.retain(|r| r.id != id);
    }

    /// Escribe la foto actual (bloquea: llamar en `spawn_blocking`).
    fn persist(&self, path: &Path) -> std::io::Result<()> {
        let _disk = self.disk.lock().unwrap_or_else(|p| p.into_inner());
        let rows: Vec<Value> = self
            .book()
            .list
            .iter()
            .map(|r| json!({"id": r.id, "print": r.print, "status": r.status, "body": r.body}))
            .collect();
        let text = serde_json::to_string(&json!({"version": 1, "receipts": rows}))
            .map_err(std::io::Error::other)?;
        comandos_store::files::write_atomic(path, text.as_bytes())
    }
}

/// FNV-1a de 64 bits: estable entre versiones (va al archivo).
fn fingerprint(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn uncertain_body(id: &str, code: &str, error: &str, pane: &str) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("ok".into(), Value::Bool(false));
    body.insert("clientMessageId".into(), Value::from(id));
    body.insert("status".into(), Value::from("uncertain"));
    body.insert("code".into(), Value::from(code));
    body.insert("error".into(), Value::from(error));
    body.insert("pane".into(), Value::from(pane));
    body
}

fn invalid(message: &str) -> Answer {
    reply(
        StatusCode::UNPROCESSABLE_ENTITY,
        &json!({"ok": false, "code": "invalid", "status": "rejected", "error": message}),
    )
}

fn rejected(status: StatusCode, code: &str, message: &str) -> Answer {
    reply(
        status,
        &json!({"ok": false, "code": code, "status": "rejected", "error": message}),
    )
}

fn unavailable() -> Result<Reply, HandlerError> {
    Reply::json(
        StatusCode::SERVICE_UNAVAILABLE,
        &json!({"ok": false, "code": "unavailable", "status": "rejected",
                "error": "El envío de mensajes no está disponible ahora; inténtalo de nuevo."}),
    )
}

/// Un recibo guardado, como respuesta a un reintento del POST.
fn replay(receipt: &Receipt) -> Answer {
    match receipt.status.as_str() {
        "delivered" => {
            let mut body = receipt.body.clone();
            if let Some(map) = body.as_object_mut() {
                map.insert("duplicate".into(), Value::Bool(true));
            }
            reply(StatusCode::OK, &body)
        }
        "delivering" => reply(
            StatusCode::ACCEPTED,
            &json!({"ok": true, "status": "delivering", "clientMessageId": receipt.id}),
        ),
        _ => reply(StatusCode::INTERNAL_SERVER_ERROR, &receipt.body),
    }
}

fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

/// La ruta entera, sin declinar nunca: lo que en otras rutas se reenviaría al
/// Python (antes de cualquier efecto) aquí es un 503 que se puede reintentar.
pub async fn respond(native: &Arc<Native>, request: &Request) -> Result<Reply, HandlerError> {
    let answer = if request.method == Method::GET {
        lookup(native, request).await
    } else {
        post(native, request).await
    };
    match answer {
        Ok(reply) => Ok(reply),
        Err(Fault::Decline) => unavailable(),
        Err(Fault::Error(error)) => Err(error),
    }
}

/// Con el conjunto nativo ausente o apagado: la ruta sigue sin heredado.
pub async fn without_live(
    native: Option<&Arc<Native>>,
    request: &Request,
) -> Result<Reply, HandlerError> {
    match native {
        Some(native) => respond(native, request).await,
        None => unavailable(),
    }
}

fn file_of(native: &Native) -> PathBuf {
    native.options().hooks.join(FILE)
}

async fn loaded(native: &Arc<Native>) -> Result<(), Fault> {
    let state = native.messages.clone();
    let path = file_of(native);
    tokio::task::spawn_blocking(move || state.load(&path))
        .await
        .map_err(|_| Fault::Error(HandlerError::Failure))
}

async fn save(native: &Native) {
    let state = native.messages.clone();
    let path = file_of(native);
    let written = tokio::task::spawn_blocking(move || state.persist(&path)).await;
    if !matches!(written, Ok(Ok(()))) {
        // En memoria el recibo sigue valiendo: solo un reinicio lo perdería.
        eprintln!("comandos dash: no se pudo guardar {FILE}");
    }
}

async fn lookup(native: &Arc<Native>, request: &Request) -> Answer {
    let q = query::Query::parse(&request.target)?;
    let id = q.first("clientMessageId").unwrap_or("").to_owned();
    if !valid_id(&id) {
        return reply(
            StatusCode::BAD_REQUEST,
            &json!({"ok": false, "code": "invalid", "error": "clientMessageId inválido"}),
        );
    }
    loaded(native).await?;
    match native.messages.find(&id) {
        Some(receipt) => reply(StatusCode::OK, &receipt.body),
        None => reply(
            StatusCode::NOT_FOUND,
            &json!({"ok": false, "status": "unknown", "clientMessageId": id}),
        ),
    }
}

struct Message {
    id: String,
    session: String,
    pane: Option<String>,
    text: String,
    submit: bool,
}

/// Valida el cuerpo; `Err` es la respuesta 422.
fn parse(data: &Map<String, Value>) -> Result<Message, &'static str> {
    let id = match data.get("clientMessageId") {
        Some(Value::String(id)) if valid_id(id) => id.clone(),
        _ => return Err("clientMessageId inválido (1–64 letras, dígitos, «-» o «_»)"),
    };
    let session = match data.get("session") {
        Some(Value::String(s)) if py::is_session(s) => s.clone(),
        _ => return Err("Nombre de sesión inválido"),
    };
    let pane = match data.get("pane") {
        None | Some(Value::Null) => None,
        Some(Value::String(p)) if p.is_empty() => None,
        Some(Value::String(p)) if py::is_pane(p) == Some(true) && p.is_ascii() => Some(p.clone()),
        _ => return Err("Pane inválido (se espera %N)"),
    };
    let submit = match data.get("submit") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        _ => return Err("submit debe ser true o false"),
    };
    let Some(Value::String(raw)) = data.get("text") else {
        return Err("Texto vacío");
    };
    // Saltos de línea de cualquier sistema → `\n` (tmux los pega como CR).
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    if text.trim().is_empty() {
        return Err("Texto vacío");
    }
    if text.chars().count() > MAX_CHARS {
        return Err("Texto demasiado largo (máx. 20000 caracteres)");
    }
    // Un ESC dentro del pegado podría cerrar los corchetes y teclear fuera.
    if text
        .chars()
        .any(|c| (c.is_control() && c != '\n' && c != '\t') || c == '\u{7f}')
    {
        return Err("El texto tiene caracteres de control");
    }
    Ok(Message {
        id,
        session,
        pane,
        text,
        submit,
    })
}

/// Suelta el candado del pane pase lo que pase.
struct PaneGuard {
    native: Arc<Native>,
    pane: String,
}

impl Drop for PaneGuard {
    fn drop(&mut self) {
        self.native.typing.locks.release(&self.pane);
    }
}

async fn tmux(native: &Native, args: &[&str]) -> Result<super::tmux::Output, Fault> {
    native
        .options()
        .tmux
        .run(args)
        .await
        .map_err(|e| Fault::Error(e.uncaught()))
}

async fn post(native: &Arc<Native>, request: &Request) -> Answer {
    let data = light::data(request)?;
    let message = match parse(data) {
        Ok(message) => message,
        Err(why) => return invalid(why),
    };
    let print = fingerprint(&message.text);
    loaded(native).await?;
    if let Some(receipt) = native.messages.find(&message.id) {
        if receipt.print != print {
            return invalid("Ese clientMessageId ya se usó con otro texto");
        }
        return replay(&receipt);
    }
    // El destino, como `/send`: la sesión real del proyecto y el pane pedido
    // solo si vive; sin pane, el del agente o `=<sesión>:`.
    let wanted = json!({"session": message.session, "pane": message.pane.clone().unwrap_or_default()});
    let pt = match target::post_target(native, PATH, &wanted).await {
        Ok(pt) => pt,
        Err(Ok(_)) => return invalid("Nombre de sesión inválido"),
        Err(Err(fault)) => return Err(fault),
    };
    if !tmux(native, &["has-session", "-t", &pt.target]).await?.ok {
        return rejected(
            StatusCode::NOT_FOUND,
            "session_gone",
            &format!("No hay sesión tmux '{}'", pt.sess),
        );
    }
    if let Some(want) = &message.pane
        && pt.pane != *want
    {
        return rejected(
            StatusCode::NOT_FOUND,
            "pane_gone",
            &format!("El pane {want} ya no existe"),
        );
    }
    // El pane exacto: todo lo que sigue (pegado, Enter, candado) va a él
    // aunque el foco de la sesión cambie entre medias.
    let shown = tmux(
        native,
        &["display-message", "-p", "-t", &pt.pane, "#{pane_id} #{pane_in_mode}"],
    )
    .await?;
    let line = py::strip(&shown.stdout).to_owned();
    let (pane, in_mode) = line.split_once(' ').unwrap_or((line.as_str(), "0"));
    if !shown.ok || py::is_pane(pane) != Some(true) {
        let named = message.pane.as_deref().unwrap_or(pane);
        return rejected(
            StatusCode::NOT_FOUND,
            "pane_gone",
            &format!("El pane {named} ya no existe"),
        );
    }
    let pane = pane.to_owned();
    let in_mode = in_mode.trim() == "1";
    // Un menú de permiso abierto: pegar texto y pulsar Enter ahí elegiría la
    // opción resaltada (normalmente «Yes»). Se rechaza sin recibo: el mismo id
    // puede reintentarse cuando la persona haya respondido el permiso.
    if let Ok(screen) = native.options().tmux.run(&["capture-pane", "-p", "-t", &pane]).await
        && screen.ok
        && super::chat::ask(&screen.stdout).is_some()
    {
        return rejected(
            StatusCode::CONFLICT,
            "prompt_open",
            "La sesión espera que respondas un permiso; respóndelo antes de enviar.",
        );
    }
    if !native.typing.locks.acquire(&pane) {
        // Quizá es el mismo id, que otra petición está entregando.
        if let Some(receipt) = native.messages.find(&message.id) {
            return replay(&receipt);
        }
        return rejected(
            StatusCode::CONFLICT,
            "pane_busy",
            "Ya se está escribiendo en ese pane; espera a que termine.",
        );
    }
    let guard = PaneGuard {
        native: native.clone(),
        pane: pane.clone(),
    };
    let accepted = Receipt {
        id: message.id.clone(),
        print,
        status: "delivering".into(),
        body: json!({"ok": true, "status": "delivering", "clientMessageId": message.id, "pane": pane}),
    };
    let id = message.id.clone();
    if let Some(existing) = native.messages.claim(accepted) {
        drop(guard);
        if existing.print != fingerprint(&message.text) {
            return invalid("Ese clientMessageId ya se usó con otro texto");
        }
        return replay(&existing);
    }
    // Desde aquí todo corre en su propia tarea (sin `await` entre el registro
    // y el lanzamiento): si el cliente se va, la entrega termina igual y su
    // recibo queda guardado para el reintento.
    let job = native
        .tasks()
        .spawn_handle({
            let native = native.clone();
            async move {
                // Registrado (y en disco) antes del primer efecto.
                save(&native).await;
                let (status, state, body) =
                    deliver(&native, &message, &pt.sess, &pane, in_mode).await;
                match state {
                    Some(state) => native.messages.settle(&message.id, state, &body),
                    None => native.messages.forget(&message.id),
                }
                save(&native).await;
                drop(guard);
                (status, body)
            }
        })
        .map_err(|_| {
            // No llegó a correr: nada se pegó, el id queda libre.
            native.messages.forget(&id);
            Fault::Error(HandlerError::Failure)
        })?;
    let (status, body) = job.await.map_err(|_| Fault::Error(HandlerError::Failure))?;
    reply(status, &body)
}

fn buffer_name() -> Result<String, ()> {
    let mut bytes = [0u8; 6];
    getrandom::fill(&mut bytes).map_err(|_| ())?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("comandos-msg-{hex}"))
}

fn stderr_or(out: &super::tmux::Output, fallback: &str) -> String {
    let text = py::strip(&out.stderr);
    if text.is_empty() { fallback.to_owned() } else { text.to_owned() }
}

/// `(estado HTTP, recibo final o None si no quedó nada pegado, cuerpo)`.
async fn deliver(
    native: &Native,
    message: &Message,
    sess: &str,
    pane: &str,
    in_mode: bool,
) -> (StatusCode, Option<&'static str>, Value) {
    let opts = native.options();
    let failed = |error: String| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            None,
            json!({"ok": false, "clientMessageId": message.id, "status": "rejected",
                   "code": "tmux_failed", "error": error}),
        )
    };
    let uncertain = |code: &str, error: &str| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Some("uncertain"),
            Value::Object(uncertain_body(&message.id, code, error, pane)),
        )
    };
    let Ok(buf) = buffer_name() else {
        return failed("No se pudo preparar el pegado".into());
    };
    // En modo copia el Enter lo tomaría la copia: se sale antes de pegar.
    if in_mode {
        let _ = opts.tmux.run(&["send-keys", "-X", "-t", pane, "cancel"]).await;
    }
    let loaded = procs::run_program_input(
        &opts.tmux.program,
        &["load-buffer", "-b", &buf, "-"],
        message.text.as_bytes(),
        opts.tmux.timeout,
    )
    .await;
    match loaded {
        Ok(out) if out.ok => {}
        Ok(out) => return failed(stderr_or(&out, "load-buffer falló")),
        Err(_) => {
            let _ = opts.tmux.run(&["delete-buffer", "-b", &buf]).await;
            return failed("load-buffer no respondió".into());
        }
    }
    match opts
        .tmux
        .run(&["paste-buffer", "-p", "-d", "-b", &buf, "-t", pane])
        .await
    {
        Ok(out) if out.ok => {}
        Ok(out) => {
            let _ = opts.tmux.run(&["delete-buffer", "-b", &buf]).await;
            return failed(stderr_or(&out, "paste-buffer falló"));
        }
        Err(_) => {
            let _ = opts.tmux.run(&["delete-buffer", "-b", &buf]).await;
            return uncertain(
                "uncertain",
                "tmux no confirmó el pegado; revisa la sesión antes de reenviar.",
            );
        }
    }
    if message.submit {
        tokio::time::sleep(ENTER_PAUSE).await;
        let entered = opts.tmux.run(&["send-keys", "-t", pane, "Enter"]).await;
        if !matches!(entered, Ok(ref out) if out.ok) {
            return uncertain(
                "partial",
                "El texto se pegó pero no se pudo enviar el Enter; revisa la sesión antes de reenviar.",
            );
        }
    }
    let delivered_at = (opts.clock)();
    (
        StatusCode::OK,
        Some("delivered"),
        json!({
            "ok": true,
            "clientMessageId": message.id,
            "status": "delivered",
            "duplicate": false,
            "session": sess,
            "pane": pane,
            "submitted": message.submit,
            "deliveredAtMs": delivered_at,
            "chars": message.text.chars().count(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn parse_validates_ids_text_and_pane() {
        let ok = parse(&body(json!({"clientMessageId": "a-1_B", "session": "s1", "text": "hola\r\nmundo"}))).ok().unwrap();
        assert_eq!(ok.text, "hola\nmundo");
        assert!(ok.submit);
        assert_eq!(ok.pane, None);
        let with_pane = parse(&body(json!({"clientMessageId": "x", "session": "s1", "pane": "%3", "text": "a", "submit": false}))).ok().unwrap();
        assert_eq!(with_pane.pane.as_deref(), Some("%3"));
        assert!(!with_pane.submit);
        for bad in [
            json!({"session": "s1", "text": "a"}),
            json!({"clientMessageId": "", "session": "s1", "text": "a"}),
            json!({"clientMessageId": "a b", "session": "s1", "text": "a"}),
            json!({"clientMessageId": "x".repeat(65), "session": "s1", "text": "a"}),
            json!({"clientMessageId": "x", "session": "a b", "text": "a"}),
            json!({"clientMessageId": "x", "session": "s1", "pane": "s1", "text": "a"}),
            json!({"clientMessageId": "x", "session": "s1", "text": "   "}),
            json!({"clientMessageId": "x", "session": "s1", "text": 5}),
            json!({"clientMessageId": "x", "session": "s1", "text": "a\u{1b}[201~"}),
            json!({"clientMessageId": "x", "session": "s1", "text": "a", "submit": "yes"}),
            json!({"clientMessageId": "x", "session": "s1", "text": "y".repeat(MAX_CHARS + 1)}),
        ] {
            assert!(parse(&body(bad.clone())).is_err(), "{bad}");
        }
    }

    #[test]
    fn receipts_are_bounded_and_survive_a_restart_as_uncertain_when_cut() {
        let dir = std::env::temp_dir().join(format!("cmd-session-msg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(FILE);
        let state = MessageState::default();
        state.load(&path);
        let receipt = |id: &str, status: &str| Receipt {
            id: id.into(),
            print: fingerprint(id),
            status: status.into(),
            body: json!({"ok": true, "status": status, "clientMessageId": id, "pane": "%1"}),
        };
        for n in 0..KEEP + 3 {
            assert!(state.claim(receipt(&format!("m{n}"), "delivered")).is_none());
        }
        assert!(state.claim(receipt("m10", "delivered")).is_some(), "el mismo id no entra dos veces");
        assert!(state.claim(receipt("cut", "delivering")).is_none());
        state.persist(&path).unwrap();
        let again = MessageState::default();
        again.load(&path);
        assert_eq!(again.book().list.len(), KEEP);
        assert!(again.find("m0").is_none(), "los más viejos salen primero");
        let cut = again.find("cut").unwrap();
        assert_eq!(cut.status, "uncertain");
        assert_eq!(cut.body["code"], "interrupted");
        assert_eq!(cut.body["pane"], "%1");
        assert_eq!(again.find("m600").map(|r| r.status), None);
        assert_eq!(again.find("m514").unwrap().status, "delivered");
        std::fs::write(&path, b"{roto").unwrap();
        let broken = MessageState::default();
        broken.load(&path);
        assert!(broken.book().list.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fingerprint_is_stable() {
        assert_eq!(fingerprint(""), "cbf29ce484222325");
        assert_eq!(fingerprint("a"), "af63dc4c8601ec8c");
        assert!(claims(&Method::POST, "/session/message"));
        assert!(claims(&Method::GET, "/session/message?clientMessageId=a"));
        assert!(!claims(&Method::GET, "/session/messages"));
    }
}
