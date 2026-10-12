//! Adaptadores de Task14, con propietario débil y respuestas versionadas.
use super::*;
use webkit2gtk::{SettingsExt, WebViewExt};
const SHOW_FAVORITES: bool = false;
impl App {
    pub(in crate::ui) fn install_header(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.install_handler(
            "T14.wizard",
            Rc::new(move |_| {
                let app = weak.upgrade().filter(|a| a.writable()).ok_or_else(|| {
                    ui::app_commands::CommandError::Refused(
                        "restore not ready, shadow, or closed".into(),
                    )
                })?;
                app.active_notebook().show();
                app.dashboard_script("try{nsOpen()}catch(e){}");
                Ok(())
            }),
        );
        // open_ai_session_here pertenece al flujo de cuentas de T17.
        let weak = Rc::downgrade(self);
        self.install_handler(
            "T14.start_ai_here",
            Rc::new(move |args| {
                weak.upgrade()
                    .filter(|a| a.writable())
                    .ok_or_else(|| {
                        ui::app_commands::CommandError::Refused(
                            "restore not ready, shadow, or closed".into(),
                        )
                    })?
                    .handlers
                    .invoke("T17.start_ai_here", args)
            }),
        );
        for (button, key) in &self.header.actions {
            let weak = Rc::downgrade(self);
            let key = *key;
            button.connect_clicked(move |button| {
                if let Some(app) = weak.upgrade().filter(|a| {
                    !a.closed.load(Ordering::Acquire) && (key == "notif" || a.writable())
                }) {
                    match key {
                        "settings" => app.dashboard_click("btn-settings"),
                        "notif" => {
                            if let Err(error) = app.handlers.invoke("T17.notices", &json!({})) {
                                app.status.set_text(&error.to_string());
                            }
                        }
                        "pomo" => app.panel_popover(button, "pomo", 520, 640),
                        _ => {
                            if let Err(error) = app
                                .handlers
                                .invoke("T18.reader", &json!({"action":"toggle"}))
                            {
                                app.status.set_text(&error.to_string());
                            }
                        }
                    }
                }
            });
        }
        if let Some(theme) = self.applied_theme.borrow().as_ref() {
            self.header.paint(theme);
        }
    }
    pub(super) fn new_tab_menu(self: &Rc<Self>, button: &gtk::Button) {
        if !self.writable() {
            return;
        }
        let menu = gtk::Menu::new();
        for (experimental, label) in [
            (
                false,
                if self.english {
                    "New terminal here (VTE)  ·  Ctrl+T"
                } else {
                    "Nueva terminal aquí (VTE)  ·  Ctrl+T"
                },
            ),
            (
                true,
                if self.english {
                    "Experimental terminal (xterm.js, with ligatures)  ·  Ctrl+Shift+T"
                } else {
                    "Terminal experimental (xterm.js, con ligaduras)  ·  Ctrl+Shift+T"
                },
            ),
        ] {
            let item = gtk::MenuItem::with_label(label);
            let weak = Rc::downgrade(self);
            item.connect_activate(move |_| {
                if let Some(app) = weak.upgrade().filter(|a| a.writable()) {
                    if experimental {
                        if let Err(error) = app
                            .handlers
                            .invoke("open_xterm_tab", &json!({"session":"local"}))
                        {
                            app.status.set_text(&error.to_string());
                        }
                    } else {
                        app.new_terminal();
                    }
                }
            });
            menu.append(&item);
        }
        menu.show_all();
        menu.popup_at_widget(
            button,
            gdk::Gravity::SouthWest,
            gdk::Gravity::NorthWest,
            None,
        );
    }
    fn dashboard_script(self: &Rc<Self>, script: &str) {
        let weak = Rc::downgrade(self);
        self.webview.evaluate_javascript(
            script,
            None,
            None,
            Some(&self.protocol_cancellable),
            move |result| {
                if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire))
                    && let Err(e) = result
                {
                    app.status.set_text(&format!("Tablero: {e}"));
                }
            },
        );
    }
    fn dashboard_click(self: &Rc<Self>, id: &str) {
        if !self.writable() {
            return;
        }
        self.webview.show();
        self.dashboard_script(&format!(
            "try{{document.getElementById({}).click()}}catch(e){{}}",
            json!(id)
        ));
    }
    fn panel_popover(self: &Rc<Self>, button: &gtk::Button, panel: &str, width: i32, height: i32) {
        if !self.writable() {
            return;
        }
        if !self.popovers.borrow().contains_key(panel) {
            let (Some(base), Some(context)) = (self.cfg.dash_url(), self.webview.context()) else {
                return;
            };
            // Comparte el perfil aislado de la ventana y carga el panel una sola vez.
            let view = webkit2gtk::WebView::with_context(&context);
            if let Some(settings) = WebViewExt::settings(&view) {
                settings.set_enable_webaudio(true);
            }
            view.set_background_color(&gdk::RGBA::new(0., 0., 0., 0.));
            let pop = gtk::Popover::new(Some(button));
            pop.set_position(gtk::PositionType::Bottom);
            pop.style_context().add_class("cc-popover");
            view.set_size_request(width, height);
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            head.style_context().add_class("mosaic-head");
            let title = gtk::Label::new(Some("Pomodoro"));
            title.set_xalign(0.0);
            head.pack_start(&title, true, true, 8);
            let close = ui::icons::button("close", 14, "Cerrar (Esc)");
            let weak = pop.downgrade();
            close.connect_clicked(move |_| {
                if let Some(pop) = weak.upgrade() {
                    pop.popdown();
                }
            });
            head.pack_end(&close, false, false, 4);
            content.pack_start(&head, false, false, 0);
            content.pack_start(&view, true, true, 0);
            pop.add(&content);
            view.load_uri(&format!(
                "{base}/?panel={panel}&v={}",
                env!("CARGO_PKG_VERSION")
            ));
            self.popovers.borrow_mut().insert(panel.into(), (pop, view));
        } else if let Some((_, view)) = self.popovers.borrow().get(panel) {
            view.evaluate_javascript("try{typeof pomoRender==='function'&&pomoRender();typeof notifRender==='function'&&notifRender()}catch(e){}",None,None,Some(&self.protocol_cancellable),|_|{});
        }
        if let Some((pop, _)) = self.popovers.borrow().get(panel) {
            pop.show_all();
            pop.popup();
        }
    }
    pub(super) fn shutdown_header(&self) {
        for (_, (popover, view)) in self.popovers.borrow_mut().iter() {
            view.stop_loading();
            popover.popdown();
        }
        self.popovers.borrow_mut().clear();
        self.favorite_queue.borrow_mut().clear();
    }
    pub(super) fn paint_marks(&self) {
        let marks = self.work_marks.borrow();
        for (key, tab) in self.labels.borrow().iter() {
            let row = marks.row("session", key);
            tab.set_mark(row.get("mark").and_then(Value::as_str).unwrap_or("none"));
            self.marks.borrow_mut().insert(key.clone(), row);
        }
    }
    pub(super) fn fetch_marks(self: &Rc<Self>) {
        if self.closed.load(Ordering::Acquire) || self.marks_fetching.replace(true) {
            return;
        }
        let generation = self.work_marks.borrow().generation();
        let dash = self.dash.clone();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || dash.get("/work-marks", Duration::from_secs(3)),
            move |result| {
                if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) {
                    app.marks_fetching.set(false);
                    if let Ok(value) = result {
                        app.marks_received.set(true);
                        let adopted = app.work_marks.borrow_mut().adopt_poll(&value, generation);
                        if adopted {
                            app.paint_marks();
                        }
                    }
                }
            },
        );
    }
    pub(super) fn set_work_mark(self: &Rc<Self>, scope: &str, key: &str, value: &Value) {
        if !self.writable() {
            return;
        }
        let Some(request) = self.work_marks.borrow_mut().begin(scope, key, value) else {
            return;
        };
        if self.cfg.mode() == RunMode::Sandbox && self.cfg.dash_url().is_none() {
            let mut row = self.work_marks.borrow().row(scope, key);
            if let Some(obj) = row.as_object_mut() {
                if value.is_string() {
                    obj.insert("mark".into(), value.clone());
                } else if let Some(f) = value.get("favorite") {
                    obj.insert("favorite".into(), f.clone());
                }
                let revision = obj.get("revision").and_then(Value::as_u64).unwrap_or(0) + 1;
                obj.insert("revision".into(), json!(revision));
            }
            self.work_marks.borrow_mut().finish(&request, Some(&row));
            self.paint_marks();
            return;
        }
        let body = request.body.clone();
        let dash = self.dash.clone();
        let weak = Rc::downgrade(self);
        let closed = self.closed.clone();
        self.jobs.spawn(
            move || {
                if closed.load(Ordering::Acquire) {
                    Err(crate::dash_client::DashError::Cancelled)
                } else {
                    dash.post("/work-marks", &body, Duration::from_secs(5))
                }
            },
            move |result| {
                if let Some(app) = weak.upgrade().filter(|a| a.writable()) {
                    let row = match &result {
                        Ok((200, value)) => value.get("mark"),
                        Ok((409, value)) => value.get("current"),
                        _ => None,
                    };
                    if app.work_marks.borrow_mut().finish(&request, row) {
                        app.paint_marks();
                        match result {
                            Ok((200, _)) => {}
                            Ok((409, _)) => app.status.set_text(
                                "Otro dispositivo cambió esta marca; se muestra la actual.",
                            ),
                            other => app.status.set_text(&format!("Marcas: {other:?}")),
                        }
                    }
                }
            },
        );
    }
    pub(super) fn mark_menu(
        self: &Rc<Self>,
        session: &str,
        pane: Option<&str>,
        event: &gdk::EventButton,
    ) {
        let menu = gtk::Menu::new();
        self.append_mark_menu(&menu, session, pane);
        if menu.children().is_empty() {
            return;
        }
        menu.show_all();
        menu.popup_at_pointer(Some(event));
    }
    pub(super) fn append_mark_menu(
        self: &Rc<Self>,
        menu: &gtk::Menu,
        session: &str,
        pane: Option<&str>,
    ) {
        self.append_mark_menu_when(menu, session, pane, Rc::new(|| true));
    }
    pub(super) fn append_mark_menu_when(
        self: &Rc<Self>,
        menu: &gtk::Menu,
        session: &str,
        pane: Option<&str>,
        allowed: Rc<dyn Fn() -> bool>,
    ) {
        // Favorites are retired (11-oct): the work-mark section is empty.
        if !SHOW_FAVORITES {
            return;
        }
        let mut sections = Vec::new();
        if !session.is_empty() && session != "local" {
            sections.push((
                "session",
                session.to_string(),
                if self.english {
                    "This tab"
                } else {
                    "Esta pestaña"
                },
            ));
        }
        if let Some(key) = pane.and_then(|p| self.work_marks.borrow().pane_key(session, p)) {
            sections.push((
                "pane",
                key,
                if self.english {
                    "This pane"
                } else {
                    "Este pane"
                },
            ));
        }
        for (i, (scope, key, title)) in sections.iter().enumerate() {
            if i > 0 {
                menu.append(&gtk::SeparatorMenuItem::new());
            }
            if sections.len() > 1 {
                let head = gtk::MenuItem::with_label(title);
                head.set_sensitive(false);
                menu.append(&head);
            }
            let row = self.work_marks.borrow().row(scope, key);
            // Hecho / Aparcado / Esperando are retired (11-oct); only the favorite stays.
            let enabled = if *scope == "pane" {
                row.get("favorite")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            } else {
                self.registry.borrow().favorite_keys().contains(key)
            };
            let item = ui::marks::menu_item(
                "favorite",
                if self.english {
                    "Favorite"
                } else if *scope == "pane" {
                    "Favorito"
                } else {
                    "Favorita"
                },
                enabled,
            );
            let weak = Rc::downgrade(self);
            let key = key.clone();
            let scope = *scope;
            let session = session.to_string();
            let pane = pane.map(str::to_string);
            let allowed = allowed.clone();
            item.connect_activate(move |_| {
                if let Some(app) = weak.upgrade().filter(|a| a.writable() && allowed()) {
                    if scope == "pane" {
                        if pane.as_deref().is_some_and(|p| {
                            app.work_marks.borrow().pane_key(&session, p).as_deref() == Some(&key)
                        }) {
                            app.set_work_mark("pane", &key, &json!({"favorite":!enabled}));
                        }
                    } else {
                        app.toggle_favorite(&key);
                    }
                }
            });
            menu.append(&item);
        }
    }
    pub(super) fn merge_pending_favorites(&self, values: &Value) -> Value {
        let mut keys = values
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|k| *k != "local")
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        for (key, enabled) in self.favorite_pending.borrow().iter() {
            if *enabled {
                keys.insert(key.clone());
            } else {
                keys.remove(key);
            }
        }
        json!(keys)
    }
    pub(super) fn queue_favorite(self: &Rc<Self>, key: &str) {
        if !self.writable()
            || key.is_empty()
            || key == "local"
            || self.favorite_pending.borrow().contains_key(key)
        {
            return;
        }
        let enabled = !self
            .registry
            .borrow()
            .favorite_keys()
            .iter()
            .any(|k| k == key);
        self.favorite_pending
            .borrow_mut()
            .insert(key.into(), enabled);
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.apply_pending_favorites();
        self.favorite_queue.borrow_mut().push_back(key.into());
        self.post_next_favorite();
    }
    fn apply_pending_favorites(&self) {
        let values = self.merge_pending_favorites(&json!(self.registry.borrow().favorite_keys()));
        self.registry
            .borrow_mut()
            .apply_favorites(&values, self.generation.load(Ordering::Acquire));
        self.paint_favorites();
        for (key, tab) in self.labels.borrow().iter() {
            if let Some(b) = &tab.favorite {
                b.set_sensitive(!self.favorite_pending.borrow().contains_key(key));
            }
        }
    }
    fn post_next_favorite(self: &Rc<Self>) {
        if self.favorite_posting.get() {
            return;
        }
        let Some(key) = self.favorite_queue.borrow_mut().pop_front() else {
            return;
        };
        let enabled = self
            .favorite_pending
            .borrow()
            .get(&key)
            .copied()
            .unwrap_or(false);
        self.favorite_posting.set(true);
        let dash = self.dash.clone();
        let closed = self.closed.clone();
        let weak = Rc::downgrade(self);
        let requested = key.clone();
        self.jobs.spawn(
            move || {
                if closed.load(Ordering::Acquire) {
                    Err(crate::dash_client::DashError::Cancelled)
                } else {
                    dash.post(
                        "/prefs-set",
                        &json!({"favorite":{"session":requested,"enabled":enabled}}),
                        Duration::from_secs(5),
                    )
                }
            },
            move |result| {
                if let Some(app) = weak.upgrade().filter(|a| a.writable()) {
                    app.favorite_posting.set(false);
                    app.favorite_pending.borrow_mut().remove(&key);
                    app.generation.fetch_add(1, Ordering::AcqRel);
                    let values = if let Ok((200, value)) = &result
                        && let Some(values) = value.get("favorites")
                    {
                        values.clone()
                    } else {
                        app.status
                            .set_text("No se pudo guardar el favorito. Inténtalo de nuevo.");
                        let mut keys = app.registry.borrow().favorite_keys();
                        keys.retain(|k| k != &key);
                        if !enabled {
                            keys.push(key);
                        }
                        json!(keys)
                    };
                    let merged = app.merge_pending_favorites(&values);
                    app.registry
                        .borrow_mut()
                        .apply_favorites(&merged, app.generation.load(Ordering::Acquire));
                    app.apply_pending_favorites();
                    app.persist();
                    app.post_next_favorite();
                }
            },
        );
    }
}
