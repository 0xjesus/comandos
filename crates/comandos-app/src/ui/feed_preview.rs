//! Hover preview of a session tile (B4 with the G4 content). An override-redirect
//! popup under the tile: never takes the focus, never takes the pointer (empty
//! input shape, like `pointer-events:none` in the approved mockup) and never
//! covers the tile it describes.
use crate::feed::{self, FeedEntry};
use gtk::prelude::*;

pub struct FeedPreview {
    window: gtk::Window,
    card: gtk::Box,
    avatar: gtk::Label,
    glyph: gtk::Label,
    title: gtk::Label,
    time: gtk::Label,
    prompt: gtk::Label,
    quote: gtk::Box,
    question: gtk::Label,
    options: gtk::Box,
    answer_hint: gtk::Box,
    answer_keys: gtk::Box,
}

fn label(class: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_xalign(0.);
    l.style_context().add_class(class);
    l
}
fn kbd(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.style_context().add_class("k5-kbd");
    l
}
fn hex(color: gdk::RGBA) -> String {
    let byte = |v: f64| (v.clamp(0., 1.) * 255.).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(color.red()),
        byte(color.green()),
        byte(color.blue())
    )
}
fn escape(text: &str) -> String {
    glib::markup_escape_text(text).to_string()
}

impl FeedPreview {
    pub fn new(parent: &gtk::ApplicationWindow, english: bool) -> Self {
        let window = gtk::Window::new(gtk::WindowType::Popup);
        window.set_transient_for(Some(parent));
        window.set_type_hint(gdk::WindowTypeHint::Tooltip);
        window.set_accept_focus(false);
        window.set_focus_on_map(false);
        window.set_resizable(false);
        window.set_decorated(false);
        window.set_skip_taskbar_hint(true);
        window.set_skip_pager_hint(true);
        // Rounded corners need a compositor; without one the card stays square.
        if let Some(visual) = WidgetExt::screen(&window).and_then(|s| s.rgba_visual())
            && WidgetExt::screen(&window).is_some_and(|s| s.is_composited())
        {
            window.set_visual(Some(&visual));
            window.set_app_paintable(true);
            window.style_context().add_class("k5-peek-win");
        }
        // Click-through: the pointer belongs to whatever is under the card.
        window.connect_realize(|w| {
            if let Some(gdk_window) = w.window() {
                gdk_window.input_shape_combine_region(&cairo::Region::create(), 0, 0);
            }
        });
        let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card.style_context().add_class("k5-peek");
        card.set_size_request(feed::PREVIEW_WIDTH, -1);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.style_context().add_class("k5-peek-row");
        let avatar_box = gtk::Overlay::new();
        avatar_box.set_valign(gtk::Align::Start);
        let avatar = label("k5-av");
        avatar.set_xalign(0.5);
        avatar.set_size_request(28, 28);
        avatar_box.add(&avatar);
        let glyph = label("k5-av-glyph");
        glyph.set_halign(gtk::Align::End);
        glyph.set_valign(gtk::Align::End);
        avatar_box.add_overlay(&glyph);
        row.pack_start(&avatar_box, false, false, 0);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 5);
        body.set_hexpand(true);
        let line1 = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = label("k5-l1");
        title.set_ellipsize(pango::EllipsizeMode::End);
        title.set_max_width_chars(1);
        title.set_hexpand(true);
        line1.pack_start(&title, true, true, 0);
        let time = label("k5-tm");
        time.set_xalign(1.);
        line1.pack_end(&time, false, false, 0);
        body.pack_start(&line1, false, false, 0);

        let prompt = label("k5-yo");
        prompt.set_ellipsize(pango::EllipsizeMode::End);
        prompt.set_max_width_chars(1);
        prompt.set_no_show_all(true);
        body.pack_start(&prompt, false, false, 0);

        let quote = gtk::Box::new(gtk::Orientation::Vertical, 2);
        quote.style_context().add_class("k5-qt");
        let question = label("k5-qt-text");
        question.set_line_wrap(true);
        question.set_line_wrap_mode(pango::WrapMode::WordChar);
        question.set_ellipsize(pango::EllipsizeMode::End);
        question.set_lines(4);
        question.set_max_width_chars(1);
        quote.pack_start(&question, false, false, 0);
        let options = gtk::Box::new(gtk::Orientation::Vertical, 1);
        options.set_no_show_all(true);
        quote.pack_start(&options, false, false, 0);
        body.pack_start(&quote, false, false, 0);
        row.pack_start(&body, true, true, 0);
        card.pack_start(&row, false, false, 0);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        footer.style_context().add_class("k5-ft");
        let open = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        open.pack_start(
            &kbd(if english { "click" } else { "clic" }),
            false,
            false,
            0,
        );
        let open_text = label("k5-ft-text");
        open_text.set_text(if english { "open" } else { "abrir" });
        open.pack_start(&open_text, false, false, 0);
        footer.pack_start(&open, false, false, 0);
        let answer_hint = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        let answer_keys = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        answer_hint.pack_start(&answer_keys, false, false, 0);
        let answer_text = label("k5-ft-text");
        answer_text.set_text(if english { "answer" } else { "responder" });
        answer_hint.pack_start(&answer_text, false, false, 0);
        answer_hint.set_no_show_all(true);
        footer.pack_start(&answer_hint, false, false, 0);
        card.pack_start(&footer, false, false, 0);

