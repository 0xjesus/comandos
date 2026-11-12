//! Measures the CPU of a dashboard web view off screen (no window shown), first
//! as is and then with every CSS animation and transition stopped.
//! webview_cpu <url> [ancho] [alto] [segundos]
use gtk::prelude::*;
use std::time::{Duration, Instant};
use webkit2gtk::WebViewExt;

fn ticks(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| {
            let rest = s.rsplit_once(')')?.1.split_whitespace().collect::<Vec<_>>();
            Some(rest.get(11)?.parse::<u64>().ok()? + rest.get(12)?.parse::<u64>().ok()?)
        })
        .unwrap_or(0)
}
fn web_processes() -> Vec<u32> {
    let me = std::process::id().to_string();
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            std::fs::read_to_string(format!("/proc/{pid}/stat")).ok().is_some_and(|s| {
                s.contains("(WebKitWebProces") && s.rsplit_once(')').and_then(|(_, r)| r.split_whitespace().nth(1)) == Some(me.as_str())
            })
        })
        .collect()
}
fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        while gtk::events_pending() {
            gtk::main_iteration();
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}
fn measure(label: &str, secs: u64) {
    let pids = web_processes();
    let a: u64 = pids.iter().map(|p| ticks(*p)).sum();
    let me0 = ticks(std::process::id());
    pump(secs * 1000);
    let b: u64 = pids.iter().map(|p| ticks(*p)).sum();
    let me1 = ticks(std::process::id());
    println!(
        "{label}: proceso web {:.1}% · proceso UI {:.1}% ({pids:?})",
        (b - a) as f64 / secs as f64,
        (me1 - me0) as f64 / secs as f64
    );
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let url = args.first().cloned().unwrap_or_else(|| "http://127.0.0.1:4777/?web=native&app=1&anwin=1".into());
    let width: i32 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(352);
    let height: i32 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(1000);
    let secs: u64 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(10);
    gtk::init().expect("gtk");
    let view = webkit2gtk::WebView::new();
    let window = gtk::OffscreenWindow::new();
    view.set_size_request(width, height);
    window.add(&view);
    window.show_all();
    view.load_uri(&url);
    pump(8000);
    measure("como está", secs);
    view.evaluate_javascript(
        "document.head.insertAdjacentHTML('beforeend','<style>*,*::before,*::after{animation:none!important;transition:none!important}</style>')",
        None, None, None::<&gio::Cancellable>, |_| {},
    );
    pump(1000);
    measure("sin animaciones", secs);
}
