//! Native dashboard transport. Construction never discovers live application state.
pub mod blocking;
pub mod dash;
pub mod events_routes;
pub mod ws;
pub use ws::{WsAdmission, WsHandler, WsRequest, WsRoute};
mod write_timeout;
use bytes::Bytes;
use comandos_core::dashboard_access as access;
use futures_util::FutureExt;
use http::{HeaderMap, Method, StatusCode};
use http_body::{Body, Frame, SizeHint};
use http_body_util::BodyExt;
use hyper::{body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::Value;
use std::{
    convert::Infallible,
    future::Future,
    io,
    net::SocketAddr,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, mpsc, watch},
    task::JoinSet,
    time::timeout,
};

#[derive(Clone)]
pub struct Request {
    pub method: Method,
    pub target: String,
    pub peer: SocketAddr,
    /// Header names are lowercase; duplicates of the same name retain order.
    /// Non-ASCII header bytes use HTTP's Latin-1 mapping, matching Python.
    pub headers: Vec<(String, String)>,
    /// GET has no parsed body; admitted POST/DELETE always has an object.
    pub data: Option<Value>,
    /// Raw admitted body bytes, exactly as received (empty for GET/HEAD).
    /// `data` is parsed from these same bytes; relays forward them untouched.
    pub body: Bytes,
    /// Direct local hook/service authentication, stricter than ordinary access.
    pub internal_producer: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum HandlerError {
    Timeout,
    Failure,
}

pub enum ReplyBody {
    Bytes(Bytes),
    /// Producers own their task cancellation and must bound channel capacity
    /// and frame size. Dropping the response drops the receiver.
    Stream(mpsc::Receiver<io::Result<Bytes>>),
    /// A stream whose total length is known up front (a relayed
    /// `Content-Length`). The transport frames it with that length instead of
    /// chunking; yielding more or fewer bytes aborts the connection.
    SizedStream {
        length: u64,
        receiver: mpsc::Receiver<io::Result<Bytes>>,
    },
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReplyCache {
    #[default]
    NoStore,
    /// Only versioned build artifacts opt into persistent caching.
    Immutable,
}
pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: ReplyBody,
    pub cache: ReplyCache,
}
impl Reply {
    pub fn bytes(status: StatusCode, content_type: &'static str, body: impl Into<Bytes>) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static(content_type),
        );
        Self {
            status,
            headers,
            body: ReplyBody::Bytes(body.into()),
            cache: ReplyCache::NoStore,
        }
    }
    pub fn json(status: StatusCode, value: &Value) -> Result<Self, HandlerError> {
        let raw = comandos_core::json::response_dumps(value).map_err(|_| HandlerError::Failure)?;
        Ok(Self::bytes(status, "application/json", raw))
    }
}
pub type HandlerFuture = Pin<Box<dyn Future<Output = Result<Reply, HandlerError>> + Send>>;
pub type Handler = Arc<dyn Fn(Request) -> HandlerFuture + Send + Sync>;
/// Receives a validated absolute URL path without query or fragment. The
/// adapter supplies the installed asset root and follows its intended symlinks.
pub type AssetExists = Arc<dyn Fn(&str) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct Limits {
    pub connections: usize,
    pub websockets: usize,
    /// Maximum HTTP/1 connection read buffer, including request headers.
    pub header_bytes: usize,
    /// Aggregate admitted wire bytes, held through handler completion.
    /// Parsed JSON and responses have additional allocations owned by adapters.
    pub buffered_wire_bytes: u32,
    pub header_timeout: Duration,
    pub body_timeout: Duration,
    pub handler_timeout: Duration,
    /// Maximum wait for progress on a pending socket write/flush. Waiting for
    /// the next stream event does not consume this deadline.
    pub write_timeout: Duration,
    pub shutdown_grace: Duration,
}
pub struct Config {
    /// Token del arranque (validado por `serve`).
    pub token: Vec<u8>,
    /// Archivo del token (`~/.claude/hooks/dash-token` del tablero): si está,
    /// la puerta usa lo que dice el archivo en cada petición, como el Python
    /// (rotarlo surte efecto sin reiniciar). `None`: siempre `token`.
    pub token_file: Option<std::path::PathBuf>,
    pub asset_exists: AssetExists,
    pub handler: Handler,
    pub websocket: Option<WsRoute>,
    pub limits: Limits,
}

