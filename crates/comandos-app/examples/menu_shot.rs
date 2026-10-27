//! Draws the terminal's right-click menu off screen (no window is shown) to a PNG,
//! with the real desktop CSS, one row hovered and one close row hovered.
//!
//! menu_shot <out.png> [theme] [panes]
use comandos_app::ui::menu::{MenuAction, MenuRow, is_destructive, terminal_rows};
use gtk::prelude::*;
use std::time::Duration;

// A box whose CSS name is "menu": a real menu only draws inside its own popup window,
// so this stands in for it with the same style rules.
mod imp {
    use gtk::subclass::prelude::*;
    #[derive(Default)]
    pub struct MenuLike;
    #[glib::object_subclass]
    impl ObjectSubclass for MenuLike {
        const NAME: &'static str = "MenuShotMenuLike";
        type Type = super::MenuLike;
        type ParentType = gtk::Box;
        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("menu");
        }
    }
    impl ObjectImpl for MenuLike {}
    impl WidgetImpl for MenuLike {}
    impl ContainerImpl for MenuLike {}
    impl BoxImpl for MenuLike {}
}
glib::wrapper! {
    pub struct MenuLike(ObjectSubclass<imp::MenuLike>) @extends gtk::Box, gtk::Container, gtk::Widget, @implements gtk::Orientable;
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().map(String::as_str).unwrap_or("menu.png");
    let theme_name = args.get(1).map(String::as_str).unwrap_or("noche");
    let npanes: usize = args.get(2).and_then(|n| n.parse().ok()).unwrap_or(2);
    gtk::init().expect("gtk");
    let theme = comandos_app::theme::desktop_theme(theme_name, &serde_json::Value::Null).expect("tema");
    let provider = gtk::CssProvider::new();
    let css = format!("{}\n{}", comandos_app::theme::base_css(&theme), comandos_app::theme::header_css(&theme));
    provider.load_from_data(css.as_bytes()).expect("css");
    gtk::StyleContext::add_provider_for_screen(
        &gdk::Screen::default().expect("screen"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let menu: MenuLike = glib::Object::new();
    menu.set_orientation(gtk::Orientation::Vertical);
    for row in terminal_rows(false, Some("fixture"), Some("%1"), npanes, true) {
        match row {
            MenuRow::Separator => menu.add(&gtk::SeparatorMenuItem::new()),
            MenuRow::Marks => {}
            MenuRow::Action { action, label, enabled } => {
                let item = gtk::MenuItem::with_label(&label);
                item.set_sensitive(enabled);
                if is_destructive(action) {
                    item.style_context().add_class("cc-danger");
                }
                if matches!(action, MenuAction::Split { side: "up", .. } | MenuAction::CloseTab) {
                    item.set_state_flags(gtk::StateFlags::PRELIGHT, false);
                }
                menu.add(&item);
            }
        }
    }
    let window = gtk::OffscreenWindow::new();
    window.add(&menu);
    window.show_all();
    let end = std::time::Instant::now() + Duration::from_millis(600);
    while std::time::Instant::now() < end {
        while gtk::events_pending() {
            gtk::main_iteration();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let pixbuf = window.pixbuf().expect("pixbuf");
    pixbuf.savev(out, "png", &[]).expect("png");
    println!("{}x{} -> {out}", pixbuf.width(), pixbuf.height());
}
