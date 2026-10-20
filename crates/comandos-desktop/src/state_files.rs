//! Reads and writes shared app state files under `hooks_dir`.
use crate::mode::RunMode;
use comandos_store::snapshot_files::{Backend, Policy};
use nix::fcntl::{Flock, FlockArg};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub trait StateConfig: Clone {
    fn mode(&self) -> RunMode;
    fn home(&self) -> &Path;
    fn hooks_dir(&self) -> &Path;
}
pub trait StateGuard: Clone {
    type Error: std::fmt::Debug;
    fn create_dir_all(&self, path: &Path, mode: u32) -> Result<(), Self::Error>;
    fn open_lock(&self, path: &Path) -> Result<std::fs::File, Self::Error>;
    fn write_atomic(&self, path: &Path, bytes: &[u8], prefix: &str) -> Result<(), Self::Error>;
    fn write_atomic_when(
        &self,
        path: &Path,
        bytes: &[u8],
        prefix: &str,
        allowed: impl Fn() -> bool,
    ) -> Result<(), Self::Error>;
    fn archive_once(&self, path: &Path, bytes: &[u8]) -> Result<bool, Self::Error>;
    fn remove_file(&self, path: &Path) -> Result<(), Self::Error>;
}

#[derive(Debug)]
pub enum StateError<E> {
    Name(String),
    Guard(E),
    Io(PathBuf, String),
    Json(PathBuf, String),
}

#[derive(Clone)]
pub struct StateFiles<C: StateConfig, G: StateGuard> {
    config: C,
    guard: G,
}

const ALLOWED: &[&str] = &[
    "app-tabs.json",
    "app-tabs-history.json",
    "app-tabs-snapshot.json",
    "app-sessions-v2.json",
    "app-tab-active.json",
    "app-tab-models.json",
    "app-extension-shelf.json",
    "app-layout.json",
    "app-pane-position.json",
    "app-focus.json",
    "app-tab-close.json",
    "app-tab-open.json",
    "app-command.json",
    "snippets.json",
    "acp-panes.json",
];

