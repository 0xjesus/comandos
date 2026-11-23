//! Corte `tabs` (plan 2f-1, Tarea 6): teclas, foco, desplazamiento y
//! exportación. POST `/send` (9634), `/paste` (9646), `/key` (9659), `/focus`
//! (9705), `/kill` (9693), `/tmux-scroll` (9496, `tmux_scroll` 5850 con
//! `tmux_pane_at` 5807) y `/export` (9678, `export_response` 7910 con
//! `md_to_html` 7840) de `bin/cc-dash`.
//!
//! `/tmux-scroll` va tras la validación de `session` del `do_POST` y antes de
//! `resolve_project_session` (en el Python está antes de esa llamada); las
//! otras seis, tras `target::post_target`, que resuelve el destino exactamente
//! como el Python: el pane pedido solo si vive, si no `=<sesión>:`. `/key` con
//! un pane pedido que no es el resuelto responde 404 y nunca teclea en el pane
//! activo.
//!
//! `Decline` solo antes del primer efecto: cada efecto (orden de tmux que
//! muta, escritura de archivo, lanzamiento de Chrome o del abridor) marca la
//! petición y un `Decline` tras la marca es un 500 con su línea en stderr
//! (ruling 2 del sub-plan). `/focus` marca antes de `select_claude_window` (que
//! siempre emite un `select-window`) y de `focus_session`.
//!
//! Cancelación: cada petición corre en su propia tarea, contada en
//! `Native::tasks` (`spawn_handle`): si el cliente se va a mitad de un pegado,
//! el `delete-buffer` del `finally` del Python se ejecuta igual.
//!
//! Efectos en vivo (los del Python, en su orden; tmux por `opts.tmux`, plazo
//! 5 s; el texto va en un argumento tras `-l --` o por stdin, nunca por un
//! shell): `send-keys -t <pane> -l -- <texto>` y `send-keys -t <pane> Enter`;
//! `load-buffer -b comandos-snip-<12 hex> -` (texto por stdin),
//! `paste-buffer -p -d -b <buf> -t <pane>` y `delete-buffer -b <buf>` si no
//! pegó; `send-keys -t <pane> <tecla>`; `kill-session -t =<s>` (nunca `hub`,
//! `local` ni `control`); `select-window`/`switch-client`/terminal/`wmctrl`
//! de `focus_session`; `send-keys -X -t <t> cancel`, `send-keys -l`/`-H` de
//! la rueda, `copy-mode -e -t <t>` y `send-keys -X -N <n> -t <t>
//! scroll-up|scroll-down`, con `<t>` = `=<s>:` o el `%<pane>` bajo el cursor;
//! el `.txt`/`.html`/`.pdf` de la exportación en `~/Descargas`,
//! `~/Downloads` o `~`, Chrome headless (plazo 40 s), `app-tab-models.json`
//! (lo escribe `read_states`, como el Python) y `xdg-open <ruta>` suelto.
use super::{
    Answer, Fault, Key, Native, NativeOptions, NativeRoute, Verb, light, procs, py, reply,
    sessions::{focus_session, select_claude_window},
    states::gather,
    target,
    tmux::{Output, RunError, run_program},
};
use crate::{HandlerError, Request};
use comandos_core::json::truthy;
use http::StatusCode;
use serde_json::{Map, Value, json};
use std::{
    ffi::OsString,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRoute {
    Send,
    Paste,
    Key,
    Focus,
    Kill,
    TmuxScroll,
    Export,
}

impl InputRoute {
    /// La ruta del Python (`self.path == …`).
    pub const fn path(self) -> &'static str {
        match self {
            InputRoute::Send => "/send",
            InputRoute::Paste => "/paste",
            InputRoute::Key => "/key",
            InputRoute::Focus => "/focus",
            InputRoute::Kill => "/kill",
            InputRoute::TmuxScroll => "/tmux-scroll",
            InputRoute::Export => "/export",
        }
    }
}

const fn entry(route: InputRoute) -> super::Entry {
    super::Entry {
        verb: Verb::Post,
        key: Key::Raw(route.path()),
        route: NativeRoute::Input(route),
    }
}

pub const ROUTES: &[super::Entry] = &[
    entry(InputRoute::Send),
    entry(InputRoute::Paste),
    entry(InputRoute::Key),
    entry(InputRoute::Focus),
    entry(InputRoute::Kill),
    entry(InputRoute::TmuxScroll),
    entry(InputRoute::Export),
];

/// `ALLOWED_KEYS` (5588).
pub const ALLOWED_KEYS: [&str; 16] = [
    "Enter", "Escape", "Up", "Down", "Tab", "y", "n", "1", "2", "3", "4", "5", "6", "7", "8", "9",
];

/// `HIDDEN_SESSIONS` (6307): nunca se matan.
pub const HIDDEN_SESSIONS: [&str; 3] = ["hub", "local", "control"];

/// `str(data.get("text", ""))[:4000]` de `/send`.
const SEND_CHARS: usize = 4000;

/// Tope de `/paste` (en caracteres, como `len(text)`).
const PASTE_CHARS: usize = 20000;

/// Plazo de Chrome headless en `export_response`.
const CHROME_SECONDS: u64 = 40;

/// Navegadores de `export_response`, en su orden.
const CHROMES: [&str; 4] = [
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
];

/// `PDF_CSS` (7883), literal.
const PDF_CSS: &str = "
body{font-family:'Inter',system-ui,sans-serif;font-size:12.5px;color:#1B2130;
  line-height:1.55;max-width:760px;margin:24px auto}
