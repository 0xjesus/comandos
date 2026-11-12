//! SVG de la misma familia Lucide que el escritorio Python y el tablero.
use gtk::prelude::*;
pub fn image(name: &str, pixels: i32, color: &str) -> gtk::Image {
    let svg = match name {
        "plus" => include_str!("../../../../dash/icons/plus.svg"),
        "terminal" => include_str!("../../../../dash/icons/terminal.svg"),
        "chat" => include_str!("../../../../dash/icons/chat.svg"),
        "arrow-up-down" => include_str!("../../../../dash/icons/arrow-up-down.svg"),
        "rows" => include_str!("../../../../dash/icons/rows.svg"),
        "chevron-left" => include_str!("../../../../dash/icons/chevron-left.svg"),
        "chevron-right" => include_str!("../../../../dash/icons/chevron-right.svg"),
        "chevron-up" => include_str!("../../../../dash/icons/chevron-up.svg"),
        "chevron-down" => include_str!("../../../../dash/icons/chevron-down.svg"),
        "panel-left" => include_str!("../../../../dash/icons/panel-left.svg"),
        "close" => include_str!("../../../../dash/icons/close.svg"),
        "minimize" => include_str!("../../../../dash/icons/minimize.svg"),
        "maximize" => include_str!("../../../../dash/icons/maximize.svg"),
        "timer" => include_str!("../../../../dash/icons/timer.svg"),
        "bell" => include_str!("../../../../dash/icons/bell.svg"),
        "settings" => include_str!("../../../../dash/icons/settings.svg"),
        "folder-open" => include_str!("../../../../dash/icons/folder-open.svg"),
        "file" => include_str!("../../../../dash/icons/file.svg"),
        "copy" => include_str!("../../../../dash/icons/copy.svg"),
        "external-link" => include_str!("../../../../dash/icons/external-link.svg"),
        "sparkles" => include_str!("../../../../dash/icons/sparkles.svg"),
        "columns" => include_str!("../../../../dash/icons/columns.svg"),
        "layers" => include_str!("../../../../dash/icons/layers.svg"),
        "eye" => include_str!("../../../../dash/icons/eye.svg"),
        "eye-off" => include_str!("../../../../dash/icons/eye-off.svg"),

        _ => return gtk::Image::from_icon_name(Some(name), gtk::IconSize::SmallToolbar),
    };
    let svg = svg.replace("currentColor", color);
    if let Ok(loader) = gdk_pixbuf::PixbufLoader::with_type("svg") {
        loader.set_size(pixels, pixels);
        if loader.write(svg.as_bytes()).is_ok() && loader.close().is_ok() {
            return gtk::Image::from_pixbuf(loader.pixbuf().as_ref());
        }
    }
    gtk::Image::from_icon_name(Some(name), gtk::IconSize::SmallToolbar)
}
pub fn button(name: &str, pixels: i32, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_widget_name(name);
    button.set_image(Some(&image(name, pixels, "#AAAAAA")));
    button.set_always_show_image(true);
    button.set_relief(gtk::ReliefStyle::None);
    button.style_context().add_class("tabplus");
    button.set_tooltip_text(Some(tooltip));
    button
}
