//! Mirrors a session of a private tmux through control mode into engines,
//! the way the terminal view does, and compares them with what tmux itself
//! holds. No display needed.
//!
//! `control_diff <socket> <session>`; commands on stdin, one per line:
//! `size <cols> <rows>`, `check <label>`, `own`, `break`, `quit`.
use alacritty_terminal::{
    index::{Column, Line},
    term::cell::Flags,
};
use comandos_app::term::{
    control::{Action, Session},
    engine::{Palette, TermEngine},
};
use std::{
    collections::BTreeMap,
    io::{BufRead, Read, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct Mirror {
    socket: String,
    stdin: std::process::ChildStdin,
    bytes: mpsc::Receiver<Vec<u8>>,
    session: Session,
    engines: BTreeMap<u32, TermEngine>,
    epoch: Instant,
    bad: u32,
    seeds: u32,
}

fn palette() -> Palette {
    Palette::xterm_default([211, 218, 228], [10, 13, 19], [211, 218, 228], [10, 13, 19], [60, 80, 120])
}

impl Mirror {
    fn apply(&mut self, bytes: &[u8]) {
        let (mut wire, mut actions) = (Vec::new(), Vec::new());
        self.session.read(bytes, &mut wire, &mut actions);
        self.send(&wire);
        let now = Instant::now();
        for action in actions {
            match action {
                Action::Layout(panes) => {
                    self.engines.retain(|id, _| panes.iter().any(|p| p.id == *id));
                    for p in panes {
                        let epoch = self.epoch;
                        let engine = self
                            .engines
                            .entry(p.id)
                            .or_insert_with(|| TermEngine::new(p.width, p.height, 4000, palette(), true, epoch));
                        if engine.size() != (p.width, p.height) {
                            engine.resize(p.width, p.height, (9, 18));
                        }
                    }
                }
                Action::Feed { pane, bytes } => {
                    if let Some(engine) = self.engines.get_mut(&pane) {
                        engine.feed(&bytes, now);
                        let _ = engine.drain();
                    }
                }
                Action::Seed { pane, size, bytes } => {
                    if let Some(engine) = self.engines.get_mut(&pane) {
                        if engine.size() != size {
                            engine.resize(size.0, size.1, (9, 18));
                        }
                        engine.feed(&bytes, now);
                        let _ = engine.drain();
                        self.seeds += 1;
                    }
                }
                // The view's own check, as it would repair: counted apart
                // from the comparison this tool makes from outside.
                Action::Check { pane, size, cursor, rows } => {
                    if let Some(engine) = self.engines.get_mut(&pane) {
                        let (ours, at) = engine.live_screen();
                        let same = engine.size() == size
                            && at == cursor
                            && comandos_app::term::control::first_difference(&ours, &rows).is_none();
                        println!("     own check %{pane}: {}", if same { "same" } else { "WRONG, filled again" });
                        if !same {
                            let mut wire = Vec::new();
                            self.session.reseed(pane, &mut wire);
                            self.send(&wire);
                        }
                    }
                }
                Action::Notice(_) | Action::Exited => {}
            }
        }
    }

    fn send(&mut self, wire: &[u8]) {
        if !wire.is_empty() {
            self.stdin.write_all(wire).expect("control stdin");
            self.stdin.flush().expect("control stdin");
        }
    }

    /// Reads until tmux has said nothing for `quiet`.
    fn settle(&mut self, quiet: Duration) {
        while let Ok(chunk) = self.bytes.recv_timeout(quiet) {
            self.apply(&chunk);
        }
    }

    fn tmux(&self, args: &[&str]) -> String {
        let out = Command::new("tmux").arg("-S").arg(&self.socket).args(args).output().expect("tmux");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn check(&mut self, label: &str) {
        self.settle(Duration::from_millis(400));
        let ids: Vec<u32> = self.engines.keys().copied().collect();
        for id in ids {
            let target = format!("%{id}");
            let state = self.tmux(&["display-message", "-p", "-t", &target, "#{cursor_x} #{cursor_y} #{pane_width} #{pane_height}"]);
            let theirs: Vec<String> =
                self.tmux(&["capture-pane", "-p", "-t", &target]).lines().map(|l| l.trim_end().to_string()).collect();
            let coloured: Vec<Vec<u8>> =
                self.tmux(&["capture-pane", "-p", "-e", "-t", &target]).lines().map(|l| l.as_bytes().to_vec()).collect();
            let engine = &self.engines[&id];
            let term = engine.engine().term();
            let grid = term.grid();
            let (cols, rows) = engine.size();
            let mut wrong = Vec::new();
            for row in 0..usize::from(rows) {
                let mut ours = String::new();
                for col in 0..usize::from(cols) {
                    let cell = &grid[Line(i32::try_from(row).unwrap_or(0))][Column(col)];
                    if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                        continue;
                    }
                    ours.push(if cell.c == '\t' { ' ' } else { cell.c });
                }
                let ours = ours.trim_end().to_string();
                let their = theirs.get(row).cloned().unwrap_or_default();
                if ours != their {
                    wrong.push((row, ours, their));
                }
            }
            // Rows that differ only in how line drawing is written are the same.
            let (live, _) = engine.live_screen();
            if comandos_app::term::control::first_difference(&live, &coloured).is_none() {
                wrong.clear();
            }
            let cursor = grid.cursor.point;
            let ours_state = format!("{} {} {cols} {rows}", cursor.column.0, cursor.line.0);
            let state = state.trim();
            if wrong.is_empty() && ours_state == state {
                println!("OK   {label} %{id} [{state}]");
                continue;
            }
            self.bad += 1;
            println!("DIFF {label} %{id}: {} rows differ; state ours [{ours_state}] tmux [{state}]", wrong.len());
            for (row, ours, their) in wrong.iter().take(6) {
                println!("  row {row}\n    ours: {ours:?}\n    tmux: {their:?}");
            }
        }
        println!("DONE {label}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (socket, name) = (args[1].clone(), args[2].clone());
    let mut child = Command::new("tmux")
        .args(["-S", &socket, "-C", "attach", "-t", &format!("={name}")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("tmux");
    let stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");
    let (tx, bytes) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 65536];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut mirror = Mirror {
        socket,
        stdin,
        bytes,
        session: Session::default(),
        engines: BTreeMap::new(),
        epoch: Instant::now(),
        bad: 0,
        seeds: 0,
    };
    // Commands arrive on their own thread so the stream is read meanwhile.
    let (ctx, commands) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if ctx.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        mirror.settle(Duration::from_millis(20));
        let line = match commands.try_recv() {
            Ok(line) => line,
            Err(mpsc::TryRecvError::Empty) => continue,
            Err(mpsc::TryRecvError::Disconnected) => break,
        };
        let mut words = line.split_whitespace();
        match words.next() {
            Some("size") => {
                let mut n = || words.next().and_then(|w| w.parse::<u16>().ok()).unwrap_or(80);
                let (cols, rows) = (n(), n());
                let mut wire = Vec::new();
                mirror.session.resize(cols, rows, &mut wire);
                mirror.send(&wire);
            }
            Some("check") => mirror.check(words.next().unwrap_or("-")),
            // What the view does when a pane goes quiet.
            Some("own") => {
                mirror.settle(Duration::from_millis(300));
                let mut wire = Vec::new();
                mirror.session.check(&mut wire);
                mirror.send(&wire);
                mirror.settle(Duration::from_millis(300));
            }
            // Breaks an engine on purpose, to see the own check repair it.
            Some("break") => {
                for engine in mirror.engines.values_mut() {
                    engine.feed(b"\r\nBROKEN ON PURPOSE\r\n\r\n", Instant::now());
                }
            }
            Some("quit") => break,
            _ => {}
        }
    }
    println!("TOTAL DIFF {} (seeds {})", mirror.bad, mirror.seeds);
    let _ = child.kill();
    let _ = child.wait();
}
