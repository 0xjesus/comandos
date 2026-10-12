//! Thin key-based adapter for the GTK notebook implementation owned by T7.
use gtk::prelude::*;
use std::collections::HashMap;

pub struct TabStripNotebook {
    notebook: gtk::Notebook,
    pages: HashMap<String, gtk::Widget>,
}

impl TabStripNotebook {
    pub fn new(notebook: gtk::Notebook) -> Self {
        notebook.set_show_tabs(false);
        notebook.set_show_border(false);
        Self {
            notebook,
            pages: HashMap::new(),
        }
    }

    pub fn widget(&self) -> &gtk::Notebook {
        &self.notebook
    }

    pub fn insert(&mut self, key: String, child: &gtk::Widget, label: &gtk::Widget) -> u32 {
        let index = self.notebook.append_page(child, Some(label));
        self.notebook.set_tab_reorderable(child, true);
        self.pages.insert(key, child.clone());
        index
    }

    pub fn focus(&self, key: &str) -> bool {
        let Some(child) = self.pages.get(key) else {
            return false;
        };
        if let Some(page) = self.notebook.page_num(child) {
            self.notebook.set_current_page(Some(page));
            true
        } else {
            false
        }
    }

    pub fn reorder(&self, key: &str, position: u32) -> bool {
        let Some(child) = self.pages.get(key) else {
            return false;
        };
        self.notebook.reorder_child(child, Some(position));
        true
    }

    pub fn remove(&mut self, key: &str) -> bool {
        let Some(child) = self.pages.remove(key) else {
            return false;
        };
        if let Some(page) = self.notebook.page_num(&child) {
            self.notebook.remove_page(Some(page));
            true
        } else {
            false
        }
    }

    pub fn overview(&self) -> Vec<String> {
        let mut pages: Vec<_> = self
            .pages
            .iter()
            .filter_map(|(key, child)| {
                self.notebook
                    .page_num(child)
                    .map(|index| (index, key.clone()))
            })
            .collect();
        pages.sort_by_key(|(index, _)| *index);
        pages.into_iter().map(|(_, key)| key).collect()
    }

    pub fn selected_key(&self) -> Option<String> {
        let current = self.notebook.current_page()?;
        self.pages
            .iter()
            .find(|(_, widget)| self.notebook.page_num(*widget) == Some(current))
            .map(|(key, _)| key.clone())
    }

    pub fn page_key(&self, page: &gtk::Widget) -> Option<String> {
        self.pages
            .iter()
            .find(|(_, widget)| *widget == page)
            .map(|(key, _)| key.clone())
    }

    pub fn page_index(&self, key: &str) -> Option<u32> {
        self.notebook.page_num(self.pages.get(key)?)
    }
}