impl<C: StateConfig, G: StateGuard> StateFiles<C, G> {
    /// These two pane UI documents have distinct catalog owners.
    pub fn read_pane_document(&self, name: &str) -> Result<Value, StateError<G::Error>> {
        self.read_ui_document(name)
    }
    pub fn read_ui_document(&self, name: &str) -> Result<Value, StateError<G::Error>> {
        let (domain, key) = match name {
            "app-tab-models.json" => ("tabs", "hooks/app-tab-models.json"),
            "app-extension-shelf.json" => ("app-ui", "hooks/app-extension-shelf.json"),
            "app-layout.json" => ("app-ui", "hooks/app-layout.json"),
            "app-pane-position.json" => ("app-ui", "hooks/app-pane-position.json"),
            _ => return Err(StateError::Name(name.into())),
        };
        let path = self.path(name)?;
        let doc = comandos_store::domains::DomainStore {
            home: self.config.home(),
        }
        .document(key, domain, path.clone());
        // Every caller reads from a worker job, so a brief writer is waited out.
        match comandos_store::wait_admitted(comandos_store::BACKGROUND_READ_WAIT, || {
            if self.config.mode() == RunMode::Live {
                doc.read_live()
            } else {
                doc.read_readonly()
            }
        })
        .map_err(|e| StateError::Io(path.clone(), e.to_string()))?
        {
            None => Ok(Value::Null),
            Some(bytes) if bytes.len() <= MAX_BYTES => {
                comandos_core::json::workspace_loads_bytes(&bytes)
                    .ok_or_else(|| StateError::Json(path, "invalid pane document JSON".into()))
            }
            Some(_) => Err(StateError::Name("pane document exceeds 8 MiB".into())),
        }
    }
    pub fn write_shelf_when(
        &self,
        expected: &Value,
        next: &Value,
        allowed: impl Fn() -> bool,
    ) -> Result<bool, StateError<G::Error>> {
        self.write_ui_document_when("app-extension-shelf.json", expected, next, allowed)
    }
    pub fn write_ui_document_when(
        &self,
        name: &str,
        expected: &Value,
        next: &Value,
        allowed: impl Fn() -> bool,
    ) -> Result<bool, StateError<G::Error>> {
        if !matches!(
            name,
            "app-extension-shelf.json" | "app-layout.json" | "app-pane-position.json"
        ) {
            return Err(StateError::Name(name.into()));
        }
        if !allowed() {
            return Ok(false);
        }
        self.update_named_when(
            name,
            "app-ui",
            true,
            |current| {
                if &current != expected {
                    return Ok(None);
                }
                Ok(Some(next.clone()))
            },
            allowed,
        )
    }
    pub fn read_snippets(&self) -> Result<Vec<Value>, StateError<G::Error>> {
        let path = self.path("snippets.json")?;
        let doc = comandos_store::domains::DomainStore {
            home: self.config.home(),
        }
        .document("hooks/snippets.json", "ui-docs", path.clone());
        match (if self.config.mode() == RunMode::Live {
            doc.read_live()
        } else {
            doc.read_readonly()
        })
        .map_err(|e| StateError::Io(path.clone(), e.to_string()))?
        {
            None => Ok(Vec::new()),
            Some(bytes) if bytes.len() <= MAX_BYTES => {
                let value = comandos_core::json::workspace_loads_bytes(&bytes)
                    .ok_or_else(|| StateError::Json(path, "invalid snippets JSON".into()))?;
                Ok(value.as_array().cloned().unwrap_or_default())
            }
            Some(_) => Err(StateError::Name("snippets document exceeds 8 MiB".into())),
        }
    }
    pub fn write_snippets_when(
        &self,
        expected: &[Value],
        next: &[Value],
        allowed: impl Fn() -> bool,
    ) -> Result<bool, StateError<G::Error>> {
        if !allowed() {
            return Ok(false);
        }
        self.update_named_when(
            "snippets.json",
            "ui-docs",
            true,
            |current| {
                if current.as_array().cloned().unwrap_or_default() != expected {
                    return Ok(None);
                }
                Ok(Some(Value::Array(next.to_vec())))
            },
            allowed,
        )
    }
    fn update_named_when(
        &self,
        name: &str,
        domain: &'static str,
        strict: bool,
        transform: impl FnOnce(Value) -> Result<Option<Value>, StateError<G::Error>>,
        allowed: impl Fn() -> bool,
    ) -> Result<bool, StateError<G::Error>> {
        let path = self.path(name)?;
        if !allowed() {
            return Ok(false);
        }
        if self.config.mode() == RunMode::Shadow {
            return Err(StateError::Name("shadow cannot write domain state".into()));
        }
        let access =
            comandos_store::domains::caller::CallerAccess::open(self.config.home(), domain)
                .map_err(|e| StateError::Io(path.clone(), e.to_string()))?;
        let _lock = if access.mode() == comandos_store::unified::Mode::Sealed {
            None
        } else {
            if self.config.mode() == RunMode::Sandbox {
                self.guard
                    .create_dir_all(self.config.hooks_dir(), 0o700)
                    .map_err(StateError::Guard)?;
            }
            let lock_path = self.config.hooks_dir().join(format!("{name}.lock"));
            let file = self
                .guard
                .open_lock(&lock_path)
                .map_err(StateError::Guard)?;
            Some(lock_exclusive::<G::Error>(file, &lock_path)?)
        };
        access
            .with_write_transaction(|| {
                let key = format!("hooks/{name}");
                let old = access
                    .read_document(&key, &path)
                    .map_err(|e| StateError::Io(path.clone(), e.to_string()))?;
                let current = match old {
                    None => Value::Null,
                    Some(body) if body.len() <= MAX_BYTES => {
                        match comandos_core::json::workspace_loads_bytes(&body) {
                            Some(value) => value,
                            None if !strict => Value::Null,
                            None => {
                                return Err(StateError::Json(path.clone(), "invalid JSON".into()));
                            }
                        }
                    }
                    Some(_) => {
                        return Err(StateError::Name("domain document exceeds 8 MiB".into()));
                    }
                };
                let Some(next) = transform(current)? else {
                    return Ok(false);
                };
                if !allowed() {
                    return Ok(false);
                }
                let text = comandos_core::json::response_dumps(&next).map_err(StateError::Name)?;
                if text.len() > MAX_BYTES {
                    return Err(StateError::Name("domain document exceeds 8 MiB".into()));
                }
                let stopped = std::cell::Cell::new(false);
                let may_publish = || {
                    let yes = !stopped.get() && allowed();
                    if !yes {
                        stopped.set(true);
                    }
                    yes
                };
                let result = access.write(
                    || {
                        self.guard
                            .write_atomic_when(
                                &path,
                                text.as_bytes(),
                                tmp_prefix::<G::Error>(name).map_err(|e| {
                                    comandos_store::Error::Validation(format!("{e:?}"))
                                })?,
                                may_publish,
                            )
                            .map_err(|e| comandos_store::Error::Validation(format!("{e:?}")))
                    },
                    |db, origin| {
                        if !may_publish() {
                            return Err(comandos_store::Error::Validation("cancelled".into()));
                        }
                        comandos_store::unified::doc_put(
                            db,
                            &key,
                            domain,
                            text.as_bytes(),
                            origin,
                            now_ms(),
                        )
                        .map(|_| ())
                    },
                );
                if stopped.get() {
                    return Ok(false);
                }
                result.map_err(|e| StateError::Io(path.clone(), e.to_string()))?;
                Ok(true)
            })
            .map_err(|e| StateError::Io(path.clone(), e.to_string()))?
    }

