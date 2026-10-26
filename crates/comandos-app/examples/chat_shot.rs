//! Renders the desktop chat view off screen (no window is shown) to a PNG, with the
//! app's theme CSS, from a JSON list of `[role, text]` messages.
//!
//! chat_shot <messages.json> <out.png> [width] [theme]
use gtk::prelude::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, out, rest @ ..] = args.as_slice() else {
        eprintln!("uso: chat_shot <messages.json> <out.png> [ancho] [tema]");
        std::process::exit(2);
    };
    let width: i32 = rest.first().and_then(|w| w.parse().ok()).unwrap_or(1100);
    let theme_name = rest.get(1).map(String::as_str).unwrap_or("noche");
    gtk::init().expect("gtk");
    let raw: Vec<(String, String)> =
        serde_json::from_str(&std::fs::read_to_string(input).expect("json")).expect("[[r,t]]");
    let theme = comandos_app::theme::desktop_theme(theme_name, &serde_json::Value::Null).expect("tema");
    let css = gtk::CssProvider::new();
    css.load_from_data(comandos_app::theme::theme_css(&theme).as_bytes()).expect("css");
    gtk::StyleContext::add_provider_for_screen(
        &gdk::Screen::default().expect("screen"),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let window = gtk::OffscreenWindow::new();
    let view = comandos_app::ui::app::chat_preview(&raw, Some(&theme));
    view.set_size_request(width, -1);
    window.add(&view);
    window.show_all();
    let settle = || {
        for _ in 0..200 {
            while gtk::events_pending() {
                gtk::main_iteration();
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    settle();
    // The reading column takes its width after the first layout: measure again.
    let (_, natural) = view.preferred_height_for_width(width);
    window.resize(width, natural);
    view.set_size_request(width, natural);
    settle();
    if std::env::var_os("CHAT_SHOT_EXPORT").is_some()
        && let Some((_, text)) = raw.iter().rev().find(|(r, _)| r == "a")
    {
        let colors = comandos_app::ui::chat_rich::Colors {
            dim: "#999999".into(),
            faint: "#777777".into(),
            brand: "#8b7cf6".into(),
            code_bg: "#000000".into(),
            quote_bg: "#111111".into(),
        };
        let palette = comandos_app::ui::chat_markdown::CodePalette::from_ansi(&[], "#777777", "#dddddd", "#000000");
        let hooks = comandos_app::ui::chat_rich::Hooks { copy: std::rc::Rc::new(|_, _| {}), open: std::rc::Rc::new(|_| {}) };
        let (html, plain) = comandos_app::ui::chat_rich::Rich::new(text, &colors, &palette, &hooks).export();
        println!("--- plain\n{plain}\n--- html\n{html}");
    }
    let pixbuf = window.pixbuf().expect("pixbuf");
    pixbuf.savev(out, "png", &[]).expect("png");
    println!("{}x{} -> {out}", pixbuf.width(), pixbuf.height());
}