h1{font-size:17px;border-bottom:2px solid #5B4BD6;padding-bottom:6px}
h3{font-size:13.5px;color:#5B4BD6;margin:14px 0 4px}
.meta{color:#8892A6;font-size:10.5px;margin-bottom:14px}
code,pre{font-family:'JetBrains Mono',monospace;font-size:11px;background:#F2F4F8;
  border-radius:4px;padding:1px 4px}
pre{display:block;padding:9px 11px;margin:8px 0;white-space:pre-wrap;
  border:1px solid #D9DEE8}
table{border-collapse:collapse;margin:8px 0;font-size:11.5px}
th,td{border:1px solid #C3CAD8;padding:4px 9px;text-align:left}
th{background:#EDF0F6;color:#5B4BD6}
hr{border:none;border-top:1px solid #D9DEE8;margin:10px 0}
";

fn failure() -> Fault {
    Fault::Error(HandlerError::Failure)
}

/// Justo antes de un efecto.
fn mark(fx: &AtomicBool) {
    fx.store(true, Ordering::Release);
}

/// Tras el primer efecto ya no se declina (ruling 2): un `Decline` es un 500
/// con su línea en stderr.
fn settle(path: &str, answer: Answer, fx: &AtomicBool) -> Answer {
    match answer {
        Err(Fault::Decline) if fx.load(Ordering::Acquire) => {
            eprintln!("comandos dash: estado incierto tras un efecto; {path} responde 500");
            Err(failure())
        }
        other => other,
    }
}

/// Un trabajo de disco o `/proc`; su pánico es una excepción sin capturar.
async fn blocking<T, F>(job: F) -> Result<T, Fault>
where
    F: FnOnce() -> Result<T, Fault> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|_| failure())?
}

/// `tmux(*args)` (plazo 5 s) cuyas excepciones el Python no captura.
async fn tmux(opts: &NativeOptions, args: &[&str]) -> Result<Output, Fault> {
    opts.tmux
        .run(args)
        .await
        .map_err(|e| Fault::Error(e.uncaught()))
}

/// `tmux` de una orden que muta (marca el efecto antes).
async fn mutate(fx: &AtomicBool, opts: &NativeOptions, args: &[&str]) -> Result<Output, Fault> {
    mark(fx);
    tmux(opts, args).await
}

/// `r.stderr.strip() or fallback`.
fn stderr_or(out: &Output, fallback: &str) -> String {
    let text = py::strip(&out.stderr);
    if text.is_empty() {
        fallback.to_owned()
    } else {
        text.to_owned()
    }
}

/// `RunError` de un `subprocess.run` que el Python no captura.
fn run_fault(error: &RunError) -> Fault {
    match error {
        RunError::Timeout => Fault::Error(HandlerError::Timeout),
        RunError::Spawn(_) | RunError::Decode => failure(),
    }
}

fn error_reply(status: StatusCode, message: &str) -> Answer {
    reply(status, &json!({"error": message}))
}

fn ok_reply() -> Answer {
    reply(StatusCode::OK, &json!({"ok": true}))
}

pub async fn answer(native: &Arc<Native>, route: InputRoute, request: &Request) -> Answer {
    let data = light::data(request)?.clone();
    let fx = Arc::new(AtomicBool::new(false));
    let job = native
        .tasks()
        .spawn_handle({
            let native = Arc::clone(native);
            let fx = Arc::clone(&fx);
            async move { run(&native, route, data, &fx).await }
        })
        .map_err(|_| failure())?;
    let answer = job.await.map_err(|_| failure())?;
    settle(route.path(), answer, &fx)
}

async fn run(
    native: &Native,
    route: InputRoute,
    data: Map<String, Value>,
    fx: &AtomicBool,
) -> Answer {
    if route == InputRoute::TmuxScroll {
        return scroll(native, &data, fx).await;
    }
    let value = Value::Object(data);
    let pt = match target::post_target(native, route.path(), &value).await {
        Ok(pt) => pt,
        Err(answer) => return answer,
    };
    let Value::Object(data) = value else {
        return Err(failure());
    };
    match route {
        InputRoute::Send => send(native, &pt, &data, fx).await,
        InputRoute::Paste => paste(native, &pt, &data, fx).await,
        InputRoute::Key => key(native, &pt, &data, fx).await,
        InputRoute::Focus => focus(native, &pt, fx).await,
        InputRoute::Kill => kill(native, &pt, fx).await,
        InputRoute::Export => export(native, &pt, &data, fx).await,
        InputRoute::TmuxScroll => Err(failure()),
    }
}

/// `str(data.get("text", ""))`: un flotante o un contenedor declina (su
/// `repr` no se reproduce con certeza); antes de cualquier efecto.
fn text_of(data: &Map<String, Value>) -> Result<String, Fault> {
    match data.get("text") {
        None => Ok(String::new()),
        Some(value) => py::str_scalar(value).ok_or(Fault::Decline),
    }
}

/// `has-session -t =<s>`: `returncode == 0`.
async fn has_session(opts: &NativeOptions, target: &str) -> Result<bool, Fault> {
    Ok(tmux(opts, &["has-session", "-t", target]).await?.ok)
}

/// POST `/send` (9634).
async fn send(
    native: &Native,
    pt: &target::PostTarget,
    data: &Map<String, Value>,
    fx: &AtomicBool,
) -> Answer {
    let opts = native.options();
    let text = py::take_chars(&text_of(data)?, SEND_CHARS);
    if py::strip(&text).is_empty() {
        return error_reply(StatusCode::BAD_REQUEST, "Texto vacio");
    }
    if !has_session(opts, &pt.target).await? {
        let message = format!("No hay sesion tmux '{}'. Levantala primero.", pt.sess);
        return error_reply(StatusCode::NOT_FOUND, &message);
    }
    let typed = mutate(fx, opts, &["send-keys", "-t", &pt.pane, "-l", "--", &text]).await?;
    if !typed.ok {
        return error_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            &stderr_or(&typed, "tmux fallo"),
        );
    }
    mutate(fx, opts, &["send-keys", "-t", &pt.pane, "Enter"]).await?;
    ok_reply()
}

/// POST `/paste` (9646).
async fn paste(
    native: &Native,
    pt: &target::PostTarget,
    data: &Map<String, Value>,
    fx: &AtomicBool,
) -> Answer {
    let opts = native.options();
    let text = text_of(data)?;
    if py::strip(&text).is_empty() {
        return error_reply(StatusCode::BAD_REQUEST, "Texto vacio");
    }
    if text.chars().count() > PASTE_CHARS {
        return error_reply(StatusCode::BAD_REQUEST, "body demasiado largo (max 20000)");
    }
    if !has_session(opts, &pt.target).await? {
        let message = format!("No hay sesion tmux '{}'. Levantala primero.", pt.sess);
        return error_reply(StatusCode::NOT_FOUND, &message);
    }
    match snippet_paste_to_pane(opts, fx, &pt.pane, &text).await? {
        Some(error) => error_reply(StatusCode::INTERNAL_SERVER_ERROR, &error),
        None => ok_reply(),
    }
}

/// `"comandos-snip-" + uuid.uuid4().hex[:12]`: los 12 primeros dígitos de un
/// UUID4 son aleatorios (la versión va en el decimotercero).
fn snippet_buffer() -> Result<String, Fault> {
    let mut bytes = [0u8; 6];
    getrandom::fill(&mut bytes).map_err(|_| failure())?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("comandos-snip-{hex}"))
}

/// `snippet_paste_to_pane(pane, text, tmux_stdin)` (5724): `load-buffer` por
/// stdin, `paste-buffer -p -d` y, si no pegó, `delete-buffer` (también si
/// `paste-buffer` lanzó: el `finally`, cuya excepción manda si también lanza).
/// Nunca envía Enter.
pub(crate) async fn snippet_paste_to_pane(
    opts: &NativeOptions,
    fx: &AtomicBool,
    pane: &str,
    text: &str,
) -> Result<Option<String>, Fault> {
    let buf = snippet_buffer()?;
    mark(fx);
    let loaded = procs::run_program_input(
        &opts.tmux.program,
        &["load-buffer", "-b", &buf, "-"],
        text.as_bytes(),
        opts.tmux.timeout,
    )
    .await
    .map_err(|e| run_fault(&e))?;
    if !loaded.ok {
        return Ok(Some(stderr_or(&loaded, "load-buffer fallo")));
    }
    let pasted = opts
        .tmux
        .run(&["paste-buffer", "-p", "-d", "-b", &buf, "-t", pane])
        .await;
    match pasted {
        Ok(out) if out.ok => Ok(None),
        Ok(out) => {
            tmux(opts, &["delete-buffer", "-b", &buf]).await?;
            Ok(Some(stderr_or(&out, "paste-buffer fallo")))
        }
        Err(error) => {
            tmux(opts, &["delete-buffer", "-b", &buf]).await?;
            Err(Fault::Error(error.uncaught()))
        }
    }
}

/// POST `/key` (9659). Un pane pedido que ya no vive no es el resuelto
/// (`post_target` cayó a `=<sesión>:`) y es un 404: nunca se teclea en el pane
/// activo. Un pane vivo se acepta aunque sea de otra sesión, como en el Python
/// (`display-message -t <pane>` solo comprueba que exista).
async fn key(
    native: &Native,
    pt: &target::PostTarget,
    data: &Map<String, Value>,
    fx: &AtomicBool,
) -> Answer {
    let opts = native.options();
    // `key not in ALLOWED_KEYS`: un valor que no se puede hashear (lista u
    // objeto) es un `TypeError` (500); cualquier otro no texto no está.
    let key = match data.get("key") {
        Some(Value::String(k)) if ALLOWED_KEYS.contains(&k.as_str()) => k.as_str(),
        Some(Value::Array(_) | Value::Object(_)) => return Err(failure()),
        _ => return error_reply(StatusCode::BAD_REQUEST, "Tecla no permitida"),
    };
    // `str(data.get("pane") or "")`: el `str()` de un no texto verdadero
    // nunca empieza por `%`, así que nunca casa PANE_RE.
    let wanted = match data.get("pane") {
        Some(Value::String(p)) if !p.is_empty() => Some(p.as_str()),
        Some(v) if truthy(v) => {
            return error_reply(StatusCode::BAD_REQUEST, "Pane invalido");
        }
        _ => None,
    };
    if let Some(wanted) = wanted {
        match py::is_pane(wanted) {
            None => return Err(Fault::Decline),
            Some(false) => return error_reply(StatusCode::BAD_REQUEST, "Pane invalido"),
            Some(true) if pt.pane != wanted => {
                let message = format!("El pane {wanted} ya no existe");
                return error_reply(StatusCode::NOT_FOUND, &message);
            }
            Some(true) => {}
        }
    }
    if !has_session(opts, &pt.target).await? {
        let message = format!("No hay sesion tmux '{}'", pt.sess);
        return error_reply(StatusCode::NOT_FOUND, &message);
    }
    // A swipe on the phone's terminal opens tmux's history (copy mode), and there
    // every key moves the history instead of reaching the agent: the dock's
    // arrows could not pick an option in a menu. A key leaves the history first;
    // Escape only leaves it, so it never interrupts the agent by surprise.
    let mode = opts
        .tmux
        .run(&["display-message", "-p", "-t", &pt.pane, "#{pane_in_mode}"])
        .await;
    if mode.is_ok_and(|m| m.ok && py::strip(&m.stdout) == "1") {
        let left = mutate(fx, opts, &["send-keys", "-X", "-t", &pt.pane, "cancel"]).await?;
        if key == "Escape" && left.ok {
            return ok_reply();
        }
    }
    let typed = mutate(fx, opts, &["send-keys", "-t", &pt.pane, key]).await?;
    if !typed.ok {
        return error_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            &stderr_or(&typed, "tmux fallo"),
        );
    }
    ok_reply()
}

/// POST `/kill` (9693): nunca las sesiones internas de la app.
async fn kill(native: &Native, pt: &target::PostTarget, fx: &AtomicBool) -> Answer {
    let opts = native.options();
    if HIDDEN_SESSIONS.contains(&pt.sess.as_str()) {
        return error_reply(StatusCode::BAD_REQUEST, "Esa sesion es interna de la app");
    }
    if !has_session(opts, &pt.target).await? {
        let message = format!("No hay sesion tmux '{}'", pt.sess);
        return error_reply(StatusCode::NOT_FOUND, &message);
    }
    let killed = mutate(fx, opts, &["kill-session", "-t", &pt.target]).await?;
    if !killed.ok {
        return error_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            &stderr_or(&killed, "tmux fallo"),
        );
    }
    ok_reply()
}

/// POST `/focus` (9705).
async fn focus(native: &Native, pt: &target::PostTarget, fx: &AtomicBool) -> Answer {
    let opts = native.options();
    if has_session(opts, &pt.target).await? {
        mark(fx);
        select_claude_window(native, &pt.sess).await?;
    }
    // Sin sesión, `focus_session` responde con su `has-session` antes de
    // cualquier efecto; con ella, la marca ya está puesta.
    match focus_session(native, &pt.sess, "claude").await? {
        Some(error) => error_reply(StatusCode::NOT_FOUND, &error),
        None => ok_reply(),
    }
}

/// `sess = data.get("session", "")` y `SESSION_RE.match(sess)` del preámbulo
/// de `do_POST`, sin `resolve_project_session`.
fn session_of(data: &Map<String, Value>) -> Result<Result<&str, Answer>, Fault> {
    let sess = match data.get("session") {
        None => "",
        Some(Value::String(s)) => s.as_str(),
        // `SESSION_RE.match` de un no texto: `TypeError` (500).
        Some(_) => return Err(failure()),
    };
    if !py::is_session(sess) {
        return Ok(Err(error_reply(
            StatusCode::BAD_REQUEST,
            "Nombre de sesion invalido",
        )));
    }
    Ok(Ok(sess))
}

/// POST `/tmux-scroll` (9496).
async fn scroll(native: &Native, data: &Map<String, Value>, fx: &AtomicBool) -> Answer {
    let sess = match session_of(data)? {
        Ok(sess) => sess,
        Err(answer) => return answer,
    };
    let err = tmux_scroll(
        native.options(),
        fx,
        sess,
        data.get("delta"),
        data.get("col"),
        data.get("row"),
    )
    .await?;
    match err {
        Some(err) if err.starts_with("No hay sesion") => error_reply(StatusCode::NOT_FOUND, &err),
        Some(err) => error_reply(StatusCode::BAD_REQUEST, &err),
        None => ok_reply(),
    }
}

/// `int(value)` donde el Python solo captura `TypeError` y `ValueError`:
/// `Ok(None)` es lo capturado; `OverflowError` (infinito) no se captura (500).
fn int_caught(value: &Value) -> Result<Option<i64>, Fault> {
    match py::int_of(value) {
        Ok(n) => Ok(Some(n)),
        Err(py::Conversion::Value(_) | py::Conversion::Type) => Ok(None),
        Err(py::Conversion::Overflow) => Err(failure()),
        Err(py::Conversion::Exotic) => Err(Fault::Decline),
    }
}

/// `None` en el Python: la clave falta o es `null`.
fn present(value: Option<&Value>) -> Option<&Value> {
    value.filter(|v| !v.is_null())
}

/// Una orden de tmux dentro de un `try … except Exception as exc: return
/// str(exc).strip() or "tmux fallo"`: `Err` es el texto de la excepción.
/// Una excepción cuyo texto no se reproduce declina.
async fn caught(opts: &NativeOptions, args: &[&str]) -> Result<Result<Output, String>, Fault> {
    match opts.tmux.run(args).await {
        Ok(out) => Ok(Ok(out)),
        Err(error) => {
            let message = error.python_message().ok_or(Fault::Decline)?;
            let message = py::strip(&message);
            Ok(Err(if message.is_empty() {
                "tmux fallo".to_owned()
            } else {
                message.to_owned()
            }))
        }
    }
}

/// `int(text)` de un campo de `list-panes` (dentro del `try … except
/// ValueError: continue`).
fn field_int(text: &str) -> Result<Option<i64>, Fault> {
    match py::int(text) {
        Ok(n) => Ok(Some(n)),
        Err(py::NumError::Invalid) => Ok(None),
        Err(py::NumError::Exotic) => Err(Fault::Decline),
    }
}

/// ¿Contiene el pane (`left|top|right|bottom`) la celda? `None`: un `int()`
/// lanzó `ValueError` (`continue`). Evaluación perezosa como la comparación
/// encadenada con `and` del Python.
fn contains(fields: [&str; 4], col: i64, row: i64) -> Result<Option<bool>, Fault> {
    let [left, top, right, bottom] = fields;
    let Some(left) = field_int(left)? else {
        return Ok(None);
    };
    if left > col {
        return Ok(Some(false));
    }
    let Some(right) = field_int(right)? else {
        return Ok(None);
    };
    if col > right {
        return Ok(Some(false));
    }
    let Some(top) = field_int(top)? else {
        return Ok(None);
    };
    if top > row {
        return Ok(Some(false));
    }
    let Some(bottom) = field_int(bottom)? else {
        return Ok(None);
    };
    Ok(Some(row <= bottom))
}

/// `tmux_pane_at(sess, col, row)` (5807): `Ok((target, col, row))` o el
/// texto de error.
async fn tmux_pane_at(
    opts: &NativeOptions,
    sess: &str,
    col: Option<&Value>,
    row: Option<&Value>,
) -> Result<Result<(String, i64, i64), String>, Fault> {
    let target = format!("={sess}:");
    let invalid = || Ok(Err("Posicion de pane invalida".to_owned()));
    let (col, row) = match (present(col), present(row)) {
        (None, None) => return Ok(Ok((target, 1, 1))),
        (Some(col), Some(row)) => (col, row),
        _ => return invalid(),
    };
    let Some(col) = int_caught(col)? else {
        return invalid();
    };
    let Some(row) = int_caught(row)? else {
        return invalid();
    };
    if !(0..=10000).contains(&col) || !(0..=10000).contains(&row) {
        return invalid();
    }
    let listed = match caught(
        opts,
        &[
            "list-panes",
            "-t",
            &target,
            "-F",
            "#{pane_id}|#{pane_left}|#{pane_top}|#{pane_right}|#{pane_bottom}",
        ],
    )
    .await?
    {
        Ok(out) => out,
        Err(message) => return Ok(Err(message)),
    };
    if !listed.ok {
        let fallback = format!("No hay sesion tmux '{sess}'");
        return Ok(Err(stderr_or(&listed, &fallback)));
    }
    for line in py::splitlines(&listed.stdout) {
        let fields: Vec<&str> = line.split('|').collect();
        let [pane, left, top, right, bottom] = fields.as_slice() else {
            continue;
        };
        if contains([*left, *top, *right, *bottom], col, row)? == Some(true) {
            // Los mismos `int()` ya resolvieron sin error.
            let left = field_int(left)?.ok_or_else(failure)?;
            let top = field_int(top)?.ok_or_else(failure)?;
            return Ok(Ok(((*pane).to_owned(), col - left + 1, row - top + 1)));
        }
    }
    Ok(Err("No encontre el pane bajo el cursor".to_owned()))
}

/// `tmux_scroll(sess, delta, col, row)` (5850): `Some(error)` o `None`.
async fn tmux_scroll(
    opts: &NativeOptions,
    fx: &AtomicBool,
    sess: &str,
    delta: Option<&Value>,
    col: Option<&Value>,
    row: Option<&Value>,
) -> Result<Option<String>, Fault> {
    let invalid = || Ok(Some("Scroll invalido".to_owned()));
    // `int(None)` es un `TypeError`, como `int_of(null)`.
    let Some(delta) = int_caught(delta.unwrap_or(&Value::Null))? else {
        return invalid();
    };
    if delta == 0 {
        return invalid();
    }
    let delta = delta.clamp(-60, 60);
    let (target, mouse_col, mouse_row) = match tmux_pane_at(opts, sess, col, row).await? {
        Ok(found) => found,
        Err(error) => return Ok(Some(error)),
    };
    // El `try` del Python: toda excepción de tmux es su texto.
    let run = |args: Vec<String>, fallback: &'static str, mutates: bool| {
        let target_args = args;
        async move {
            if mutates {
                mark(fx);
            }
            let refs: Vec<&str> = target_args.iter().map(String::as_str).collect();
            Ok::<_, Fault>(match caught(opts, &refs).await? {
                Ok(out) if out.ok => Ok(out),
                Ok(out) => Err(stderr_or(&out, fallback)),
                Err(message) => Err(message),
            })
        }
    };
    let owned = |args: &[&str]| -> Vec<String> { args.iter().map(|a| (*a).to_owned()).collect() };
    let mode = match run(
        owned(&[
            "display-message",
            "-p",
            "-t",
            &target,
            "#{mouse_sgr_flag}|#{mouse_utf8_flag}|#{mouse_standard_flag}|#{pane_in_mode}",
        ]),
        "No pude leer el modo del pane",
        false,
    )
    .await?
    {
        Ok(out) => out,
        Err(error) => return Ok(Some(error)),
    };
    let joined = format!("{}|||", py::strip(&mode.stdout));
    let fields: Vec<&str> = joined.split('|').collect();
    let field = |i: usize| fields.get(i).copied().unwrap_or("");
    let (sgr, utf8, standard, in_mode) = (field(0), field(1), field(2), field(3));
    if [sgr, utf8, standard].contains(&"1") {
        // Una app de pantalla completa: la rueda se reenvía sin activar el
        // ratón de la sesión.
        if in_mode == "1"
            && let Err(error) = run(
                owned(&["send-keys", "-X", "-t", &target, "cancel"]),
                "No pude cerrar el historial",
                true,
            )
            .await?
        {
            return Ok(Some(error));
        }
        let button: i64 = if delta < 0 { 64 } else { 65 };
        let ticks = ((delta.abs() + 2) / 3).clamp(1, 20);
        let ticks = usize::try_from(ticks).map_err(|_| failure())?;
        let args = if sgr == "1" {
            let one = format!("\x1b[<{button};{mouse_col};{mouse_row}M");
            owned(&["send-keys", "-l", "-t", &target, &one.repeat(ticks)])
        } else {
            let limit = if utf8 == "1" { 2015 } else { 223 };
            let code = |n: i64| char::from_u32(u32::try_from(n + 32).unwrap_or(0));
            let (Some(b), Some(c), Some(r)) = (
                code(button),
                code(mouse_col.clamp(1, limit)),
                code(mouse_row.clamp(1, limit)),
            ) else {
                return Err(failure());
            };
            let one = format!("\x1b[M{b}{c}{r}");
            let mut args = owned(&["send-keys", "-H", "-t", &target]);
            args.extend(one.repeat(ticks).bytes().map(|byte| format!("{byte:02x}")));
            args
        };
        return Ok(run(args, "No pude mover la app remota", true).await?.err());
    }
    if delta < 0 {
        if let Err(error) = run(
            owned(&["copy-mode", "-e", "-t", &target]),
            "No pude abrir el historial",
            true,
        )
        .await?
        {
            return Ok(Some(error));
        }
    } else if in_mode != "1" {
        return Ok(None);
    }
    let count = delta.abs().to_string();
    let way = if delta < 0 {
        "scroll-up"
    } else {
        "scroll-down"
    };
    Ok(run(
        owned(&["send-keys", "-X", "-N", &count, "-t", &target, way]),
        "No pude mover el historial",
        true,
    )
    .await?
    .err())
}