struct State {
    config: Config,
    token: dash::token::TokenCache,
    body_budget: Arc<Semaphore>,
    websockets: Arc<Semaphore>,
    ws_tasks: std::sync::Mutex<JoinSet<()>>,
    shutdown: watch::Receiver<bool>,
}

struct OutputBody {
    source: ReplyBody,
    done: bool,
}
fn length_mismatch() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "stream length differs from its declared length",
    )
}
impl Body for OutputBody {
    type Data = Bytes;
    type Error = io::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<io::Result<Frame<Bytes>>>> {
        if self.done {
            return Poll::Ready(None);
        }
        match &mut self.source {
            ReplyBody::Bytes(bytes) => {
                let data = std::mem::take(bytes);
                self.done = true;
                Poll::Ready((!data.is_empty()).then_some(Ok(Frame::data(data))))
            }
            ReplyBody::Stream(receiver) => match receiver.poll_recv(cx) {
                Poll::Ready(Some(Ok(bytes))) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
                Poll::Ready(Some(Err(error))) => {
                    self.done = true;
                    Poll::Ready(Some(Err(error)))
                }
                Poll::Ready(None) => {
                    self.done = true;
                    Poll::Ready(None)
                }
                Poll::Pending => Poll::Pending,
            },
            ReplyBody::SizedStream { length, receiver } => match receiver.poll_recv(cx) {
                Poll::Ready(Some(Ok(bytes))) => match length.checked_sub(bytes.len() as u64) {
                    Some(rest) => {
                        *length = rest;
                        Poll::Ready(Some(Ok(Frame::data(bytes))))
                    }
                    None => {
                        self.done = true;
                        Poll::Ready(Some(Err(length_mismatch())))
                    }
                },
                Poll::Ready(Some(Err(error))) => {
                    self.done = true;
                    Poll::Ready(Some(Err(error)))
                }
                Poll::Ready(None) => {
                    let short = *length != 0;
                    self.done = true;
                    Poll::Ready(short.then(|| Err(length_mismatch())))
                }
                Poll::Pending => Poll::Pending,
            },
        }
    }
    fn is_end_stream(&self) -> bool {
        self.done || matches!(&self.source, ReplyBody::Bytes(b) if b.is_empty())
    }
    fn size_hint(&self) -> SizeHint {
        match &self.source {
            ReplyBody::Bytes(bytes) => SizeHint::with_exact(bytes.len() as u64),
            ReplyBody::Stream(_) | ReplyBody::SizedStream { .. } if self.done => {
                SizeHint::with_exact(0)
            }
            ReplyBody::Stream(_) => SizeHint::default(),
            ReplyBody::SizedStream { length, .. } => SizeHint::with_exact(*length),
        }
    }
}

/// Large JSON answers travel gzipped to clients that accept it: the phone
/// polls `/state` and the feed every few seconds over the tailnet.
fn compressed(mut reply: Reply) -> Reply {
    const MIN_BYTES: usize = 16 * 1024;
    let ReplyBody::Bytes(body) = &reply.body else {
        return reply;
    };
    let json = reply
        .headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if body.len() < MIN_BYTES
        || !json
        || reply.status != StatusCode::OK
        || reply.headers.contains_key(http::header::CONTENT_ENCODING)
    {
        return reply;
    }
    let mut encoder = flate2::write::GzEncoder::new(
        Vec::with_capacity(body.len() / 6),
        flate2::Compression::new(3),
    );
    if std::io::Write::write_all(&mut encoder, body).is_err() {
        return reply;
    }
    let Ok(packed) = encoder.finish() else {
        return reply;
    };
    reply.body = ReplyBody::Bytes(packed.into());
    reply.headers.insert(
        http::header::CONTENT_ENCODING,
        http::HeaderValue::from_static("gzip"),
    );
    reply.headers.insert(
        http::header::VARY,
        http::HeaderValue::from_static("accept-encoding"),
    );
    reply
}

