//! Control mode against a private tmux server: what the parser and the
//! commands were written from is what tmux really sends.
mod support;

use comandos_app::config::RunMode;
use comandos_app::term::control::{Ask, Commands, Event, Parser};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use support::tmux::TestTmux;

struct Client {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    bytes: mpsc::Receiver<Vec<u8>>,
    parser: Parser,
    commands: Commands,
    seen: Vec<Event>,
    /// Replies before this index are already matched to their command.
    replied: usize,
}

impl Client {
    fn attach(tmux: &TestTmux, session: &str) -> Self {
        let mut child = Command::new("tmux")
            .arg("-S")
            .arg(tmux.socket())
            .args(["-C", "attach", "-t", &format!("={session}")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let (tx, bytes) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self { child, stdin, bytes, parser: Parser::default(), commands: Commands::default(), seen: Vec::new(), replied: 0 }
    }

    fn send(&mut self, build: impl FnOnce(&mut Commands, &mut Vec<u8>)) {
        let mut wire = Vec::new();
        build(&mut self.commands, &mut wire);
        self.stdin.write_all(&wire).unwrap();
        self.stdin.flush().unwrap();
    }

    /// Reads until an event past `from` satisfies `found`; returns its index.
    fn wait(&mut self, from: usize, what: &str, found: impl Fn(&Event) -> bool) -> usize {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(i) = self.seen.iter().skip(from).position(&found) {
                return from + i;
            }
            let left = until.checked_duration_since(Instant::now()).unwrap_or_else(|| panic!("no {what}: {:?}", self.seen));
            if let Ok(chunk) = self.bytes.recv_timeout(left) {
                self.parser.feed(&chunk, &mut self.seen);
            }
        }
    }

    /// The next reply to a command of ours, with what it asked.
    fn reply(&mut self) -> (usize, Ask, bool, Vec<Vec<u8>>) {
        loop {
            let at = self.wait(self.replied, "reply", |e| matches!(e, Event::Reply { .. }));
            self.replied = at + 1;
            let Event::Reply { ok, lines, .. } = self.seen[at].clone() else { unreachable!() };
            if let Some(ask) = self.commands.answered(ok) {
                return (at, ask, ok, lines);
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_control_client_gets_pane_output_layout_history_and_input() {
    let Some(f) = TestTmux::for_mode(RunMode::Sandbox) else { return };
    f.tmux.new_session("big", 100, 30);
    assert!(f.tmux.raw(&["split-window", "-h", "-t", "=big:"]).status.success());
    let ids = String::from_utf8(f.tmux.raw(&["list-panes", "-t", "=big:", "-F", "#{pane_id}"]).stdout).unwrap();
    let panes: Vec<u32> = ids.lines().map(|l| l.trim_start_matches('%').parse().unwrap()).collect();
    assert_eq!(panes.len(), 2);

    let mut c = Client::attach(&f.tmux, "big");
    // The window takes this client's size and says how its panes ended up.
    c.send(|k, w| k.resize(w, 120, 40));
    let (at, ask, ok, _) = c.reply();
    assert_eq!((ask, ok), (Ask::Plain, true));
    let at = c.wait(at, "layout", |e| matches!(e, Event::LayoutChange { layout, .. } if (layout.cols, layout.rows) == (120, 40)));
    let Event::LayoutChange { layout, .. } = c.seen[at].clone() else { unreachable!() };
    assert_eq!(layout.panes.iter().map(|p| p.id).collect::<Vec<_>>(), panes);
    assert_eq!(layout.panes[0].height, 40);

    // Typed bytes reach the pane as typed; its output comes back raw, tagged
    // with the pane, including UTF-8 and escape sequences.
    let typed = "printf 'se\\303\\261al \\033[31mrojo\\033[0m\\n'\r";
    c.send(|k, w| k.send_bytes(w, panes[0], typed.as_bytes()));
    let mut got = Vec::new();
    let mut from = at;
    while !String::from_utf8_lossy(&got).contains("se\u{f1}al \x1b[31mrojo") {
        from = c.wait(from, "pane output", |e| matches!(e, Event::Output { pane, .. } if *pane == panes[0])) + 1;
        if let Event::Output { data, .. } = &c.seen[from - 1] {
            got.extend_from_slice(data);
        }
    }

    // A snapshot arrives as four replies in order, not mixed with the
    // output of other panes: history, screen rows, joined screen, state.
    c.send(|k, w| k.snapshot(w, panes[0], 50));
    let mut parts = Vec::new();
    while parts.len() < 6 {
        let (_, ask, ok, lines) = c.reply();
        if ask != Ask::Plain {
            assert!(ok, "{ask:?}");
            parts.push((ask, lines));
        }
    }
    let asks: Vec<Ask> = parts.iter().map(|(ask, _)| ask.clone()).collect();
    let pane = panes[0];
    assert_eq!(
        asks,
        vec![
            Ask::Above { pane },
            Ask::Rows { pane },
            Ask::Joined { pane },
            Ask::MainRows { pane },
            Ask::MainJoined { pane },
            Ask::State { pane }
        ]
    );
    // No full-screen program is running: nothing is covered.
    assert!(parts[3].1.iter().all(Vec::is_empty) && parts[4].1.iter().all(Vec::is_empty));
    let text = |i: usize| parts[i].1.iter().map(|l| String::from_utf8_lossy(l).into_owned()).collect::<Vec<_>>().join("\n");
    assert!(text(1).contains("se\u{f1}al \x1b[31mrojo"), "{}", text(1));
    assert!(text(2).contains("se\u{f1}al rojo"), "{}", text(2));
    let height: usize = text(5).split(' ').nth(2).and_then(|h| h.parse().ok()).unwrap();
    assert_eq!(parts[1].1.len(), height, "one line per screen row");
    assert!(text(5).starts_with(&format!("%{pane} ")), "{}", text(5));

    // A command that fails takes the rest of its line with it: one error
    // reply, and the next reply belongs to the next line.
    c.send(|k, w| {
        k.snapshot(w, 9999, 50);
        k.check(w, pane);
    });
    let (_, ask, ok, _) = c.reply();
    assert_eq!((ask, ok), (Ask::Above { pane: 9999 }, false));
    let (_, ask, ok, _) = c.reply();
    assert_eq!((ask, ok), (Ask::CheckRows { pane }, true));
    let (_, ask, ok, _) = c.reply();
    assert_eq!((ask, ok), (Ask::CheckState { pane }, true));

    // A wrong command is an error reply, and the client keeps working.
    c.send(|k, w| k.pause_after(w, 5));
    c.stdin.write_all(b"no-such-command\n").unwrap();
    let (_, _, ok, _) = c.reply();
    assert!(ok);
    let at = c.wait(c.replied, "error", |e| matches!(e, Event::Reply { .. }));
    assert!(matches!(&c.seen[at], Event::Reply { ok: false, lines, .. } if !lines.is_empty()));

    // The control client never drew a screen: the pane was not resized to a
    // terminal of its own and the session survives the client.
    drop(c);
    assert!(f.tmux.raw(&["has-session", "-t", "=big"]).status.success());
}
