use super::*;
use ui::{app_commands::CommandError, modals::ModalKind};
use webkit2gtk::{UserContentManagerExt, WebViewExt};
type Cells = Rc<Vec<(String, String, gtk::Box, gtk::Label)>>;
impl App {
    pub(super) fn reader_action(self: &Rc<Self>, args: &Value) -> Result<(), CommandError> {
        let action = ui::app_commands::string_arg(args, "action", "toggle")?;
        match action {
            "close" => self.reader_close(),
            "terminal" => {
                self.t18
                    .reader_terminal
                    .set(args.get("on").is_some_and(comandos_core::json::truthy));
                self.reader_apply();
            }
            "toggle" if self.t18.reader_open.get() => self.reader_close(),
            "open" | "toggle" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or_default();
                let base = self
                    .cfg
                    .dash_url()
                    .ok_or_else(|| CommandError::Refused("dashboard unavailable".into()))?;
                let uri = ui::reader::reader_uri(base, id, env!("CARGO_PKG_VERSION"))
                    .map_err(CommandError::Invalid)?;
                let reuse = self.t18.reader_open.get()
                    && id.is_empty()
                    && self.t18.reader.borrow().is_some();
                if !reuse {
                    self.reader_close();
                    let page = ui::webview::create_page(&self.cfg, &self.webview, "centro", &uri)
                        .map_err(|e| CommandError::Failed(format!("{e:?}")))?;
                    let ticket = self.t18.reader_scope.ticket();
                    self.bind_aux_bridge(&page.view, &ticket, &uri, "reader");
                    self.t18.reader_host.pack_start(&page.view, true, true, 0);
                    *self.t18.reader.borrow_mut() = Some(page);
                }
                self.t18.reader_open.set(true);
                self.reader_apply();
                if let Some(page) = self.t18.reader.borrow().as_ref() {
                    page.view.grab_focus();
                }
            }
            _ => return Err(CommandError::Invalid("invalid reader action".into())),
        }
        Ok(())
    }
    fn reader_apply(self: &Rc<Self>) {
        let state = ui::reader::reader_layout_state(
            self.t18.reader_open.get(),
            self.t18.reader_terminal.get(),
        );
        self.t17
            .shelf
            .paned
            .set_visible(state.get("terminals").unwrap_or(&Value::Null) == true);
        self.t18
            .reader_host
            .set_visible(state.get("reader").unwrap_or(&Value::Null) == true);
        if state.get("reader").unwrap_or(&Value::Null) == true {
            if let Some(page) = self.t18.reader.borrow().as_ref() {
                page.view.show();
            }
            if state.get("terminals").unwrap_or(&Value::Null) == true {
                let weak = Rc::downgrade(self);
                let ticket = self.t18.reader_scope.ticket();
                self.queue_reader_layout(weak, ticket);
            }
        }
    }
    fn queue_reader_layout(&self, weak: std::rc::Weak<Self>, ticket: ui::snippets::Ticket) {
        if let Some(old) = self.t18.reader_layout.borrow_mut().take() {
            old.remove();
        }
        *self.t18.reader_layout.borrow_mut() = Some(glib::idle_add_local_once(move || {
            if let Some(app) = weak.upgrade() {
                app.t18.reader_layout.borrow_mut().take();
                if ticket.current() {
                    app.t18.reader_paned.set_position(
                        (f64::from(app.t18.reader_paned.allocated_width()) * 0.42) as i32,
                    );
                }
            }
        }));
    }
    fn reader_close(self: &Rc<Self>) {
        self.t18.reader_scope.advance();
        for (object, id) in self.t18.reader_signals.borrow_mut().drain(..) {
            object.disconnect(id);
        }
        if let Some(old) = self.t18.reader_layout.borrow_mut().take() {
            old.remove();
        }
        let page = self.t18.reader.borrow_mut().take();
        if let Some(page) = page {
            self.t18.reader_host.remove(&page.view);
            page.cancel();
        }
        self.t18.reader_open.set(false);
        self.reader_apply();
        self.focus_current_term();
    }
    fn bind_aux_bridge(
        self: &Rc<Self>,
        view: &webkit2gtk::WebView,
        ticket: &ui::snippets::Ticket,
        uri: &str,
        kind: &'static str,
    ) {
        use javascriptcore::ValueExt;
        let Some(manager) = view.user_content_manager() else {
            return;
        };
        let weak = Rc::downgrade(self);
        let target = view.downgrade();
        let ticket = ticket.clone();
        let frozen = uri.to_string();
        let base = self.cfg.dash_url().unwrap_or_default().to_string();
        let id = manager.connect_script_message_received(Some("centro"), move |_, result| {
            let (Some(app), Some(view)) = (weak.upgrade(), target.upgrade()) else {
                return;
            };
            if !ticket.current()
                || !app.writable()
                || view
                    .uri()
                    .is_none_or(|u| !ui::reader::message_owned(u.as_str(), &frozen, &base, kind))
            {
                return;
            }
            let raw = result
                .js_value()
                .map(|v| v.to_str().to_string())
                .unwrap_or_default();
            match ui::bridge::parse_bridge(&raw) {
                Ok(message) => {
                    if let Err(error) = app.dispatch_bridge(message) {
                        app.status.set_text(&error.to_string());
                    }
                }
                Err(error) => app.status.set_text(&error.to_string()),
            }
        });
        self.t18
            .reader_signals
            .borrow_mut()
            .push((manager.upcast(), id));
    }
    pub(super) fn open_web_modal(self: &Rc<Self>, uri: &str, label: &str) -> Result<(), CommandError> {
        if uri.len() > 8192 || !(uri.starts_with("http://") || uri.starts_with("https://")) {
            return Err(CommandError::Invalid("invalid web URL".into()));
        }
        let name = "link";
        self.close_window(name);
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_transient_for(Some(&self.window));
        window.set_modal(true);
        window.set_decorated(false);
        window.style_context().add_class("mosaic-zoom");
        let (w, h) = self.window.size();
        let size = ui::modals::modal_size(ModalKind::Url, w, h);
        window.set_default_size(size.0, size.1);
        window.set_position(gtk::WindowPosition::CenterOnParent);
        let page = ui::webview::create_page(&self.cfg, &self.webview, "centro", uri)
            .map_err(|e| CommandError::Failed(format!("{e:?}")))?;
        let scope = Rc::new(ui::snippets::Scope::default());
        let view = page.view.clone();
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        {
            container.style_context().add_class("mosaic-zoom");
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            head.style_context().add_class("mosaic-head");
            let title = gtk::Label::new(None);
            title.set_ellipsize(pango::EllipsizeMode::End);
            let title_text = if label.is_empty() { uri } else { label };
            let text = title_text.chars().take(80).collect::<String>();
            title.set_markup(&format!("<span weight=\"bold\"> 🌐 {}</span><span foreground=\"#8A8F98\" size=\"small\">  · Esc {}</span>",glib::markup_escape_text(&text),if self.english{"closes"}else{"cierra"}));
            title.set_xalign(0.0);
            head.pack_start(&title, true, true, 8);
            let close = ui::icons::button("close", 14, "Cerrar (Esc)");
            let weak = Rc::downgrade(self);
            close.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.close_window(name);
                }
            });
            head.pack_end(&close, false, false, 4);
            container.pack_start(&head, false, false, 0);
        }
        container.pack_start(&view, true, true, 0);
        window.add(&container);
        self.t18.windows.borrow_mut().insert(
            name.into(),
            super::app_t18::WindowView {
                window: window.clone(),
                page: Some(page),
                client: None,
                signals: vec![],
                closing: Rc::new(Cell::new(false)),
                scope,
            },
        );
        self.bind_window_close(name, &window, false);
        window.show_all();
        Ok(())
    }
    fn bind_window_close(self: &Rc<Self>, name: &'static str, window: &gtk::Window, fade: bool) {
        let weak = Rc::downgrade(self);
        let signal = window.connect_key_press_event(move |_, event| {
            if event.keyval() == gdk::keys::constants::Escape
                && let Some(app) = weak.upgrade()
            {
                if fade {
                    app.zoom_out();
                } else {
                    app.close_window(name);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        let signal2 = window.connect_delete_event(move |_, _| {
            if let Some(app) = weak.upgrade() {
                if let Some(owner) = app.t18.windows.borrow().get(name) {
                    owner.closing.set(true);
                }
                app.close_window(name);
            }
            glib::Propagation::Proceed
        });
        let signals = vec![
            (window.clone().upcast(), signal),
            (window.clone().upcast(), signal2),
        ];
        if let Some(owner) = self.t18.windows.borrow_mut().get_mut(name) {
            owner.signals.extend(signals);
        }
    }
    pub(super) fn open_web_tab(
        self: &Rc<Self>,
        uri: &str,
        label: &str,
    ) -> Result<(), CommandError> {
        if uri.len() > 8192 || !(uri.starts_with("http://") || uri.starts_with("https://")) {
            return Err(CommandError::Invalid("invalid web URL".into()));
        }
        let key = format!("web:{}", uri.chars().take(120).collect::<String>());
        if self.t18.web_tabs.borrow().contains_key(&key) {
            self.select(&key);
            return Ok(());
        }
        if self.t18.web_tabs.borrow().len() >= 40 {
            return Err(CommandError::Refused("web tab limit".into()));
        }
        let page = ui::webview::create_page(&self.cfg, &self.webview, "centro", uri)
            .map_err(|e| CommandError::Failed(format!("{e:?}")))?;
        let label = if label.is_empty() {
            uri.split('/').nth(2).unwrap_or(uri)
        } else {
            label
        };
        let title = format!("🌐 {label}").chars().take(40).collect::<String>();
        let tab = ui::tab_label::TabLabel::new(&title, false, true);
        tab.set_language(self.english);
        if let Some(button) = &tab.close {
            let weak = Rc::downgrade(self);
            let target = key.clone();
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.close_web_tab(&target);
                }
            });
        }
        let weak = Rc::downgrade(self);
        let target = key.clone();
        tab.item.connect_button_press_event(move |_, e| {
            if e.button() == 1
                && let Some(app) = weak.upgrade()
            {
                app.select(&target);
            }
            glib::Propagation::Proceed
        });
        let child = page.view.clone().upcast::<gtk::Widget>();
        self.workspace.register_tab(&key, &child);
        self.strip
            .borrow_mut()
            .insert(key.clone(), &child, tab.item.upcast_ref());
        self.registry.borrow_mut().insert(TabRecord {
            key: key.clone(),
            label: title,
            favorite: false,
            kind: TabKind::Web,
        });
        self.labels.borrow_mut().insert(key.clone(), tab);
        self.t18.web_tabs.borrow_mut().insert(key.clone(), page);
        child.show_all();
        self.sync_strip();
        self.select(&key);
        self.persist();
        Ok(())
    }
    pub(super) fn close_web_tab(self: &Rc<Self>, key: &str) {
        let page = self.t18.web_tabs.borrow_mut().remove(key);
        if let Some(page) = page {
            page.cancel();
            self.workspace.remove_tab(key);
            self.strip.borrow_mut().remove(key);
            self.registry.borrow_mut().archive(key, "closed");
            let label = self.labels.borrow_mut().remove(key);
            if let Some(label) = label
                && let Some(parent) = label
                    .item
                    .parent()
                    .and_then(|p| p.downcast::<gtk::Container>().ok())
            {
                parent.remove(&label.item);
            }
            self.sync_strip();
            self.forget_navigation_page(key);
            self.persist();
        }
    }
    pub(super) fn mosaic_open(self: &Rc<Self>) {
        if self.closed.load(Ordering::Acquire)
            || self.t18.mosaic.borrow().on
            || self.t18.mosaic_loading.replace(true)
        {
            return;
        }
        self.t18.mosaic_scope.advance();
        let ticket = self.t18.mosaic_scope.ticket();
        let worker = ticket.clone();
        let tmux = self.tmux.clone();
        let labels = self.registry.borrow().to_json();
        let closed = self.closed.clone();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(move||->Result<Vec<Value>,String>{if !worker.current()||closed.load(Ordering::Acquire){return Err("Mosaic cancelled".into());}let out=tmux.read(&["list-sessions","-F","#{session_name}"]).map_err(|e|format!("{e:?}"))?;if !worker.current()||closed.load(Ordering::Acquire)||!out.ok(){return Err("Mosaic sessions unavailable".into());}Ok(out.stdout.split_whitespace().map(|s|json!({"session":s,"label":labels.get(s).filter(|v|comandos_core::json::truthy(v)).cloned().unwrap_or(json!(s))})).collect())},move|result|{
            if let Some(app)=weak.upgrade().filter(|a|!a.closed.load(Ordering::Acquire))&&ticket.current(){app.t18.mosaic_loading.set(false);match result{Ok(sessions)=>app.build_mosaic(&sessions),Err(error)=>app.status.set_text(&error)}}
        });
    }
    fn build_mosaic(self: &Rc<Self>, sessions: &[Value]) {
        self.t18.mosaic.borrow_mut().open(sessions);
        if !self.t18.mosaic.borrow().on {
            return;
        }
        let sessions = self.t18.mosaic.borrow().sessions.clone();
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.style_context().add_class("mosaic-head");
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.set_markup(&format!(
            "<span weight=\"bold\">⊞ {}</span><span foreground=\"#8A8F98\"> · {} {}</span>",
            if self.english { "Mosaic" } else { "Mosaico" },
            sessions.len(),
            if self.english {
                "sessions — click a cell to type there"
            } else {
                "sesiones — click en una celda para teclear ahí"
            }
        ));
        head.pack_start(&title, true, true, 8);
        let close = ui::icons::button("close", 14, "Cerrar mosaico (Ctrl+G)");
        let weak = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.mosaic_close();
            }
        });
        head.pack_end(&close, false, false, 4);
        let side = ui::icons::button("layers", 14, "Mostrar/ocultar el tablero lateral");
        let weak = Rc::downgrade(self);
        side.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                let hidden = !app.t18.state.borrow().left_hidden;
                app.left_panel_set(hidden);
            }
        });
        head.pack_end(&side, false, false, 0);
        outer.pack_start(&head, false, false, 0);
        let grid = gtk::Grid::new();
        grid.set_row_spacing(4);
        grid.set_column_spacing(4);
        let mut cells = vec![];
        for (session, (row, column, span)) in sessions
            .into_iter()
            .zip(ui::mosaic::slots(self.t18.mosaic.borrow().sessions.len()))
        {
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 0);
            cell.style_context().add_class("mosaic-cell");
            let title = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            title.style_context().add_class("mosaic-cell-title");
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            label.set_markup(&format!(
                "<span size=\"small\" weight=\"bold\" foreground=\"#C9CDD4\"> {}</span>",
                glib::markup_escape_text(&session.label)
            ));
            let event = gtk::EventBox::new();
            event.add(&label);
            let weak = Rc::downgrade(self);
            let key = session.session.clone();
            event.connect_button_press_event(move |_, _| {
                if let Some(app) = weak.upgrade() {
                    app.mosaic_zoom(&key);
                }
                glib::Propagation::Stop
            });
            title.pack_start(&event, true, true, 0);
            let zoom = ui::icons::button("maximize", 12, "Enfocar en grande (con sus splits)");
            let weak = Rc::downgrade(self);
            let key = session.session.clone();
            zoom.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.mosaic_zoom(&key);
                }
            });
            title.pack_end(&zoom, false, false, 1);
            cell.pack_start(&title, false, false, 1);
            let placeholder = gtk::Label::new(Some("· · ·"));
            placeholder.style_context().add_class("mosaic-ph");
            cell.pack_start(&placeholder, true, true, 0);
            cell.set_hexpand(true);
            cell.set_vexpand(true);
            grid.attach(&cell, column, row, span, 1);
            cells.push((session.session, session.label, cell, placeholder));
        }
        outer.pack_start(&grid, true, true, 0);
        self.t17.shelf.close_extension();
        self.paned.remove(&self.t18.column);
        self.t18.column.hide();
        self.paned.pack2(&outer, true, false);
        outer.show_all();
        self.t18.mosaic_position.set(self.paned.position());
        self.paned.set_position(0);
        *self.t18.mosaic_box.borrow_mut() = Some(outer);
        *self.t18.cells.borrow_mut() = cells
            .iter()
            .map(|(s, _, c, _)| (s.clone(), c.clone()))
            .collect();
        self.mosaic_fill(Rc::new(cells), 0, 10);
    }
    fn mosaic_fill(self: &Rc<Self>, cells: Cells, index: usize, delay: u64) {
        if index >= cells.len() {
            return;
        }
        let ticket = self.t18.mosaic_scope.ticket();
        let weak = Rc::downgrade(self);
        *self.t18.fill.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_millis(delay),
            move || {
                let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                    return;
                };
                app.t18.fill.borrow_mut().take();
                if !ticket.current() || !app.t18.mosaic.borrow().on {
                    return;
                }
                let Some((session, _, cell, placeholder)) = cells.get(index) else {
                    return;
                };
                let id = format!("mosaic:{session}");
                match app.aux_terminal(session, &id, "mosaic", 400) {
                    Ok(overlay) => {
                        cell.remove(placeholder);
                        cell.pack_start(&overlay.widget, true, true, 0);
                        overlay.widget.show_all();
                        app.bind_mosaic_cell(&id, session, cell, &overlay.term);
                    }
                    Err(error) => app.status.set_text(&error),
                }
                app.mosaic_fill(cells, index + 1, 25);
            },
        ));
    }
    fn bind_mosaic_cell(
        self: &Rc<Self>,
        id: &str,
        session: &str,
        cell: &gtk::Box,
        term: &TermView,
    ) {
        let weak = Rc::downgrade(self);
        let w = cell.downgrade();
        let key = id.to_string();
        term.widget().connect_focus_in_event(move |_, _| {
            if let (Some(app), Some(cell)) = (weak.upgrade(), w.upgrade()) {
                let others = app.t18.cells.borrow().clone();
                for (other, widget) in others {
                    if widget != cell {
                        widget.style_context().remove_class("focused");
                        app.cell_tween(&format!("mosaic:{other}"), &widget, false);
                    }
                }
                cell.style_context().add_class("focused");
                app.cell_tween(&key, &cell, true);
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        let w = cell.downgrade();
        let key = id.to_string();
        term.widget().connect_focus_out_event(move |_, _| {
            if let (Some(app), Some(cell)) = (weak.upgrade(), w.upgrade()) {
                cell.style_context().remove_class("focused");
                app.cell_tween(&key, &cell, false);
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        let w = cell.downgrade();
        let session = session.to_string();
        let key = id.to_string();
        term.widget().connect_button_press_event(move |_, event| {
            if let (Some(app), Some(cell)) = (weak.upgrade(), w.upgrade()) {
                if event.button() == 1 && event.event_type() == gdk::EventType::DoubleButtonPress {
                    app.mosaic_zoom(&session);
                    return glib::Propagation::Stop;
                }
                cell.style_context().add_class("tap");
                if let Some(old) = app.t18.cell_taps.borrow_mut().remove(&key) {
                    old.remove();
                }
                let weak = Rc::downgrade(&app);
                let w = cell.downgrade();
                let id = key.clone();
                let ticket = app.t18.mosaic_scope.ticket();
                let source = glib::timeout_add_local_once(Duration::from_millis(160), move || {
                    if let Some(app) = weak.upgrade() {
                        app.t18.cell_taps.borrow_mut().remove(&id);
                        if ticket.current()
                            && let Some(cell) = w.upgrade()
                        {
                            cell.style_context().remove_class("tap");
                        }
                    }
                });
                app.t18.cell_taps.borrow_mut().insert(key.clone(), source);
            }
            glib::Propagation::Proceed
        });
    }
    fn cell_tween(self: &Rc<Self>, id: &str, cell: &gtk::Box, grow: bool) {
        if let Some(tick) = self.t18.cell_ticks.borrow_mut().remove(id) {
            tick.remove();
        }
        let cached_base = self.t18.cell_sizes.borrow().get(id).copied();
        let base = if let Some(base) = cached_base {
            base
        } else {
            let a = cell.allocation();
            if a.width() <= 50 {
                return;
            }
            let value = (a.width(), a.height());
            self.t18.cell_sizes.borrow_mut().insert(id.into(), value);
            value
        };
        let old = self
            .t18
            .cell_grown
            .borrow_mut()
            .insert(id.into(), grow)
            .unwrap_or(false);
        if old == grow {
            if !grow {
                cell.set_size_request(-1, -1);
            }
            return;
        }
        let key = id.to_string();
        let weak = Rc::downgrade(self);
        let ticket = self.t18.mosaic_scope.ticket();
        let tween = ui::side::Tween::new(
            if grow { 0. } else { 1. },
            if grow { 1. } else { 0. },
            if grow { 160 } else { 140 },
        );
        let tick = cell.add_tick_callback(move |cell, clock| {
            if !ticket.current() {
                return glib::ControlFlow::Break;
            }
            let (amount, done) = tween.step(clock.frame_time());
            cell.set_size_request(
                (f64::from(base.0) + (f64::from(base.0) * 0.12) * amount) as i32,
                (f64::from(base.1) + (f64::from(base.1) * 0.12) * amount) as i32,
            );
            if done {
                if !grow {
                    cell.set_size_request(-1, -1);
                }
                if let Some(app) = weak.upgrade() {
                    app.t18.cell_ticks.borrow_mut().remove(&key);
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        self.t18.cell_ticks.borrow_mut().insert(id.into(), tick);
    }
    pub(super) fn mosaic_close(self: &Rc<Self>) {
        self.t18.mosaic_scope.advance();
        self.t18.mosaic_loading.set(false);
        if let Some(fill) = self.t18.fill.borrow_mut().take() {
            fill.remove();
        }
        self.close_window("zoom");
        for (_, tick) in std::mem::take(&mut *self.t18.cell_ticks.borrow_mut()) {
            tick.remove();
        }
        for (_, tap) in std::mem::take(&mut *self.t18.cell_taps.borrow_mut()) {
            tap.remove();
        }
        let keys = self
            .t18
            .clients
            .borrow()
            .keys()
            .filter(|k| k.starts_with("mosaic:"))
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            self.close_aux(&key);
        }
        self.t18.cells.borrow_mut().clear();
        self.t18.cell_sizes.borrow_mut().clear();
        self.t18.cell_grown.borrow_mut().clear();
        let outer = self.t18.mosaic_box.borrow_mut().take();
        if let Some(outer) = outer {
            self.paned.remove(&outer);
            self.paned.pack2(&self.t18.column, true, false);
            self.t18.column.show();
            self.paned.set_position(self.t18.mosaic_position.get());
            self.focus_current_term();
        }
        self.t18.mosaic.borrow_mut().close();
    }
    pub(super) fn mosaic_zoom(self: &Rc<Self>, session: &str) {
        if !crate::tab_actions::valid_session(session) {
            return;
        }
        self.close_window("zoom");
        let label = self
            .registry
            .borrow()
            .to_json()
            .get(session)
            .and_then(Value::as_str)
            .unwrap_or(session)
            .to_string();
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_transient_for(Some(&self.window));
        window.set_modal(true);
        window.set_decorated(false);
        let (w, h) = self.window.size();
        window.set_default_size((w - 90).max(600), (h - 70).max(400));
        window.set_position(gtk::WindowPosition::CenterOnParent);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.style_context().add_class("mosaic-zoom");
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.style_context().add_class("mosaic-head");
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.set_markup(&format!("<span weight=\"bold\"> ⛶ {}</span><span foreground=\"#8A8F98\" size=\"small\">  · Esc {}</span>",glib::markup_escape_text(&label),if self.english{"back to mosaic"}else{"para volver al mosaico"}));
        head.pack_start(&title, true, true, 8);
        let close = ui::icons::button("close", 14, "Volver al mosaico (Esc)");
        let weak = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.zoom_out();
            }
        });
        head.pack_end(&close, false, false, 4);
        container.pack_start(&head, false, false, 0);
        let id = format!("zoom:{session}");
        let overlay = match self.aux_terminal(session, &id, "zoom", 10000) {
            Ok(overlay) => overlay,
            Err(error) => {
                self.status.set_text(&error);
                return;
            }
        };
        container.pack_start(&overlay.widget, true, true, 0);
        window.add(&container);
        self.t18.windows.borrow_mut().insert(
            "zoom".into(),
            super::app_t18::WindowView {
                window: window.clone(),
                page: None,
                client: Some(id),
                signals: vec![],
                closing: Rc::new(Cell::new(false)),
                scope: Rc::new(ui::snippets::Scope::default()),
            },
        );
        if self.t18.mosaic.borrow().on {
            let _ = self.t18.mosaic.borrow_mut().zoom(session);
        }
        self.bind_window_close("zoom", &window, true);
        window.set_opacity(0.0);
        window.show_all();
        overlay.term.widget().grab_focus();
        self.window_fade("zoom", 0.0, 1.0, 150, false);
    }
    fn zoom_out(self: &Rc<Self>) {
        if self.t18.zoom_closing.replace(true) {
            return;
        }
        if self.t18.windows.borrow().contains_key("zoom") {
            self.window_fade("zoom", 1.0, 0.0, 120, true);
        } else {
            self.t18.zoom_closing.set(false);
        }
    }
    fn window_fade(
        self: &Rc<Self>,
        name: &'static str,
        from: f64,
        to: f64,
        millis: u64,
        close: bool,
    ) {
        let key = format!("window:{name}");
        if let Some(tick) = self.t18.cell_ticks.borrow_mut().remove(&key) {
            tick.remove();
        }
        let window = self
            .t18
            .windows
            .borrow()
            .get(name)
            .map(|w| w.window.clone());
        let Some(window) = window else {
            return;
        };
        let tween = ui::side::Tween::new(from, to, millis);
        let weak = Rc::downgrade(self);
        let tick_key = key.clone();
        let tick = window.add_tick_callback(move |w, clock| {
            let (value, done) = tween.step(clock.frame_time());
            w.set_opacity(value);
            if done {
                if let Some(app) = weak.upgrade() {
                    app.t18.cell_ticks.borrow_mut().remove(&tick_key);
                    if close {
                        app.close_window(name);
                        app.t18.mosaic.borrow_mut().unzoom();
                        app.t18.zoom_closing.set(false);
                    }
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        self.t18.cell_ticks.borrow_mut().insert(key, tick);
    }
    pub(super) fn left_panel_set(self: &Rc<Self>, hidden: bool) {
        let was_hidden = !self.t18.left.is_visible();
        let position = self.paned.position();
        if hidden {
            self.t18
                .state
                .borrow_mut()
                .set_left(true, if !was_hidden { position } else { 0 });
            if !was_hidden {
                self.left_snapshot();
                let width = self.t18.snapshot.borrow().as_ref().map(|s| s.3);
                if let Some(width) = width {
                    self.left_fx_run(true, width);
                }
            }
            self.t18.left.hide();
        } else {
            let target = self.t18.state.borrow().left_pos.max(280);
            if was_hidden {
                self.left_fx_run(false, target);
            }
            self.t18.left.show();
            self.paned.set_position(target);
            self.t18.state.borrow_mut().set_left(false, position);
        }
        self.paint_left_button(hidden);
        self.queue_ui_save("app-layout.json", 600);
        self.quiet_js(&format!("window.appLeftPanel&&appLeftPanel({})", hidden));
    }
    pub(super) fn paint_left_button(&self, hidden: bool) {
        for widget in self.tab_layout.start().children() {
            if widget.widget_name() == "left-panel-toggle" {
                let css = widget.style_context();
                if hidden {
                    css.add_class("left-hidden");
                } else {
                    css.remove_class("left-hidden");
                }
                if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
                    let color = self
                        .applied_theme
                        .borrow()
                        .as_ref()
                        .and_then(|t| t.values.get(if hidden { "brand" } else { "dim" }))
                        .and_then(Value::as_str)
                        .unwrap_or("#AAAAAA")
                        .to_string();
                    button.set_image(Some(&ui::icons::image("panel-left", 18, &color)));
                }
                widget.set_tooltip_text(Some(if hidden {
                    if self.english {
                        "Show the left panel"
                    } else {
                        "Mostrar el panel izquierdo"
                    }
                } else if self.english {
                    "Hide the left panel"
                } else {
                    "Ocultar el panel izquierdo"
                }));
            }
        }
    }
    fn left_snapshot(&self) {
        self.t18.fx.borrow_mut().take();
        self.t18.snapshot.borrow_mut().take();
        if !self.t18.left.is_mapped() {
            return;
        }
        let a = self.t18.left.allocation();
        if a.width() < 40 || a.height() < 40 {
            return;
        }
        let Some(window) = self.t18.left.window() else {
            return;
        };
        let Some((x, y)) = self
            .t18
            .left
            .translate_coordinates(&self.modal_overlay, 0, 0)
        else {
            return;
        };
        if let Some(image) = window.pixbuf(a.x(), a.y(), a.width(), a.height()) {
            *self.t18.snapshot.borrow_mut() = Some((image, x, y, a.width(), a.height()));
        }
    }
    fn left_fx_run(self: &Rc<Self>, hiding: bool, width: i32) {
        use gdk::prelude::GdkContextExt;
        self.t18.fx.borrow_mut().take();
        if !self.presence_visible.get() || !ui::header::animations_enabled() {
            self.t18.snapshot.borrow_mut().take();
            return;
        }
        let snapshot = self
            .t18
            .snapshot
            .borrow_mut()
            .take()
            .filter(|s| hiding || (s.3 - width).abs() <= 2);
        let (x, y, h) = if let Some(s) = &snapshot {
            (s.1, s.2, s.4)
        } else {
            let Some((x, y)) = self.paned.translate_coordinates(&self.modal_overlay, 0, 0) else {
                return;
            };
            (x, y, self.paned.allocated_height())
        };
        let bar = self
            .applied_theme
            .borrow()
            .as_ref()
            .and_then(|t| t.values.get("bar"))
            .and_then(Value::as_str)
            .and_then(|s| gdk::RGBA::parse(s).ok())
            .unwrap_or_else(|| gdk::RGBA::new(0.04, 0.07, 0.11, 1.));
        let area = gtk::DrawingArea::new();
        area.set_halign(gtk::Align::Fill);
        area.set_valign(gtk::Align::Fill);
        let progress = Rc::new(Cell::new(0.0));
        let value = progress.clone();
        area.connect_draw(move |_, cr| {
            let frame = ui::side::left_frame(hiding, value.get(), f64::from(x), f64::from(width));
            if let Some(start) = frame.mask_start {
                cr.set_source_rgba(bar.red(), bar.green(), bar.blue(), 1.);
                cr.rectangle(
                    start,
                    f64::from(y),
                    f64::from(x + width) - start,
                    f64::from(h),
                );
                let _ = cr.fill();
            }
            if let Some((image, _, _, _, _)) = &snapshot {
                let _ = cr.save();
                cr.rectangle(f64::from(x), f64::from(y), f64::from(width), f64::from(h));
                cr.clip();
                cr.set_source_pixbuf(image, f64::from(x) + frame.offset, f64::from(y));
                let _ = cr.paint_with_alpha(frame.alpha);
                let _ = cr.restore();
            }
            if let Some(alpha) = frame.shadow_alpha {
                let shadow = cairo::LinearGradient::new(frame.edge, 0., frame.edge + 18., 0.);
                shadow.add_color_stop_rgba(0., 0., 0., 0., alpha);
                shadow.add_color_stop_rgba(1., 0., 0., 0., 0.);
                cr.rectangle(frame.edge, f64::from(y), 18., f64::from(h));
                let _ = cr.set_source(&shadow);
                let _ = cr.fill();
            }
            glib::Propagation::Proceed
        });
        self.modal_overlay.add_overlay(&area);
        self.modal_overlay.set_overlay_pass_through(&area, true);
        area.show();
        let weak = Rc::downgrade(self);
        let tween = ui::side::Tween::new(0., 1., 220);
        let tick = area.add_tick_callback(move |area, clock| {
            let (value, done) = tween.step(clock.frame_time());
            progress.set(value);
            area.queue_draw();
            if done {
                if let Some(app) = weak.upgrade() {
                    app.t18.fx.borrow_mut().take();
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        *self.t18.fx.borrow_mut() = Some(super::app_t18::LeftFx {
            area,
            tick: Some(tick),
        });
    }
}