fn response(reply: Reply, close: bool) -> http::Response<OutputBody> {
    let declared = match &reply.body {
        ReplyBody::SizedStream { length, .. } => Some(*length),
        _ => None,
    };
    let mut response = http::Response::new(OutputBody {
        source: reply.body,
        done: false,
    });
    *response.status_mut() = reply.status;
    *response.headers_mut() = reply.headers;
    // The maintained transport derives framing from the actual body. An adapter
    // cannot accidentally send conflicting framing. Cache policy is explicit;
    // relayed Cache-Control headers never make an API response cacheable.
    let headers = response.headers_mut();
    match declared {
        // `insert` keeps a relayed header in its original position.
        Some(length) => {
            headers.insert(
                http::header::CONTENT_LENGTH,
                http::HeaderValue::from(length),
            );
        }
        None => {
            headers.remove(http::header::CONTENT_LENGTH);
        }
    }
    headers.remove(http::header::TRANSFER_ENCODING);
    headers.insert(
        http::header::CACHE_CONTROL,
        http::HeaderValue::from_static(match reply.cache {
            ReplyCache::NoStore => "no-store",
            ReplyCache::Immutable => "public, max-age=31536000, immutable",
        }),
    );
    if close {
        headers.insert(
            http::header::CONNECTION,
            http::HeaderValue::from_static("close"),
        );
    }
    response
}
fn reject(status: u16, message: &'static str, close: bool) -> http::Response<OutputBody> {
    let status = StatusCode::from_u16(status).expect("internal HTTP status");
    // Static internal errors cannot exceed the core encoder's depth limit.
    let reply =
        Reply::json(status, &serde_json::json!({"error":message})).expect("static JSON error");
    response(reply, close)
}

async fn body_bytes(mut body: Incoming, length: usize) -> Result<Bytes, ()> {
    let mut bytes = Vec::with_capacity(length);
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| ())?;
        if let Ok(data) = frame.into_data() {
            if data.len() > length.saturating_sub(bytes.len()) {
                return Err(());
            }
            bytes.extend_from_slice(&data);
        }
    }
    if bytes.len() != length {
        return Err(());
    }
    // Zero-copy: the buffer becomes the shared `Request.body`.
    Ok(Bytes::from(bytes))
}

