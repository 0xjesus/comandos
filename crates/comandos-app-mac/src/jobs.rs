//! Bounded background work; neither HTTP nor tmux waits on the AppKit thread.
#![forbid(unsafe_code)]
use crate::{
    app::{Action, AppConfig, RunMode, Ticket},
    tmux::{TmuxRunner, runner},
};
use comandos_desktop::{Lang, dash_client::DashClient, proc::ProcOutput};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::VecDeque,
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
pub trait Backend: Send + Sync {
    fn post_timeout(&self, path: &str, body: &Value, _timeout: Duration) -> Result<Value, String> {
        self.post(path, body)
    }
    fn saved_tabs(&self) -> Result<comandos_desktop::mac_tabs::SavedTabs, String> {
        Ok(vec![])
    }
    fn metadata(
        &self,
    ) -> Result<std::collections::BTreeMap<String, comandos_desktop::TabMeta>, String> {
        Ok(Default::default())
    }
    fn saved_label(&self, session: &str) -> Result<Option<String>, String> {
        Ok(self
            .saved_tabs()?
            .into_iter()
            .find(|(s, _)| s == session)
            .and_then(|(_, label)| label.as_str().filter(|s| !s.is_empty()).map(str::to_string)))
    }
    fn project_dir(&self, _session: &str) -> Option<String> {
        None
    }
    fn save_tabs(&self, _value: &Value, _allowed: &dyn Fn() -> bool) -> Result<(), String> {
        Err("tab persistence backend unavailable".into())
    }
    fn archive(&self, _item: &Value, _allowed: &dyn Fn() -> bool) -> Result<(), String> {
        Err("history backend unavailable".into())
    }

