//! Un PTY por socket. Solo se mantiene una trama de salida hasta que el sink
//! la haya vaciado: mientras tanto no se lee del PTY. La memoria del transporte
//! (TCP/tungstenite) se configura por quien acepta el WebSocket, no se cuenta aquí.
use super::attach::{AttachCommand, TmuxTarget, supports_active_pane, valid_session};
use comandos_term::proto::{self, ClientMsg, Dialect, Init, ProtoError};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use std::{io, path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
};
use tokio_tungstenite::tungstenite::{
    Error as WsError, Message,
    protocol::{CloseFrame, frame::coding::CloseCode},
};

#[derive(Debug, Clone, Copy)]
pub struct BridgeLimits {
    /// Límite de la trama de salida PTY/replay; título, prefs y selector son controles.
    pub outbox_bytes: usize,
    pub read_chunk: usize,
}
impl Default for BridgeLimits {
    fn default() -> Self {
        Self {
            outbox_bytes: 1 << 20,
            read_chunk: 64 << 10,
        }
    }
}
pub enum Source {
    Pty {
        target: TmuxTarget,
        session: Option<String>,
    },
    Replay(PathBuf),
}
#[derive(Debug, Default)]
pub struct BridgeStats {
    /// Máximo de bytes en una trama pendiente del puente (incluye prefijo).
    pub max_outbox_bytes: usize,
    /// Bytes PTY/replay leídos y preparados; no confirma recepción del cliente.
    pub output_bytes: usize,
    pub child_pid: Option<u32>,
    pub child_reaped: bool,
}
fn ws_error(error: WsError) -> io::Error {
    match error {
        WsError::Io(error) => error,
        WsError::ConnectionClosed | WsError::AlreadyClosed => {
            io::Error::new(io::ErrorKind::ConnectionAborted, error)
        }
        error => io::Error::other(error),
    }
}
fn stopped(stop: &watch::Receiver<bool>) -> bool {
    *stop.borrow()
}
async fn stop_wait(stop: &mut watch::Receiver<bool>) {
    if stopped(stop) {
        return;
    }
    while stop.changed().await.is_ok() {
        if stopped(stop) {
            return;
        }
    }
}
async fn send<S>(sink: &mut S, bytes: Vec<u8>, stop: &mut watch::Receiver<bool>) -> io::Result<()>
where
    S: Sink<Message, Error = WsError> + Unpin,
{
    tokio::select! {
        _ = stop_wait(stop) => Err(io::Error::new(io::ErrorKind::Interrupted, "shutdown")),
        result = tokio::time::timeout(Duration::from_secs(2), sink.send(Message::binary(bytes))) => result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut,"socket"))?.map_err(ws_error),
    }
}
async fn probe(target: &TmuxTarget, command: AttachCommand) -> Option<std::process::Output> {
    tokio::time::timeout(Duration::from_secs(2), target.probe(&command).output())
        .await
        .ok()?
        .ok()
}
struct Picked {
    command: Option<AttachCommand>,
    tail: Vec<u8>,
    skip_lf: bool,
}
async fn pick<S, R>(
    sink: &mut S,
    stream: &mut R,
    target: &TmuxTarget,
    requested: Option<String>,
    init: &mut Init,
    stop: &mut watch::Receiver<bool>,
) -> io::Result<Picked>
where
    S: Sink<Message, Error = WsError> + Unpin,
    R: Stream<Item = Result<Message, WsError>> + Unpin,
{
    let active_pane = probe(target, AttachCommand::Version)
        .await
        .is_some_and(|out| supports_active_pane(&String::from_utf8_lossy(&out.stdout)));
    if let Some(session) = requested.filter(|s| valid_session(s)) {
        if probe(target, AttachCommand::HasSession(session.clone()))
            .await
            .is_some_and(|out| out.status.success())
        {
            return Ok(Picked {
                command: Some(AttachCommand::Attach {
                    session,
                    active_pane,
                }),
                tail: Vec::new(),
                skip_lf: false,
            });
        }
        send(
            sink,
            proto::output(format!("La sesion '{session}' ya no existe.\r\n\r\n").as_bytes()),
            stop,
        )
        .await?;
    }
    let names: Vec<String> = match probe(target, AttachCommand::ListSessionNames).await {
        Some(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|s| !s.is_empty() && !matches!(*s, "local" | "hub" | "control"))
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    };
    if names.is_empty() {
        send(
            sink,
            proto::output(b"No hay sesiones. Crea una:  ccx nombre\r\n\r\n"),
            stop,
        )
        .await?;
        return Ok(Picked {
            command: None,
            tail: Vec::new(),
            skip_lf: false,
        });
    }
    let mut menu = "Sesiones de ComandOS:\r\n".to_owned();
    for (index, name) in names.iter().enumerate() {
        menu.push_str(&format!("  {:2}) {name}\r\n", index + 1));
    }
    menu.push_str("\r\nNumero (o Enter para shell libre): ");
    send(sink, proto::output(menu.as_bytes()), stop).await?;
    let mut digits = Vec::new();
    loop {
        let msg = tokio::select! { _=stop_wait(stop)=>return Err(io::Error::new(io::ErrorKind::Interrupted,"shutdown")), msg=stream.next()=>msg };
        let Some(Ok(msg)) = msg else {
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "socket"));
        };
        if msg.is_close() {
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "socket"));
        }
        if msg.is_ping() || msg.is_pong() {
            continue;
        }
        match proto::parse_client(&msg.into_data()) {
            Ok(ClientMsg::Input(bytes)) => {
                for (offset, &byte) in bytes.iter().enumerate() {
                    if matches!(byte, b'\r' | b'\n') {
                        send(sink, proto::output(b"\r\n"), stop).await?;
                        let index = std::str::from_utf8(&digits)
                            .ok()
                            .and_then(|s| s.parse::<usize>().ok())
                            .and_then(|n| n.checked_sub(1));
                        let mut tail = bytes[offset + 1..].to_vec();
                        let skip_lf = byte == b'\r' && tail.is_empty();
                        if byte == b'\r' && tail.first() == Some(&b'\n') {
                            tail.remove(0);
                        }
                        return Ok(Picked {
                            command: index.and_then(|n| names.get(n)).map(|session| {
                                AttachCommand::Attach {
                                    session: session.clone(),
                                    active_pane,
                                }
                            }),
                            tail,
                            skip_lf,
                        });
                    }
                    if byte == 3 {
                        send(sink, proto::output(b"^C\r\n"), stop).await?;
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "selector interrupted",
                        ));
                    }
                    if matches!(byte, 21 | 23) {
                        let old_len = digits.len();
                        if byte == 21 {
                            digits.clear();
                        } else {
                            while digits.last().is_some_and(u8::is_ascii_whitespace) {
                                digits.pop();
                            }
                            while digits.last().is_some_and(|b| !b.is_ascii_whitespace()) {
                                digits.pop();
                            }
                        }
                        if old_len > digits.len() {
                            send(
                                sink,
                                proto::output(&b"\x08 \x08".repeat(old_len - digits.len())),
                                stop,
                            )
                            .await?;
                        }
                        continue;
                    }
                    if matches!(byte, 8 | 127) {
                        if digits.pop().is_some() {
                            send(sink, proto::output(b"\x08 \x08"), stop).await?;
                        }
                        continue;
                    }
                    if digits.len() < 80 {
                        digits.push(byte);
                        send(sink, proto::output(&[byte]), stop).await?;
                    }
                }
            }
            Ok(ClientMsg::Resize { cols, rows }) => {
                init.cols = cols;
                init.rows = rows;
            }
            Ok(_) | Err(ProtoError::TooLarge) => {}
            Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
        }
    }
}