async fn dispatch(
    request: hyper::Request<Incoming>,
    peer: SocketAddr,
    state: Arc<State>,
) -> http::Response<OutputBody> {
    let (parts, body) = request.into_parts();
    // Hyper has already rejected conflicting lengths. It removes lengths in the
    // presence of Transfer-Encoding, so reject that whole unsupported request mode.
    if parts.headers.contains_key(http::header::TRANSFER_ENCODING) {
        return reject(400, "JSON invalido", true);
    }
    let method = match parts.method.as_str() {
        // HEAD passes the same admission as GET; hyper omits the response body.
        // Adapters receive Method::HEAD and decide whether the route has one.
        "GET" | "HEAD" => access::Method::Get,
        "POST" => access::Method::Post,
        "DELETE" => access::Method::Delete,
        _ => return reject(501, "Método no implementado", true),
    };
    let target = parts.uri.to_string();
    let peer_ip = peer.ip().to_string();
    let headers: Vec<(String, String)> = parts
        .headers
        .iter()
        .map(|(key, value)| {
            (
                key.as_str().to_owned(),
                value
                    .as_bytes()
                    .iter()
                    .map(|byte| char::from(*byte))
                    .collect(),
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let policy_request = access::Request {
        method,
        path: &target,
        peer_ip: Some(&peer_ip),
        headers: &borrowed,
    };
    let file_exists = if method == access::Method::Get
        && access::API_GET
            .iter()
            .any(|prefix| target.starts_with(prefix))
        && access::public_asset(&target, true).unwrap_or(false)
    {
        // Invalid hosts are rejected before consulting an injected filesystem fact.
        let hosts: Vec<_> = borrowed
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("host"))
            .collect();
        if hosts.len() == 1 && access::authority_matches(access::AuthorityKind::Allowed, hosts[0].1)
        {
            match std::panic::catch_unwind(AssertUnwindSafe(|| {
                (state.config.asset_exists)(parts.uri.path())
            })) {
                Ok(exists) => exists,
                Err(_) => return reject(500, "Error interno del tablero", true),
            }
        } else {
            false
        }
    } else {
        false
    };
    // El token vigente (un `stat` del archivo; se relee solo si cambió).
    let expected = state.token.current();
    let admission = match access::request_admission(&policy_request, &expected, file_exists) {
        Ok(admission) => admission,
        Err(error) => {
            return reject(
                error.status,
                error.message,
                error.close || !body.is_end_stream(),
            );
        }
    };
    let internal_producer = access::internal_producer(&policy_request, &expected);
    if parts.method == Method::POST && parts.uri.path() == "/web/markdown" {
        if !access::token_matches(
            access::presented_token(&borrowed, &target).as_bytes(),
            &expected,
        ) {
            return reject(401, "No autorizado", true);
        }
        if admission
            .body_length
            .is_some_and(|n| n > dash::web::markdown::MAX_INPUT_BYTES)
        {
            return reject(413, "texto demasiado largo", true);
        }
    }
    if method == access::Method::Get && !body.is_end_stream() {
        // No GET endpoint consumes a body. Closing prevents it being mistaken for
        // a subsequent keep-alive request; this is explicit wire hardening.
        return reject(400, "JSON invalido", true);
    }
    if ws::is_upgrade(&parts.headers)
        && let Some(route) = state.config.websocket.as_ref()
    {
        if parts.method != Method::GET {
            return reject(400, "WebSocket invalido", true);
        }
        let allowed = match std::panic::catch_unwind(AssertUnwindSafe(|| {
            (route.accepts)(parts.uri.path())
        })) {
            Ok(Some(allowed)) => allowed,
            Ok(None) => return reject(404, "No encontrado", true),
            Err(_) => return reject(500, "Error interno del tablero", true),
        };
        let protocol = match ws::pick_protocol(&parts.headers, allowed) {
            Ok(protocol) => protocol,
            Err(_) => return reject(400, "Subprotocolo no soportado", true),
        };
        let admission =
            match std::panic::catch_unwind(AssertUnwindSafe(|| route.admission(protocol))) {
                Ok(admission) => admission,
                Err(_) => return reject(500, "Error interno del tablero", true),
            };
        let strict_origin = matches!(
            admission,
            WsAdmission::HandlerTokenOrigin | WsAdmission::DashboardTokenOrigin
        );
        if strict_origin || admission == WsAdmission::HandlerToken {
            // GET admission already checked exactly one allowed Host.
            let host = borrowed
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("host"))
                .map(|(_, value)| *value)
                .unwrap_or("");
            let origins: Vec<_> = borrowed
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case("origin"))
                .collect();
            if origins.len() > 1
                || (strict_origin && origins.is_empty())
                || origins
                    .iter()
                    .any(|(_, origin)| !access::origin_matches_host(origin, host))
            {
                return reject(403, "Origen no permitido", true);
            }
        }
        if matches!(
            admission,
            WsAdmission::Dashboard | WsAdmission::DashboardTokenOrigin
        ) {
            if let Some(error) = access::security_gate(&policy_request, &expected) {
                return reject(error.status, error.message, true);
            }
            if admission == WsAdmission::DashboardTokenOrigin
                && !access::token_matches(
                    access::presented_token(&borrowed, &target).as_bytes(),
                    &expected,
                )
            {
                return reject(
                    401,
                    "No autorizado (token requerido para acceso remoto)",
                    true,
                );
            }
        }
        return websocket(
            state.clone(),
            parts,
            peer,
            headers,
            internal_producer,
            protocol,
            route.handler.clone(),
        );
    }
    let mut budget = None;
    let mut raw_body = Bytes::new();
    let data = if let Some(length) = admission.body_length {
        let permits = u32::try_from(length).expect("core policy body cap");
        let Ok(permit) = state.body_budget.clone().try_acquire_many_owned(permits) else {
            return reject(503, "Tablero ocupado", true);
        };
        budget = Some(permit);
        let raw = match timeout(state.config.limits.body_timeout, body_bytes(body, length)).await {
            Ok(Ok(raw)) => raw,
            _ => return reject(400, "JSON invalido", true),
        };
        let parsed = if raw.is_empty() {
            Some(serde_json::json!({}))
        } else if parts.method == Method::POST && parts.uri.path() == "/web/markdown" {
            dash::web::markdown::parse_request(&raw)
        } else {
            comandos_core::json::workspace_loads_bytes(&raw)
        };
        if let Some(error) = access::parsed_body_admission(parsed.as_ref()) {
            return reject(error.status, error.message, error.close);
        }
        raw_body = raw;
        parsed
    } else {
        None
    };
    let gzip = headers
        .iter()
        .any(|(name, value)| name == "accept-encoding" && value.contains("gzip"));
    let request = Request {
        method: parts.method,
        target,
        peer,
        headers,
        data,
        body: raw_body,
        internal_producer,
    };
    let future =
        match std::panic::catch_unwind(AssertUnwindSafe(|| (state.config.handler)(request))) {
            Ok(future) => future,
            Err(_) => return reject(500, "Error interno del tablero", true),
        };
    let result = timeout(
        state.config.limits.handler_timeout,
        AssertUnwindSafe(future).catch_unwind(),
    )
    .await;
    // Keep admission capacity through parsing and handler completion/cancellation.
    drop(budget);
    match result {
        Ok(Ok(Ok(reply))) => response(if gzip { compressed(reply) } else { reply }, false),
        Ok(Ok(Err(HandlerError::Timeout))) | Err(_) => {
            reject(504, "Tiempo de espera agotado", true)
        }
        _ => reject(500, "Error interno del tablero", true),
    }
}

