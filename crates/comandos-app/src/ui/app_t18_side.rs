use super::*;
use ui::app_commands::{self as commands, CommandError};
use webkit2gtk::WebViewExt;
impl App {
    pub(super) fn install_auxiliary_views(self: &Rc<Self>) {
        for name in [
            "T18.reader",
            "T18.left_panel",
            "T18.web_modal",
            "T18.web_tab",
            "T18.mosaic",
            "T18.mosaic_zoom",
        ] {
            let weak = Rc::downgrade(self);
            self.install_handler(
                name,
                Rc::new(move |args| {
                    let app = weak
                        .upgrade()
                        .filter(|a| !a.closed.load(Ordering::Acquire))
                        .ok_or_else(|| CommandError::Refused("application closed".into()))?;
                    match name {
                        "T18.reader" => app.reader_action(args),
                        "T18.left_panel" => {
                            let action = args.get("action").and_then(Value::as_str);
                            let hidden = match action {
                                Some("toggle") => !app.t18.state.borrow().left_hidden,
                                Some("hide") => true,
                                Some("show") => false,
                                None => args.get("hidden").is_some_and(comandos_core::json::truthy),
                                _ => {
                                    return Err(CommandError::Invalid(
                                        "invalid left panel action".into(),
                                    ));
                                }
                            };
                            app.left_panel_set(hidden);
                            Ok(())
                        }
                        "T18.web_tab" => app.open_web_tab(
                            commands::string_arg(args, "url", "")?,
                            args.get("label")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ),
                        "T18.web_modal" => app.open_web_modal(
                            commands::string_arg(args, "url", "")?,
                            args.get("label")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ),
                        "T18.mosaic" => {
                            let state = commands::string_arg(args, "state", "toggle")?;
                            match state {
                                "on" => app.mosaic_open(),
                                "off" => app.mosaic_close(),
                                "toggle" => {
                                    let on =
                                        app.t18.mosaic.borrow().on || app.t18.mosaic_loading.get();
                                    if on {
                                        app.mosaic_close()
                                    } else {
                                        app.mosaic_open()
                                    }
                                }
                                _ => {
                                    return Err(CommandError::Invalid(
                                        "invalid mosaic state".into(),
                                    ));
                                }
                            };
                            Ok(())
                        }
                        _ => {
                            let session = commands::session_arg(args, "")?;
                            app.mosaic_zoom(session);
                            Ok(())
                        }
                    }
                }),
            );
        }
        let weak = Rc::downgrade(self);
        self.paned.connect_size_allocate(move |_, _| {
            if let Some(app) = weak.upgrade()
                && app.t18.state.borrow().left_hidden
            {
                app.t18.left.hide();
            }
        });
        self.install_aux_signals();
        self.load_aux_layout();
    }
    fn load_aux_layout(self: &Rc<Self>) {
        let state = self.state.clone();
        let ticket = self.t18.scope.ticket();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || state.read_ui_document("app-layout.json"),
            move |result| {
                if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire))
                    && ticket.current()
                {
                    match result {
                        Ok(layout) => match ui::side::SideState::from_json(&layout) {
                            Ok(state) => {
                                *app.t18.layout_doc.borrow_mut() = layout;
                                *app.t18.state.borrow_mut() = state;
                                let hidden = app.t18.state.borrow().left_hidden;
                                app.t18.left.set_visible(!hidden);
                                app.paint_left_button(hidden);
                            }
                            Err(error) => app.status.set_text(&error.to_string()),
                        },
                        Err(error) => app.status.set_text(&format!("Auxiliary layout: {error:?}")),
                    }
                }
            },
        );
    }
    pub(super) fn queue_ui_save(self: &Rc<Self>, name: &'static str, delay: u64) {
        if !self.writable() {
            return;
        }
        let (slot, busy) = if name == "app-layout.json" {
            (&self.t18.layout_save, &self.t18.layout_busy)
        } else {
            (&self.t18.pane_save, &self.t18.pane_busy)
        };
        if let Some(old) = slot.borrow_mut().take() {
            old.remove();
        }
        if busy.get() {
            return;
        }
        let weak = Rc::downgrade(self);
        *slot.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_millis(delay),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.flush_ui_save(name);
                }
            },
        ));
    }
    fn flush_ui_save(self: &Rc<Self>, name: &'static str) {
        let layout = name == "app-layout.json";
        let (slot, busy, scope) = if layout {
            (
                &self.t18.layout_save,
                &self.t18.layout_busy,
                &self.t18.layout_scope,
            )
        } else {
            (
                &self.t18.pane_save,
                &self.t18.pane_busy,
                &self.t18.pane_scope,
            )
        };
        slot.borrow_mut().take();
        if !self.writable() || busy.replace(true) {
            return;
        }
        let old = if layout {
            self.t18.layout_doc.borrow().clone()
        } else {
            self.t18.pane_doc.borrow().clone()
        };
        let next = if layout {
            self.t18.state.borrow().to_json()
        } else {
            self.pane_document(&old)
        };
        let worker_next = next.clone();
        let state = self.state.clone();
        let ticket = scope.ticket();
        let worker = ticket.clone();
        let closed = self.closed.clone();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || {
                state.write_ui_document_when(name, &old, &worker_next, || {
                    worker.current() && !closed.load(Ordering::Acquire)
                })
            },
            move |result| {
                if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire))
                    && ticket.current()
                {
                    let busy = if layout {
                        &app.t18.layout_busy
                    } else {
                        &app.t18.pane_busy
                    };
                    busy.set(false);
                    match result {
                        Ok(true) => {
                            if layout {
                                *app.t18.layout_doc.borrow_mut() = next.clone();
                                if app.t18.state.borrow().to_json() != next {
                                    app.queue_ui_save(name, 600);
                                }
                            } else {
                                *app.t18.pane_doc.borrow_mut() = next.clone();
                                if app.pane_document(&next) != next {
                                    app.queue_ui_save(name, 250);
                                }
                            }
                        }
                        Ok(false) => app
                            .status
                            .set_text("Layout changed concurrently; save refused"),
                        Err(error) => app.status.set_text(&format!("Layout save: {error:?}")),
                    }
                }
            },
        );
    }
}