/// La puerta HTTP ya autenticó y decodificó init. Source::Pty.session tiene
/// prioridad (arg de tty); en v1 se usa init.session cuando no hay arg.
pub async fn run_bridge<S>(
    socket: S,
    _dialect: Dialect,
    mut init: Init,
    source: Source,
    limits: BridgeLimits,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<BridgeStats>
where
    S: Stream<Item = Result<Message, WsError>> + Sink<Message, Error = WsError> + Unpin,
{
    if limits.outbox_bytes < 2
        || limits.read_chunk == 0
        || !(2..=1000).contains(&init.cols)
        || !(1..=500).contains(&init.rows)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bridge limits/size",
        ));
    }
    if stopped(&shutdown) {
        return Ok(BridgeStats::default());
    }
    let (mut sink, mut stream) = socket.split();
    let host = comandos_runtime::platform::hostname().unwrap_or_else(|_| "localhost".into());
    send(
        &mut sink,
        proto::title(&proto::tty_title("cc-webterm-attach", host.trim())),
        &mut shutdown,
    )
    .await?;
    send(&mut sink, proto::prefs(proto::TTY_PREFS), &mut shutdown).await?;
    let mut file = None;
    let mut pty = None;
    let mut child = None;
    let mut writer = None;
    let mut stats = BridgeStats::default();
    match source {
        Source::Replay(path) => {
            file = Some(super::replay::open(&path).await?);
        }
        Source::Pty { target, session } => {
            let requested = session.or_else(|| init.session.take());
            let mut prep_stop = shutdown.clone();
            let picked = tokio::select! {
                _=stop_wait(&mut prep_stop)=>return Ok(stats),
                result=pick(&mut sink,&mut stream,&target,requested,&mut init,&mut shutdown)=>result,
            };
            let command = match picked {
                Ok(command) => command,
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    let frame = CloseFrame {
                        code: CloseCode::Unsupported,
                        reason: "terminal protocol".into(),
                    };
                    let _ = tokio::time::timeout(
                        Duration::from_millis(200),
                        sink.send(Message::Close(Some(frame))),
                    )
                    .await;
                    return Ok(stats);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                    let _ = tokio::time::timeout(
                        Duration::from_millis(200),
                        sink.send(Message::Close(Some(CloseFrame {
                            code: CloseCode::Normal,
                            reason: "selector interrupted".into(),
                        }))),
                    )
                    .await;
                    return Ok(stats);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::BrokenPipe
                    ) =>
                {
                    return Ok(stats);
                }
                Err(error) => return Err(error),
            };
            let (master, slave) = pty_process::open().map_err(io::Error::other)?;
            master
                .resize(pty_process::Size::new(init.rows, init.cols))
                .map_err(io::Error::other)?;
            let spawned = target
                .pty_command(command.command.as_ref())?
                .spawn(slave)
                .map_err(io::Error::other)?;
            stats.child_pid = spawned.id();
            child = Some(spawned);
            let (read, write) = master.into_split();
            pty = Some(read);
            let mut input_writer =
                InputWriter::start(write, command.command.is_some() || !command.tail.is_empty());
            input_writer.skip_lf = command.skip_lf;
            if !command.tail.is_empty() {
                let permit = input_writer
                    .budget
                    .clone()
                    .try_acquire_many_owned(command.tail.len() as u32)
                    .map_err(io::Error::other)?;
                input_writer
                    .tx
                    .try_send(QueuedInput {
                        msg: ClientMsg::Input(command.tail),
                        _permit: Some(permit),
                    })
                    .map_err(io::Error::other)?;
            }
            writer = Some(input_writer);
        }
    }
    let mut buffer = vec![0; limits.read_chunk.min(limits.outbox_bytes - 1)];
    let mut paused = false;
    let mut eof = false;
    let mut pending = None;
    let mut close_code = None;
    let mut exited = false;
    let outcome: io::Result<()> = async {
        loop {
            // Keep send pinned across input events so a frame is submitted once.
            if let Some(bytes) = pending.take() {
                let bytes: Vec<u8> = bytes;
                stats.max_outbox_bytes = stats.max_outbox_bytes.max(bytes.len());
                stats.output_bytes += bytes.len() - 1;
                let send = sink.send(Message::binary(bytes));
                tokio::pin!(send);
                loop {
                    tokio::select! {
                        _ = stop_wait(&mut shutdown) => return Ok(()),
                        result = &mut send => {
                            result.map_err(ws_error)?;
                            break;
                        },
                        _ = child_exit(&mut child), if child.is_some() && !exited => {
                            exited = true;
                            paused = false;
                        },
                        msg = stream.next() => {
                            if !input(msg, &mut writer, &mut paused, &mut close_code).await? {
                                return Ok(());
                            }
                        },
                    }
                }
            }
            tokio::select! {
                _ = stop_wait(&mut shutdown) => return Ok(()),
                msg = stream.next() => {
                    if !input(msg, &mut writer, &mut paused, &mut close_code).await? {
                        return Ok(());
                    }
                },
                result = async {
                    if let Some(pty) = pty.as_mut() {
                        pty.read(&mut buffer).await
                    } else if let Some(file) = file.as_mut() {
                        file.read(&mut buffer).await
                    } else {
                        std::future::pending().await
                    }
                }, if !paused && !eof => {
                    match result {
                        Ok(0) => {
                            eof = true;
                            if child.is_some() { return Ok(()); }
                        },
                        Ok(n) => {
                            if let Some(writer) = writer.as_mut()
                                && let Some(ready) = writer.ready.take()
                            {
                                let _ = ready.send(());
                            }
                            pending = Some(proto::output(&buffer[..n]));
                        },
                        Err(error) if error.raw_os_error() == Some(5) && child.is_some() => return Ok(()),
                        Err(error) => return Err(error),
                    }
                },
                _ = child_exit(&mut child), if child.is_some() && !exited => {
                    exited = true;
                    paused = false;
                },
            }
        }
    }.await;
    if let Some(mut writer) = writer {
        writer.task.abort();
        // Await cancellation so both master halves are closed before reaping.
        let _ = tokio::time::timeout(Duration::from_secs(2), &mut writer.task).await;
    }
    drop(pty);
    if let Some(mut child) = child {
        if child.try_wait()?.is_none() {
            let _ = child.start_kill();
        }
        stats.child_reaped = tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .is_ok_and(|r| r.is_ok());
    }
    if let Some(code) = close_code.or(exited.then_some(CloseCode::Normal)) {
        let frame = CloseFrame {
            code,
            reason: if code == CloseCode::Size {
                "terminal input overloaded"
            } else {
                "terminal protocol"
            }
            .into(),
        };
        let _ = tokio::time::timeout(
            Duration::from_millis(200),
            sink.send(Message::Close(Some(frame))),
        )
        .await;
    }
    match outcome {
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::BrokenPipe
            ) =>
        {
            Ok(stats)
        }
        result => result.map(|()| stats),
    }
}
async fn child_exit(child: &mut Option<tokio::process::Child>) {
    if let Some(child) = child.as_mut() {
        let _ = child.wait().await;
    } else {
        std::future::pending::<()>().await;
    }
}