    fn get(&self, path: &str) -> Result<Value, String>;
    fn post(&self, path: &str, body: &Value) -> Result<Value, String>;
    fn tmux(&self, args: &[&str], allowed: &dyn Fn() -> bool) -> Result<ProcOutput, String>;
    fn cancel(&self);
}
pub struct SystemBackend {
    client: DashClient,
    tmux: Box<dyn TmuxRunner>,
    files: crate::files::MacFiles,
    home: std::path::PathBuf,
}
impl SystemBackend {
    pub fn new(cfg: &AppConfig) -> Result<Self, String> {
        let mode = if matches!(cfg.mode, RunMode::Live) {
            comandos_desktop::mode::RunMode::Live
        } else {
            comandos_desktop::mode::RunMode::Sandbox
        };
        let (home, hooks) = match &cfg.mode {
            RunMode::Live => {
                let home = std::env::var_os("HOME")
                    .map(std::path::PathBuf::from)
                    .ok_or("HOME unavailable")?;
                let hooks = home.join(".claude/hooks");
                (home, hooks)
            }
            RunMode::Sandbox { hooks, .. } => (
                hooks
                    .parent()
                    .ok_or("sandbox scope unavailable")?
                    .to_path_buf(),
                hooks.clone(),
            ),
        };
        Ok(Self {
            files: crate::files::MacFiles::new(home.clone(), hooks).map_err(|e| e.to_string())?,
            home,
            client: DashClient::new(Some(&cfg.dash_url), mode).map_err(dash_error)?,
            tmux: runner(&cfg.mode),
        })
    }
}
impl Backend for SystemBackend {
    fn saved_tabs(&self) -> Result<comandos_desktop::mac_tabs::SavedTabs, String> {
        self.files.load_saved().map_err(|e| e.to_string())
    }
    fn metadata(
        &self,
    ) -> Result<std::collections::BTreeMap<String, comandos_desktop::TabMeta>, String> {
        self.files.metadata().map_err(|e| e.to_string())
    }
    fn saved_label(&self, session: &str) -> Result<Option<String>, String> {
        self.files.saved_label(session).map_err(|e| e.to_string())
    }
    fn project_dir(&self, session: &str) -> Option<String> {
        comandos_desktop::find_project_dir(&self.home.join("codebase"), session)
            .map(|p| p.to_string_lossy().into_owned())
    }
    fn save_tabs(&self, value: &Value, allowed: &dyn Fn() -> bool) -> Result<(), String> {
        self.files
            .save_tabs(value, allowed, now_ms())
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn archive(&self, item: &Value, allowed: &dyn Fn() -> bool) -> Result<(), String> {
        self.files
            .archive(item, allowed, now_ms())
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn post_timeout(&self, path: &str, body: &Value, timeout: Duration) -> Result<Value, String> {
        let (status, value) = self.client.post(path, body, timeout).map_err(dash_error)?;
        if !(200..300).contains(&status) {
            return Err(format!("HTTP {status}"));
        }
        Ok(value)
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        self.client
            .get(path, Duration::from_secs(3))
            .map_err(dash_error)
    }
    fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        let (status, value) = self
            .client
            .post(path, body, Duration::from_secs(3))
            .map_err(dash_error)?;
        if !(200..300).contains(&status) {
            return Err(format!("HTTP {status}"));
        }
        Ok(value)
    }
    fn tmux(&self, args: &[&str], allowed: &dyn Fn() -> bool) -> Result<ProcOutput, String> {
        self.tmux.run_when(args, allowed)
    }
    fn cancel(&self) {
        self.client.cancel_pending();
    }
}
#[derive(Debug, Clone)]
pub struct Opened {
    pub session: String,
    pub label: String,
    pub is_hub: bool,
    pub raise: bool,
    pub select: bool,
    pub metadata: Option<comandos_desktop::TabMeta>,
}
pub fn execute_open(
    backend: &dyn Backend,
    action: &Action,
    existing: bool,
    ticket: &Ticket,
) -> Result<Opened, String> {
    let (session, win, label, background) = match action {
        Action::Open {
            session,
            win,
            label,
        } => (session, win.as_str(), label, false),
        Action::OpenBackground { session, label } => (session, "claude", label, true),
        _ => return Err("acción no abrible".into()),
    };
    check(ticket)?;
    if !existing {
        backend.post("/ensure", &json!({"session":session,"win":win}))?;
        check(ticket)?;
    }
    select_window(backend, session, win, ticket)?;
    check(ticket)?;
    Ok(Opened {
        session: session.clone(),
        label: if existing {
            session.clone()
        } else {
            label
                .clone()
                .filter(|s| !s.is_empty())
                .or(backend.saved_label(session)?)
                .unwrap_or_else(|| session.clone())
        },
        is_hub: false,
        raise: !background,
        select: !background,
        metadata: if existing {
            None
        } else {
            Some(publish_metadata(backend, session, None, ticket)?)
        },
    })
}
fn select_window(
    backend: &dyn Backend,
    session: &str,
    win: &str,
    ticket: &Ticket,
) -> Result<(), String> {
    select_window_when(backend, session, win, ticket, &|| ticket.current())
}
fn select_window_when(
    backend: &dyn Backend,
    session: &str,
    win: &str,
    ticket: &Ticket,
    allowed: &dyn Fn() -> bool,
) -> Result<(), String> {
    check(ticket)?;
    if !allowed() {
        return Err("operación cancelada".into());
    }
    let target = format!("={session}:{win}");
    let first = backend.tmux(&["select-window", "-t", &target], allowed)?;
    check(ticket)?;
    if !allowed() {
        return Err("operación cancelada".into());
    }
    if first.code == Some(0) {
        return Ok(());
    }
    if win != "claude" {
        return Ok(());
    }
    let target = format!("={session}");
    let output = backend.tmux(
        &[
            "list-panes",
            "-s",
            "-t",
            &target,
            "-F",
            "#{window_index}|#{pane_current_command}",
        ],
        allowed,
    )?;
    check(ticket)?;
    if !allowed() {
        return Err("operación cancelada".into());
    }
    let output = String::from_utf8_lossy(&output.stdout);
    let index = output
        .lines()
        .find_map(|line| {
            line.split_once('|')
                .filter(|(_, command)| *command == "claude")
                .map(|(idx, _)| idx)
        })
        .unwrap_or("^");
    let target = format!("={session}:{index}");
    backend.tmux(&["select-window", "-t", &target], allowed)?;
    check(ticket)?;
    if !allowed() {
        return Err("operación cancelada".into());
    }
    Ok(())
}
fn check(ticket: &Ticket) -> Result<(), String> {
    if ticket.current() {
        Ok(())
    } else {
        Err("operación cancelada".into())
    }
}
pub enum Task {
    Scoped {
        key: String,
        operation: crate::tabs_ops::Operation,
        work: Box<Task>,
    },
    Save {
        value: Value,
    },
    RemoveMetadata {
        session: String,
    },
    ArchiveClose {
        tab: crate::app::Tab,
    },
    Rename {
        session: String,
        label: String,
    },
    Restore {
        saved: comandos_desktop::mac_tabs::SavedTabs,
        metadata: std::collections::BTreeMap<String, comandos_desktop::TabMeta>,
        home: std::path::PathBuf,
        state: Arc<std::sync::Mutex<crate::tabs_ops::RestoreState>>,
    },