fn websocket(
    state: Arc<State>,
    mut parts: http::request::Parts,
    peer: SocketAddr,
    headers: Vec<(String, String)>,
    internal_producer: bool,
    protocol: Option<&'static str>,
    handler: WsHandler,
) -> http::Response<OutputBody> {
    let key = match ws::handshake_key(&parts.headers) {
        Ok(key) => key,
        Err(_) => return reject(400, "WebSocket invalido", true),
    };
    let Ok(permit) = state.websockets.clone().try_acquire_owned() else {
        return reject(503, "Tablero ocupado", true);
    };
    let Some(upgrade) = parts.extensions.remove::<hyper::upgrade::OnUpgrade>() else {
        return reject(400, "WebSocket invalido", true);
    };
    let request = WsRequest {
        target: parts
            .uri
            .path_and_query()
            .map(|p| p.as_str().to_owned())
            .unwrap_or_default(),
        peer,
        headers,
        protocol,
        internal_producer,
        shutdown: state.shutdown.clone(),
    };
    state
        .ws_tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .spawn(async move {
            let _permit = permit;
            if let Ok(stream) = ws::into_stream(upgrade).await {
                let _ = AssertUnwindSafe(async move { handler(request, stream).await })
                    .catch_unwind()
                    .await;
            }
        });
    let response = ws::switching_response(&key, protocol);
    let (parts, _) = response.into_parts();
    http::Response::from_parts(
        parts,
        OutputBody {
            source: ReplyBody::Bytes(Bytes::new()),
            done: false,
        },
    )
}