/// POST `/export` (9678).
async fn export(
    native: &Native,
    pt: &target::PostTarget,
    data: &Map<String, Value>,
    fx: &AtomicBool,
) -> Answer {
    // `data.get("format", "txt")` contra la tupla: solo esos dos textos.
    let fmt = match data.get("format") {
        None => "txt",
        Some(Value::String(f)) if f == "txt" || f == "pdf" => f.as_str(),
        Some(_) => return error_reply(StatusCode::BAD_REQUEST, "format debe ser txt o pdf"),
    };
    // `read_states()` sin caché, como el Python (que también reescribe
    // `app-tab-models.json`): un cómputo de `/state` que no sirve el resultado
    // guardado, dentro del vuelo único de la caché (ronda 1): nunca dos
    // cómputos a la vez (el rastreador de configuración no se pisa) y una
    // ráfaga de exportaciones hace uno solo.
    let clock = native.options().clock.clone();
    let states = native
        .states
        .cache
        .get_fresh(&*clock, || gather::compute(native))
        .await
        .map_err(Fault::from)?;
    mark(fx);
    let mut found = None;
    for item in states.items.iter() {
        // `i["session"]`: un `KeyError` es un 500.
        let session = item.get("session").ok_or_else(failure)?;
        if session.as_str() != Some(pt.sess.as_str()) {
            continue;
        }
        let detail = item.get("detail").filter(|v| truthy(v));
        let last = item.get("last").filter(|v| truthy(v));
        if let Some(text) = detail.or(last) {
            found = Some((item, text.clone()));
            break;
        }
    }
    let Some((item, detail)) = found else {
        return error_reply(
            StatusCode::NOT_FOUND,
            "Esa sesion no tiene respuesta guardada",
        );
    };
    // `item.get("project") or item["session"]`: un no texto es un `TypeError`
    // de `re.sub` (500).
    let title = match item.get("project") {
        Some(Value::String(p)) if !p.is_empty() => p.clone(),
        Some(v) if truthy(v) => return Err(failure()),
        _ => pt.sess.clone(),
    };
    let exported = export_response(native, &title, &detail, fmt, fx).await?;
    let path = match exported {
        Ok(path) => path,
        Err(error) => return error_reply(StatusCode::INTERNAL_SERVER_ERROR, &error),
    };
    open_url(native, &path).await;
    reply(StatusCode::OK, &json!({"ok": true, "path": path}))
}

