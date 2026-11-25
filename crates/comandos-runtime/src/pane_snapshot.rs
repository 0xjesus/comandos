//! Port de `lib/pane_snapshot.py:PaneInspector`: el CLI en primer plano de un
//! pane de tmux, sin GTK ni HTTP, sobre un `/proc` y un HOME inyectables.
//!
//! Donde el Python lanzaría una excepción que no atrapa (un `pid` que no se
//! puede usar como clave de `dict`, `acp-panes.json` que no es un objeto, un
//! JSON demasiado anidado) o leería un texto que Rust no puede representar
//! (sustitutos sueltos, rutas que no son UTF-8), se devuelve `Unsure`.
use crate::Unsure;
use crate::agent_procs::{read_environ, stat_fields};
use crate::hooks::py::{as_f64, is_float, str_of};
use crate::tui_state::{Obs, loads_text};
use comandos_core::json::{python_eq, truthy};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

const SHELLS: [&str; 6] = ["zsh", "bash", "sh", "fish", "dash", "ksh"];
const GROK_EXES: [&str; 3] = ["grok", "grok-linux-x86_64", "grok-linux-aarch64"];
const VALUES: [&str; 17] = [
    "--model",
    "-m",
    "--effort",
    "--permission-mode",
    "--add-dir",
    "--settings",
    "--mcp-config",
    "--variant",
    "--agent",
    "--sandbox",
    "-s",
    "--ask-for-approval",
    "-a",
    "--config",
    "-c",
    "--profile",
    "-p",
];
const SWITCHES: [&str; 8] = [
    "--dangerously-skip-permissions",
    "--dangerously-bypass-approvals-and-sandbox",
    "--allow-dangerously-skip-permissions",
    "--danger",
    "--full-auto",
    "--no-alt-screen",
    "--strict-mcp-config",
    "--disable-slash-commands",
];
/// Valores que solo son de codex: en otros CLI `-c`/`-p` son continuar/imprimir.
const CODEX_ONLY: [&str; 6] = ["-c", "--config", "-p", "--profile", "-s", "-a"];
/// `readline(262144)` del rollout (caracteres).
const ROLLOUT_LINE_CHARS: usize = 262_144;

/// Lo que `PaneInspector.__call__` lee de un pane.
#[derive(Debug, Clone, Copy)]
pub struct PaneRef<'a> {
    pub id: &'a str,
    pub pid: i64,
    pub command: &'a str,
}

/// Inventario de procesos y cuentas tomado una vez; cada pane se inspecciona aparte.
pub struct PaneInspector {
    home: PathBuf,
    proc: PathBuf,
    processes: Option<HashMap<i64, crate::procs::ProcInfo>>,
    source: Option<Box<dyn crate::procs::ProcSource + Send + Sync>>,
    children: HashMap<i64, Vec<i64>>,
    claude: HashMap<i64, Obs>,
    grok: HashMap<i64, Obs>,
    acp: Value,
    /// `acp-panes.json` tenía sustitutos sueltos (se mira al emitir).
    acp_touched: bool,
}

/// Clave de un `dict` de Python para un `pid` leído de JSON.
enum PidKey {
    /// `int`, `float` integral o `bool` (`True == 1`).
    Int(i64),
    /// Hashable pero nunca igual a un pid entero (cadena, flotante con fracción…).
    Never,
    /// Lista u objeto: `TypeError: unhashable type`.
    Unhashable,
}

fn pid_key(value: &Value) -> PidKey {
    match value {
        Value::Bool(b) => PidKey::Int(i64::from(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                PidKey::Int(i)
            } else if is_float(n) {
                let f = as_f64(n);
                // 2^63 ya no cabe en `i64`.
                if f.is_finite()
                    && f.fract() == 0.0
                    && (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f)
                {
                    PidKey::Int(f as i64)
                } else {
                    PidKey::Never
                }
            } else {
                PidKey::Never
            }
        }
        Value::Array(_) | Value::Object(_) => PidKey::Unhashable,
        _ => PidKey::Never,
    }
}