/// Run only on the supplied listener. The caller owns bind addresses, token
/// loading, endpoint routing, and the shutdown signal. No implicit defaults.
pub async fn serve(
    listener: TcpListener,
    config: Config,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    let limits = &config.limits;
    if config.token.is_empty()
        || std::str::from_utf8(&config.token).is_err()
        || limits.connections == 0
        || limits.connections > Semaphore::MAX_PERMITS
        || limits.websockets > Semaphore::MAX_PERMITS
        || (config.websocket.is_some() && limits.websockets == 0)
        || limits.buffered_wire_bytes == 0
        || !(8192..=8 * 1024 * 1024).contains(&limits.header_bytes)
        || [
            limits.header_timeout,
            limits.body_timeout,
            limits.handler_timeout,
            limits.write_timeout,
            limits.shutdown_grace,
        ]
        .iter()
        .any(Duration::is_zero)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid transport configuration",
        ));
    }
    let connections = Arc::new(Semaphore::new(limits.connections));
    let token = dash::token::TokenCache::new(config.token.clone(), config.token_file.clone())?;
    let (stop_connections, connection_shutdown) = watch::channel(false);
    let state = Arc::new(State {
        body_budget: Arc::new(Semaphore::new(limits.buffered_wire_bytes as usize)),
        websockets: Arc::new(Semaphore::new(limits.websockets)),
        ws_tasks: std::sync::Mutex::new(JoinSet::new()),
        shutdown: connection_shutdown.clone(),
        token,
        config,
    });
    let mut tasks = JoinSet::new();
    let mut websocket_reap = tokio::time::interval(Duration::from_secs(1));
    let result = loop {
        while state
            .ws_tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .try_join_next()
            .is_some()
        {}
        if *shutdown.borrow() {
            break Ok(());
        }
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { break Ok(()); }
            }
            _ = tasks.join_next(), if !tasks.is_empty() => {}
            _ = websocket_reap.tick() => {}
            accepted = listener.accept() => {
                let (stream,peer) = match accepted { Ok(pair)=>pair, Err(error)=>break Err(error) };
                let Ok(permit) = connections.clone().try_acquire_owned() else { drop(stream); continue; };
                let state = state.clone();
                let mut shutdown = connection_shutdown.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    let service_state = state.clone();
                    let service = service_fn(move |request| {
                        let state = service_state.clone();
                        async move { Ok::<_,Infallible>(dispatch(request,peer,state).await) }
                    });
                    let mut builder = http1::Builder::new();
                    builder.keep_alive(true).timer(TokioTimer::new())
                        .header_read_timeout(state.config.limits.header_timeout)
                        .max_headers(100).max_buf_size(state.config.limits.header_bytes)
                        .pipeline_flush(false);
                    let stream = write_timeout::WriteTimeout::new(stream, state.config.limits.write_timeout);
                    let connection = builder.serve_connection(TokioIo::new(stream),service).with_upgrades();
                    tokio::pin!(connection);
                    tokio::select! {
                        _ = &mut connection => {}
                        _ = shutdown.changed() => {
                            connection.as_mut().graceful_shutdown();
                            let _ = connection.await;
                        }
                    }
                });
            }
        }
    };
    drop(listener);
    let _ = stop_connections.send(true);
    if timeout(state.config.limits.shutdown_grace, async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
    let mut websocket_tasks = std::mem::take(
        &mut *state
            .ws_tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    if timeout(state.config.limits.shutdown_grace, async {
        while websocket_tasks.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        websocket_tasks.abort_all();
        while websocket_tasks.join_next().await.is_some() {}
    }
    result
}