    pub fn new(config: C, guard: G) -> Self {
        Self { config, guard }
    }

    pub fn read(&self, name: &str) -> Result<Value, StateError<G::Error>> {
        let path = self.path(name)?;
        if name == "app-sessions-v2.json" {
            let snapshot = comandos_store::domains::LayoutSnapshot {
                home: self.config.home(),
                file: path.clone(),
            };
            return (if self.config.mode() == RunMode::Live {
                snapshot.read_live()
            } else {
                snapshot.read_readonly()
            })
            .map_err(|e| StateError::Io(path, e.to_string()));
        }
        let domain = document_domain::<G::Error>(name)?;
        let key = format!("hooks/{name}");
        let doc = comandos_store::domains::DomainStore {
            home: self.config.home(),
        }
        .document(&key, domain, path.clone());
        match (if self.config.mode() == RunMode::Live {
            doc.read_live()
        } else {
            doc.read_readonly()
        })
        .map_err(|e| StateError::Io(path.clone(), e.to_string()))?
        {
            None => Ok(Value::Null),
            Some(bytes) => comandos_core::json::workspace_loads_bytes(&bytes)
                .ok_or_else(|| StateError::Json(path, "invalid JSON".into())),
        }
    }

    pub fn write_tabs(&self, value: &Value) -> Result<(), StateError<G::Error>> {
        self.write_tabs_when(value, || true)
    }

    pub fn write_tabs_when(
        &self,
        value: &Value,
        allowed: impl Fn() -> bool,
    ) -> Result<(), StateError<G::Error>> {
        self.update_named_when(
            "app-tabs.json",
            "tabs",
            false,
            |_| Ok(Some(value.clone())),
            allowed,
        )
        .map(|_| ())
    }

    pub fn write_snapshot_when(
        &self,
        name: &str,
        value: &Value,
        allowed: impl Fn() -> bool,
    ) -> Result<(), StateError<G::Error>> {
        if !matches!(name, "app-sessions-v2.json" | "app-tabs-snapshot.json") {
            return Err(StateError::Name(name.into()));
        }
        if name == "app-sessions-v2.json" {
            if !allowed() {
                return Ok(());
            }
            if self.config.mode() == RunMode::Shadow {
                return Err(StateError::Name("shadow cannot write layout".into()));
            }
            comandos_store::domains::LayoutSnapshot {
                home: self.config.home(),
                file: self.path(name)?,
            }
            .write_with_backend_when(
                value,
                now_ms().div_euclid(1000),
                Policy::App,
                &SnapshotBackend(&self.guard),
                || {
                    let path = self.config.hooks_dir().join("app-tabs.json.lock");
                    let file = self
                        .guard
                        .open_lock(&path)
                        .map_err(|e| comandos_store::Error::Validation(format!("{e:?}")))?;
                    lock_exclusive::<G::Error>(file, &path)
                        .map_err(|e| comandos_store::Error::Validation(format!("{e:?}")))
                },
                allowed,
            )
            .map(|_| ())
            .map_err(|e| StateError::Io(self.config.hooks_dir().join(name), e.to_string()))
        } else {
            self.update_named_when(name, "tabs", false, |_| Ok(Some(value.clone())), allowed)
                .map(|_| ())
        }
    }

    pub fn read_session_snapshot(&self) -> Value {
        let snapshot = comandos_store::domains::LayoutSnapshot {
            home: self.config.home(),
            file: self.config.hooks_dir().join("app-sessions-v2.json"),
        };
        (if self.config.mode() == RunMode::Live {
            snapshot.read_live()
        } else {
            snapshot.read_readonly()
        })
        .unwrap_or_else(|_| serde_json::json!({"version":2,"sessions":{}}))
    }