    Preferences {
        env_lang: String,
    },
    Boot {
        hooks: std::path::PathBuf,
        home: std::path::PathBuf,
    },
    Open {
        action: Action,
        existing: bool,
    },
    NewLocal {
        home: std::path::PathBuf,
        active: Option<String>,
        epoch: u64,
        pid: u32,
    },
    Dump {
        path: std::path::PathBuf,
        html: String,
    },
}
pub enum ResultData {
    Scoped {
        key: String,
        operation: crate::tabs_ops::Operation,
        result: Box<Result<ResultData, String>>,
    },
    Saved,
    Closed,
    Renamed,
    Restored(Vec<RestoreOpened>),

    Preferences {
        theme: String,
        lang: Lang,
    },
    Boot {
        token: String,
        hub: Opened,
        saved: comandos_desktop::mac_tabs::SavedTabs,
        metadata: std::collections::BTreeMap<String, comandos_desktop::TabMeta>,
    },
    Opened(Opened),
    Dumped,
    States(Value),
}
pub struct Delivery {
    pub ticket: Ticket,
    pub result: Result<ResultData, String>,
}
pub struct Jobs {
    sender: Option<SyncSender<(Ticket, Task)>>,
    receiver: Receiver<Delivery>,
    backend: Arc<dyn Backend>,
    stop: Arc<AtomicBool>,
    notified: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    admission: RefCell<Admission>,
}
/// Main-thread overflow retains the same 128 actions accepted by App during
/// restore. A separate, coalesced save slot cannot be displaced by that burst.
#[derive(Default)]
struct Admission {
    tasks: VecDeque<(Ticket, Task)>,
    save: Option<(Ticket, Task)>,
}
fn work_current(ticket: &Ticket, task: &Task) -> bool {
    ticket.current()
        && match task {
            Task::Scoped { operation, .. } => operation.current(),
            _ => true,
        }
}
impl Admission {
    fn prune(&mut self) {
        self.tasks
            .retain(|(ticket, task)| work_current(ticket, task));
        if self
            .save
            .as_ref()
            .is_some_and(|(ticket, task)| !work_current(ticket, task))
        {
            self.save.take();
        }
    }
}
impl Jobs {
    pub fn new(
        backend: Arc<dyn Backend>,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        let (tx, rx) = mpsc::sync_channel::<(Ticket, Task)>(8);
        let (out, input) = mpsc::sync_channel(8);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let notified = Arc::new(AtomicBool::new(false));
        let pending = notified.clone();
        let service = backend.clone();
        let worker = thread::Builder::new()
            .name("comandos-mac-jobs".into())
            .spawn(move || {
                let mut poll: Option<Ticket> = None;
                let mut next_poll = Instant::now() + Duration::from_secs(3);
                let signal = || {
                    if !pending.swap(true, Ordering::AcqRel) {
                        notify();
                    }
                };
                let publish = |mut delivery: Delivery| {
                    loop {
                        if flag.load(Ordering::Acquire) || !delivery.ticket.current() {
                            return;
                        }
                        match out.try_send(delivery) {
                            Ok(()) => break,
                            Err(mpsc::TrySendError::Full(pending_delivery)) => {
                                delivery = pending_delivery;
                                thread::sleep(Duration::from_millis(2));
                            }
                            Err(mpsc::TrySendError::Disconnected(_)) => return,
                        }
                    }
                    signal();
                };
                while !flag.load(Ordering::Acquire) {
                    if poll.as_ref().is_some_and(Ticket::current) && Instant::now() >= next_poll {
                        if let Some(ticket) = &poll
                            && let Ok(value) = service.get("/state")
                            && ticket.current()
                            && !flag.load(Ordering::Acquire)
                        {
                            publish(Delivery {
                                ticket: ticket.clone(),
                                result: Ok(ResultData::States(value)),
                            });
                        }
                        next_poll = Instant::now() + Duration::from_secs(3);
                    }
                    let wait = if poll.as_ref().is_some_and(Ticket::current) {
                        next_poll.saturating_duration_since(Instant::now())
                    } else {
                        Duration::from_secs(10)
                    };
                    let (ticket, task) = match rx.recv_timeout(wait) {
                        Ok(work) => work,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    // Even a cancelled request frees an admission slot. Wake main
                    // to refill it without waiting for a result that may not exist.
                    signal();
                    if !ticket.current() {
                        continue;
                    }
                    let ticket = ticket.with_stop(flag.clone());
                    poll = Some(ticket.clone());
                    let result = execute_task(service.as_ref(), task, &ticket);
                    if ticket.current() && !flag.load(Ordering::Acquire) {
                        publish(Delivery { ticket, result });
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            sender: Some(tx),
            receiver: input,
            backend,
            stop,
            notified,
            worker: Some(worker),
            admission: RefCell::new(Admission::default()),
        })
    }
    /// Accept UI work without blocking AppKit when the eight-slot worker channel
    /// is full. Call drain on notifications to retry bounded pending work.
    pub fn enqueue(&self, ticket: Ticket, task: Task) -> Result<(), String> {
        if self.sender.is_none() {
            return Err("worker cerrado".into());
        }
        {
            let mut admission = self.admission.borrow_mut();
            admission.prune();
            if matches!(&task, Task::Scoped { key, .. } if key == "\0save") {
                admission.save = Some((ticket, task));
            } else {
                if admission.tasks.len() >= 128 {
                    return Err("cola de operaciones llena".into());
                }
                admission.tasks.push_back((ticket, task));
            }
        }
        self.flush()
    }
    fn flush(&self) -> Result<(), String> {
        let sender = self.sender.as_ref().ok_or("worker cerrado")?;
        let mut admission = self.admission.borrow_mut();
        admission.prune();
        loop {
            let is_save = admission.tasks.is_empty();
            let Some(work) = (if is_save {
                admission.save.take()
            } else {
                admission.tasks.pop_front()
            }) else {
                return Ok(());
            };
            match sender.try_send(work) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(work)) => {
                    if is_save {
                        admission.save = Some(work);
                    } else {
                        admission.tasks.push_front(work);
                    }
                    return Ok(());
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    *admission = Admission::default();
                    return Err("worker cerrado".into());
                }
            }
        }
    }
    pub fn submit(&self, ticket: Ticket, task: Task) -> Result<(), String> {
        self.sender
            .as_ref()
            .ok_or("worker cerrado")?
            .try_send((ticket, task))
            .map_err(|_| "cola de operaciones llena".into())
    }
    pub fn drain(&self) -> Vec<Delivery> {
        self.notified.store(false, Ordering::Release);
        let deliveries = self.receiver.try_iter().collect();
        let _ = self.flush();
        deliveries
    }
    pub fn close(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.backend.cancel();
        self.sender.take();
        *self.admission.borrow_mut() = Admission::default();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Jobs {
    fn drop(&mut self) {
        self.close();
    }
}
fn execute_task(backend: &dyn Backend, task: Task, ticket: &Ticket) -> Result<ResultData, String> {
    match task {
        Task::Scoped {
            key,
            operation,
            work,
        } => {
            let result = match operation.ticket_for(ticket) {
                Some(scoped) => execute_task(backend, *work, &scoped),
                None => Err("operation instance unavailable".into()),
            };
            Ok(ResultData::Scoped {
                key,
                operation,
                result: Box::new(result),
            })
        }
        Task::RemoveMetadata { session } => {
            check(ticket)?;
            backend.post("/tab-metadata-remove", &json!({"session":session}))?;
            check(ticket)?;
            Ok(ResultData::Closed)
        }
        Task::Save { value } => {
            check(ticket)?;
            backend.save_tabs(&value, &|| ticket.current())?;
            check(ticket)?;
            Ok(ResultData::Saved)
        }
        Task::Rename { session, label } => {
            check(ticket)?;
            backend.post("/rename", &json!({"session":session,"label":label}))?;
            check(ticket)?;
            Ok(ResultData::Renamed)
        }
        Task::ArchiveClose { tab } => {
            close_work(backend, &tab, ticket)?;
            Ok(ResultData::Closed)
        }
        Task::Restore {
            saved,
            metadata,
            home,
            state,
        } => restore_work(backend, &saved, &metadata, &home, &state, ticket)
            .map(ResultData::Restored),
        Task::Dump { path, html } => {
            write_dom(&path, &html, ticket)?;
            Ok(ResultData::Dumped)
        }
        Task::Open { action, existing } => {
            execute_open(backend, &action, existing, ticket).map(ResultData::Opened)
        }
        Task::Preferences { env_lang } => {
            check(ticket)?;
            let prefs = backend.get("/prefs").unwrap_or(Value::Null);
            check(ticket)?;
            let conf = backend.get("/conf").unwrap_or(Value::Null);
            check(ticket)?;
            Ok(ResultData::Preferences {
                theme: comandos_desktop::lang::initial_theme(&prefs).into(),
                lang: comandos_desktop::ui_lang(
                    conf.get("_lang").and_then(Value::as_str),
                    &env_lang,
                ),
            })
        }
        Task::Boot { hooks, home } => {
            check(ticket)?;
            let response = backend.get("/webterm-token").unwrap_or(Value::Null);
            check(ticket)?;
            let token = response
                .get("token")
                .filter(|v| comandos_core::json::truthy(v))
                .map(comandos_core::pomodoro::python_str)
                .unwrap_or_else(|| read_token(&hooks.join("dash-token")));
            if token.is_empty() {
                return Err("terminal token unavailable; tab opens remain queued".into());
            }
            let exists = backend.tmux(&["has-session", "-t", "=local"], &|| ticket.current())?;
            check(ticket)?;
            if exists.code != Some(0) {
                let output = backend.tmux(
                    &[
                        "new-session",
                        "-d",
                        "-s",
                        "local",
                        "-c",
                        &home.to_string_lossy(),
                    ],
                    &|| ticket.current(),
                )?;
                check(ticket)?;
                if output.code != Some(0) {
                    return Err("no pude crear hub local".into());
                }
            }
            let saved = backend.saved_tabs()?;
            check(ticket)?;
            let metadata = backend.metadata()?;
            check(ticket)?;
            Ok(ResultData::Boot {
                saved,
                metadata,
                token,
                hub: Opened {
                    session: "local".into(),
                    label: "local".into(),
                    is_hub: true,
                    raise: false,
                    select: true,
                    metadata: None,
                },
            })
        }
        Task::NewLocal {
            home,
            active,
            epoch,
            pid,
        } => {
            check(ticket)?;
            let mut cwd = home.to_string_lossy().into_owned();
            if let Some(key) = active {
                let target = format!("={key}:");
                let out = backend.tmux(
                    &[
                        "display-message",
                        "-p",
                        "-t",
                        &target,
                        "#{pane_current_path}",
                    ],
                    &|| ticket.current(),
                )?;
                check(ticket)?;
                if out.code == Some(0) {
                    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !text.is_empty() {
                        cwd = text;
                    }
                }
            }
            let session = format!("term-{}-{}", pid % 10000, epoch % 100000);
            let out = backend.tmux(&["new-session", "-d", "-s", &session, "-c", &cwd], &|| {
                ticket.current()
            })?;
            check(ticket)?;
            if out.code != Some(0) {
                return Err("tmux fallo".into());
            }
            let label = std::path::Path::new(&cwd)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "term".into());
            Ok(ResultData::Opened(Opened {
                session: session.clone(),
                label,
                is_hub: false,
                raise: false,
                select: true,
                metadata: Some(publish_metadata(
                    backend,
                    &session,
                    Some(comandos_desktop::TabMeta {
                        kind: comandos_desktop::TabKind::Scratch,
                        host: None,
                        cwd: Some(cwd),
                    }),
                    ticket,
                )?),
            }))
        }
    }
}
/// Publish only a complete private dump; existing files/FIFOs/symlinks are never
/// opened or replaced. The temporary file is owned by this invocation.
pub fn write_dom(path: &std::path::Path, html: &str, ticket: &Ticket) -> Result<(), String> {
    use std::{
        io::Write,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
        sync::atomic::AtomicU64,
    };
    static NEXT: AtomicU64 = AtomicU64::new(1);
    check(ticket)?;
    if html.len() > 8 << 20 {
        return Err("DOM demasiado grande".into());
    }
    let parent = path.parent().ok_or("dump sin directorio")?;
    let metadata = std::fs::symlink_metadata(parent).map_err(|e| e.to_string())?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.mode() & 0o022 != 0
    {
        return Err("directorio dump inseguro".into());
    }
    let temp = parent.join(format!(
        ".comandos-dom-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    let _cleanup = Cleanup(temp.clone());
    for chunk in html.as_bytes().chunks(65536) {
        check(ticket)?;
        file.write_all(chunk).map_err(|e| e.to_string())?;
    }
    file.sync_all().map_err(|e| e.to_string())?;
    check(ticket)?;
    std::fs::hard_link(&temp, path).map_err(|e| e.to_string())?;
    Ok(())
}
fn read_token(path: &std::path::Path) -> String {
    use std::os::unix::fs::OpenOptionsExt;
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(
            (nix::fcntl::OFlag::O_NONBLOCK
                | nix::fcntl::OFlag::O_NOFOLLOW
                | nix::fcntl::OFlag::O_NOCTTY)
                .bits(),
        )
        .open(path)
    else {
        return String::new();
    };
    if !file
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() <= 65536)
    {
        return String::new();
    }
    let mut text = String::new();
    if file.take(65537).read_to_string(&mut text).is_err() || text.len() > 65536 {
        return String::new();
    }
    comandos_core::text::strip(&text).into()
}

fn dash_error(error: comandos_desktop::dash_client::DashError) -> String {
    use comandos_desktop::dash_client::DashError;
    match error {
        DashError::HttpStatus(status, _) => format!("HTTP {status}"),
        DashError::Cancelled => "operación cancelada".into(),
        DashError::Disconnected => "tablero desconectado".into(),
        _ => "fallo de comunicación con el tablero".into(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[derive(Debug, Clone)]
pub struct RestoreOpened {
    pub original: String,
    pub opened: Opened,
}
fn publish_metadata(
    backend: &dyn Backend,
    session: &str,
    explicit: Option<comandos_desktop::TabMeta>,
    ticket: &Ticket,
) -> Result<comandos_desktop::TabMeta, String> {
    publish_metadata_when(backend, session, explicit, ticket, &|| ticket.current())
}
fn publish_metadata_when(
    backend: &dyn Backend,
    session: &str,
    explicit: Option<comandos_desktop::TabMeta>,
    ticket: &Ticket,
    allowed: &dyn Fn() -> bool,
) -> Result<comandos_desktop::TabMeta, String> {
    check(ticket)?;
    if !allowed() {
        return Err("operación cancelada".into());
    }
    let saved = backend.metadata()?.remove(session);
    check(ticket)?;
    if !allowed() {
        return Err("operación cancelada".into());
    }
    let mut meta = explicit.unwrap_or_else(|| {
        saved.clone().unwrap_or(comandos_desktop::TabMeta {
            kind: comandos_desktop::TabKind::Shell,
            host: None,
            cwd: None,
        })
    });
    if let Some(saved) = saved {
        if meta.host.as_deref().is_none_or(str::is_empty) {
            meta.host = saved.host;
        }
        if meta.cwd.as_deref().is_none_or(str::is_empty) {
            meta.cwd = saved.cwd;
        }
    }
    backend.post("/tab-metadata",&json!({"session":session,"kind":meta.kind,"host":meta.host.as_deref().unwrap_or(""),"cwd":meta.cwd.as_deref().unwrap_or("")}))?;
    check(ticket)?;
    if !allowed() {
        return Err("operación cancelada".into());
    }
    Ok(meta)
}
fn restore_work(
    backend: &dyn Backend,
    saved: &comandos_desktop::mac_tabs::SavedTabs,
    metadata: &std::collections::BTreeMap<String, comandos_desktop::TabMeta>,
    home: &std::path::Path,
    state: &std::sync::Mutex<crate::tabs_ops::RestoreState>,
    ticket: &Ticket,
) -> Result<Vec<RestoreOpened>, String> {
    use comandos_desktop::{TabKind, TabMeta, restore_tab_spec};
    let mut seen = std::collections::BTreeSet::new();
    let mut result = vec![];
    for (key, label) in saved {
        check(ticket)?;
        let raw = key.split(':').next().unwrap_or("");
        if raw.is_empty() || !comandos_desktop::validation::valid_session(raw) {
            continue;
        }
        let project = if metadata.contains_key(raw) {
            None
        } else {
            backend.project_dir(raw)
        };
        let spec = restore_tab_spec(key, metadata, project.as_deref().unwrap_or(""));
        let original = spec.session.clone();
        let mut session = original.clone();
        let mut label = label.as_str().filter(|s| !s.is_empty()).map(str::to_string);
        if spec.kind == TabKind::Xterm || !seen.insert(original.clone()) {
            continue;
        }
        let allowed = || {
            ticket.current()
                && state
                    .lock()
                    .is_ok_and(|s| !s.cancelled(&original) && !s.cancelled(&session))
        };
        if !allowed() {
            continue;
        }
        let has = backend.tmux(&["has-session", "-t", &format!("={session}")], &allowed)?;
        check(ticket)?;
        if !allowed() {
            continue;
        }
        if has.code != Some(0) {
            let response = if matches!(spec.kind, TabKind::Scratch | TabKind::Shell) {
                let cwd = if !spec.cwd.is_empty() && std::path::Path::new(&spec.cwd).is_dir() {
                    std::path::Path::new(&spec.cwd)
                } else {
                    home
                };
                let out = backend.tmux(
                    &[
                        "new-session",
                        "-d",
                        "-s",
                        &session,
                        "-c",
                        &cwd.to_string_lossy(),
                    ],
                    &allowed,
                )?;
                if out.code != Some(0) {
                    eprintln!("no pude restaurar {key}: tmux fallo");
                    continue;
                }
                Ok(Value::Null)
            } else if matches!(spec.kind, TabKind::Ssh | TabKind::SshTab) && !spec.host.is_empty() {
                backend.post_timeout(
                    if spec.kind == TabKind::Ssh {
                        "/ssh-connect"
                    } else {
                        "/ssh-new-tab"
                    },
                    &json!({"host":spec.host}),
                    Duration::from_secs(20),
                )
            } else {
                backend.post(
                    "/ensure",
                    &json!({"session":session,"cwd":spec.cwd,"win":"claude"}),
                )
            };
            check(ticket)?;
            match response {
                Ok(value) => {
                    if let Some(actual) = value
                        .get("session")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        if !comandos_desktop::validation::valid_session(actual) {
                            continue;
                        }
                        session = actual.into();
                        if let Ok(mut state) = state.lock() {
                            state.record_alias(&original, &session);
                        } else {
                            return Err("restore lock poisoned".into());
                        }
                    }
                    if spec.kind == TabKind::SshTab {
                        label = value
                            .get("label")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .or(label);
                    }
                }
                Err(error) => {
                    eprintln!("no pude restaurar {key}: {error}");
                    continue;
                }
            }
            let allowed = || {
                ticket.current()
                    && state
                        .lock()
                        .is_ok_and(|s| !s.cancelled(&original) && !s.cancelled(&session))
            };
            if !allowed() {
                continue;
            }
            if backend
                .tmux(&["has-session", "-t", &format!("={session}")], &allowed)?
                .code
                != Some(0)
            {
                continue;
            }
        }
        let allowed = || {
            ticket.current()
                && state
                    .lock()
                    .is_ok_and(|s| !s.cancelled(&original) && !s.cancelled(&session))
        };
        if !allowed()
            || result
                .iter()
                .any(|tab: &RestoreOpened| tab.opened.session == session)
        {
            continue;
        }
        if select_window_when(backend, &session, "claude", ticket, &allowed).is_err() {
            if !allowed() {
                continue;
            }
            return Err("restore selection failed".into());
        }
        if !allowed() {
            continue;
        }
        let meta = match publish_metadata_when(
            backend,
            &session,
            Some(TabMeta {
                kind: spec.kind,
                host: (!spec.host.is_empty()).then_some(spec.host),
                cwd: (!spec.cwd.is_empty()).then_some(spec.cwd),
            }),
            ticket,
            &allowed,
        ) {
            Ok(meta) => meta,
            Err(_) if !allowed() => continue,
            Err(error) => return Err(error),
        };
        if !allowed() {
            continue;
        }
        result.push(RestoreOpened {
            original,
            opened: Opened {
                label: label.unwrap_or_else(|| session.clone()),
                session,
                is_hub: false,
                raise: false,
                select: false,
                metadata: Some(meta),
            },
        });
    }
    check(ticket)?;
    Ok(result)
}
fn close_work(backend: &dyn Backend, tab: &crate::app::Tab, ticket: &Ticket) -> Result<(), String> {
    check(ticket)?;
    let target = format!("={}", tab.session);
    let pane = format!("{target}:");
    let identity = || -> Result<Option<String>, String> {
        check(ticket)?;
        let out = backend.tmux(
            &[
                "display-message",
                "-p",
                "-t",
                &pane,
                "#{pid}|#{session_id}|#{s/[$]//:session_id}|#{session_name}",
            ],
            &|| ticket.current(),
        )?;
        check(ticket)?;
        if out.code != Some(0) {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let fields: Vec<_> = text.splitn(4, '|').collect();
        Ok((fields.len() == 4
            && fields[0].parse::<u32>().is_ok_and(|n| n > 0)
            && fields[1]
                .strip_prefix('$')
                .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            && fields[2].parse::<u64>().is_ok()
            && fields[3] == tab.session)
            .then_some(text))
    };
    let pinned = if tab
        .metadata
        .as_ref()
        .is_some_and(|meta| meta.kind == comandos_desktop::TabKind::Scratch)
    {
        identity()?
    } else {
        None
    };
    let mut texts = vec![];
    for format in ["#{pane_current_path}", "#{pane_current_command}"] {
        check(ticket)?;
        let out = backend.tmux(&["display-message", "-p", "-t", &pane, format], &|| {
            ticket.current()
        })?;
        check(ticket)?;
        texts.push(if out.code == Some(0) {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            String::new()
        });
    }
    let item = comandos_desktop::mac_tabs::history_item(
        &tab.key,
        &tab.label,
        &texts[0],
        &texts[1],
        "closed",
        now_ms() / 1000,
    );
    backend.archive(&item, &|| ticket.current())?;
    check(ticket)?;
    backend.post("/tab-metadata-remove", &json!({"session":tab.key}))?;
    check(ticket)?;
    if tab
        .metadata
        .as_ref()
        .is_none_or(|meta| meta.kind != comandos_desktop::TabKind::Scratch)
    {
        return Ok(());
    }
    let Some(pinned) = pinned else {
        return Ok(());
    };
    let out = backend.tmux(
        &[
            "list-panes",
            "-s",
            "-t",
            &target,
            "-F",
            "#{pane_current_command}",
        ],
        &|| ticket.current(),
    )?;
    check(ticket)?;
    let text = String::from_utf8_lossy(&out.stdout);
    let commands: Vec<_> = text.split_whitespace().collect();
    if out.code == Some(0)
        && !commands.is_empty()
        && commands
            .iter()
            .all(|cmd| ["zsh", "bash", "sh", "fish"].contains(cmd))
        && identity()?.as_ref() == Some(&pinned)
    {
        check(ticket)?;
        // Session names can be reused between the identity check and this call.
        // The captured tmux ID remains unique for this server's lifetime.
        let session_id = pinned.split('|').nth(1).ok_or("invalid session identity")?;
        backend.tmux(&["kill-session", "-t", session_id], &|| ticket.current())?;
        check(ticket)?;
    }
    Ok(())
}