/// `time.strftime(fmt)` en la hora local de `time.time()`.
fn local_time(opts: &NativeOptions, fmt: &str) -> Result<String, Fault> {
    let epoch = (opts.clock_seconds)().floor();
    if !epoch.is_finite() {
        return Err(Fault::Decline);
    }
    let epoch = epoch as i64;
    let offset = opts.zone.offset_at(epoch).ok_or(Fault::Decline)?;
    let local = chrono::DateTime::from_timestamp(epoch.saturating_add(offset), 0)
        .ok_or(Fault::Decline)?
        .naive_utc();
    Ok(local.format(fmt).to_string())
}

/// `os.path.expanduser(path)` con `HOME` = `opts.home` (`~` y `~/…`).
fn expand(home: &str, rest: &str) -> String {
    let home = home.trim_end_matches('/');
    let out = format!("{home}{rest}");
    if out.is_empty() { "/".to_owned() } else { out }
}

/// `export_dir()` (7902): `~/Descargas`, `~/Downloads` o `~`. Bloquea.
fn export_dir(home: &str) -> String {
    for rest in ["/Descargas", "/Downloads"] {
        let dir = expand(home, rest);
        if Path::new(&dir).is_dir() {
            return dir;
        }
    }
    expand(home, "")
}

/// `os.path.join(dir, name)`.
fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// `re.sub(r"[^A-Za-z0-9._-]", "-", title)[:60]`.
fn file_stem(title: &str) -> String {
    title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(60)
        .collect()
}

