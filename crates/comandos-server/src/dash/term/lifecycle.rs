//! Terminal enablement and compatibility leases are independent of HTTP lifetime.
use super::{
    compat::{self, CompatProfile},
    routes::TermMode,
};
use crate::dash::{self, DashState};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle};
pub fn mode_file(home: &Path) -> PathBuf {
    home.join(".claude/hooks/webterm-mode.json")
}
fn enabled_domain(home: &Path, path: &Path, live: bool) -> comandos_store::Result<bool> {
    let read = |mode, db: Option<&rusqlite::Connection>| {
        if matches!(
            mode,
            comandos_store::unified::Mode::Unified | comandos_store::unified::Mode::Sealed
        ) {
            let Some(db) = db else { return Ok(false) };
            Ok(comandos_store::unified::doc_get(db, "hooks/webterm-enabled")?.is_some())
        } else {
            Ok(path.is_file())
        }
    };
    if live {
        comandos_store::domains::caller::read(home, "ui-docs", read)
    } else {
        comandos_store::unified::with_readonly_access(home, "ui-docs", read)
    }
}
/// Toggle the existing native terminal lease. No retired executable is invoked.
pub fn set_enabled(home: &Path, hooks: &Path, enabled: bool) -> io::Result<()> {
    use crate::dash::native::files::{DomainDocument, Strict};
    let marker = DomainDocument::new(home, hooks, "webterm-enabled")?;
    let access = marker.access().map_err(io::Error::other)?;
    if enabled {
        let mode = DomainDocument::new(home, hooks, "webterm-mode.json")?;
        if !matches!(mode.strict_under(&access), Strict::Value(ref value)
            if matches!(value.get("mode").and_then(serde_json::Value::as_str), Some("native" | "ttyd")))
        {
            return Err(io::Error::other("Terminal web nativo deshabilitado"));
        }
    }
    let _lock = if access.mode() == comandos_store::unified::Mode::Sealed {
        None
    } else {
        Some(comandos_store::files::FileLock::exclusive(
            &hooks.join("webterm-enabled.lock"),
        )?)
    };
    access
        .write(
            || {
                if enabled {
                    comandos_store::files::write_atomic(&marker.file, b"")?;
                } else if let Err(error) = std::fs::remove_file(&marker.file)
                    && error.kind() != io::ErrorKind::NotFound
                {
                    return Err(error.into());
                }
                Ok(())
            },
            |db, origin| {
                if enabled {
                    comandos_store::unified::doc_put(
                        db,
                        &marker.name,
                        marker.domain,
                        b"",
                        origin,
                        0,
                    )?;
                } else {
                    db.execute("DELETE FROM documents WHERE name=?1", [&marker.name])?;
                }
                Ok(())
            },
        )
        .map_err(io::Error::other)
}
pub struct Control {
    enabled_file: PathBuf,
    home: PathBuf,
    live: bool,
    enabled: watch::Sender<bool>,
    refresh_gate: tokio::sync::Mutex<()>,
    ports: Mutex<BTreeSet<u16>>,
    errors: Mutex<BTreeMap<u16, String>>,
}
impl Control {
    pub fn new(home: &Path) -> Self {
        Self::with_mode(home, false)
    }
    pub fn new_live(home: &Path) -> Self {
        Self::with_mode(home, true)
    }
    fn with_mode(home: &Path, live: bool) -> Self {
        let enabled_file = home.join(".claude/hooks/webterm-enabled");
        let (tx, _) = watch::channel(enabled_domain(home, &enabled_file, live).unwrap_or(false));
        Self {
            enabled_file,
            home: home.to_path_buf(),
            live,
            enabled: tx,
            refresh_gate: tokio::sync::Mutex::new(()),
            ports: Mutex::new(BTreeSet::new()),
            errors: Mutex::new(BTreeMap::new()),
        }
    }
    pub fn enabled(&self) -> bool {
        if self.live {
            *self.enabled.borrow()
        } else {
            enabled_domain(&self.home, &self.enabled_file, false).unwrap_or(false)
        }
    }
    pub async fn refresh(&self) -> bool {
        let _refresh = self.refresh_gate.lock().await;
        let home = self.home.clone();
        let path = self.enabled_file.clone();
        let live = self.live;
        // A worker thread: a brief writer is waited out instead of logged as an error.
        let read = move || {
            comandos_store::wait_admitted(comandos_store::BACKGROUND_READ_WAIT, || {
                enabled_domain(&home, &path, live)
            })
        };
        match tokio::task::spawn_blocking(read).await {
            Ok(Ok(value)) => {
                self.enabled.send_if_modified(|old| {
                    if *old == value {
                        false
                    } else {
                        *old = value;
                        true
                    }
                });
                value
            }
            Ok(Err(error)) => {
                eprintln!("comandos terminal: retaining last enablement: {error}");
                *self.enabled.borrow()
            }
            Err(_) => *self.enabled.borrow(),
        }
    }
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.enabled.subscribe()
    }
    pub fn ports(&self) -> Vec<u16> {
        self.ports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .copied()
            .collect()
    }
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({"enabled":self.enabled(),"ports":self.ports(),"bind_errors":*self.errors.lock().unwrap_or_else(std::sync::PoisonError::into_inner)})
    }
    fn port(&self, port: u16, live: bool) {
        let mut ports = self
            .ports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if live {
            ports.insert(port);
        } else {
            ports.remove(&port);
        }
    }
    fn error(&self, port: u16, error: Option<&io::Error>) {
        let mut errors = self
            .errors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(e) = error {
            errors.insert(port, e.kind().to_string());
        } else {
            errors.remove(&port);
        }
    }
}
struct Lease {
    port: u16,
    stop: watch::Sender<bool>,
    task: JoinHandle<io::Result<()>>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        self.task.abort();
    }
}
async fn close(leases: &mut Vec<Lease>, control: &Control) {
    for l in leases.iter() {
        let _ = l.stop.send(true);
        control.port(l.port, false);
    }
    for mut l in leases.drain(..) {
        let _ = (&mut l.task).await;
    }
}
pub async fn run(state: Arc<DashState>, mut shutdown: watch::Receiver<bool>) -> io::Result<()> {
    let mut leases = Vec::new();
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        let enabled = if state.config.term == TermMode::Off {
            false
        } else {
            tokio::select! {
                enabled = state.term_control.refresh() => enabled,
                _ = shutdown.changed() => break,
            }
        };
        state.term_control.enabled.send_if_modified(|old| {
            if *old == enabled {
                false
            } else {
                *old = enabled;
                true
            }
        });
        if *shutdown.borrow() {
            break;
        }
        if enabled {
            for &port in &state.config.webterm_compat {
                if leases
                    .iter()
                    .any(|l: &Lease| l.port == port && !l.task.is_finished())
                {
                    continue;
                }
                if let Some(index) = leases.iter().position(|l| l.port == port) {
                    leases.remove(index);
                    state.term_control.port(port, false);
                }
                match dash::bind(port).await {
                    Ok(listener) => {
                        let (stop, rx) = watch::channel(false);
                        let profile = if port == 4779 {
                            CompatProfile::Plain
                        } else {
                            CompatProfile::Path
                        };
                        let task = tokio::spawn(compat::serve_listener(
                            listener,
                            profile,
                            state.clone(),
                            rx,
                        ));
                        leases.push(Lease { port, stop, task });
                        state.term_control.port(port, true);
                        state.term_control.error(port, None);
                    }
                    Err(error) => {
                        if state
                            .term_control
                            .errors
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .get(&port)
                            != Some(&error.kind().to_string())
                        {
                            eprintln!("comandos terminal: compat {port}: {}", error.kind());
                        }
                        state.term_control.error(port, Some(&error));
                    }
                }
            }
        } else {
            close(&mut leases, &state.term_control).await;
        }
        tokio::select! {changed=shutdown.changed()=>{if changed.is_err()||*shutdown.borrow(){break}},_=interval.tick()=>{}}
    }
    state.term_control.enabled.send_replace(false);
    close(&mut leases, &state.term_control).await;
    Ok(())
}
pub fn persist(config: &dash::DashConfig) -> io::Result<()> {
    if config.shadow_readonly
        || (config.term == TermMode::Off
            && comandos_store::domains::DomainStore { home: &config.home }
                .document(
                    "hooks/webterm-mode.json",
                    "ui-docs",
                    mode_file(&config.home),
                )
                .read_live()
                .ok()
                .flatten()
                .is_none())
    {
        return Ok(());
    }
    let value = serde_json::json!({"mode":config.term.name(),"ports":config.webterm_compat});
    dash::native::files::DomainDocument::new(
        &config.home,
        &config.home.join(".claude/hooks"),
        "webterm-mode.json",
    )?
    .write_bytes(value.to_string().as_bytes(), 0)
}
/// Until B2 combines component and terminal status, keep bind failures visible.
pub fn reply_status(
    state: &DashState,
    request: &crate::Request,
) -> Result<crate::Reply, crate::HandlerError> {
    use comandos_core::dashboard_access as access;
    let peer = request.peer.ip().to_string();
    let headers: Vec<_> = request
        .headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let policy = access::Request {
        method: access::Method::Get,
        path: &request.target,
        peer_ip: Some(&peer),
        headers: &headers,
    };
    if let Some(error) = access::security_gate(&policy, &state.config.token) {
        return crate::Reply::json(
            http::StatusCode::from_u16(error.status).map_err(|_| crate::HandlerError::Failure)?,
            &serde_json::json!({"error":error.message}),
        );
    }
    crate::Reply::json(
        http::StatusCode::OK,
        &serde_json::json!({"term":state.term_control.status()}),
    )
}