    pub fn archive_tab(&self, entry: &Value) -> Result<(), StateError<G::Error>> {
        let key = entry
            .get("session")
            .and_then(Value::as_str)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| StateError::Name("history requires session".into()))?;
        self.update_named_when(
            "app-tabs-history.json",
            "tabs",
            false,
            |current| {
                let mut history = current.as_array().cloned().unwrap_or_default();
                history.retain(|item| item.get("session").and_then(Value::as_str) != Some(key));
                history.truncate(79);
                history.insert(0, entry.clone());
                Ok(Some(Value::Array(history)))
            },
            || true,
        )
        .map(|_| ())
    }
    pub fn write(&self, name: &str, value: &Value) -> Result<(), StateError<G::Error>> {
        if name == "app-sessions-v2.json" {
            return self.write_snapshot_when(name, value, || true);
        }
        if comandos_store::domains::commands::kind(name).is_ok() {
            let path = self.path(name)?;
            if self.config.mode() == RunMode::Shadow {
                return Err(StateError::Name("shadow cannot write domain state".into()));
            }
            let text = comandos_core::json::response_dumps(value).map_err(StateError::Name)?;
            return comandos_store::domains::commands::publish(
                self.config.home(),
                name,
                text.as_bytes(),
                now_ms(),
                || {
                    if self.config.mode() == RunMode::Sandbox {
                        self.guard
                            .create_dir_all(self.config.hooks_dir(), 0o700)
                            .map_err(|e| comandos_store::Error::Validation(format!("{e:?}")))?;
                    }
                    self.guard
                        .write_atomic(
                            &path,
                            text.as_bytes(),
                            tmp_prefix::<G::Error>(name)
                                .map_err(|e| comandos_store::Error::Validation(format!("{e:?}")))?,
                        )
                        .map_err(|e| comandos_store::Error::Validation(format!("{e:?}")))
                },
            )
            .map_err(|e| StateError::Io(path, e.to_string()));
        }
        self.update_named_when(
            name,
            document_domain::<G::Error>(name)?,
            false,
            |_| Ok(Some(value.clone())),
            || true,
        )
        .map(|_| ())
    }

    fn path(&self, name: &str) -> Result<PathBuf, StateError<G::Error>> {
        if !ALLOWED.contains(&name) {
            return Err(StateError::Name(name.to_string()));
        }
        Ok(self.config.hooks_dir().join(name))
    }
}

struct SnapshotBackend<'a, G: StateGuard>(&'a G);
impl<G: StateGuard> Backend for SnapshotBackend<'_, G> {
    type Error = comandos_store::Error;
    fn invalid(message: String) -> comandos_store::Error {
        comandos_store::Error::Validation(message)
    }
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, comandos_store::Error> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(comandos_store::Error::Io(e)),
        }
    }
    fn prepare(&self, path: &Path) -> Result<(), comandos_store::Error> {
        self.0
            .create_dir_all(path, 0o700)
            .map_err(|e| comandos_store::Error::Validation(format!("{e:?}")))
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), comandos_store::Error> {
        self.0
            .write_atomic(path, bytes, "app-state.")
            .map_err(|e| comandos_store::Error::Validation(format!("{e:?}")))
    }
    fn archive(
        &self,
        _source: &Path,
        target: &Path,
        bytes: &[u8],
    ) -> Result<bool, comandos_store::Error> {
        self.0
            .archive_once(target, bytes)
            .map_err(|e| comandos_store::Error::Validation(format!("{e:?}")))
    }
    fn backup(
        &self,
        _source: &Path,
        target: &Path,
        bytes: &[u8],
    ) -> Result<(), comandos_store::Error> {
        self.0
            .write_atomic(target, bytes, "app-sessions-v2.")
            .map_err(|e| comandos_store::Error::Validation(format!("{e:?}")))
    }
    fn prune(&self, history: &Path, cutoff: i128) -> Result<(), comandos_store::Error> {
        if let Ok(entries) = std::fs::read_dir(history) {
            for entry in entries.filter_map(Result::ok) {
                if entry
                    .file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".json"))
                    .and_then(|n| n.parse::<u64>().ok())
                    .is_some_and(|n| i128::from(n) < cutoff)
                {
                    self.0
                        .remove_file(&entry.path())
                        .map_err(|e| comandos_store::Error::Validation(format!("{e:?}")))?;
                }
            }
        }
        Ok(())
    }
}

fn lock_exclusive<E>(
    file: std::fs::File,
    path: &Path,
) -> Result<Flock<std::fs::File>, StateError<E>> {
    Flock::lock(file, FlockArg::LockExclusive)
        .map_err(|(_, e)| StateError::Io(path.to_path_buf(), e.to_string()))
}

fn tmp_prefix<E>(name: &str) -> Result<&'static str, StateError<E>> {
    match name {
        "app-tabs.json" => Ok("app-tabs."),
        "app-tabs-history.json" => Ok("app-tabs-history."),
        "app-tabs-snapshot.json" => Ok("app-tabs-snapshot."),
        "snippets.json" => Ok(".snippets.json."),
        _ => Ok("app-state."),
    }
}

fn document_domain<E>(name: &str) -> Result<&'static str, StateError<E>> {
    let symbolic = format!("H/{name}");
    comandos_store::domains::catalog::source(&symbolic)
        .filter(|s| s.kind == comandos_store::domains::catalog::TargetKind::Document)
        .map(|s| s.domain)
        .ok_or_else(|| StateError::Name(name.into()))
}
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