#[derive(Clone)]
pub struct TabStripLayout(std::rc::Rc<StripLayout>);
struct StripLayout {
    root: gtk::Box,
    row: gtk::Box,
    navigation: gtk::Box,
    strip: gtk::Box,
    flow: gtk::Box,
    scroller: gtk::ScrolledWindow,
    rows_view: gtk::ScrolledWindow,
    start: gtk::Box,
    end: gtk::Box,
    items: std::cell::RefCell<Vec<(String, gtk::Widget)>>,
    rows: std::cell::Cell<bool>,
    height: std::cell::Cell<i32>,
    signature: std::cell::RefCell<Option<(i32, Vec<i32>)>>,
}
impl TabStripLayout {
    pub fn new(strip: &gtk::Box) -> Self {
        let flow = gtk::Box::new(gtk::Orientation::Vertical, 4);
        flow.style_context().add_class("tabstrip-tabs");
        flow.style_context().add_class("tabstrip-rows");
        flow.set_margin_start(6);
        flow.set_margin_end(6);
        flow.set_margin_top(6);
        flow.set_margin_bottom(6);
        strip.style_context().add_class("tabstrip-tabs");
        strip.set_spacing(0);
        let scroller = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        scroller.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
        scroller.set_overlay_scrolling(true);
        scroller.set_hexpand(true);
        scroller.add(strip);
        let rows_view = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        rows_view.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
        rows_view.set_propagate_natural_height(true);
        rows_view.set_hexpand(true);
        rows_view.set_valign(gtk::Align::Start);
        rows_view.set_no_show_all(true);
        rows_view.style_context().add_class("tabstrip-rows");
        rows_view.add(&flow);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.style_context().add_class("tabstrip");
        let start = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let end = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.pack_start(&start, false, false, 0);
        row.pack_start(&scroller, true, true, 0);
        row.pack_start(&rows_view, true, true, 0);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let navigation = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        navigation.set_no_show_all(true);
        navigation.set_halign(gtk::Align::Center);
        navigation.set_margin_top(6);
        let controls = gtk::Box::new(gtk::Orientation::Vertical, 0);
        controls.set_valign(gtk::Align::Start);
        controls.pack_start(&end, false, false, 0);
        controls.pack_start(&navigation, false, false, 0);
        row.pack_start(&controls, false, false, 0);
        root.pack_start(&row, false, false, 0);
        let inner = std::rc::Rc::new(StripLayout {
            root,
            row,
            navigation,
            strip: strip.clone(),
            flow,
            scroller,
            rows_view,
            start,
            end,
            items: std::cell::RefCell::new(Vec::new()),
            rows: std::cell::Cell::new(false),
            height: std::cell::Cell::new(0),
            signature: std::cell::RefCell::new(None),
        });
        let weak = std::rc::Rc::downgrade(&inner);
        inner.rows_view.connect_size_allocate(move |_, _| {
            if let Some(s) = weak.upgrade()
                && s.rows.get()
            {
                Self(s).reflow();
            }
        });
        let weak = std::rc::Rc::downgrade(&inner);
        inner
            .scroller
            .add_events(gdk::EventMask::SCROLL_MASK | gdk::EventMask::SMOOTH_SCROLL_MASK);
        inner.scroller.connect_scroll_event(move |_, e| {
            if let Some(s) = weak.upgrade() {
                let (dx, dy) = e.delta();
                let dx = if e.direction() == gdk::ScrollDirection::Smooth {
                    (if dx != 0. { dx } else { dy }) * 40.
                } else if matches!(
                    e.direction(),
                    gdk::ScrollDirection::Up | gdk::ScrollDirection::Left
                ) {
                    -80.
                } else {
                    80.
                };
                Self(s).scroll_by(dx);
            }
            glib::Propagation::Stop
        });
        inner.root.show_all();
        inner.rows_view.hide();
        Self(inner)
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.0.root
    }
    pub fn navigation(&self) -> &gtk::Box {
        &self.0.navigation
    }
    pub fn navigation_buttons(&self) -> Vec<gtk::Widget> {
        [&self.0.start, &self.0.end, &self.0.navigation]
            .into_iter()
            .flat_map(|host| host.children())
            .filter(|child| child.style_context().has_class("tab-cycle"))
            .collect()
    }
    fn arrange_navigation(&self, rows: bool) {
        for button in self.navigation_buttons() {
            let previous = button.widget_name() == "tab-cycle-prev";
            let host = if rows {
                &self.0.navigation
            } else if previous {
                &self.0.start
            } else {
                &self.0.end
            };
            if button.parent().as_ref() != Some(host.upcast_ref()) {
                detach_item(&button);
                host.pack_start(&button, false, false, 0);
            }
            if previous || !rows {
                host.reorder_child(&button, 0);
            }
            button.show();
        }
        self.0.navigation.set_visible(rows);
    }
    pub fn start(&self) -> &gtk::Box {
        &self.0.start
    }
    pub fn actions(&self) -> &gtk::Box {
        &self.0.end
    }
    pub fn rows(&self) -> bool {
        self.0.rows.get()
    }
    pub fn set_rows(&self, rows: bool) {
        self.arrange_navigation(rows);
        if self.0.rows.replace(rows) == rows {
            return;
        }
        self.0.height.set(0);
        self.0.rows_view.set_size_request(-1, -1);
        *self.0.signature.borrow_mut() = None;
        if rows {
            self.0.row.style_context().add_class("rows");
            self.0.scroller.hide();
            self.0.rows_view.show();
            self.0.flow.show();
        } else {
            self.0.row.style_context().remove_class("rows");
            self.0.rows_view.hide();
            self.0.scroller.show();
        }
        for b in [&self.0.start, &self.0.end] {
            for child in b.children() {
                if child.style_context().has_class("tab-cycle") {
                    child.set_visible(true);
                }
            }
            b.set_valign(if rows {
                gtk::Align::Start
            } else {
                gtk::Align::Fill
            });
            b.set_size_request(-1, if rows { 44 } else { -1 });
        }
        self.attach();
    }
    pub fn set_items(&self, items: Vec<(String, gtk::Widget)>) {
        if self.0.items.borrow().as_slice() == items.as_slice() {
            // Labels and available width can change without replacing widgets.
            self.reflow();
            return;
        }
        let changed = self
            .0
            .items
            .borrow()
            .iter()
            .map(|(k, _)| k)
            .ne(items.iter().map(|(k, _)| k));
        if changed {
            self.0.height.set(0);
            self.0.rows_view.set_size_request(-1, -1);
        }
        *self.0.items.borrow_mut() = items;
        *self.0.signature.borrow_mut() = None;
        self.attach();
    }
    pub fn entries(&self) -> Vec<(String, gtk::Widget)> {
        self.0.items.borrow().clone()
    }
    pub fn set_current(&self, key: &str) {
        for (k, item) in self.0.items.borrow().iter() {
            if k == key {
                let changed = !item.style_context().has_class("cur");
                item.style_context().add_class("cur");
                if changed && !self.rows() {
                    self.reveal(item);
                }
            } else {
                item.style_context().remove_class("cur");
            }
        }
    }
    pub fn scroll_by(&self, dx: f64) {
        if self.rows() {
            return;
        }
        let a = self.0.scroller.hadjustment();
        a.set_value(
            (a.value() + dx)
                .max(a.lower())
                .min((a.upper() - a.page_size()).max(a.lower())),
        );
    }
    fn reveal(&self, item: &gtk::Widget) {
        let a = self.0.scroller.hadjustment();
        let r = item.allocation();
        if r.width() <= 1 {
            return;
        }
        // The active tab sits in the middle of the strip, like the remote carousel.
        let center = f64::from(r.x()) + f64::from(r.width()) / 2. - a.page_size() / 2.;
        a.set_value(
            center
                .max(a.lower())
                .min((a.upper() - a.page_size()).max(a.lower())),
        );
    }
    fn attach(&self) {
        for (_, item) in self.0.items.borrow().iter() {
            detach_item(item);
            configure_name(item, self.rows());
            item.set_size_request(-1, if self.rows() { 28 } else { -1 });
            if let Some(child) = item
                .clone()
                .downcast::<gtk::EventBox>()
                .ok()
                .and_then(|b| b.child())
            {
                child.set_margin_start(if self.rows() { 10 } else { 12 });
                child.set_margin_end(if self.rows() { 10 } else { 12 });
            }
        }
        for child in self.0.strip.children() {
            self.0.strip.remove(&child);
        }
        if self.rows() {
            self.reflow();
        } else {
            for (_, item) in self.0.items.borrow().iter() {
                self.0.strip.pack_start(item, false, false, 0);
                item.show_all();
            }
        }
    }
    pub fn reflow(&self) {
        if !self.rows() {
            return;
        }
        let width = self.0.rows_view.allocated_width() - 12;
        if width <= 1 {
            return;
        }
        let items = self.0.items.borrow();
        let widths = items.iter().map(|_| 240).collect::<Vec<_>>();
        let signature = (width + 12, widths.clone());
        if self.0.signature.borrow().as_ref() == Some(&signature) {
            return;
        }
        *self.0.signature.borrow_mut() = Some(signature);
        for line in self.0.flow.children() {
            if let Ok(line) = line.downcast::<gtk::Box>() {
                for child in line.children() {
                    line.remove(&child);
                }
                self.0.flow.remove(&line);
            }
        }
        let rows = crate::workspace_view::wrap_rows(width, &widths);
        let columns = rows.first().map_or(0, Vec::len);
        for indices in rows {
            let missing = columns.saturating_sub(indices.len());
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            line.set_homogeneous(true);
            self.0.flow.pack_start(&line, false, false, 0);
            for index in indices {
                if let Some((_, item)) = items.get(index) {
                    detach_item(item);
                    line.pack_start(item, true, true, 0);
                    item.show_all();
                }
            }
            for _ in 0..missing {
                let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                line.pack_start(&spacer, true, true, 0);
                spacer.show();
            }
            line.show();
        }
    }
}
fn configure_name(widget: &gtk::Widget, rows: bool) {
    if widget.style_context().has_class("tab-name")
        && let Some(label) = widget.downcast_ref::<gtk::Label>()
    {
        label.set_ellipsize(if rows {
            pango::EllipsizeMode::End
        } else {
            pango::EllipsizeMode::None
        });
        label.set_max_width_chars(if rows { 24 } else { -1 });
    }
    if let Some(container) = widget.downcast_ref::<gtk::Container>() {
        for child in container.children() {
            configure_name(&child, rows);
        }
    }
}
fn detach_item(widget: &gtk::Widget) {
    if let Some(parent) = widget
        .parent()
        .and_then(|p| p.downcast::<gtk::Container>().ok())
    {
        parent.remove(widget);
    }
}

/// Navigate notebook pages with Local first, preserving all other page positions.
pub fn cycle_page(count: u32, current: Option<u32>, local: Option<u32>, delta: i32) -> Option<u32> {
    if count == 0 {
        return None;
    }
    let mut pages = (0..count).collect::<Vec<_>>();
    if let Some(local) = local.filter(|index| *index < count) {
        pages.retain(|index| *index != local);
        pages.insert(0, local);
    }
    let position = pages.iter().position(|index| Some(*index) == current);
    let position = position.map_or(0, |p| {
        (p as i64 + i64::from(delta)).clamp(0, i64::from(count) - 1) as usize
    });
    pages.get(position).copied()
}
