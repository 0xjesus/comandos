//! Etiqueta de pestaña: dos canales de estado, favorito y edición en la tira.
use gdk::prelude::*;
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
#[derive(Clone)]
pub struct TabLabel {
    pub item: gtk::EventBox,
    pub row: gtk::Box,
    pub name: gtk::EventBox,
    pub text: gtk::Label,
    pub favorite: Option<gtk::Button>,
    pub close: Option<gtk::Button>,
    pub suggest: Option<gtk::Button>,
    pub indicator: gtk::Image,
    pub dot: gtk::EventBox,
    sticker: gtk::Label,
    state: Rc<RefCell<String>>,
    mark: Rc<RefCell<String>>,
    editing: Rc<Cell<bool>>,
    painted: Rc<RefCell<String>>,
    cache: Rc<RefCell<super::marks::IndicatorCache>>,
    favorite_on: Rc<Cell<bool>>,
    favorite_frame: Rc<Cell<usize>>,
    last_seconds: Rc<Cell<f64>>,
    english: Rc<Cell<bool>>,
}
impl TabLabel {
    pub fn diagnostic_state(&self) -> serde_json::Value {
        serde_json::json!({"state":*self.state.borrow(),"mark":*self.mark.borrow(),"editing":self.editing.get()})
    }
    pub fn new(text: &str, local: bool, closable: bool) -> Self {
        let item = gtk::EventBox::new();
        item.set_visible_window(true);
        item.style_context().add_class("strip-tab");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_margin_start(12);
        row.set_margin_end(12);
        let indicator = gtk::Image::new();
        let dot = gtk::EventBox::new();
        dot.set_visible_window(false);
        dot.add(&indicator);
        // Status lights are retired (11-oct): the AI indicator never shows.
        dot.set_no_show_all(true);
        row.pack_start(&dot, false, false, 0);
        let text = gtk::Label::new(Some(text));
        text.set_xalign(0.);
        text.set_ellipsize(pango::EllipsizeMode::End);
        text.set_max_width_chars(24);
        text.style_context().add_class("tab-name");
        let name = gtk::EventBox::new();
        name.set_visible_window(false);
        name.add(&text);
        row.pack_start(&name, true, true, 0);
        let model_icon = gtk::Image::new();
        model_icon.set_no_show_all(true);
        row.pack_start(&model_icon, false, false, 0);
        let model = gtk::Label::new(None);
        model.set_xalign(0.);
        model.style_context().add_class("tab-model");
        row.pack_start(&model, false, false, 0);
        let sticker = gtk::Label::new(None);
        sticker.set_valign(gtk::Align::Center);
        sticker.style_context().add_class("tab-sticker");
        sticker.set_no_show_all(true);
        let suggest = (!local).then(|| {
            row.pack_start(&sticker, false, false, 0);
            let b = gtk::Button::with_label("¿Hecho? ✓");
            b.set_relief(gtk::ReliefStyle::None);
            b.set_focus_on_click(false);
            b.style_context().add_class("tab-suggest");
            b.set_no_show_all(true);
            b.set_valign(gtk::Align::Center);
            row.pack_start(&b, false, false, 0);
            b
        });
        // Favorites are retired (11-oct): no star on tabs.
        let favorite: Option<gtk::Button> = None;
        let close = closable.then(|| {
            let b = gtk::Button::new();
            b.set_image(Some(&super::icons::image("close", 12, "#AAAAAA")));
            b.set_relief(gtk::ReliefStyle::None);
            b.style_context().add_class("tabclose");
            b.set_valign(gtk::Align::Center);
            row.pack_start(&b, false, false, 0);
            b
        });
        item.add(&row);
        item.add_events(
            gdk::EventMask::ENTER_NOTIFY_MASK
                | gdk::EventMask::LEAVE_NOTIFY_MASK
                | gdk::EventMask::BUTTON_PRESS_MASK,
        );
        item.connect_enter_notify_event(|w, _| {
            w.set_state_flags(gtk::StateFlags::PRELIGHT, false);
            glib::Propagation::Proceed
        });
        item.connect_leave_notify_event(|w, e| {
            if e.detail() != gdk::NotifyType::Inferior {
                w.unset_state_flags(gtk::StateFlags::PRELIGHT);
            }
            glib::Propagation::Proceed
        });
        let out = Self {
            item,
            row,
            name,
            text,
            favorite,
            close,
            suggest,
            indicator,
            dot,
            sticker,
            state: Rc::new(RefCell::new(String::new())),
            mark: Rc::new(RefCell::new("none".into())),
            editing: Rc::new(Cell::new(false)),
            painted: Rc::new(RefCell::new(String::new())),
            cache: Rc::new(RefCell::new(super::marks::IndicatorCache::default())),
            favorite_on: Rc::new(Cell::new(false)),
            favorite_frame: Rc::new(Cell::new(usize::MAX)),
            last_seconds: Rc::new(Cell::new(0.)),
            english: Rc::new(Cell::new(false)),
        };
        out.item.show_all();
        out.paint(0.);
        out
    }
    pub fn widget(&self) -> gtk::Widget {
        self.item.clone().upcast()
    }
    pub fn set_language(&self, english: bool) {
        self.english.set(english);
        self.set_favorite(self.favorite_on.get());
        self.paint(self.last_seconds.get());
    }
    pub fn set_text(&self, text: &str) {
        self.text.set_text(text);
    }
    pub fn set_favorite(&self, value: bool) {
        if let Some(b) = &self.favorite {
            b.set_tooltip_text(Some(match (self.english.get(), value) {
                (true, true) => "Remove from favorites",
                (true, false) => "Add to favorites",
                (false, true) => "Quitar de favoritos",
                (false, false) => "Marcar como favorita",
            }));
            if self.favorite_on.replace(value) == value {
                return;
            }

            self.favorite_frame.set(usize::MAX);
            b.set_label(if value { "" } else { "☆" });
            if !value {
                b.set_image(gtk::Image::NONE);
            } else {
                self.paint_favorite(0.);
            }
            if value {
                b.style_context().add_class("favorite");
                b.set_opacity(1.);
            } else {
                b.style_context().remove_class("favorite");
            }
        }
    }
    pub fn set_state(&self, state: &str) {
        if *self.state.borrow() == state {
            return;
        }
        *self.state.borrow_mut() = state.into();
        self.paint(self.last_seconds.get());
    }
    pub fn set_mark(&self, mark: &str) {
        if *self.mark.borrow() == mark {
            return;
        }
        *self.mark.borrow_mut() = mark.into();
        self.paint(self.last_seconds.get());
    }
    pub fn paint(&self, seconds: f64) {
        self.last_seconds.set(seconds);
        // Hecho / Aparcado / Esperando and «¿Hecho? ✓» are retired (11-oct).
        self.sticker.hide();
        if let Some(suggest) = &self.suggest {
            suggest.hide();
        }
        if !self.dot.is_visible() {
            return;
        }
        self.paint_favorite(seconds);
        use comandos_core::work_marks as marks;
        let state = marks::ai_status(&self.state.borrow());
        let scale = self.indicator.scale_factor().max(1);
        let frame = marks::ai_frame_index(state, seconds).unwrap_or(0);
        let signature = format!(
            "{state}:{scale}:{frame}:{}:{}",
            self.mark.borrow(),
            self.english.get()
        );
        if *self.painted.borrow() == signature {
            return;
        }
        *self.painted.borrow_mut() = signature;
        if let Some(pb) =
            self.cache
                .borrow_mut()
                .pixbuf(&format!("ai:{state}"), None, frame, scale, None)
        {
            if let Some(surface) = pb.create_surface(scale, self.indicator.window().as_ref()) {
                self.indicator.set_from_surface(Some(&surface));
            } else {
                self.indicator.set_from_pixbuf(Some(&pb));
            }
        }
        self.indicator.set_tooltip_text(Some(&format!(
            "IA: {}",
            marks::ai_label(state, self.english.get())
        )));
    }
    fn paint_favorite(&self, seconds: f64) {
        if !self.favorite_on.get() {
            return;
        }
        let frame = comandos_core::work_marks::frame_index("favorite", seconds).unwrap_or(0);
        if self.favorite_frame.replace(frame) == frame {
            return;
        }
        if let Some(button) = &self.favorite
            && let Some(pb) = self.cache.borrow_mut().pixbuf(
                "favorite",
                None,
                frame,
                self.indicator.scale_factor().max(1),
                None,
            )
        {
            button.set_image(Some(&gtk::Image::from_pixbuf(Some(&pb))));
            button.set_always_show_image(true);
        }
    }
    pub fn begin_rename(&self, save: Rc<dyn Fn(String)>) {
        if self.editing.replace(true) {
            return;
        }
        let entry = gtk::Entry::new();
        let current = self.text.text().to_string();
        entry.set_text(&current);
        let n = (current.chars().count() + 1).clamp(6, 28) as i32;
        entry.set_width_chars(n);
        entry.set_max_width_chars(n);
        entry.set_has_frame(false);
        entry.set_alignment(0.);
        entry.style_context().add_class("tab-rename");
        self.name.hide();
        self.row.pack_start(&entry, false, false, 0);
        self.row.reorder_child(&entry, 1);
        entry.show();
        entry.grab_focus();
        entry.select_region(0, -1);
        let weak = entry.downgrade();
        let row = self.row.downgrade();
        let name = self.name.clone();
        let editing = self.editing.clone();
        let finish: Rc<dyn Fn(bool)> = Rc::new(move |accept| {
            if !editing.replace(false) {
                return;
            }
            if let Some(entry) = weak.upgrade() {
                let value = entry.text().trim().to_string();
                if let Some(row) = row.upgrade() {
                    row.remove(&entry);
                }
                name.show();
                if accept && !value.is_empty() && value != current {
                    save(value);
                }
            }
        });
        let f = finish.clone();
        entry.connect_activate(move |_| f(true));
        let f = finish.clone();
        entry.connect_focus_out_event(move |_, _| {
            f(true);
            glib::Propagation::Proceed
        });
        entry.connect_key_press_event(move |_, e| {
            if matches!(
                e.keyval(),
                gdk::keys::constants::Return | gdk::keys::constants::KP_Enter
            ) {
                finish(true);
                glib::Propagation::Stop
            } else if e.keyval() == gdk::keys::constants::Escape {
                finish(false);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
}
pub fn tab_label(label: &str, favorite: bool) -> gtk::Box {
    let tab = TabLabel::new(label, false, true);
    tab.set_favorite(favorite);
    tab.row.clone()
}
