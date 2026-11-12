//! Lightweight lag log (`~/.cache/comandos/lag.log`): slow paints, slow
//! echo after a key, output waiting for its frame and main-loop stalls.
//! Only events over a threshold are written, so the cost stays near zero.
use std::cell::Cell;
use std::io::Write;
use std::time::Instant;

pub const SLOW_PAINT_MS: f64 = 8.;
pub const SLOW_ECHO_MS: f64 = 60.;
pub const SLOW_FRAME_MS: f64 = 34.;
pub const STALL_MS: f64 = 60.;

thread_local! {
    static OPENED: Cell<bool> = const { Cell::new(false) };
    static PHASES: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static SLOW: Cell<usize> = const { Cell::new(0) };
}

/// Adds `name=<ms>` to the breakdown of the paint being measured.
pub fn phase(name: &str, since: Instant) {
    let took = ms(since);
    PHASES.with(|p| p.borrow_mut().push_str(&format!("{name}={took:.1} ")));
}

/// Characters drawn one by one (the slow text path) in this paint.
pub fn count_slow(chars: usize) {
    SLOW.with(|s| s.set(s.get() + chars));
}

pub fn take_phases() -> String {
    let slow = SLOW.with(|s| s.replace(0));
    let mut out = PHASES.with(|p| std::mem::take(&mut *p.borrow_mut()));
    out.push_str(&format!("slow_chars={slow}"));
    out
}

pub fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.
}

pub fn note(line: &str) {
    let Some(home) = std::env::var_os("HOME") else { return };
    let dir = std::path::Path::new(&home).join(".cache/comandos");
    if !OPENED.with(Cell::get) {
        OPENED.with(|o| o.set(true));
        #[allow(clippy::disallowed_methods)]
        let _ = std::fs::create_dir_all(&dir);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("lag.log"))
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.);
        let _ = writeln!(file, "{now:.3} {line}");
    }
}

/// Watches the GTK main loop: a 20 ms tick that arrives late means the
/// loop was busy with something else for that long.
pub fn watch_main_loop() {
    let last = Cell::new(Instant::now());
    glib::timeout_add_local(std::time::Duration::from_millis(20), move || {
        let gap = ms(last.replace(Instant::now())) - 20.;
        if gap > STALL_MS {
            note(&format!("stall {gap:.0}ms"));
        }
        glib::ControlFlow::Continue
    });
}