/// The active write and queued input share a 1 MiB byte budget and 256 slots. An additional
/// input beyond that budget closes with 1009 instead of suspending socket reads:
/// Close/EOF and shutdown remain observable even when the PTY stops consuming.
struct QueuedInput {
    msg: ClientMsg,
    _permit: Option<tokio::sync::OwnedSemaphorePermit>,
}
struct InputWriter {
    tx: tokio::sync::mpsc::Sender<QueuedInput>,
    budget: std::sync::Arc<tokio::sync::Semaphore>,
    task: tokio::task::JoinHandle<io::Result<()>>,
    skip_lf: bool,
    ready: Option<tokio::sync::oneshot::Sender<()>>,
}
impl InputWriter {
    fn start(mut pty: pty_process::OwnedWritePty, wait_ready: bool) -> Self {
        let (ready, ready_rx) = tokio::sync::oneshot::channel();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<QueuedInput>(256);
        let budget = std::sync::Arc::new(tokio::sync::Semaphore::new(proto::MAX_INPUT));
        let task = tokio::spawn(async move {
            if wait_ready {
                let _ = ready_rx.await;
            }
            // The pixel width flips on every resize, so the kernel always signals the
            // tmux client even when rows and columns did not change: the page sends its
            // size when it is seen again and takes the window back from the desktop.
            let mut flip = false;
            while let Some(queued) = rx.recv().await {
                match queued.msg {
                    ClientMsg::Input(bytes) => pty.write_all(&bytes).await?,
                    ClientMsg::Resize { cols, rows } => {
                        flip = !flip;
                        pty.resize(pty_process::Size::new_with_pixel(rows, cols, u16::from(flip), 0))
                            .map_err(io::Error::other)?
                    }
                    _ => {}
                }
                drop(queued._permit);
            }
            Ok(())
        });
        Self {
            tx,
            budget,
            task,
            skip_lf: false,
            ready: wait_ready.then_some(ready),
        }
    }
}
impl Drop for InputWriter {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn input(
    msg: Option<Result<Message, WsError>>,
    writer: &mut Option<InputWriter>,
    paused: &mut bool,
    close: &mut Option<CloseCode>,
) -> io::Result<bool> {
    let Some(msg) = msg else { return Ok(false) };
    let msg = msg.map_err(ws_error)?;
    if msg.is_close() {
        return Ok(false);
    }
    if msg.is_ping() || msg.is_pong() {
        return Ok(true);
    }
    let mut frame = msg.into_data();
    if let Some(writer) = writer.as_mut()
        && writer.skip_lf
        && frame.first() == Some(&b'0')
        && frame.len() > 1
    {
        writer.skip_lf = false;
        if frame.get(1) == Some(&b'\n') {
            let mut bytes = frame.to_vec();
            bytes.remove(1);
            frame = bytes.into();
        }
    }
    // Reserve before parse_client copies the input. An overloaded peer cannot
    // allocate a second payload copy beyond the shared input budget.
    let permit = if frame.first() == Some(&b'0') && frame.len() <= proto::MAX_CLIENT_FRAME {
        if let Some(writer) = writer {
            match writer
                .budget
                .clone()
                .try_acquire_many_owned(frame.len().saturating_sub(1) as u32)
            {
                Ok(permit) => Some(permit),
                Err(_) => {
                    *close = Some(CloseCode::Size);
                    return Ok(false);
                }
            }
        } else {
            None
        }
    } else {
        None
    };
    match proto::parse_client(&frame) {
        Ok(msg @ (ClientMsg::Input(_) | ClientMsg::Resize { .. })) => {
            if let Some(writer) = writer
                && writer
                    .tx
                    .try_send(QueuedInput {
                        msg,
                        _permit: permit,
                    })
                    .is_err()
            {
                *close = Some(CloseCode::Size);
                return Ok(false);
            }
        }
        Ok(ClientMsg::Pause) => *paused = true,
        Ok(ClientMsg::Resume) => *paused = false,
        Err(ProtoError::TooLarge) => {
            eprintln!("comandos terminal: trama demasiado grande descartada")
        }
        Err(_) => {
            *close = Some(CloseCode::Unsupported);
            return Ok(false);
        }
    }
    Ok(true)
}