/// `open(path, "w").write(text)`; un `detail` que no es texto deja el
/// archivo vacío y lanza (`TypeError` de `write`, 500).
fn write_text(path: &str, text: Option<&str>) -> Result<(), Fault> {
    std::fs::write(path, text.unwrap_or("")).map_err(|_| failure())?;
    if text.is_none() {
        return Err(failure());
    }
    Ok(())
}

/// `export_response(item, fmt)` (7910): `Ok(Ok(ruta))` o `Ok(Err(error))`.
async fn export_response(
    native: &Native,
    title: &str,
    detail: &Value,
    fmt: &str,
    fx: &AtomicBool,
) -> Result<Result<String, String>, Fault> {
    let opts = native.options();
    let ts = local_time(opts, "%Y-%m-%d_%H%M")?;
    let home = opts.home.to_str().ok_or(Fault::Decline)?.to_owned();
    let stem = format!("claude-{}-{ts}", file_stem(title));
    let base = blocking(move || Ok(join(&export_dir(&home), &stem))).await?;
    let text = detail.as_str().map(str::to_owned);
    if fmt == "txt" {
        let path = format!("{base}.txt");
        let target = path.clone();
        mark(fx);
        blocking(move || write_text(&target, text.as_deref())).await?;
        return Ok(Ok(path));
    }
    let search = opts.search_path.clone();
    let chrome = blocking(move || {
        Ok(CHROMES
            .iter()
            .find_map(|name| procs::which_in(search.as_deref(), name)))
    })
    .await?;
    let Some(chrome) = chrome else {
        return Ok(Err("Para PDF instala Chrome o Chromium".to_owned()));
    };
    let when = local_time(opts, "%Y-%m-%d %H:%M")?;
    // `md_to_html` de un no texto lanza (`AttributeError`, 500).
    let markdown = text.ok_or_else(failure)?;
    let doc = format!(
        "<!doctype html><meta charset='utf-8'><style>{PDF_CSS}</style><h1>{}</h1><div class='meta'>Respuesta de Claude · {when} · ComandOS</div>{}",
        html_escape(title),
        md_to_html(&markdown)
    );
    let tmp = format!("{base}.html");
    let pdf = format!("{base}.pdf");
    let target = tmp.clone();
    mark(fx);
    blocking(move || write_text(&target, Some(&doc))).await?;
    let printed = format!("--print-to-pdf={pdf}");
    let args = [
        "--headless",
        "--disable-gpu",
        printed.as_str(),
        "--no-pdf-header-footer",
        tmp.as_str(),
    ];
    let ran = run_program(
        &opts.program(chrome),
        &args,
        Duration::from_secs(CHROME_SECONDS),
    )
    .await;
    // El `finally`: el HTML temporal se borra pase lo que pase.
    let (remove, check) = (tmp.clone(), pdf.clone());
    let exists = blocking(move || {
        let _ = std::fs::remove_file(&remove);
        Ok(Path::new(&check).exists())
    })
    .await?;
    let out = ran.map_err(|e| run_fault(&e))?;
    if !exists {
        let stderr = py::take_chars(py::strip(&out.stderr), 200);
        let error = if stderr.is_empty() {
            "Chrome no pudo generar el PDF".to_owned()
        } else {
            stderr
        };
        return Ok(Err(error));
    }
    Ok(Ok(pdf))
}

