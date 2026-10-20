//! Attaches the app's terminal engine to a session of a PRIVATE tmux server (`-L`),
//! walks through a list of sizes like a window being resized, and at each size:
//! saves a PNG painted exactly like the app and compares every row with what tmux
//! itself holds. Exit status 1 when any row differs.
//!
//! term_bench <tmux -L label> <session> <out dir> <COLSxROWS>...
use comandos_app::term::{
    engine::{Palette, SCROLLBACK, TermEngine},
    pty::{PtySession, ReadOutcome},
    view::rasterize_png,
};
use serde_json::Value;
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

fn palette() -> Palette {
    let theme = comandos_app::theme::desktop_theme("noche", &Value::Null).expect("noche theme");
    let hex = |name: &str| {
        let h = theme.values[name].as_str().unwrap_or("#cccccc").trim_start_matches('#').to_owned();
        let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(204);
        [b(0), b(2), b(4)]
    };
    let mut palette = Palette::xterm_default(hex("fg"), hex("bg"), hex("text"), hex("bg"), hex("brand"));
    for (slot, h) in palette.ansi.iter_mut().zip(theme.ansi.iter()) {
        let h = h.trim_start_matches('#');
        if h.len() == 6 {
            *slot = [0, 2, 4].map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0));
        }
    }
    palette
}

/// Reads until the stream has been quiet for `quiet`, answering terminal queries.
fn pump(pty: &mut PtySession, term: &mut TermEngine, quiet: Duration, cap: Duration) {
    let start = Instant::now();
    let mut last = Instant::now();
    let mut buf = vec![0u8; 65536];
    while start.elapsed() < cap {
        match pty.read_chunk(&mut buf) {
            ReadOutcome::Data(n) => {
                term.feed(&buf[..n], Instant::now());
                last = Instant::now();
            }
            ReadOutcome::Closed => break,
            ReadOutcome::WouldBlock => {
                if last.elapsed() >= quiet {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        term.tick(Instant::now());
        let replies = term.drain().replies;
        if !replies.is_empty() {
            let _ = pty.write(&replies);
        }
        let _ = pty.flush_pending();
    }
    term.tick(Instant::now() + Duration::from_secs(1));
}

fn engine_rows(term: &TermEngine) -> Vec<String> {
    let grid = term.diagnostic_grid(1_000_000).expect("grid");
    grid["cells"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| {
            line.as_array()
                .unwrap()
                .iter()
                .map(|c| c["text"].as_str().unwrap_or(""))
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

fn tmux_rows(label: &str, session: &str) -> Vec<String> {
    let out = Command::new("tmux")
        .args(["-L", label, "capture-pane", "-p", "-t", &format!("={session}:")])
        .output()
        .expect("tmux capture-pane");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim_end().to_owned())
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [label, session, out, sizes @ ..] = args.as_slice() else {
        eprintln!("uso: term_bench <label> <session> <out> <COLSxROWS>...");
        std::process::exit(2);
    };
    assert!(label != "default" && !label.is_empty(), "solo un servidor -L propio");
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).expect("out dir");
    // Steps: COLSxROWS resizes, `sleep:SECS` keeps reading, `claim` nudges tmux
    // (same rows/columns, other pixel size) like the app does on focus.
    let steps: Vec<String> = sizes.to_vec();
    let sizes: Vec<(u16, u16)> = steps
        .iter()
        .filter_map(|s| s.split_once('x'))
        .map(|(c, r)| (c.parse().unwrap(), r.parse().unwrap()))
        .collect();
    let (c0, r0) = sizes[0];
    // BENCH_VIEWER=1 attaches like a mosaic tile (`-f ignore-size`).
    let viewer = std::env::var_os("BENCH_VIEWER").is_some();
    let mut argv: Vec<String> = ["tmux", "-L", label, "attach"].iter().map(|s| s.to_string()).collect();
    if viewer {
        argv.extend(["-f".into(), "ignore-size".into()]);
    }
    argv.extend(["-t".into(), format!("={session}")]);
    let mut pty = PtySession::spawn(&argv, c0, r0, std::path::Path::new("/tmp")).expect("spawn tmux client");
    let mut term = TermEngine::new(c0, r0, SCROLLBACK, palette(), false, Instant::now());
    pump(&mut pty, &mut term, Duration::from_millis(1500), Duration::from_secs(20));
    let mut failures = 0;
    for (step, spec) in steps.iter().enumerate() {
        if let Some(secs) = spec.strip_prefix("sleep:") {
            pump(&mut pty, &mut term, Duration::from_secs(secs.parse().unwrap()), Duration::from_secs(secs.parse().unwrap()));
        } else if spec == "claim" {
            let (c, r) = term.size();
            pty.claim_size(c, r).expect("claim");
        } else {
            let (c, r) = spec.split_once('x').unwrap();
            let (cols, rows) = (c.parse().unwrap(), r.parse().unwrap());
            if (cols, rows) != term.size() {
                pty.resize(cols, rows).expect("resize");
                term.resize(cols, rows, (8, 20));
            }
        }
        let (cols, rows) = term.size();
        pump(&mut pty, &mut term, Duration::from_millis(1500), Duration::from_secs(20));
        let shot = out.join(format!("{step:02}-{cols}x{rows}.png"));
        rasterize_png(&term, "Ubuntu Sans Mono", 12., 8., &shot).expect("png");
        let mine = engine_rows(&term);
        let theirs = tmux_rows(label, session);
        let top: usize = String::from_utf8_lossy(
            &Command::new("tmux")
                .args(["-L", label, "display", "-p", "-t", &format!("={session}:"), "#{pane_top}"])
                .output()
                .expect("tmux display")
                .stdout,
        )
        .trim()
        .parse()
        .unwrap_or(0);
        let mut diff = 0;
        for (i, want) in theirs.iter().enumerate() {
            let got = mine.get(i + top).map(String::as_str).unwrap_or("");
            if got != want {
                diff += 1;
                if diff <= 4 {
                    println!("  fila {i}:\n    tmux: {want:?}\n    app : {got:?}");
                }
            }
        }
        let window = Command::new("tmux")
            .args(["-L", label, "display", "-p", "-t", &format!("={session}:"), "#{window_width}x#{window_height}"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        println!(
            "paso {step} {spec} (app {cols}x{rows}, ventana tmux {window}): {} filas, {} distintas -> {}",
            theirs.len(),
            diff,
            shot.display()
        );
        failures += diff;
    }
    std::process::exit(i32::from(failures > 0));
}