/// `str(Path)` de `pathlib`: sin `.` ni barras repetidas o finales.
fn pathlib_str(path: &Path) -> Result<String, Unsure> {
    let text = path.to_str().ok_or(Unsure)?;
    let root = if text.starts_with("//") && !text.starts_with("///") {
        "//"
    } else if text.starts_with('/') {
        "/"
    } else {
        ""
    };
    let parts: Vec<&str> = text
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let joined = format!("{root}{}", parts.join("/"));
    Ok(if joined.is_empty() {
        ".".into()
    } else {
        joined
    })
}

/// `Path(texto).name` de `pathlib`.
fn pathlib_name(text: &str) -> &str {
    text.split('/')
        .rfind(|p| !p.is_empty() && *p != ".")
        .unwrap_or("")
}

/// `_json(path, default)`: `None` es el `default` (`OSError`/`ValueError`).
/// El segundo campo dice si se cambió un sustituto suelto por U+FFFD: quien
/// emite un valor del documento lo comprueba con [`emitted`].
fn load_json(path: &Path) -> Result<Option<(Value, bool)>, Unsure> {
    let Ok(raw) = fs::read(path) else {
        return Ok(None);
    };
    let Ok(text) = String::from_utf8(raw) else {
        return Ok(None);
    };
    loads_text(&text)
}

fn contains_replacement(value: &Value) -> bool {
    match value {
        Value::String(s) => s.contains('\u{fffd}'),
        Value::Array(items) => items.iter().any(contains_replacement),
        Value::Object(map) => map
            .iter()
            .any(|(k, v)| k.contains('\u{fffd}') || contains_replacement(v)),
        _ => false,
    }
}

/// Un valor que llega a la salida: si su documento tenía sustitutos sueltos y
/// el valor lleva U+FFFD, el Python emitiría el sustituto (`\udXXX`).
fn emitted(value: &Value, touched: bool) -> Result<Value, Unsure> {
    if touched && contains_replacement(value) {
        Err(Unsure)
    } else {
        Ok(value.clone())
    }
}

