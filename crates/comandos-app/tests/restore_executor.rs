#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]
use comandos_app::{
    config::RunMode,
    restore::{EnterError, RestorePlan, RestoreTmux, ScopeLauncher, execute, remap_layout},
};
use comandos_core::workspace::snapshot::layout_checksum;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

type RecordedCall = (Vec<String>, Option<Vec<u8>>);
#[derive(Default)]
struct Recorder {
    calls: RefCell<Vec<RecordedCall>>,
    exists: Cell<bool>,
    next: Cell<u32>,
    fail: RefCell<Option<String>>,
    cleaned: Cell<bool>,
    shadow: bool,
    pasted: Cell<bool>,
    reject_after_paste: bool,
    reject_enter_at_backend: bool,
    paste_count: Cell<usize>,
    reject_second_paste: bool,
    dispatch_checks: Cell<usize>,
}
impl Recorder {
    fn record(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<String, String> {
        self.calls.borrow_mut().push((
            args.iter().map(|a| (*a).into()).collect(),
            stdin.map(|b| b.to_vec()),
        ));
        if args.first() == Some(&"paste-buffer") {
            self.pasted.set(true);
            self.paste_count.set(self.paste_count.get() + 1);
        }
        if self.fail.borrow().as_deref() == args.first().copied() {
            return Err("fixture failure".into());
        }
        Ok(match args[0] {
            "has-session" if !self.exists.get() => return Err("absent".into()),
            "new-session" | "new-window" | "split-window" => {
                let next = self.next.get() + 10;
                self.next.set(next);
                format!("%{next}")
            }
            "display-message" if args.last() == Some(&"#{window_id}") => "@7".into(),
            "display-message" if args.last() == Some(&"#{window_index}") => "0".into(),
            _ => String::new(),
        })
    }
}
impl RestoreTmux for Recorder {
    type Ownership = &'static str;
    fn mode(&self) -> RunMode {
        if self.shadow {
            RunMode::Shadow
        } else {
            RunMode::Sandbox
        }
    }
    fn read(&self, a: &[&str]) -> Result<String, String> {
        self.record(a, None)
    }
    fn mutate(&self, a: &[&str], b: Option<&[u8]>) -> Result<String, String> {
        assert!(!self.shadow);
        if a.first() == Some(&"send-keys") && self.reject_enter_at_backend {
            return Err("cancelled at dispatch boundary".into());
        }
        self.record(a, b)
    }
    fn validate(&self, _: &Self::Ownership) -> Result<(), String> {
        if (self.pasted.get() && self.reject_after_paste)
            || (self.reject_second_paste && self.paste_count.get() == 2)
        {
            Err("cancelled or ownership rejected after paste".into())
        } else {
            Ok(())
        }
    }
    fn send_enter(&self, token: &Self::Ownership, pane: &str) -> Result<(), EnterError> {
        comandos_app::restore::dispatch_enter(self, token, pane, || {
            self.dispatch_checks.set(self.dispatch_checks.get() + 1);
            if self.reject_enter_at_backend && self.dispatch_checks.get() == 2 {
                Err("cancelled at dispatch boundary".into())
            } else {
                Ok(())
            }
        })
    }
    fn create(&self, a: &[&str]) -> Result<(String, Self::Ownership), String> {
        assert!(!self.shadow);
        self.record(a, None).map(|s| (s, "owned-identity"))
    }
    fn cleanup(&self, t: Self::Ownership) -> Result<(), String> {
        assert_eq!(t, "owned-identity");
        self.cleaned.set(true);
        Ok(())
    }
}
#[derive(Default)]
struct Scopes(RefCell<Vec<(String, String)>>, RefCell<Vec<String>>);
impl ScopeLauncher for Scopes {
    fn argv(&self, p: &str, c: &str) -> Result<Vec<String>, String> {
        self.0.borrow_mut().push((p.into(), c.into()));
        self.1.borrow_mut().push(format!("argv {p}"));
        Ok(vec!["fake-scope".into(), p.into(), c.into()])
    }
    fn prepare(&self, saved: &Value, cwd: &str, _home: &Path) {
        self.1
            .borrow_mut()
            .push(format!("prepare {} {cwd}", saved["agent"].as_str().unwrap()));
    }
}
fn snapshot() -> Value {
    let body = "120x32,0,0{59x32,0,0,1,60x32,60,0,2}";
    json!({"windows":[{"index":3,"name":"split ' 日本", "width":120,"height":32,"layout":format!("{:04x},{body}",layout_checksum(body)),"active":true,"zoomed":true,"panes":[
        {"id":"%1","cwd":"/tmp","active":true,"agent":"claude","resume_id":"12345678-1234-1234-1234-123456789abc","key":"pane-one"},
        {"id":"%2","cwd":"/tmp","active":false,"agent":"codex","resume_id":"87654321-4321-4321-4321-cba987654321","flags":["-c","name='日本'"]}
    ]}]})
}
fn plan(s: Value, present: BTreeSet<String>, mode: RunMode) -> RestorePlan {
    RestorePlan::build(&json!({"term-a":"A"}), &json!({"term-a":s}), &present, mode).unwrap()
}
#[test]
fn scope_launch_is_per_pane_and_geometry_precedes_exact_resume() {
    let tmux = Recorder::default();
    let scopes = Scopes::default();
    let out = execute(
        &plan(snapshot(), BTreeSet::new(), RunMode::Sandbox),
        &tmux,
        &scopes,
        Path::new("/tmp"),
    );
    assert!(out.tabs[0].attached, "{:?}", out);
    assert_eq!(
        out.tabs[0].pane_mapping,
        BTreeMap::from([("%1".into(), "%10".into()), ("%2".into(), "%20".into())])
    );
    assert_eq!(scopes.0.borrow().len(), 2);
    assert_eq!(scopes.0.borrow()[0].0, "%10");
    assert_eq!(scopes.0.borrow()[1].0, "%20");
    // Folder trust is settled before each exact resume is typed.
    assert_eq!(
        *scopes.1.borrow(),
        ["prepare claude /tmp", "argv %10", "prepare codex /tmp", "argv %20"]
    );
    let calls = tmux.calls.borrow();
    let first_resume = calls
        .iter()
        .position(|(a, _)| a[0] == "load-buffer")
        .unwrap();
    let layout = calls
        .iter()
        .position(|(a, _)| a[0] == "select-layout" && a.last().unwrap().contains("x32"))
        .unwrap();
    assert!(layout < first_resume);
    assert!(
        calls
            .iter()
            .filter(|(a, _)| a[0] == "load-buffer")
            .all(|(_, b)| b.is_some())
    );
    assert_eq!(calls.iter().filter(|(a, _)| a[0] == "send-keys").count(), 2);
    assert!(calls.iter().all(|(a, _)| a.iter().all(|v| v != ";")));
}
#[test]
fn restore_existing_never_sends_resume_even_if_plan_is_stale() {
    let tmux = Recorder::default();
    tmux.exists.set(true);
    let scopes = Scopes::default();
    let out = execute(
        &plan(snapshot(), BTreeSet::new(), RunMode::Sandbox),
        &tmux,
        &scopes,
        Path::new("/tmp"),
    );
    assert!(out.tabs[0].attached);
    assert!(scopes.0.borrow().is_empty());
    assert_eq!(tmux.calls.borrow().len(), 1);
}
#[test]
fn failed_restore_cleans_only_owned_placeholder() {
    let tmux = Recorder::default();
    *tmux.fail.borrow_mut() = Some("split-window".into());
    let out = execute(
        &plan(snapshot(), BTreeSet::new(), RunMode::Sandbox),
        &tmux,
        &Scopes::default(),
        Path::new("/tmp"),
    );
    assert!(out.tabs[0].error.is_some());
    assert!(tmux.cleaned.get());
}

#[test]
fn cancellation_or_identity_rejection_after_paste_cleans_owned_placeholder() {
    let tmux = Recorder {
        reject_after_paste: true,
        ..Recorder::default()
    };
    let out = execute(
        &plan(snapshot(), BTreeSet::new(), RunMode::Sandbox),
        &tmux,
        &Scopes::default(),
        Path::new("/private-home"),
    );
    assert!(out.tabs[0].error.is_some());
    assert!(tmux.pasted.get());
    assert!(!tmux.calls.borrow().iter().any(|(a, _)| a[0] == "send-keys"));
    assert!(
        tmux.cleaned.get(),
        "no Enter dispatched; exact owned placeholder must be cleaned"
    );
}

#[test]
fn backend_cancel_before_enter_dispatch_cleans_owned_placeholder() {
    let tmux = Recorder {
        reject_enter_at_backend: true,
        ..Recorder::default()
    };
    let out = execute(
        &plan(snapshot(), BTreeSet::new(), RunMode::Sandbox),
        &tmux,
        &Scopes::default(),
        Path::new("/private-home"),
    );
    assert!(out.tabs[0].error.is_some());
    assert!(tmux.pasted.get());
    assert!(!tmux.calls.borrow().iter().any(|(a, _)| a[0] == "send-keys"));
    assert!(tmux.cleaned.get());
    assert_eq!(
        tmux.dispatch_checks.get(),
        2,
        "cancellation occurs after ownership validation, at the final dispatch boundary"
    );
}

#[test]
fn uncertain_enter_result_preserves_owned_session() {
    let tmux = Recorder::default();
    *tmux.fail.borrow_mut() = Some("send-keys".into());
    let out = execute(
        &plan(snapshot(), BTreeSet::new(), RunMode::Sandbox),
        &tmux,
        &Scopes::default(),
        Path::new("/private-home"),
    );
    assert!(out.tabs[0].error.is_some());
    assert_eq!(
        tmux.calls
            .borrow()
            .iter()
            .filter(|(a, _)| a[0] == "send-keys")
            .count(),
        1
    );
    assert!(
        !tmux.cleaned.get(),
        "Enter may have reached tmux despite the failed result"
    );
}

#[test]
fn later_pre_dispatch_cancellation_preserves_session_when_first_enter_was_sent() {
    let tmux = Recorder {
        reject_second_paste: true,
        ..Recorder::default()
    };
    let out = execute(
        &plan(snapshot(), BTreeSet::new(), RunMode::Sandbox),
        &tmux,
        &Scopes::default(),
        Path::new("/private-home"),
    );
    assert!(out.tabs[0].error.is_some());
    assert_eq!(tmux.paste_count.get(), 2);
    assert_eq!(
        tmux.calls
            .borrow()
            .iter()
            .filter(|(a, _)| a[0] == "send-keys")
            .count(),
        1
    );
    assert!(!tmux.cleaned.get(), "first pane already received Enter");
}
#[test]
fn invalid_checksum_prevents_all_mutation() {
    let tmux = Recorder::default();
    let mut s = snapshot();
    s["windows"][0]["layout"] = "0000,120x32,0,0,1".into();
    let out = execute(
        &plan(s, BTreeSet::new(), RunMode::Sandbox),
        &tmux,
        &Scopes::default(),
        Path::new("/tmp"),
    );
    assert!(out.tabs[0].error.is_some());
    assert_eq!(tmux.calls.borrow().len(), 1);
    assert!(!tmux.cleaned.get());
}
#[test]
fn shadow_never_mutates_or_automatically_attaches() {
    let tmux = Recorder {
        shadow: true,
        ..Recorder::default()
    };
    let scopes = Scopes::default();
    let out = execute(
        &plan(snapshot(), BTreeSet::new(), RunMode::Shadow),
        &tmux,
        &scopes,
        Path::new("/tmp"),
    );
    assert!(!out.tabs[0].attached);
    assert!(!out.tabs[0].ambiguity.is_empty());
    assert_eq!(tmux.calls.borrow().len(), 1);
}
#[test]
fn remap_matches_python_tmux_snapshot_oracle() {
    let s = snapshot();
    let layout = s["windows"][0]["layout"].as_str().unwrap();
    let map = BTreeMap::from([("%1".into(), "%110".into()), ("%2".into(), "%120".into())]);
    let reference = "2674f366bb01b9db42f6f64b728929b3acfe8b83:lib/tmux_snapshot.py";
    let bytes = comandos_oracle::oracle_at(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden"),
        "app-restore-remap",
        &json!({"source":reference,"layout":layout,"mapping":map}),
        || {
            use std::{
                io::Write,
                process::{Command, Stdio},
            };
            let source = Command::new("git")
                .args(["show", reference])
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .output()
                .map_err(|e| e.to_string())?;
            if !source.status.success() {
                return Err("original tmux_snapshot source unavailable".into());
            }
            let python =
                std::env::var_os("COMANDOS_APP_ORACLE_PYTHON").unwrap_or_else(|| "python3".into());
            let mut child = Command::new(python)
                .args([
                    "-c",
                    "import json,sys; ns={'__name__':'tmux_snapshot_oracle'}; exec(compile(sys.stdin.read(), 'frozen-tmux-snapshot', 'exec'),ns); print(ns['remap_layout'](sys.argv[1],json.loads(sys.argv[2])))",
                    layout,
                    &serde_json::to_string(&map).unwrap(),
                ])
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", "/tmp/comandos-python-oracle-private-home")
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())?;
            let write = child
                .stdin
                .take()
                .ok_or("Python stdin unavailable")?
                .write_all(&source.stdout);
            let output = child.wait_with_output().map_err(|e| e.to_string())?;
            write.map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(String::from_utf8_lossy(&output.stderr).into_owned());
            }
            Ok(output.stdout)
        },
    );
    assert_eq!(
        remap_layout(layout, &map).unwrap(),
        String::from_utf8(bytes).unwrap().trim()
    );
}

#[test]
fn web_tabs_never_become_tmux_placeholders() {
    let tmux = Recorder::default();
    let plan = RestorePlan::build(
        &json!({"xterm-local":"Web"}),
        &json!({}),
        &BTreeSet::new(),
        RunMode::Sandbox,
    )
    .unwrap();
    let result = execute(&plan, &tmux, &Scopes::default(), Path::new("/tmp"));
    assert_eq!(result.tabs[0].key, "xterm-local");
    assert!(!result.tabs[0].attached);
    assert!(tmux.calls.borrow().is_empty());
}
