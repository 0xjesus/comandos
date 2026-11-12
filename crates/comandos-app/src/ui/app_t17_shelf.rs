use super::*;
use javascriptcore::ValueExt;
use ui::{
    app_commands::{self as commands, CommandError},
    extensions::{self, ShelfView},
};
use webkit2gtk::{UserContentManagerExt, WebViewExt};
impl App {
    pub(super) fn install_shelf(self: &Rc<Self>) {
        if let Some(theme) = self.applied_theme.borrow().as_ref()
            && let Err(error) = self.t17.shelf.paint(theme)
        {
            self.status.set_text(&format!("Shelf theme: {error}"));
        }
        for name in ["T17.extensions", "T17.start_ai_here"] {
            let weak = Rc::downgrade(self);
            self.install_handler(
                name,
                Rc::new(move |args| {
                    let app = weak
                        .upgrade()
                        .filter(|a| !a.closed.load(Ordering::Acquire))
                        .ok_or_else(|| CommandError::Refused("application closed".into()))?;
                    match name {
                        "T17.extensions" => {
                            let session = commands::session_arg(args, "")?;
                            let pane = commands::string_arg(args, "pane", "")?;
                            let harness = commands::string_arg(args, "harness", "")?;
                            app.open_extension(session, pane, harness);
                            Ok(())
                        }
                        _ => {
                            let session = commands::string_arg(args, "session", "")?;
                            let pane = commands::string_arg(args, "pane", "")?;
                            if !session.is_empty() && !crate::tab_actions::valid_session(session) {
                                return Err(CommandError::Invalid("invalid session".into()));
                            }
                            app.start_ai_in_pane(session, pane);
                            Ok(())
                        }
                    }
                }),
            );
        }
        let weak = Rc::downgrade(self);
        self.t17.shelf.paned.connect_position_notify(move |_| {
            if let Some(app) = weak.upgrade() {
                app.queue_shelf_save();
            }
        });
        let weak = Rc::downgrade(self);
        self.t17.shelf.paned.connect_destroy(move |_| {
            if let Some(app) = weak.upgrade() {
                app.t17.shelf.cancel();
            }
        });
    }
    fn shelf_height_position(&self) {
        let shelf = &self.t17.shelf;
        let total = f64::from(shelf.paned.allocated_height());
        let height = extensions::shelf_height(
            total,
            shelf.saved.borrow().get("height").and_then(Value::as_f64),
        );
        shelf
            .paned
            .set_position((total - height - f64::from(extensions::SHELF_HANDLE)) as i32);
    }
    fn shelf_read_height(self: &Rc<Self>) {
        if self.t17.shelf.loaded.replace(true) {
            return;
        }
        let state = self.state.clone();
        let weak = Rc::downgrade(self);
        let ticket = self.t17.shelf.request.ticket();
        self.jobs.spawn(
            move || state.read_pane_document("app-extension-shelf.json"),
            move |result| {
                let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                    return;
                };
                if !ticket.current() {
                    app.t17.shelf.loaded.set(false);
                    return;
                }
                match result {
                    Ok(value) => {
                        *app.t17.shelf.saved.borrow_mut() = value;
                        app.shelf_height_position();
                    }
                    Err(error) => app.status.set_text(&format!("Shelf state: {error:?}")),
                }
            },
        );
    }
    pub(super) fn open_extension(self: &Rc<Self>, session: &str, pane: &str, harness: &str) {
        let Some(base) = self.cfg.dash_url() else {
            self.status.set_text("Dashboard disconnected");
            return;
        };
        let uri = match extensions::extension_uri(
            base,
            session,
            pane,
            harness,
            env!("CARGO_PKG_VERSION"),
        ) {
            Ok(uri) => uri,
            Err(error) => {
                self.status.set_text(&error.to_string());
                return;
            }
        };
        let Some(instance) = self
            .term_for_action(session)
            .as_ref()
            .map(TermView::cleanup_cancellation)
        else {
            self.status.set_text("Extension terminal unavailable");
            return;
        };
        if self.t17.shelf.extension.borrow().as_ref().is_some_and(|v| {
            v.uri == uri
                && v.instance
                    .as_ref()
                    .is_some_and(|old| Arc::ptr_eq(old, &instance))
        }) {
            return;
        }
        self.t17.shelf.request.advance();
        let ticket = self.t17.shelf.request.ticket();
        let work_ticket = ticket.clone();
        let session = session.to_string();
        let pane = pane.to_string();
        let harness = harness.to_string();
        let tmux = self.tmux.clone();
        let worker_instance = instance.clone();
        let shadow = self.cfg.mode() == RunMode::Shadow;
        let client = self.dash.isolated();
        let worker_client = client.clone();
        if let Some((_, old)) = self
            .t17
            .shelf
            .opening
            .borrow_mut()
            .replace((session.clone(), client.clone()))
        {
            old.cancel_pending();
        }
        let auxiliary = self.t18.clients.borrow().values().any(|c| {
            c.session == session && Arc::ptr_eq(&c.term.cleanup_cancellation(), &instance)
        });
        if !auxiliary {
            self.select(&session);
        }
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || -> Result<(String, String, String, Option<Value>), String> {
                if !work_ticket.current() || worker_instance.load(Ordering::Acquire) {
                    return Err("Extension target cancelled".into());
                }
                let allowed = || work_ticket.current() && !worker_instance.load(Ordering::Acquire);
                let preview = if shadow {
                    Some(extensions::read_inventory_when(
                        &tmux,
                        &session,
                        &pane,
                        &harness,
                        allowed,
                        |path| {
                            worker_client
                                .get(path, Duration::from_secs(8))
                                .map_err(|_| "Read-only inventory unavailable".to_string())
                        },
                    )?)
                } else {
                    ui::accounts::pane_identity_when(&tmux, &session, &pane, None, allowed)?;
                    None
                };
                Ok((session, pane, harness, preview))
            },
            move |result| {
                let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                    return;
                };
                if !ticket.current() || instance.load(Ordering::Acquire) {
                    return;
                }
                app.t17.shelf.opening.borrow_mut().take();
                let (session, pane, harness, preview) = match result {
                    Ok(v) => v,
                    Err(error) => {
                        app.status.set_text(&error);
                        return;
                    }
                };
                if !app.owns_term_instance(&session, &instance) {
                    return;
                }
                app.publish_extension(&uri, (session, pane, harness), instance, preview, client);
            },
        );
    }
    fn publish_extension(
        self: &Rc<Self>,
        uri: &str,
        target: (String, String, String),
        instance: Arc<AtomicBool>,
        preview: Option<Value>,
        client: DashClient,
    ) {
        let shelf = &self.t17.shelf;
        let was_open = shelf.extension.borrow().is_some();
        shelf.close_extension();
        let page = match ui::webview::create_page(&self.cfg, &self.webview, "extensions", uri) {
            Ok(page) => page,
            Err(error) => {
                self.status.set_text(&format!("Extension view: {error:?}"));
                return;
            }
        };
        page.view.set_no_show_all(true);
        page.view.set_size_request(-1, extensions::SHELF_MIN);
        if let Some(data) = &preview {
            page.view.load_html(
                &extensions::shadow_preview(data, &target.0, &target.1, self.english),
                Some(uri),
            );
        }
        let generation = shelf.next_generation();
        let mut signals = vec![];
        if let Some(manager) = page.view.user_content_manager() {
            let weak = Rc::downgrade(self);
            let view = page.view.downgrade();
            let owned_uri = uri.to_string();
            let id =
                manager.connect_script_message_received(Some("extensions"), move |_, result| {
                    let (Some(app), Some(view)) = (weak.upgrade(), view.upgrade()) else {
                        return;
                    };
                    let current = app.t17.shelf.extension.borrow();
                    let Some(owner) = current.as_ref() else {
                        return;
                    };
                    if app.closed.load(Ordering::Acquire)
                        || owner.page.view != view
                        || owner.target.as_ref().is_none_or(|(session, _, _)| {
                            owner
                                .instance
                                .as_ref()
                                .is_none_or(|instance| !app.owns_term_instance(session, instance))
                        })
                        || owner
                            .instance
                            .as_ref()
                            .is_some_and(|i| i.load(Ordering::Acquire))
                        || !extensions::message_owned(
                            view.uri().as_deref().unwrap_or_default(),
                            &owned_uri,
                            app.cfg.dash_url().unwrap_or_default(),
                            generation,
                            owner.generation,
                        )
                    {
                        return;
                    }
                    let Some(raw) = result.js_value().map(|value| value.to_str().to_string())
                    else {
                        return;
                    };
                    let close =
                        extensions::extension_message(&raw).is_ok_and(|value| !value.is_null());
                    drop(current);
                    if close {
                        app.t17.shelf.close_extension();
                    }
                });
            signals.push((manager.upcast(), id));
        }
        let close = if preview.is_some() {
            let button = gtk::Button::with_label(if self.english {
                "Close read-only shelf"
            } else {
                "Cerrar estante de sólo lectura"
            });
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    let current = app
                        .t17
                        .shelf
                        .extension
                        .borrow()
                        .as_ref()
                        .is_some_and(|view| view.generation == generation);
                    if current {
                        app.t17.shelf.close_extension();
                    }
                }
            });
            shelf.host.pack_start(&button, false, false, 0);
            button.show();
            Some(button)
        } else {
            None
        };
        shelf.host.pack_start(&page.view, true, true, 0);
        page.view.show();
        *shelf.extension.borrow_mut() = Some(ShelfView {
            page,
            scope: ui::snippets::Scope::default(),
            generation,
            uri: uri.into(),
            target: Some(target),
            instance: Some(instance),
            signals,
            close,
            client: Some(client),
        });
        shelf.sync();
        if !was_open {
            self.shelf_height_position();
            self.shelf_read_height();
        }
    }
    fn queue_shelf_save(self: &Rc<Self>) {
        if !self.writable() || self.t17.shelf.extension.borrow().is_none() {
            return;
        }
        if let Some(old) = self.t17.shelf.save.borrow_mut().take() {
            old.remove();
        }
        let weak = Rc::downgrade(self);
        *self.t17.shelf.save.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_millis(350),
            move || {
                let Some(app) = weak.upgrade().filter(|a| a.writable()) else {
                    return;
                };
                app.t17.shelf.save.borrow_mut().take();
                let shelf = &app.t17.shelf;
                let owner = shelf.extension.borrow();
                let Some(view) = owner.as_ref() else {
                    return;
                };
                let height = view.page.view.allocated_height();
                if height < extensions::SHELF_MIN {
                    return;
                }
                let ticket = view.scope.ticket();
                let worker_ticket = ticket.clone();
                let generation = view.generation;
                let instance = view.instance.clone();
                drop(owner);
                let expected = shelf.saved.borrow().clone();
                let mut next = if expected.is_object() {
                    expected.clone()
                } else {
                    json!({})
                };
                if let Some(fields) = next.as_object_mut() {
                    fields.insert("height".into(), json!(height));
                }
                let state = app.state.clone();
                let closed = app.closed.clone();
                let weak = Rc::downgrade(&app);
                let done_next = next.clone();
                app.jobs.spawn(
                    move || {
                        state.write_shelf_when(&expected, &next, || {
                            worker_ticket.current()
                                && !closed.load(Ordering::Acquire)
                                && instance.as_ref().is_none_or(|i| !i.load(Ordering::Acquire))
                        })
                    },
                    move |result| {
                        let Some(app) = weak.upgrade().filter(|a| a.writable()) else {
                            return;
                        };
                        if !ticket.current()
                            || app
                                .t17
                                .shelf
                                .extension
                                .borrow()
                                .as_ref()
                                .is_none_or(|v| v.generation != generation)
                        {
                            return;
                        }
                        match result {
                            Ok(true) => *app.t17.shelf.saved.borrow_mut() = done_next,
                            Ok(false) => {
                                app.status.set_text("Shelf changed; stale height discarded")
                            }
                            Err(error) => app.status.set_text(&format!("Shelf height: {error:?}")),
                        }
                    },
                );
            },
        ));
    }
    pub(super) fn start_ai_in_pane(self: &Rc<Self>, session: &str, pane: &str) {
        if !self.writable() {
            return;
        }
        let session = if session.is_empty() {
            self.action_session().unwrap_or_default()
        } else {
            session.into()
        };
        let Some(instance) = self
            .term_for_action(&session)
            .as_ref()
            .map(TermView::cleanup_cancellation)
        else {
            self.status.set_text("No tmux session");
            return;
        };
        self.t17.wizard.advance();
        let ticket = self.t17.wizard.ticket();
        let worker_ticket = ticket.clone();
        let tmux = self.tmux.clone();
        let closed = self.closed.clone();
        let worker_instance = instance.clone();
        let pane = pane.to_string();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || -> Result<(String, String, String), String> {
                ui::extensions::wizard_target_when(&tmux, &session, &pane, || {
                    worker_ticket.current()
                        && !closed.load(Ordering::Acquire)
                        && !worker_instance.load(Ordering::Acquire)
                })
            },
            move |result| {
                let Some(app) = weak.upgrade().filter(|a| a.writable()) else {
                    return;
                };
                if !ticket.current() || instance.load(Ordering::Acquire) {
                    return;
                }
                match result {
                    Ok((session, pane, cwd)) => {
                        if app.action_session().as_deref() != Some(session.as_str())
                            || app
                                .term_for_action(&session)
                                .as_ref()
                                .is_none_or(|t| !Arc::ptr_eq(&instance, &t.cleanup_cancellation()))
                        {
                            return;
                        }
                        if let Err(error) = app.call_dashboard(
                            ui::bridge::JsFunction::NewSessionForPane,
                            &[json!(session), json!(pane), json!(cwd)],
                        ) {
                            app.status.set_text(&error.to_string());
                        }
                    }
                    Err(error) => app.status.set_text(&error),
                }
            },
        );
    }
}