        window.add(&card);
        card.show_all();
        Self {
            window,
            card,
            avatar,
            glyph,
            title,
            time,
            prompt,
            quote,
            question,
            options,
            answer_hint,
            answer_keys,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.window.is_visible()
    }

    /// Fill the card from the feed entry; `name` is the tile's own label.
    pub fn render(&self, name: &str, entry: &FeedEntry, now: f64, english: bool) {
        let asking = entry.state == feed::FeedState::Ask;
        let waits = entry.state.waits();
        self.avatar.set_text(&feed::initials(name));
        self.glyph.set_text(feed::agent_glyph(&entry.agent));
        let unread = entry.unread || waits;
        for (class, on) in [("k5-un", unread), ("k5-read", !unread)] {
            if on {
                self.card.style_context().add_class(class);
            } else {
                self.card.style_context().remove_class(class);
            }
        }
        // Name in text colour; the verb is violet only when the session waits.
        let context = self.title.style_context();
        let text = context.lookup_color("k5_text").map(hex);
        let brand = context.lookup_color("k5_brand").map(hex);
        let name_markup = match &text {
            Some(c) => format!("<span foreground=\"{c}\">{}</span>", escape(name)),
            None => escape(name),
        };
        let verb = escape(feed::verb(entry, english));
        let verb_markup = match (&brand, waits) {
            (Some(c), true) => format!("<span foreground=\"{c}\" weight=\"600\">{verb}</span>"),
            _ => verb,
        };
        self.title.set_markup(&format!(
            "<span weight=\"{}\">{name_markup}</span> {verb_markup}",
            if unread { 600 } else { 500 }
        ));
        self.time.set_text(&feed::time_ago(now, entry.ts, english));
        let prompt = feed::one_line(&entry.prompt);
        if prompt.is_empty() {
            self.prompt.hide();
        } else {
            self.prompt.set_markup(&format!(
                "<span alpha=\"75%\">{}</span> {}",
                if english { "You:" } else { "Tú:" },
                escape(&prompt)
            ));
            self.prompt.show();
        }
        let context = self.quote.style_context();
        if waits {
            context.add_class("k5-qt-ask");
        } else {
            context.remove_class("k5-qt-ask");
        }
        let quote = feed::quote(entry);
        self.question.set_text(quote);
        self.question.set_visible(!quote.is_empty());
        for child in self.options.children() {
            self.options.remove(&child);
        }
        let menu = feed::menu(entry);
        for option in menu {
            let line = label("k5-opt");
            line.set_ellipsize(pango::EllipsizeMode::End);
            line.set_max_width_chars(1);
            line.set_markup(&format!(
                "<b>{}.</b> {}",
                escape(&option.key),
                escape(&feed::one_line(&option.label))
            ));
            self.options.pack_start(&line, false, false, 0);
            line.show();
        }
        self.options.set_visible(!menu.is_empty());
        self.question.set_lines(if menu.is_empty() { 4 } else { 3 });
        for child in self.answer_keys.children() {
            self.answer_keys.remove(&child);
        }
        let keys = feed::answer_keys(entry);
        for key in &keys {
            let k = kbd(key);
            self.answer_keys.pack_start(&k, false, false, 0);
            k.show();
        }
        self.answer_hint.set_visible(asking && !keys.is_empty());
        // Shrink back after a longer card.
        self.window.resize(feed::PREVIEW_WIDTH, 1);
        self.card.queue_resize();
    }

    /// Open under `tile`, inside `toplevel`'s horizontal bounds.
    pub fn show_under(&self, tile: &gtk::Widget, toplevel: &gtk::ApplicationWindow) {
        let Some((x, y)) = widget_root(tile) else {
            return;
        };
        let (w, h) = (tile.allocated_width(), tile.allocated_height());
        let bounds = toplevel
            .window()
            .map(|win| {
                let (_, ox, _) = win.origin();
                (ox, ox + win.width())
            })
            .unwrap_or((x, x + feed::PREVIEW_WIDTH));
        let (px, py) =
            feed::preview_position((x, y, w, h), bounds, feed::PREVIEW_WIDTH, feed::PREVIEW_GAP);
        self.window.move_(px, py);
        if !self.window.is_visible() {
            self.window.show();
        }
    }

    pub fn hide(&self) {
        if self.window.is_visible() {
            self.window.hide();
        }
    }

    pub fn close(&self) {
        self.hide();
        self.window.close();
    }
}

/// Root coordinates of a widget's top-left corner. A tile (visible-window
/// EventBox) has its own GdkWindow at its allocation, which is exact even with
/// client-side decorations around the toplevel.
fn widget_root(widget: &gtk::Widget) -> Option<(i32, i32)> {
    if widget.has_window()
        && let Some(own) = widget.window()
    {
        let (_, x, y) = own.origin();
        return Some((x, y));
    }
    let toplevel = widget.toplevel()?;
    let (tx, ty) = widget.translate_coordinates(&toplevel, 0, 0)?;
    let (_, ox, oy) = toplevel.window()?.origin();
    Some((ox + tx, oy + ty))
}
