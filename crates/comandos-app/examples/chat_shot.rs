//! Draws the desktop chat page off screen (no window is shown) to a PNG, with WebKit,
//! from a saved `/chat/transcript` answer: `{"messages":[{r,t,h}], "ask":…, "busy":…}`.
//!
//! chat_shot <transcript.json> <out.png> [width] [height] [theme]
use comandos_app::ui::chat_page;
use gtk::prelude::*;
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc, time::Duration};
use webkit2gtk::WebViewExt;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, out, rest @ ..] = args.as_slice() else {
        eprintln!("uso: chat_shot <transcript.json> <out.png> [ancho] [alto] [tema]");
        std::process::exit(2);
    };
    let width: i32 = rest.first().and_then(|w| w.parse().ok()).unwrap_or(1100);
    let height: i32 = rest.get(1).and_then(|h| h.parse().ok()).unwrap_or(900);
    let theme_name = rest.get(2).map(String::as_str).unwrap_or("noche");
    gtk::init().expect("gtk");
    let data: Value = serde_json::from_str(&std::fs::read_to_string(input).expect("json")).expect("json");
    let theme = comandos_app::theme::desktop_theme(theme_name, &Value::Null).expect("tema");
    let ready = Rc::new(RefCell::new(false));
    let flag = ready.clone();
    let view = chat_page::view(move |m| {
        if m["type"] == "ready" {
            *flag.borrow_mut() = true;
        }
    });
    let window = gtk::OffscreenWindow::new();
    view.set_size_request(width, height);
    window.add(&view);
    window.show_all();
    let pump = |ms: u64| {
        let end = std::time::Instant::now() + Duration::from_millis(ms);
        while std::time::Instant::now() < end {
            while gtk::events_pending() {
                gtk::main_iteration();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    for _ in 0..400 {
        if *ready.borrow() {
            break;
        }
        pump(10);
    }
    let messages = data["messages"].clone();
    let state = json!({"messages": messages, "folded": 12, "busy": data["busy"], "first": true, "ask": data["ask"]});
    for script in [
        chat_page::call("theme", &chat_page::theme_vars(Some(&theme))),
        chat_page::call("update", &state),
    ] {
        view.evaluate_javascript(&script, None, None, None::<&gio::Cancellable>, |_| {});
    }
    pump(1200);
    let done = Rc::new(RefCell::new(None));
    let slot = done.clone();
    view.snapshot(
        webkit2gtk::SnapshotRegion::Visible,
        webkit2gtk::SnapshotOptions::NONE,
        None::<&gio::Cancellable>,
        move |result| {
            *slot.borrow_mut() = Some(result);
        },
    );
    for _ in 0..500 {
        if done.borrow().is_some() {
            break;
        }
        pump(10);
    }
    let surface = done.borrow_mut().take().expect("snapshot").expect("snapshot");
    let image = cairo::ImageSurface::try_from(surface).expect("image surface");
    let (w, h) = (image.width(), image.height());
    let pixbuf = gdk::pixbuf_get_from_surface(&image, 0, 0, w, h).expect("pixbuf");
    pixbuf.savev(out, "png", &[]).expect("png");
    println!("{w}x{h} -> {out}");
}
