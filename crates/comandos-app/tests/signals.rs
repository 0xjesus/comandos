#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]
use comandos_app::ui::signals::{Action, SignalRelay, debug_script};
#[test]
fn signal_relay_owns_only_fresh_helper_and_releases_fds_after_twenty_cycles() {
    let out = comandos_app::proc::run(&comandos_app::proc::ProcSpec {
        program: std::env::current_exe().unwrap().display().to_string(),
        args: vec![
            "--ignored".into(),
            "--exact".into(),
            "private_signal_helper".into(),
            "--nocapture".into(),
        ],
        stdin: None,
        env: vec![("COMANDOS_OWN_SIGNAL_HELPER".into(), "yes".into())],
        clear_env: false,
        env_remove: vec![],
        cwd: None,
        timeout: std::time::Duration::from_secs(10),
    })
    .unwrap();
    assert_eq!(
        out.code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("OWN_SIGNAL_CYCLES=20"));
}
#[test]
#[ignore = "Only invoked as a fresh private subprocess, never in the shared test runner"]
fn private_signal_helper() {
    assert_eq!(std::env::var("COMANDOS_OWN_SIGNAL_HELPER").unwrap(), "yes");
    // /proc/self belongs exclusively to this newly launched private helper.
    let count = || std::fs::read_dir("/proc/self/fd").unwrap().count();
    let baseline = count();
    for _ in 0..20 {
        let mut relay = SignalRelay::new().unwrap();
        for action in [Action::Mosaic, Action::JavascriptDebug] {
            assert_ne!(action.signal(), nix::libc::SIGUSR1);
            signal_hook::low_level::raise(action.signal()).unwrap();
            let mut received = vec![];
            for _ in 0..100 {
                received.extend(relay.drain());
                if !received.is_empty() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert_eq!(received, vec![action]);
        }
        drop(relay);
        assert_eq!(count(), baseline);
    }
    println!("OWN_SIGNAL_CYCLES=20");
}
#[test]
fn debug_script_requires_private_owned_directory_regular_utf8_bounded_file() {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let root = std::env::temp_dir().join(format!("t18-debug-script-{}", std::process::id()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    let uid = root.metadata().unwrap().uid();
    let file = root.join("comandos-dbg.js");
    std::fs::write(&file, "fixture('ñ😀')").unwrap();
    assert_eq!(debug_script(&file, uid).unwrap(), "fixture('ñ😀')");
    assert!(debug_script(&file, uid.wrapping_add(1)).is_err());
    std::fs::write(&file, vec![b'x'; 262145]).unwrap();
    assert!(debug_script(&file, uid).is_err());
    std::fs::write(&file, [255u8]).unwrap();
    assert!(debug_script(&file, uid).is_err());
    std::fs::remove_file(&file).unwrap();
    std::os::unix::fs::symlink(root.join("absent"), &file).unwrap();
    assert!(debug_script(&file, uid).is_err());
    std::fs::remove_file(&file).unwrap();
    std::fs::write(&file, "fixture").unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(debug_script(&file, uid).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