impl App {
    fn install_aux_signals(self: &Rc<Self>) {
        let relay = match ui::signals::SignalRelay::new() {
            Ok(relay) => relay,
            Err(error) => {
                self.status.set_text(&format!("Desktop signals: {error}"));
                return;
            }
        };
        let fd = relay.fd();
        *self.t18.relay.borrow_mut() = Some(relay);
        let weak = Rc::downgrade(self);
        let ticket = self.t18.scope.ticket();
        self.t18.sources.borrow_mut().push(glib::source::unix_fd_add_local(fd,glib::IOCondition::IN,move|_,_|{
            let Some(app)=weak.upgrade().filter(|a|!a.closed.load(Ordering::Acquire))else{return glib::ControlFlow::Break;};
            if !ticket.current(){return glib::ControlFlow::Break;}let actions=app.t18.relay.borrow_mut().as_mut().map(|r|r.drain()).unwrap_or_default();
            for action in actions{match action {
                ui::signals::Action::Mosaic=>{let active=app.t18.mosaic.borrow().on||app.t18.mosaic_loading.get();if active{app.mosaic_close();}else{app.mosaic_open();}},
                ui::signals::Action::JavascriptDebug=>app.debug_js_file(),
            }}glib::ControlFlow::Continue
        }));
    }
    fn debug_js_file(self: &Rc<Self>) {
        if !self.writable() || self.t18.debug_loading.replace(true) {
            return;
        }
        let path = self.cfg.runtime_dir().join("comandos-dbg.js");
        let ticket = self.t18.scope.ticket();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || ui::signals::debug_script(&path, nix::unistd::geteuid().as_raw()),
            move |result| {
                if let Some(app) = weak.upgrade()
                    && {
                        app.t18.debug_loading.set(false);
                        app.writable()
                    }
                    && ticket.current()
                {
                    match result {
                        Ok(code) => {
                            println!(
                                "NFGTK paned_pos={} wv_alloc={}x{} zoom={}",
                                app.paned.position(),
                                app.webview.allocated_width(),
                                app.webview.allocated_height(),
                                app.webview.zoom_level()
                            );
                            app.quiet_js(&code);
                        }
                        Err(error) => app.status.set_text(&format!("Debug script: {error}")),
                    }
                }
            },
        );
    }
}