/// `re.fullmatch(r'[A-Za-z0-9_-]{1,256}', texto)`.
fn is_ident(text: &str) -> bool {
    (1..=256).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

/// `re.fullmatch(r'[0-9a-f-]{36}', texto)`.
fn is_uuidish(text: &str) -> bool {
    text.len() == 36
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
}

/// Nombres de un directorio en el orden de `read_dir` (`Path.glob('*')`, que
/// en `pathlib` incluye los que empiezan por `.`).
fn entries(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|it| it.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

fn argv(path: &Path) -> Option<Vec<String>> {
    let raw = fs::read(path).ok()?;
    Some(
        raw.split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect(),
    )
}

/// `_ROLLOUT.search(destino)`: `rollout-.*-([0-9a-f-]{36})\.jsonl$`.
fn rollout_id(target: &[u8]) -> Option<String> {
    let core = target.strip_suffix(b"\n").unwrap_or(target);
    let body = core.strip_suffix(b".jsonl")?;
    let cut = body.len().checked_sub(36)?;
    let (head, id) = body.split_at(cut);
    let id = std::str::from_utf8(id).ok()?;
    if !is_uuidish(id) {
        return None;
    }
    let head = head.strip_suffix(b"-")?;
    // `rollout-` y luego `.*` (sin saltos de línea) hasta ese guion.
    let start = head.windows(8).rposition(|w| w == b"rollout-")?;
    let gap = head.get(start + 8..).unwrap_or(&[]);
    (!gap.contains(&b'\n')).then(|| id.to_owned())
}

/// `/antigravity-cli/(?:conversations/(\w+)\.db|presence/(\w+)\.lock)$` → sid.
fn antigravity_sid(target: &[u8]) -> Option<String> {
    let core = target.strip_suffix(b"\n").unwrap_or(target);
    let (body, folder) = if let Some(body) = core.strip_suffix(b".db") {
        (body, &b"/antigravity-cli/conversations/"[..])
    } else {
        (
            core.strip_suffix(b".lock")?,
            &b"/antigravity-cli/presence/"[..],
        )
    };
    let cut = body.iter().rposition(|b| *b == b'/').map_or(0, |i| i + 1);
    let (head, sid) = body.split_at(cut);
    if sid.is_empty()
        || !sid
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        || !head.ends_with(folder)
    {
        return None;
    }
    std::str::from_utf8(sid).ok().map(str::to_owned)
}

/// Primera línea del rollout como la lee `fd.open().readline(262144)`.
fn first_line(path: &Path) -> Option<String> {
    first_line_from(fs::File::open(path).ok()?)
}

/// Lee en bloques de 8 KiB (los del `TextIOWrapper`) solo hasta el primer
/// salto, con el mismo tope de bytes: un rollout de varios MiB con una
/// primera línea corta cuesta un bloque, no el tope entero.
fn first_line_from(reader: impl Read) -> Option<String> {
    let limit = (ROLLOUT_LINE_CHARS as u64) * 4 + 4;
    let mut reader = BufReader::with_capacity(8192, reader.take(limit));
    let mut raw = Vec::new();
    loop {
        let chunk = reader.fill_buf().ok()?;
        if chunk.is_empty() {
            break;
        }
        // Saltos universales del modo texto: `\n`, `\r` o `\r\n` cierran la línea.
        if let Some(end) = chunk.iter().position(|b| matches!(b, b'\n' | b'\r')) {
            raw.extend_from_slice(chunk.get(..end).unwrap_or(&[]));
            break;
        }
        let used = chunk.len();
        raw.extend_from_slice(chunk);
        reader.consume(used);
    }
    let line = std::str::from_utf8(&raw).ok()?;
    Some(line.chars().take(ROLLOUT_LINE_CHARS).collect())
}

fn obs(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Obs {
    pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

impl PaneInspector {
    fn process_argv(&self, pid: &str) -> Option<Vec<String>> {
        if let Some(procs) = &self.processes {
            return procs.get(&pid.parse::<i64>().ok()?).map(|p| p.argv.clone());
        }
        argv(&self.proc.join(pid).join("cmdline"))
    }
    fn open_targets(&self, pid: i64, dir: &Path) -> Box<dyn Iterator<Item = (PathBuf, PathBuf)>> {
        if let Some(source) = &self.source {
            return Box::new(
                i32::try_from(pid)
                    .ok()
                    .map(|p| source.open_files(p))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| (p.clone(), p)),
            );
        }
        Box::new(
            entries(&dir.join("fd"))
                .into_iter()
                .filter_map(|fd| fs::read_link(&fd).ok().map(|target| (fd, target))),
        )
    }
    /// `PaneInspector.__init__`: hijos por padre, sesiones de Claude y Grok por
    /// pid y `acp-panes.json`.
    pub fn new(home: &Path, proc_root: &Path) -> Result<Self, Unsure> {
        #[cfg(target_os = "macos")]
        if proc_root == Path::new("/proc") {
            return Self::with_source(home, proc_root, Box::new(crate::procs::system()));
        }
        Self::build(home, proc_root, None)
    }

    /// Same inspector with an owned platform inventory. The default Linux
    /// constructor retains its proc-root readers; this seam also permits
    /// Darwin fixture tests without enumerating the host's processes.
    pub fn with_source(
        home: &Path,
        proc_root: &Path,
        source: Box<dyn crate::procs::ProcSource + Send + Sync>,
    ) -> Result<Self, Unsure> {
        Self::build(home, proc_root, Some(source))
    }

    fn build(
        home: &Path,
        proc_root: &Path,
        source: Option<Box<dyn crate::procs::ProcSource + Send + Sync>>,
    ) -> Result<Self, Unsure> {
        let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
        let processes = source.as_ref().map(|s| {
            let rows = s.snapshot();
            for p in &rows {
                children
                    .entry(i64::from(p.ppid))
                    .or_default()
                    .push(i64::from(p.pid));
            }
            rows.into_iter().map(|p| (i64::from(p.pid), p)).collect()
        });

        // The supplied inventory is authoritative. Do not also enumerate
        // proc_root when a Darwin/fake source is present.
        if processes.is_none() {
            for entry in entries(proc_root) {
                let Some(name) = entry.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if name.is_empty() || !name.bytes().all(|b| b.is_ascii_digit()) {
                    continue;
                }
                let parent = stat_fields(proc_root, name)
                    .and_then(|f| f.into_iter().nth(1))
                    .and_then(|p| p.parse::<i64>().ok());
                if let (Some(parent), Ok(pid)) = (parent, name.parse::<i64>()) {
                    children.entry(parent).or_default().push(pid);
                }
            }
        }
        let mut claude = HashMap::new();
        let mut configs = vec![home.join(".claude")];
        configs.extend(entries(&home.join(".claude-accounts")));
        for cfg in &configs {
            for file in entries(&cfg.join("sessions")) {
                if !file.as_os_str().as_bytes().ends_with(b".json") {
                    continue;
                }
                let Some((Value::Object(d), touched)) = load_json(&file)? else {
                    continue;
                };
                let (Some(pid), Some(sid)) = (
                    d.get("pid").filter(|v| truthy(v)),
                    d.get("sessionId").filter(|v| truthy(v)),
                ) else {
                    continue;
                };
                match pid_key(pid) {
                    PidKey::Unhashable => return Err(Unsure),
                    PidKey::Never => {}
                    PidKey::Int(pid) => {
                        claude.insert(
                            pid,
                            obs([
                                ("agent", Value::from("claude")),
                                ("resume_id", emitted(sid, touched)?),
                                ("claude_config_dir", Value::from(pathlib_str(cfg)?)),
                            ]),
                        );
                    }
                }
            }
        }
        let mut grok = HashMap::new();
        let mut configs = vec![home.join(".grok")];
        configs.extend(entries(&home.join(".grok-accounts")));
        for cfg in &configs {
            let Some((Value::Array(records), touched)) =
                load_json(&cfg.join("active_sessions.json"))?
            else {
                continue;
            };
            for record in &records {
                let Value::Object(d) = record else {
                    continue;
                };
                let (Some(pid), Some(sid)) = (
                    d.get("pid").filter(|v| truthy(v)),
                    d.get("session_id").filter(|v| truthy(v)),
                ) else {
                    continue;
                };
                match pid_key(pid) {
                    PidKey::Unhashable => return Err(Unsure),
                    PidKey::Never => {}
                    PidKey::Int(pid) => {
                        grok.insert(
                            pid,
                            obs([
                                ("agent", Value::from("grok")),
                                ("resume_id", emitted(sid, touched)?),
                                ("grok_home", Value::from(pathlib_str(cfg)?)),
                            ]),
                        );
                    }
                }
            }
        }
        let (acp, acp_touched) = load_json(&home.join(".claude/hooks/acp-panes.json"))?
            .unwrap_or_else(|| (Value::Object(Map::new()), false));
        Ok(Self {
            home: home.to_path_buf(),
            proc: proc_root.to_path_buf(),
            processes,
            source,
            children,
            claude,
            grok,
            acp,
            acp_touched,
        })
    }

    /// `_flags(pid)`: los argumentos de ejecución que se conservan al reanudar.
    pub fn flags(&self, pid: i64) -> Vec<String> {
        self.flags_for(&pid.to_string())
    }

    /// `_flags` con el `str(pid)` del Python (un `401.0` de JSON no existe).
    fn flags_for(&self, pid: &str) -> Vec<String> {
        if self.processes.is_some() {
            return Vec::new();
        }

        let Some(args) = self.process_argv(pid) else {
            return Vec::new();
        };
        let exe = args.first().map_or("", |a| pathlib_name(a));
        let is_value = |arg: &str| {
            VALUES.contains(&arg)
                && !(exe == "cc-acp" && arg == "--agent")
                && !(exe != "codex" && CODEX_ONLY.contains(&arg))
        };
        let mut out = Vec::new();
        let mut i = 1;
        while let Some(arg) = args.get(i) {
            if exe == "codex" && matches!(arg.as_str(), "--yolo" | "--no-daemon") {
                out.push(if arg == "--yolo" {
                    "--dangerously-bypass-approvals-and-sandbox".to_owned()
                } else {
                    arg.clone()
                });
                i += 1;
                continue;
            }
            let next = args.get(i + 1).filter(|n| !n.starts_with('-'));
            if let (true, Some(next)) = (is_value(arg), next) {
                out.push(if arg == "-m" {
                    "--model".to_owned()
                } else {
                    arg.clone()
                });
                out.push(next.clone());
                i += 2;
            } else {
                let keyed = arg.split_once('=').is_some_and(|(key, _)| is_value(key));
                if SWITCHES.contains(&arg.as_str()) || keyed {
                    out.push(arg.clone());
                }
                i += 1;
            }
        }
        out
    }

    /// `native_metadata(pid, harness)`: el registro de proceso nativo si es de
    /// este proceso (pid, inicio y arnés) y de una conversación raíz.
    pub fn native_metadata(&self, pid: i64, harness: &str) -> Result<Obs, Unsure> {
        let empty = Ok(Map::new());
        let started = match &self.processes {
            Some(procs) => procs.get(&pid).map(|p| p.start.to_string()),
            None => stat_fields(&self.proc, &pid.to_string()).and_then(|f| f.into_iter().nth(19)),
        };
        let Some(started) = started else {
            return empty;
        };
        let path = self
            .home
            .join(".claude/hooks/native-processes")
            .join(format!("{pid}.json"));
        let access = comandos_store::domains::caller::CallerAccess::open(&self.home, "processes")
            .map_err(|_| Unsure)?;
        let parsed = if matches!(
            access.mode(),
            comandos_store::unified::Mode::Unified | comandos_store::unified::Mode::Sealed
        ) {
            let Some(db) = access.db() else {
                return Err(Unsure);
            };
            use rusqlite::OptionalExtension;
            let raw = db
                .query_row(
                    "SELECT body FROM native_processes WHERE pid=?1",
                    [pid],
                    |r| r.get::<_, Vec<u8>>(0),
                )
                .optional()
                .map_err(|_| Unsure)?;
            let Some(raw) = raw.filter(|b| b.len() <= 16384) else {
                return empty;
            };
            match String::from_utf8(raw) {
                Ok(text) => loads_text(&text)?,
                Err(_) => None,
            }
        } else {
            match fs::metadata(&path) {
                Ok(m) if m.len() <= 16384 => {}
                _ => return empty,
            }
            load_json(&path)?
        };
        let Some((Value::Object(data), touched)) = parsed else {
            return empty;
        };
        let get = |k: &str| data.get(k).cloned().unwrap_or(Value::Null);
        if !python_eq(&get("pid"), &Value::from(pid))
            || !python_eq(&get("start"), &Value::from(started))
            || !python_eq(&get("harness"), &Value::from(harness))
        {
            return empty;
        }
        let sid = get("sessionId");
        let sid_text = if truthy(&sid) {
            str_of(&sid)
        } else {
            String::new()
        };
        if !is_ident(&sid_text) || truthy(&get("parentId")) {
            return empty;
        }
        let mut out = Map::new();
        for key in ["sessionId", "model", "effort", "busy", "updatedAt"] {
            out.insert(key.to_owned(), emitted(&get(key), touched)?);
        }
        Ok(out)
    }

    /// `PaneInspector.__call__(pane)`.
    pub fn inspect(&self, pane: &PaneRef<'_>) -> Result<Obs, Unsure> {
        if SHELLS.contains(&pane.command) {
            return Ok(obs([("agent", Value::from(""))]));
        }
        if pane.command == "cc-acp" {
            return self.inspect_acp(pane);
        }
        let mut queue = VecDeque::from([pane.pid]);
        let mut seen = HashSet::new();
        // Un `claude` sin registro de sesión (p. ej. uno suspendido con Ctrl+Z
        // bajo el shell) cede ante un hermano que sí lo tiene: ése es el que
        // se ve en el pane.
        let mut unregistered = None;
        while let Some(pid) = queue.pop_front() {
            if !seen.insert(pid) {
                continue;
            }
            let dir = self.proc.join(pid.to_string());
            let Some(args) = self.process_argv(&pid.to_string()) else {
                continue;
            };
            let Some(first) = args.first() else {
                continue;
            };
            // El CLI real antes que sus subprocesos o subagentes.
            let exe = pathlib_name(first);
            if exe == "claude" {
                if self.claude.contains_key(&pid) {
                    return Ok(self.known(&self.claude, pid, "claude"));
                }
                unregistered.get_or_insert(pid);
                continue;
            }
            if GROK_EXES.contains(&exe) {
                return Ok(self.known(&self.grok, pid, "grok"));
            }
            if exe == "opencode" || exe == "agy" {
                return self.inspect_native(pid, exe, &args, &dir);
            }
            if exe == "codex" {
                return self.inspect_codex(pid, &args, &dir);
            }
            if let Some(kids) = self.children.get(&pid) {
                queue.extend(kids);
            }
        }
        if let Some(pid) = unregistered {
            return Ok(self.known(&self.claude, pid, "claude"));
        }
        Ok(obs([("agent", Value::from(""))]))
    }

    fn known(&self, table: &HashMap<i64, Obs>, pid: i64, agent: &str) -> Obs {
        match table.get(&pid) {
            Some(record) => {
                let mut out = record.clone();
                out.insert("flags".into(), Value::from(self.flags(pid)));
                out
            }
            None => obs([("agent", Value::from(agent))]),
        }
    }

    fn inspect_acp(&self, pane: &PaneRef<'_>) -> Result<Obs, Unsure> {
        let Value::Object(acp) = &self.acp else {
            return Err(Unsure);
        };
        let empty = Value::Object(Map::new());
        let Value::Object(record) = acp.get(pane.id).unwrap_or(&empty) else {
            return Err(Unsure);
        };
        let mut descendants = HashSet::new();
        let mut pending = vec![pane.pid];
        while let Some(process) = pending.pop() {
            if !descendants.insert(process) {
                continue;
            }
            if let Some(kids) = self.children.get(&process) {
                pending.extend(kids);
            }
        }
        let pid = record.get("pid").cloned().unwrap_or(Value::Null);
        let inside = match pid_key(&pid) {
            PidKey::Unhashable => return Err(Unsure),
            PidKey::Never => false,
            PidKey::Int(p) => descendants.contains(&p),
        };
        if !inside {
            return Ok(obs([("agent", Value::from("acp"))]));
        }
        let mut details = Map::new();
        for key in ["agent", "model", "effort", "account", "sessionId"] {
            let value = record.get(key).cloned().unwrap_or_else(|| Value::from(""));
            details.insert(key.to_owned(), emitted(&value, self.acp_touched)?);
        }
        Ok(obs([
            ("agent", Value::from("acp")),
            ("flags", Value::from(self.flags_for(&str_of(&pid)))),
            ("acp", Value::Object(details)),
        ]))
    }

    fn inspect_native(
        &self,
        pid: i64,
        exe: &str,
        args: &[String],
        dir: &Path,
    ) -> Result<Obs, Unsure> {
        let mut result = obs([
            ("agent", Value::from(exe)),
            ("flags", Value::from(self.flags(pid))),
            ("processPid", Value::from(pid)),
        ]);
        let flag_names: &[&str] = if exe == "opencode" {
            &["--session", "-s"]
        } else {
            &["--conversation"]
        };
        let mut explicit = args
            .windows(2)
            .find(|w| w.first().is_some_and(|a| flag_names.contains(&a.as_str())))
            .and_then(|w| w.get(1))
            .cloned()
            .unwrap_or_default();
        if explicit.is_empty() {
            explicit = args
                .iter()
                .find(|a| flag_names.iter().any(|f| a.starts_with(&format!("{f}="))))
                .and_then(|a| a.split_once('=').map(|(_, v)| v.to_owned()))
                .unwrap_or_default();
        }
        if !explicit.is_empty() && !is_ident(&explicit) {
            explicit.clear();
        }
        let explicit = Value::from(explicit);
        let metadata = self.native_metadata(pid, exe)?;
        let meta_sid = metadata.get("sessionId").cloned().unwrap_or(Value::Null);
        let native_for = |sid: &Value| {
            if python_eq(&meta_sid, sid) {
                Value::Object(metadata.clone())
            } else {
                Value::Object(Map::new())
            }
        };
        if exe == "opencode" {
            let sid = if truthy(&meta_sid) {
                meta_sid.clone()
            } else {
                explicit
            };
            if truthy(&sid) {
                let path = pathlib_str(&self.home.join(".local/share/opencode/opencode.db"))?;
                let native = native_for(&sid);
                result.insert("resume_id".into(), sid);
                result.insert("nativeMetadata".into(), native);
                result.insert("transcriptPath".into(), Value::from(path));
            }
            return Ok(result);
        }
        // agy: conversaciones o candados abiertos de `antigravity-cli`.
        let mut candidates: Vec<(String, Vec<u8>)> = Vec::new();
        for (_, target) in self.open_targets(pid, dir) {
            let target = target.as_os_str().as_bytes();
            let Some(sid) = antigravity_sid(target) else {
                continue;
            };
            let marker: &[u8] = b"/antigravity-cli/";
            let prefix = target
                .windows(marker.len())
                .position(|w| w == marker)
                .and_then(|i| target.get(..i))
                .unwrap_or(target);
            let mut path = prefix.to_vec();
            path.extend_from_slice(b"/antigravity-cli/conversations/");
            path.extend_from_slice(sid.as_bytes());
            path.extend_from_slice(b".db");
            match candidates.iter_mut().find(|(s, _)| *s == sid) {
                Some((_, old)) => *old = path,
                None => candidates.push((sid, path)),
            }
        }
        let sid = match candidates.as_slice() {
            [(only, _)] => Value::from(only.clone()),
            [] if truthy(&meta_sid) => meta_sid.clone(),
            [] => explicit,
            _ => Value::from(""),
        };
        if truthy(&sid) {
            let path = match candidates
                .iter()
                .find(|(s, _)| sid.as_str() == Some(s.as_str()))
            {
                Some((_, path)) => String::from_utf8(path.clone()).map_err(|_| Unsure)?,
                None => pathlib_str(
                    &self
                        .home
                        .join(".gemini/antigravity-cli/conversations")
                        .join(format!("{}.db", str_of(&sid))),
                )?,
            };
            result.insert("resume_id".into(), sid.clone());
            result.insert("nativeMetadata".into(), native_for(&sid));
            result.insert("transcriptPath".into(), Value::from(path));
        }
        Ok(result)
    }

    fn inspect_codex(&self, pid: i64, args: &[String], dir: &Path) -> Result<Obs, Unsure> {
        // Un proceso Codex tiene rollouts raíz Y delegados abiertos; su primer
        // fd no es necesariamente la conversación visible.
        let mut roots: Vec<String> = Vec::new();
        for (fd, target) in self.open_targets(pid, dir) {
            let Some(ident) = rollout_id(target.as_os_str().as_bytes()) else {
                continue;
            };
            let Some(line) = first_line(&fd) else {
                continue;
            };
            // `RecursionError` no está entre lo que el Python atrapa.
            let Some((Value::Object(meta), _)) = loads_text(&line)? else {
                continue;
            };
            let empty = Value::Object(Map::new());
            let Value::Object(payload) = meta.get("payload").unwrap_or(&empty) else {
                continue;
            };
            let delegated = match payload.get("source") {
                Some(Value::Object(source)) => source.contains_key("subagent"),
                Some(Value::String(source)) => source.starts_with("subagent"),
                _ => false,
            };
            let is_meta = python_eq(
                meta.get("type").unwrap_or(&Value::Null),
                &Value::from("session_meta"),
            );
            let same = python_eq(
                payload.get("id").unwrap_or(&Value::Null),
                &Value::from(ident.as_str()),
            );
            if is_meta && !delegated && same && fs::metadata(&fd).is_ok() && !roots.contains(&ident)
            {
                roots.push(ident);
            }
        }
        if roots.len() > 1 {
            return Ok(obs([("agent", Value::from("codex"))]));
        }
        let mut sid = roots.into_iter().next();
        if sid.is_none()
            && let Some(i) = args.iter().position(|a| a == "resume")
            && let Some(next) = args.get(i + 1).filter(|n| is_uuidish(n))
        {
            sid = Some(next.clone());
        }
        let Some(sid) = sid else {
            // Nunca se toma el rollout de un Codex delegado como el de su padre.
            return Ok(obs([("agent", Value::from("codex"))]));
        };
        let mut result = obs([
            ("agent", Value::from("codex")),
            ("resume_id", Value::from(sid)),
            ("flags", Value::from(self.flags(pid))),
        ]);
        let env = read_environ(&self.proc, pid);
        if let Some(home) = env.get(b"CODEX_HOME".as_slice()).filter(|v| !v.is_empty()) {
            // `os.fsdecode` dejaría sustitutos que Rust no puede representar.
            let home = String::from_utf8(home.clone()).map_err(|_| Unsure)?;
            result.insert("codex_home".into(), Value::from(home));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_helpers_follow_pathlib() {
        assert_eq!(pathlib_name("a/.."), "..");
        assert_eq!(pathlib_name("."), "");
        assert_eq!(pathlib_name("foo/"), "foo");
        assert_eq!(pathlib_name("/"), "");
        assert_eq!(pathlib_name("/usr/bin/claude"), "claude");
        assert_eq!(pathlib_str(Path::new("/h//./x/")).unwrap(), "/h/x");
        assert_eq!(pathlib_str(Path::new("//h")).unwrap(), "//h");
    }

    /// Cuenta los bytes que se piden al archivo.
    struct Counting<R> {
        inner: R,
        read: std::rc::Rc<std::cell::Cell<usize>>,
    }

    impl<R: Read> Read for Counting<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.read.set(self.read.get() + n);
            Ok(n)
        }
    }

    #[test]
    fn first_line_reads_one_block_of_a_large_rollout() {
        let mut data = b"{\"type\":\"session_meta\"}\n".to_vec();
        data.resize(3 * 1024 * 1024, b'x');
        let read = std::rc::Rc::new(std::cell::Cell::new(0));
        let reader = Counting {
            inner: std::io::Cursor::new(data),
            read: read.clone(),
        };
        assert_eq!(
            first_line_from(reader).as_deref(),
            Some("{\"type\":\"session_meta\"}")
        );
        assert!(read.get() <= 8192, "leídos {} bytes", read.get());
        // Una línea más larga que un bloque se sigue leyendo entera; `\r` también corta.
        let mut long = vec![b'a'; 20_000];
        long.extend_from_slice(b"\rrest");
        assert_eq!(first_line_from(&long[..]).map(|l| l.len()), Some(20_000));
        // Sin salto: hasta el tope de caracteres.
        let huge = vec![b'b'; 300_000];
        assert_eq!(
            first_line_from(&huge[..]).map(|l| l.len()),
            Some(ROLLOUT_LINE_CHARS)
        );
    }

    #[test]
    fn rollout_and_antigravity_patterns() {
        let id = "0f0f0f0f-0000-4000-8000-000000000001";
        assert_eq!(
            rollout_id(format!("/s/rollout-2026-{id}.jsonl").as_bytes()).as_deref(),
            Some(id)
        );
        assert_eq!(
            rollout_id(format!("/s/rollout--{id}.jsonl\n").as_bytes()).as_deref(),
            Some(id)
        );
        assert!(rollout_id(format!("/s/rollout-\nx-{id}.jsonl").as_bytes()).is_none());
        assert!(rollout_id(format!("/s/roll-{id}.jsonl").as_bytes()).is_none());
        assert_eq!(
            antigravity_sid(b"/h/.gemini/antigravity-cli/presence/ab_c.lock").as_deref(),
            Some("ab_c")
        );
        assert!(antigravity_sid(b"/h/antigravity-cli/conversations/a.b.db").is_none());
    }
}
