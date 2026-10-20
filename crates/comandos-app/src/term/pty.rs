//! Nonblocking desktop PTY. Only its own session leader/group is signalled.
use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::sys::signal::{Signal, killpg};
use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
use nix::unistd::Pid;
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::path::Path;
use std::process::Child;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::time::{Duration, Instant};

pub const MAX_PENDING_BYTES: usize = 1024 * 1024;
#[derive(Debug)]
pub enum PtyError {
    Open(String),
    Spawn(String),
    Io(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    Data(usize),
    WouldBlock,
    Closed,
}

pub struct PtySession {
    pty: pty_process::blocking::Pty,
    child: Option<Child>,
    pid: u32,
    exit_reported: bool,
    pending: VecDeque<u8>,
    reaper: SyncSender<Child>,
    /// Pixel width last reported with `claim_size`, flipped on every call.
    nudge: std::cell::Cell<bool>,
}

impl PtySession {
    pub fn spawn(argv: &[String], cols: u16, rows: u16, cwd: &Path) -> Result<Self, PtyError> {
        Self::spawn_with_env(argv, cols, rows, cwd, &[], false)
    }

    /// Explicit environment snapshot, also used by confined native fixtures.
    pub fn spawn_with_env(
        argv: &[String],
        cols: u16,
        rows: u16,
        cwd: &Path,
        env: &[(String, String)],
        clear_env: bool,
    ) -> Result<Self, PtyError> {
        let Some(program) = argv.first() else {
            return Err(PtyError::Spawn("empty argv".into()));
        };
        let (pty, pts) =
            pty_process::blocking::open().map_err(|e| PtyError::Open(e.to_string()))?;
        pty.resize(pty_process::Size::new(rows.max(1), cols.max(2)))
            .map_err(|e| PtyError::Io(e.to_string()))?;
        let flags =
            fcntl(pty.as_fd(), FcntlArg::F_GETFL).map_err(|e| PtyError::Io(e.to_string()))?;
        fcntl(
            pty.as_fd(),
            FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
        )
        .map_err(|e| PtyError::Io(e.to_string()))?;
        // Reserve the worker before spawning: failure cannot leave a live child
        // whose Drop would require blocking the UI thread to reap it.
        let (reaper, receive) = sync_channel::<Child>(1);
        std::thread::Builder::new()
            .name("app-pty-reaper".into())
            .spawn(move || {
                if let Ok(child) = receive.recv() {
                    reap_group(child);
                }
            })
            .map_err(|e| PtyError::Spawn(e.to_string()))?;
        let mut cmd = pty_process::blocking::Command::new(program);
        if clear_env {
            cmd = cmd.env_clear();
        }
        cmd = cmd
            .args(argv.iter().skip(1))
            .envs(env.iter().map(|(k, v)| (k, v)))
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("NO_COLOR")
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env("VTE_VERSION", "6800")
            .current_dir(cwd);
        let child = cmd.spawn(pts).map_err(|e| PtyError::Spawn(e.to_string()))?;
        let pid = child.id();
        Ok(Self {
            pty,
            child: Some(child),
            pid,
            exit_reported: false,
            pending: VecDeque::new(),
            reaper,
            nudge: std::cell::Cell::new(false),
        })
    }

