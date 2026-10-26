//! The tmux server must not be born inside the app's cgroup: closing the app kills
//! that cgroup, and with it every session (2026-09-04, 2026-10-26).
use comandos_app::tmux::detached_server_argv;
use std::path::Path;

#[test]
fn the_server_starts_in_its_own_scope_and_stays_up_when_empty() {
    let argv: Vec<String> = detached_server_argv(Path::new("/tmp/tmux-1000/default"), "comandos-tmux-1.scope")
        .into_iter()
        .map(|a| a.into_string().unwrap())
        .collect();
    assert_eq!(&argv[..4], ["--user", "--scope", "--collect", "--quiet"]);
    assert!(argv.contains(&"--unit=comandos-tmux-1.scope".to_owned()));
    assert!(argv.contains(&"ManagedOOMPreference=avoid".to_owned()));
    let tmux = argv.iter().position(|a| a == "tmux").unwrap();
    assert_eq!(&argv[tmux..], ["tmux", "-S", "/tmp/tmux-1000/default", "start-server", ";", "set-option", "-s", "exit-empty", "off"]);
    assert!(!argv.iter().any(|a| a.contains("kill")), "starting a server never kills anything");
}
