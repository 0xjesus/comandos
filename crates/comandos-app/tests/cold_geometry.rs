#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod support;
use comandos_app::{
    config::RunMode,
    restore::{self, RestorePlan, ScopeLauncher},
    ui::overlays::read_geometry_when,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

struct ShellOnly;
impl ScopeLauncher for ShellOnly {
    fn resume(&self, _: &Value, _: &std::path::Path) -> Option<String> {
        None
    }
    fn argv(&self, _: &str, _: &str) -> Result<Vec<String>, String> {
        panic!("fixture must not start an agent")
    }
}

#[test]
fn cold_restore_geometry_and_optimized_snapshot_keep_unattached_local_alive() {
    let f = support::tmux::TestTmux::cold_for_mode(RunMode::Sandbox)
        .expect("private tmux required for cold geometry regression");
    let home = f.config.sandbox_root().unwrap().join("home");
    let plan = RestorePlan::build(
        &json!({"local":"Local"}),
        &json!({}),
        &BTreeSet::new(),
        RunMode::Sandbox,
    )
    .unwrap();
    let restored = restore::execute(&plan, &f.ctl, &ShellOnly, &home);
    assert!(restored.tabs[0].attached, "{restored:?}");
    let clients = f
        .ctl
        .read(&["list-clients", "-t", "=local", "-F", "#{client_tty}"])
        .unwrap();
    assert!(clients.ok() && clients.stdout.trim().is_empty());
    let before = f
        .ctl
        .read(&[
            "display-message",
            "-p",
            "-t",
            "=local:",
            "#{pid}|#{session_id}|#{s/[$]//:session_id}|#{pane_id}|#{pane_pid}",
        ])
        .unwrap();
    for _ in 0..3 {
        let geometry = read_geometry_when(&f.ctl, "local", || true);
        assert!(geometry.is_ok(), "cold geometry: {geometry:?}");
        assert_eq!(geometry.unwrap().len(), 1);
        let snapshot =
            comandos_app::snapshot::capture_with(&f.ctl, "local", |_| Ok(serde_json::Map::new()));
        assert!(snapshot.is_ok(), "cold snapshot: {snapshot:?}");
    }
    let after = f
        .ctl
        .read(&[
            "display-message",
            "-p",
            "-t",
            "=local:",
            "#{pid}|#{session_id}|#{s/[$]//:session_id}|#{pane_id}|#{pane_pid}",
        ])
        .unwrap();
    assert!(before.ok() && after.ok());
    assert_eq!(
        before.stdout, after.stdout,
        "reads must preserve owned identity"
    );
    assert!(f.ctl.read(&["has-session", "-t", "=local"]).unwrap().ok());
}

struct ReplaceDuringGeometry<'a> {
    fixture: &'a support::tmux::Fixture,
    replaced: std::cell::Cell<bool>,
}
impl comandos_app::ui::clipboard::TmuxIo for ReplaceDuringGeometry<'_> {
    fn mode(&self) -> RunMode {
        self.fixture.ctl.mode()
    }
    fn socket(&self) -> &std::path::Path {
        self.fixture.ctl.socket_path()
    }
    fn read(
        &self,
        args: &[&str],
    ) -> Result<comandos_app::tmux::TmuxOut, comandos_app::tmux::TmuxError> {
        if args.first() == Some(&"list-panes") && !self.replaced.replace(true) {
            assert!(
                self.fixture
                    .tmux
                    .raw(&["kill-session", "-t", "=local"])
                    .status
                    .success()
            );
            self.fixture.tmux.new_session("local", 120, 32);
        }
        self.fixture.ctl.read(args)
    }
    fn mutate(
        &self,
        _: &[&str],
        _: Option<&[u8]>,
    ) -> Result<comandos_app::tmux::TmuxOut, comandos_app::tmux::TmuxError> {
        panic!("geometry must not mutate the owned test server")
    }
}

#[test]
fn geometry_rejects_same_name_session_replacement_without_killing_new_session() {
    let f = support::tmux::TestTmux::for_mode(RunMode::Sandbox)
        .expect("private tmux required for identity regression");
    f.tmux.new_session("local", 120, 32);
    let replacing = ReplaceDuringGeometry {
        fixture: &f,
        replaced: std::cell::Cell::new(false),
    };
    assert_eq!(
        read_geometry_when(&replacing, "local", || true),
        Err("Session changed".into())
    );
    assert!(replacing.replaced.get());
    assert!(f.ctl.read(&["has-session", "-t", "=local"]).unwrap().ok());
    assert_eq!(
        read_geometry_when(&f.ctl, "local", || true).unwrap().len(),
        1
    );
}
