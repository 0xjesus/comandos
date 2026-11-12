//! Paints the Foco + repisa layout off screen: a terminal raster (a PNG from
//! `term_bench`) beside the real `FocusShelf` with sample cards.
//! focus_shot <terminal.png> <out.png> [ancho] [alto] [tema]
use comandos_app::feed::{Feed, FeedEntry, FeedState, PaneFeed};
use comandos_app::ui::focus::{self, FocusShelf};
use gtk::prelude::*;
use serde_json::Value;
use std::time::Duration;

fn pane(id: &str, state: FeedState, account: &str, reply: &str) -> PaneFeed {
    PaneFeed { pane: id.into(), state, ts: 1000., agent: "claude".into(), model: "opus-5".into(), account: account.into(), reply: reply.into() }
}
fn entry(session: &str, label: &str, state: FeedState, reply: &str, panes: Vec<PaneFeed>) -> FeedEntry {
    FeedEntry { session: session.into(), label: label.into(), state, unread: true, ts: 900., seen: 0., pane: "%1".into(),
        panes: panes.len() as u64, agent: "claude".into(), model: "opus-5".into(), prompt: String::new(), reply: reply.into(), ask: None, pane_list: panes }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [terminal, out, rest @ ..] = args.as_slice() else {
        eprintln!("uso: focus_shot <terminal.png> <out.png> [ancho] [alto] [tema]");
        std::process::exit(2);
    };
    let width: i32 = rest.first().and_then(|w| w.parse().ok()).unwrap_or(1500);
    let height: i32 = rest.get(1).and_then(|h| h.parse().ok()).unwrap_or(860);
    let theme_name = rest.get(2).map(String::as_str).unwrap_or("noche");
    gtk::init().expect("gtk");
    let theme = comandos_app::theme::desktop_theme(theme_name, &Value::Null).expect("tema");
    let css = gtk::CssProvider::new();
    if let Err(e) = css.load_from_data(comandos_app::theme::theme_css(&theme).as_bytes()) { eprintln!("css: {e}"); }
    gtk::StyleContext::add_provider_for_screen(&gdk::Screen::default().expect("screen"), &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

    let mut feed = Feed { now: 1060., ..Feed::default() };
    feed.sessions.insert("term-a".into(), entry("term-a", "Cobbler", FeedState::Ask, "", vec![
        pane("%1", FeedState::Ask, "main", "Ya reproduje los tres tickets críticos con evidencia de producción. ¿Sigo con el arreglo del T-0147 en local?"),
        pane("%2", FeedState::Work, "jesus-signara", "El borrador se podía eliminar aunque ya tuviera tickets impresos. Agregué la validación en el servicio y estoy escribiendo la prueba que lo cubre."),
        pane("%3", FeedState::Idle, "main", "Van las 57 preguntas completas, agrupadas por tema. Hay tres recomendaciones mías que comprometen a Golden Pets: revísalas antes de mandarlo."),
    ]));
    feed.sessions.insert("term-b".into(), entry("term-b", "SAVA", FeedState::Ask,
        "Sí al orden, primero Etapa 2 y luego Etapa 3. Pero hoy no sería seguro hacer el PR de Etapa 3 directo a feature-testing.", vec![]));
    feed.sessions.insert("term-c".into(), entry("term-c", "GoldenPets", FeedState::Idle,
        "Ya está el esqueleto del documento; empiezo por cómo usar las ligas.", vec![]));
    let cards = focus::cards(&feed, "term-a", Some("%1"), &|s| match s { "term-b" => "SAVA".into(), "term-c" => "GoldenPets".into(), _ => String::new() }, false);

    let shelf = FocusShelf::new();
    shelf.render(&cards, 3);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let pix = gdk_pixbuf::Pixbuf::from_file(terminal).expect("terminal png");
    let term_w = (width - 340).min(pix.width());
    let cropped = pix.new_subpixbuf(0, 0, term_w, height.min(pix.height()));
    let image = gtk::Image::from_pixbuf(Some(&cropped));
    image.set_valign(gtk::Align::Start);
    row.pack_start(&image, false, false, 0);
    shelf.widget.set_size_request(width - term_w, height);
    shelf.widget.show();
    row.pack_start(&shelf.widget, true, true, 0);
    let window = gtk::OffscreenWindow::new();
    window.set_size_request(width, height);
    window.add(&row);
    window.show_all();
    let end = std::time::Instant::now() + Duration::from_millis(600);
    while std::time::Instant::now() < end {
        while gtk::events_pending() { gtk::main_iteration(); }
        std::thread::sleep(Duration::from_millis(5));
    }
    let shot = window.pixbuf().expect("pixbuf");
    shot.savev(out, "png", &[]).expect("png");
    println!("{}x{} -> {out}", shot.width(), shot.height());
}
