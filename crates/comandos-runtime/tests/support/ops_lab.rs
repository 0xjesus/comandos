//! Escenario de `tests/test_session_tmux.py` para el adaptador portado: un
//! tmux privado (`PrivateTmux`: `-f /dev/null -S <dir>/tmux-<uid>/default`,
//! sin `TMUX`), un Codex falso compilado con `cc` en el HOME temporal (el stub
//! en C del Python, sin red: escribe su rollout y espera), cuentas `main` y
//! `work` bajo ese HOME y la sesión `audit` con dos panes `/bin/sh`. Ningún
//! proveedor real; las únicas señales van al stub que lanzó la prueba. El
//! `Drop` de `PrivateTmux` hace `kill-server` con su `-S` y después borra.
#![allow(dead_code)]
use crate::private_tmux::PrivateTmux;
use comandos_runtime::{
    Unsure, dialogs,
    launch_command::{self, Ctx},
    pane_exit,
    pane_typing::TmuxResult,
    providers,
    session_configuration::{self as sc, Env, ObserveCaches},
};
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const SID: &str = "11111111-1111-1111-1111-111111111111";

/// El stub de `tests/test_session_tmux.py`, tal cual.
const STUB: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>
int main(int argc, char **argv) {
  const char *sid="11111111-1111-1111-1111-111111111111", *model="gpt-5.6-sol";
  char effort[40]="high", path[4096];
#ifdef IGNORES_EXIT
  signal(SIGINT, SIG_IGN); signal(SIGTERM, SIG_IGN); signal(SIGQUIT, SIG_IGN);
#endif
  for (int i=1; i+1<argc; i++) {
    if (!strcmp(argv[i],"resume")) sid=argv[++i];
    else if (!strcmp(argv[i],"-m")) model=argv[++i];
    else if (!strcmp(argv[i],"-c")) sscanf(argv[++i],"model_reasoning_effort=\"%39[^\"]",effort);
  }
  if (!strcmp(model,"gpt-5.6-luna")) return 23;
  const char *account_home=getenv("CODEX_HOME");
  if (account_home) snprintf(path,sizeof(path),"%s/sessions/rollout-test-%s.jsonl",account_home,sid);
  else snprintf(path,sizeof(path),"%s/.codex/sessions/rollout-test-%s.jsonl",getenv("HOME"),sid);
  FILE *f=fopen(path,"a+"); if (!f) return 24;
  fseek(f,0,SEEK_END);
  if (!ftell(f)) fprintf(f,"{\"type\":\"session_meta\",\"payload\":{\"id\":\"%s\",\"source\":\"cli\"}}\n",sid);
  fprintf(f,"{\"type\":\"turn_context\",\"payload\":{\"model\":\"%s\",\"effort\":\"%s\"}}\n",model,effort);
  fflush(f);
  const char *operation=getenv("COMANDOS_OPERATION_ID");
  if (operation && !strcmp(operation,"slow-redraw-operation")) sleep(3);
  printf("\033[2J\033[H> %s %s\n",model,effort); fflush(stdout);
  for (;;) pause();
}
"#;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stub {
    Normal,
    /// Ignora `SIGINT`, `SIGTERM` y `SIGQUIT`: nunca sale con `/exit`, Ctrl-C
    /// ni la señal (sí con el `SIGHUP` del `kill-server` final).
    IgnoresExit,
}

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// `time.sleep(min(seconds, .04))` del `monkeypatch` del Python.
fn short_sleep(d: Duration) {
    std::thread::sleep(d.min(Duration::from_millis(40)));
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

pub struct OpsLab {
    pub tmux: PrivateTmux,
    pub home: PathBuf,
    pub pane: String,
    pub other: String,
    pub registry: Value,
    pub stub: PathBuf,
    caches: Arc<Mutex<ObserveCaches>>,
    dialogs: Arc<dialogs::DialogCache>,
}

impl OpsLab {
    pub fn start(tag: &str) -> Option<Self> {
        Self::start_with(tag, Stub::Normal)
    }

    /// `None` (con aviso) sin tmux o sin compilador de C.
    pub fn start_with(tag: &str, stub: Stub) -> Option<Self> {
        let cc_ok = Command::new("cc")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !cc_ok {
            eprintln!("sin compilador de C: se salta la integración con tmux");
            return None;
        }
        let tmux = PrivateTmux::new(tag)?;
        let home = tmux.home();
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let source = home.join("stub.c");
        std::fs::write(&source, STUB).unwrap();
        let binary = bin.join("codex");
        let mut cc = Command::new("cc");
        if stub == Stub::IgnoresExit {
            cc.arg("-DIGNORES_EXIT");
        }
        let built = cc.arg(&source).arg("-o").arg(&binary).output().unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let registry = seed_registry(&home);
        for dir in [home.join(".codex"), home.join(".codex-accounts/work")] {
            std::fs::create_dir_all(dir.join("sessions")).unwrap();
            std::fs::write(
                dir.join("auth.json"),
                r#"{"tokens": {"access_token": "fixture"}}"#,
            )
            .unwrap();
        }
        let home_text = home.to_str().unwrap().to_owned();
        let started = tmux
            .command()
            .env("PS1", "$ ")
            .args([
                "new-session",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-s",
                "audit",
                "-c",
                &home_text,
                "/bin/sh",
            ])
            .output()
            .unwrap();
        assert!(started.status.success());
        let pane = String::from_utf8(started.stdout).unwrap().trim().to_owned();
        let other = tmux
            .run(&[
                "split-window",
                "-h",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-t",
                &pane,
                "-c",
                &home_text,
                "/bin/sh",
            ])
            .trim()
            .to_owned();
        let lab = Self {
            tmux,
            home,
            pane,
            other,
            registry,
            stub: binary,
            caches: Arc::new(Mutex::new(ObserveCaches::default())),
            dialogs: Arc::new(dialogs::DialogCache::new(&repo())),
        };
        let env = lab.env();
        let flags: Vec<String> = ["--sandbox", "read-only", "--ask-for-approval", "untrusted"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let command = launch_command::configuration_command(
            &lab.ctx(),
            "codex",
            "codex",
            "gpt-5.6-sol",
            "high",
            "main",
            SID,
            &flags,
            false,
        )
        .unwrap();
        pane_exit::send_shell_line(&env, &lab.pane, &command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observed = Map::new();
        while Instant::now() < deadline {
            if let Ok(found) = sc::observe_pane(&env, "audit", &lab.pane, None, None) {
                observed = found;
                if observed.get("conversationId") == Some(&json!(SID))
                    && observed.get("confirmed") == Some(&json!(true))
                {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            observed.get("confirmed"),
            Some(&json!(true)),
            "{observed:?}"
        );
        Some(lab)
    }

    pub fn ctx(&self) -> Ctx {
        Ctx {
            registry: self.registry.clone(),
            home: self.home.clone(),
            cwd: self.home.clone(),
            search_path: Some(self.home.join("bin").into_os_string()),
            proxy_port: 18765,
            repo_root: repo(),
        }
    }

    pub fn journal(&self) -> PathBuf {
        self.home.join("operations.sqlite3")
    }

    /// El `Env` del adaptador: tmux con el `-S` de este laboratorio.
    pub fn env(&self) -> Env {
        let program = self.tmux.tmux_path().to_owned();
        let socket = self.tmux.socket();
        let home = self.home.clone();
        let tmux: sc::TmuxSync = Arc::new(move |args: &[&str]| {
            assert!(
                socket.parent().is_some_and(Path::is_dir),
                "sin socket privado"
            );
            let out = Command::new(&program)
                .args(["-f", "/dev/null", "-S"])
                .arg(&socket)
                .args(args)
                .env_clear()
                .env("HOME", &home)
                .env("PATH", "/usr/bin:/bin")
                .env("LANG", "C.UTF-8")
                .stdin(Stdio::null())
                .output()
                .map_err(|e| e.to_string())?;
            Ok(TmuxResult {
                returncode: out.status.code().unwrap_or(1),
                stdout: String::from_utf8(out.stdout).map_err(|e| e.to_string())?,
                stderr: String::from_utf8(out.stderr).map_err(|e| e.to_string())?,
            })
        });
        let facts = json!({
            "harnesses": {"codex": {"available": true, "authenticated": true}},
            "motors": {}, "gateway": {},
        });
        let matrix = providers::evaluate_capability_matrix(&self.registry, &facts).unwrap();
        let mut environ = HashMap::new();
        environ.insert("HOME".to_owned(), self.home.display().to_string());
        environ.insert("PATH".to_owned(), "/usr/bin:/bin".to_owned());
        Env {
            tmux,
            home: self.home.clone(),
            hooks: self.home.join(".claude/hooks"),
            proc_root: PathBuf::from("/proc"),
            registry: self.registry.clone(),
            matrix,
            repo_root: repo(),
            cwd: self.home.clone(),
            search_path: Some(self.home.join("bin").into_os_string()),
            proxy_port: 18765,
            environ,
            journal: self.journal(),
            owner: i64::from(std::process::id()),
            sleep: short_sleep,
            clock: Arc::new(now),
            working: Arc::new(|_: &str, _: &str| -> Result<bool, Unsure> { Ok(false) }),
            trust_ledger: Arc::new(|_: &str| {}),
            caches: Arc::clone(&self.caches),
            dialogs: Arc::clone(&self.dialogs),
        }
    }

    pub fn identity(&self) -> Map<String, Value> {
        sc::pane_identity(&self.env(), "audit", &self.pane).unwrap()
    }

    /// `display-message` de un pane.
    pub fn show(&self, pane: &str, format: &str) -> String {
        self.tmux
            .run(&["display-message", "-p", "-t", pane, format])
            .trim()
            .to_owned()
    }

    pub fn capture(&self, pane: &str) -> String {
        self.tmux
            .run(&["capture-pane", "-p", "-t", pane, "-S", "-200"])
    }
}

/// `config/providers.json` con las casas y raíces de cuentas bajo el HOME
/// temporal (como la fixture del Python).
pub fn seed_registry(home: &Path) -> Value {
    let text = std::fs::read_to_string(repo().join("config/providers.json")).unwrap();
    let mut registry = comandos_core::json::workspace_loads(&text).unwrap();
    let harnesses = registry["harnesses"].as_object_mut().unwrap();
    for (name, spec) in harnesses.iter_mut() {
        if spec["capabilities"]["accounts"].as_bool() == Some(true) {
            let spec = spec.as_object_mut().unwrap();
            spec.insert(
                "defaultHome".into(),
                json!(home.join(format!(".{name}")).display().to_string()),
            );
            spec.insert(
                "accountsRoot".into(),
                json!(home.join(format!(".{name}-accounts")).display().to_string()),
            );
        }
    }
    registry
}

/// The extension tests require the real native executor already built beside tests,
/// or an explicitly supplied native fixture. It is linked only into this private HOME.
pub fn install_extension_launcher(home: &Path) {
    let native = std::env::var_os("COMANDOS_EXTENSION_SESSION_NATIVE_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("comandos")
        });
    assert!(
        native.is_file(),
        "build comandos-cli first or provide COMANDOS_EXTENSION_SESSION_NATIVE_FIXTURE for the private extension fixture"
    );
    let installed = home.join(".local/share/comandos/bin/comandos");
    let alias = comandos_runtime::extension_launch::helper_for_home(home);
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::create_dir_all(alias.parent().unwrap()).unwrap();
    std::fs::copy(native, &installed).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(installed, &alias).unwrap();
    comandos_runtime::extension_launch::require_helper(home).unwrap();
}
