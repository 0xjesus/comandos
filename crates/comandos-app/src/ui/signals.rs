//! Only this App process receives the original desktop debug/mosaic signals.
use std::{
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{MetadataExt, OpenOptionsExt},
            net::UnixStream,
        },
    },
    path::Path,
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Mosaic,
    JavascriptDebug,
}
impl Action {
    pub fn signal(self) -> i32 {
        match self {
            Self::Mosaic => nix::libc::SIGUSR2,
            Self::JavascriptDebug => nix::libc::SIGRTMIN() + 3,
        }
    }
    fn from_signal(signal: i32) -> Option<Self> {
        [Self::Mosaic, Self::JavascriptDebug]
            .into_iter()
            .find(|action| action.signal() == signal)
    }
}
pub struct SignalRelay {
    handle: signal_hook::iterator::Handle,
    worker: Option<JoinHandle<()>>,
    wake: UnixStream,
    receiver: Receiver<Action>,
}
impl SignalRelay {
    pub fn new() -> std::io::Result<Self> {
        let mut signals = signal_hook::iterator::Signals::new([
            Action::Mosaic.signal(),
            Action::JavascriptDebug.signal(),
        ])?;
        let handle = signals.handle();
        let (wake, mut writer) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;
        let (sender, receiver) = mpsc::sync_channel(8);
        let worker = thread::Builder::new()
            .name("app-signals".into())
            .spawn(move || {
                for signal in signals.forever() {
                    if let Some(action) = Action::from_signal(signal)
                        && sender.try_send(action).is_ok()
                    {
                        let _ = writer.write(&[1]);
                    }
                }
            })?;
        Ok(Self {
            handle,
            worker: Some(worker),
            wake,
            receiver,
        })
    }
    pub fn fd(&self) -> i32 {
        self.wake.as_raw_fd()
    }
    pub fn drain(&mut self) -> Vec<Action> {
        let mut bytes = [0u8; 64];
        while self.wake.read(&mut bytes).is_ok_and(|n| n > 0) {}
        self.receiver.try_iter().take(8).collect()
    }
}
impl Drop for SignalRelay {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
/// The runtime directory is private and the opened regular file must belong to this UID.
/// No symlink is followed and input is bounded independently of file replacement.
#[allow(clippy::disallowed_methods)] // read-only O_NOFOLLOW descriptor; never opens for writing.
pub fn debug_script(path: &Path, uid: u32) -> std::io::Result<String> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("debug directory absent"))?;
    let dir = std::fs::symlink_metadata(parent)?;
    if !dir.is_dir() || dir.uid() != uid || dir.mode() & 0o777 != 0o700 {
        return Err(std::io::Error::other("debug directory must be owned0700"));
    }
    let expected = std::fs::symlink_metadata(path)?;
    if !expected.is_file() || expected.uid() != uid {
        return Err(std::io::Error::other(
            "debug file must be regular and owned",
        ));
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(
            nix::libc::O_NOFOLLOW
                | nix::libc::O_CLOEXEC
                | nix::libc::O_NONBLOCK
                | nix::libc::O_NOCTTY,
        )
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || (metadata.dev(), metadata.ino()) != (expected.dev(), expected.ino())
    {
        return Err(std::io::Error::other("debug file owner changed"));
    }
    let mut bytes = Vec::new();
    file.take(262145).read_to_end(&mut bytes)?;
    if bytes.len() > 262144 {
        return Err(std::io::Error::other("debug script exceeds256KiB"));
    }
    String::from_utf8(bytes).map_err(|_| std::io::Error::other("debug script must be UTF8"))
}