    pub fn read_chunk(&mut self, buf: &mut [u8]) -> ReadOutcome {
        if buf.is_empty() {
            return ReadOutcome::WouldBlock;
        }
        match self.pty.read(buf) {
            Ok(0) => ReadOutcome::Closed,
            Ok(n) => ReadOutcome::Data(n),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {
                ReadOutcome::WouldBlock
            }
            Err(_) => ReadOutcome::Closed,
        }
    }
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        if bytes.len() > MAX_PENDING_BYTES.saturating_sub(self.pending.len()) {
            return Err(PtyError::Io("PTY write queue capacity exceeded".into()));
        }
        self.pending.extend(bytes);
        self.flush_pending().map(|_| ())
    }
    pub fn flush_pending(&mut self) -> Result<bool, PtyError> {
        // Bounded work per main-loop dispatch even under repeated signals.
        for _ in 0..64 {
            if self.pending.is_empty() {
                return Ok(true);
            }
            let (front, _) = self.pending.as_slices();
            match self.pty.write(front) {
                Ok(0) => return Ok(false),
                Ok(n) => {
                    self.pending.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(false),
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(PtyError::Io(e.to_string())),
            }
        }
        Ok(self.pending.is_empty())
    }
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.nudge.set(false);
        self.pty
            .resize(pty_process::Size::new(rows.max(1), cols.max(2)))
            .map_err(|e| PtyError::Io(e.to_string()))
    }
    /// With `window-size latest`, tmux sizes a window after the client that spoke last:
    /// a phone attached to the same session leaves the desktop with the phone's width.
    /// Changing only the pixel size makes the kernel signal the tmux client, which reports
    /// its size again and takes the window back. Rows and columns stay the same, so
    /// nothing else redraws when the window already had this size.
    pub fn claim_size(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        let flip = !self.nudge.get();
        self.nudge.set(flip);
        self.pty
            .resize(pty_process::Size::new_with_pixel(
                rows.max(1),
                cols.max(2),
                u16::from(flip),
                0,
            ))
            .map_err(|e| PtyError::Io(e.to_string()))
    }
    pub fn raw_fd(&self) -> i32 {
        self.pty.as_raw_fd()
    }
    pub fn pid(&self) -> u32 {
        self.pid
    }
    pub fn child_tty(&self) -> Option<String> {
        std::fs::read_link(format!("/proc/{}/fd/0", self.pid))
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }
    /// Observe status once without consuming the PID reservation: Drop still
    /// owns cleanup of descendants in the session leader's process group.
    pub fn try_reap(&mut self) -> Option<i32> {
        if self.exit_reported || self.child.is_none() {
            return None;
        }
        let pid = Pid::from_raw(i32::try_from(self.pid).ok()?);
        let status = waitid(
            Id::Pid(pid),
            WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT,
        )
        .ok()?;
        let code = match status {
            WaitStatus::Exited(_, code) => code,
            WaitStatus::Signaled(_, _, _) => -1,
            _ => return None,
        };
        self.exit_reported = true;
        Some(code)
    }
}

fn signal_group(pid: u32, signal: Signal) {
    if let Ok(pid) = i32::try_from(pid) {
        let _ = killpg(Pid::from_raw(pid), signal);
    }
}
fn reap_group(mut child: Child) {
    // Keep the leader unreaped until escalation is over: its reserved PID
    // prevents signalling an unrelated group if the leader exits before its
    // descendants. All sleeps and waits occur in this private worker.
    std::thread::sleep(Duration::from_millis(250));
    signal_group(child.id(), Signal::SIGTERM);
    std::thread::sleep(Duration::from_millis(250));
    signal_group(child.id(), Signal::SIGKILL);
    let end = Instant::now() + Duration::from_secs(1);
    while Instant::now() < end {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}
impl Drop for PtySession {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            signal_group(child.id(), Signal::SIGHUP);
            // A fresh worker owns the receiver and this channel has one slot;
            // try_send never blocks the main loop.
            if let Err(error) = self.reaper.try_send(child) {
                let mut child = match error {
                    std::sync::mpsc::TrySendError::Full(child)
                    | std::sync::mpsc::TrySendError::Disconnected(child) => child,
                };
                signal_group(child.id(), Signal::SIGKILL);
                let _ = child.try_wait();
            }
        }
    }
}

/// Exit status and PTY EOF are independent. Keep the child/descriptor until
/// both have arrived; the same bounded drain is used by GTK and native tests.
#[derive(Default)]
pub struct PtyDrain {
    exit: Option<i32>,
    eof: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainOutcome {
    Budget,
    WouldBlock,
    Closed,
}
impl PtyDrain {
    pub fn observe_exit(&mut self, pty: &mut PtySession) {
        if self.exit.is_none() {
            self.exit = pty.try_reap();
        }
    }
    pub fn has_exited(&self) -> bool {
        self.exit.is_some()
    }
    pub fn finished(&self) -> Option<i32> {
        if self.eof { self.exit } else { None }
    }
    pub fn close(&mut self) {
        self.eof = true;
    }
    pub fn read_into(&mut self, pty: &mut PtySession, mut feed: impl FnMut(&[u8])) -> DrainOutcome {
        if self.eof {
            return DrainOutcome::Closed;
        }
        let mut buffer = [0u8; 8192];
        for _ in 0..32 {
            match pty.read_chunk(&mut buffer) {
                ReadOutcome::Data(n) => {
                    if let Some(bytes) = buffer.get(..n) {
                        feed(bytes);
                    }
                }
                ReadOutcome::WouldBlock => return DrainOutcome::WouldBlock,
                ReadOutcome::Closed => {
                    self.eof = true;
                    return DrainOutcome::Closed;
                }
            }
        }
        DrainOutcome::Budget
    }
}