/// `_open_url(path, env=gui_env())` (34): `wslview` en WSL si existe; si no,
/// `xdg-open` (`open` en macOS). Suelto, sin esperar; todo error se ignora.
async fn open_url(native: &Native, url: &str) {
    let opts = native.options();
    let osrelease = opts.proc_root.join("sys/kernel/osrelease");
    let search = opts.search_path.clone();
    let opener = blocking(move || {
        let wsl = std::fs::read_to_string(&osrelease)
            .is_ok_and(|text| text.to_lowercase().contains("microsoft"));
        let wslview = procs::which_in(search.as_deref(), "wslview");
        let name = if cfg!(target_os = "macos") {
            "open"
        } else if wsl && wslview.is_some() {
            "wslview"
        } else {
            "xdg-open"
        };
        Ok(procs::which_in(search.as_deref(), name))
    })
    .await;
    let Ok(Some(path)) = opener else {
        return;
    };
    let program = opts.program(path);
    let _ = procs::spawn_detached(&program, &[OsString::from(url)], &procs::gui_env_for(opts));
}

/// `html.escape(s)` (con `quote=True`).
pub fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            c => out.push(c),
        }
    }
    out
}

/// Las expresiones fijas de `_md_inline` que el motor `regex` reproduce tal
/// cual (sin la de cursiva, que lleva una anticipación: `italic`). `\s` de
/// Python incluye `\x1c`–`\x1f`; el de `regex`, no.
fn inline_rules() -> &'static [(regex::Regex, &'static str)] {
    static RULES: std::sync::OnceLock<Vec<(regex::Regex, &'static str)>> =
        std::sync::OnceLock::new();
    RULES.get_or_init(|| {
        [
            (r"\*\*(.+?)\*\*", "<b>${1}</b>"),
            (r"`([^`]+)`", "<code>${1}</code>"),
            (r"\[([^\]]+)\]\([^)\s\x1C-\x1F]*\)", "${1}"),
        ]
        .into_iter()
        .filter_map(|(re, to)| regex::Regex::new(re).ok().map(|re| (re, to)))
        .collect()
    })
}

/// `$` de Python sin `MULTILINE`: al final o ante un `\n` final.
fn at_end(chars: &[char], i: usize) -> bool {
    i == chars.len() || (chars.get(i) == Some(&'\n') && i + 1 == chars.len())
}

/// `re.sub(r"(^|[\s(])\*([^*\n]+)\*(?=$|[\s).,;:!?])", r"\1<i>\2</i>", l)`.
fn italic(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    // Desde `p` (un `*`): fin del cuerpo y del cierre, si casa.
    let body = |p: usize| -> Option<(usize, usize)> {
        if chars.get(p) != Some(&'*') {
            return None;
        }
        let mut q = p + 1;
        while chars.get(q).is_some_and(|c| *c != '*' && *c != '\n') {
            q += 1;
        }
        if q == p + 1 || chars.get(q) != Some(&'*') {
            return None;
        }
        let after = q + 1;
        let ahead = at_end(&chars, after)
            || chars
                .get(after)
                .is_some_and(|c| py::is_space(*c) || ").,;:!?".contains(*c));
        ahead.then_some((q, after))
    };
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    let mut copied = 0;
    while i < chars.len() {
        // Alternativas de `(^|[\s(])` en su orden.
        let mut matched = None;
        if i == 0
            && let Some((q, end)) = body(0)
        {
            matched = Some((0, q, end));
        }
        if matched.is_none()
            && chars.get(i).is_some_and(|c| py::is_space(*c) || *c == '(')
            && let Some((q, end)) = body(i + 1)
        {
            matched = Some((i + 1, q, end));
        }
        match matched {
            Some((star, q, end)) => {
                out.extend(chars.get(copied..i).unwrap_or_default());
                out.extend(chars.get(i..star).unwrap_or_default());
                out.push_str("<i>");
                out.extend(chars.get(star + 1..q).unwrap_or_default());
                out.push_str("</i>");
                copied = end;
                i = end;
            }
            None => i += 1,
        }
    }
    out.extend(chars.get(copied..).unwrap_or_default());
    out
}

/// `_md_inline(l)` (7832).
fn md_inline(line: &str) -> String {
    let rules = inline_rules();
    let mut out = line.to_owned();
    if let Some((re, to)) = rules.first() {
        out = re.replace_all(&out, *to).into_owned();
    }
    out = italic(&out);
    for (re, to) in rules.iter().skip(1) {
        out = re.replace_all(&out, *to).into_owned();
    }
    out
}

/// `re.match(r"^\s*\|.*\|\s*$", l)`.
fn is_table_row(line: &str) -> bool {
    let trimmed = line.trim_start_matches(py::is_space);
    let Some(rest) = trimmed.strip_prefix('|') else {
        return false;
    };
    rest.trim_end_matches(py::is_space).ends_with('|')
}

/// `re.match(r"^\s*\|[\s\-:|]+\|\s*$", r)`: la fila separadora.
fn is_separator(row: &str) -> bool {
    let trimmed = row.trim_start_matches(py::is_space);
    let Some(rest) = trimmed.strip_prefix('|') else {
        return false;
    };
    if !rest
        .chars()
        .all(|c| py::is_space(c) || matches!(c, '-' | ':' | '|'))
    {
        return false;
    }
    let body = rest.trim_end_matches(py::is_space);
    body.ends_with('|') && body.chars().count() >= 2
}

/// `flush_tbl()` de `md_to_html`.
fn flush_table(table: &mut Vec<&str>, out: &mut String) {
    if table.is_empty() {
        return;
    }
    out.push_str("<table>");
    let rows = table.iter().filter(|r| !is_separator(r));
    for (i, row) in rows.enumerate() {
        let tag = if i == 0 { "th" } else { "td" };
        out.push_str("<tr>");
        for cell in py::strip(row).trim_matches('|').split('|') {
            let cell = md_inline(&html_escape(py::strip(cell)));
            out.push_str(&format!("<{tag}>{cell}</{tag}>"));
        }
        out.push_str("</tr>");
    }
    out.push_str("</table>");
    table.clear();
}

/// `re.match(r"^#{1,6} (.*)", esc)`: el título sin las almohadillas.
fn heading(esc: &str) -> Option<&str> {
    let hashes = esc.len() - esc.trim_start_matches('#').len();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    esc.get(hashes..)?.strip_prefix(' ')
}

/// `re.match(r"^\s*---+\s*$", esc)`.
fn is_rule(esc: &str) -> bool {
    let core = py::strip(esc);
    core.len() >= 3 && core.chars().all(|c| c == '-')
}

/// `re.sub(r"^(\s*)[-*] ", "\\1• ", esc)`.
fn bullet(esc: &str) -> String {
    let rest = esc.trim_start_matches(py::is_space);
    let lead = esc.get(..esc.len() - rest.len()).unwrap_or("");
    match rest.strip_prefix("- ").or_else(|| rest.strip_prefix("* ")) {
        Some(tail) => format!("{lead}• {tail}"),
        None => esc.to_owned(),
    }
}

/// `md_to_html(s)` (7840): el Markdown del tablero a HTML para el PDF.
pub fn md_to_html(text: &str) -> String {
    let mut out = String::new();
    let mut table: Vec<&str> = Vec::new();
    let mut in_code = false;
    for line in py::splitlines(text) {
        if py::strip(line).starts_with("```") {
            flush_table(&mut table, &mut out);
            in_code = !in_code;
            out.push_str(if in_code { "<pre>" } else { "</pre>" });
            continue;
        }
        if in_code {
            out.push_str(&html_escape(line));
            out.push('\n');
            continue;
        }
        if is_table_row(line) {
            table.push(line);
            continue;
        }
        flush_table(&mut table, &mut out);
        let esc = html_escape(line);
        if let Some(title) = heading(&esc) {
            out.push_str(&format!("<h3>{}</h3>", md_inline(title)));
            continue;
        }
        if is_rule(&esc) {
            out.push_str("<hr>");
            continue;
        }
        out.push_str(&md_inline(&bullet(&esc)));
        out.push_str("<br>");
    }
    flush_table(&mut table, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn italic_needs_a_boundary_on_both_sides() {
        assert_eq!(italic("*a* b"), "<i>a</i> b");
        assert_eq!(italic("x *a*, (*b*)"), "x <i>a</i>, (<i>b</i>)");
        assert_eq!(italic("a*b*c"), "a*b*c");
        assert_eq!(italic("**"), "**");
    }

    #[test]
    fn separator_rows_like_python() {
        assert!(is_separator("|---|:-:|"));
        assert!(is_separator("  | - |  "));
        assert!(!is_separator("||"));
        assert!(!is_separator("| x |"));
    }

    #[test]
    fn file_stem_replaces_and_cuts() {
        assert_eq!(file_stem("Mi proyecto/ñ"), "Mi-proyecto--");
        assert_eq!(file_stem(&"a".repeat(70)).len(), 60);
    }
}
