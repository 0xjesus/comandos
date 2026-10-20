//! Renders pane headers off screen at several widths, with the app's theme CSS, to
//! check that the buttons never cover the card text.
//!
//! header_shot <out.png> [theme] [widths...]
use gtk::prelude::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [out, rest @ ..] = args.as_slice() else {
        eprintln!("uso: header_shot <out.png> [tema] [anchos...]");
        std::process::exit(2);
    };
    let theme_name = rest.first().map(String::as_str).unwrap_or("noche");
    let widths: Vec<i32> = rest.iter().skip(1).filter_map(|w| w.parse().ok()).collect();
    let widths = if widths.is_empty() { vec![1200, 600, 420, 380, 300, 220] } else { widths };
    gtk::init().expect("gtk");
    let theme = comandos_app::theme::desktop_theme(theme_name, &serde_json::Value::Null).expect("tema");
    let css = gtk::CssProvider::new();
    css.load_from_data(comandos_app::theme::theme_css(&theme).as_bytes()).expect("css");
    gtk::StyleContext::add_provider_for_screen(
        &gdk::Screen::default().expect("screen"),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let pane = serde_json::json!({"pane":"%1","harness":"claude","motor":"claude","state":"verified","model":"fable-5-1","effort":"high","hAcct":"jesus-signara"});
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    for width in &widths {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some(&format!("{width}px")));
        label.set_size_request(60, -1);
        row.pack_start(&label, false, false, 0);
        row.pack_start(&comandos_app::ui::app::header_preview(&pane, "Cobbler", "jesus-signara", *width), false, false, 0);
        column.pack_start(&row, false, false, 0);
    }
    let window = gtk::OffscreenWindow::new();
    window.add(&column);
    window.show_all();
    for _ in 0..100 {
        while gtk::events_pending() {
            gtk::main_iteration();
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let pixbuf = window.pixbuf().expect("pixbuf");
    pixbuf.savev(out, "png", &[]).expect("png");
    println!("{}x{} -> {out}", pixbuf.width(), pixbuf.height());
}
