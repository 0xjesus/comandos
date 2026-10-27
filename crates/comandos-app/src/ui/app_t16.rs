//! T16 UI ports: workers carry terminal-instance cancellation, widgets remain on MainContext.
use super::*;
use std::sync::Mutex;
use ui::{
    clipboard::{Clipboard, Target},
    menu::{Context, MenuAction, MenuRow, OpenIntent},
    snippets,
};
type PendingMenu = (String, gdk::EventButton, (u16, u16), Arc<AtomicBool>);
pub(super) struct Owned {
    pub dialog: RefCell<Option<Rc<snippets::Dialog>>>,
    loading: Cell<bool>,
    dialog_epoch: Arc<AtomicU64>,
    menu_epoch: Arc<AtomicU64>,
    menu: RefCell<Option<gtk::Menu>>,
    menu_loading: Cell<bool>,
    menu_pending: RefCell<Option<PendingMenu>>,
    link: RefCell<Option<gtk::Popover>>,
    clipboard: Rc<Clipboard>,
    bridge: Arc<Mutex<crate::clipboard_bridge::Bridge>>,
    notify: Option<DashClient>,
    polling: Cell<bool>,
}
impl Owned {
    pub fn new(mode: RunMode, closed: Arc<AtomicBool>) -> Self {
        Self {
            dialog: RefCell::new(None),
            loading: Cell::new(false),
            dialog_epoch: Arc::new(AtomicU64::new(0)),
            menu_epoch: Arc::new(AtomicU64::new(0)),
            menu: RefCell::new(None),
            menu_loading: Cell::new(false),
            menu_pending: RefCell::new(None),
            link: RefCell::new(None),
            clipboard: Clipboard::new(mode, Rc::new(ui::clipboard::GtkClipboard), closed),
            bridge: Arc::new(Mutex::new(crate::clipboard_bridge::Bridge::default())),
            notify: if mode == RunMode::Live {
                DashClient::new(Some("http://127.0.0.1:4778"), mode).ok()
            } else {
                None
            },
            polling: Cell::new(false),
        }
    }
}
#[derive(Default)]
struct ClickState {
    scope: snippets::Scope,
    pending: bool,
    released: bool,
    context: Option<Context>,
    next: Option<(u16, u16)>,
}
impl App {
    pub(super) fn install_t16(self: &Rc<Self>) {
        for name in [
            "T16.snippets",
            "T16.copy_selection",
            "T16.paste_clipboard",
            "T16.copy_reply",
        ] {
            let weak = Rc::downgrade(self);
            self.install_handler(
                name,
                Rc::new(move |_| {
                    let app = weak.upgrade().filter(|a| a.writable()).ok_or_else(|| {
                        ui::app_commands::CommandError::Refused(
                            "restore not ready, shadow, or closed".into(),
                        )
                    })?;
                    if name == "T16.snippets" {
                        app.open_snippets();
                        return Ok(());
                    }
                    let key = app.action_session().ok_or_else(|| {
                        ui::app_commands::CommandError::Invalid("no active terminal".into())
                    })?;
                    match name {
                        "T16.copy_selection" => app.copy_terminal(&key, None),
                        "T16.paste_clipboard" => app.paste_terminal(&key),
                        _ => app.reply_terminal(&key, None),
                    }
                    Ok(())
                }),
            );
        }
    }
    fn instance(&self, key: &str, instance: &Arc<AtomicBool>) -> bool {
        self.writable()
            && !instance.load(Ordering::Acquire)
            && self
                .term_for_action(key)
                .as_ref()
                .is_some_and(|t| Arc::ptr_eq(&t.cleanup_cancellation(), instance))
    }
    pub(super) fn attach_term_t16(self: &Rc<Self>, key: &str, term: &TermView) {
        let weak = Rc::downgrade(self);
        let key_ = key.to_string();
        let instance = term.cleanup_cancellation();
        term.clipboard().set_valid(Rc::new(move || {
            weak.upgrade()
                .is_some_and(|a| a.writable() && a.owns_term_instance(&key_, &instance))
        }));
        let weak = Rc::downgrade(self);
        let key_ = key.to_string();
        let instance = term.cleanup_cancellation();
        term.on_context_menu(Rc::new(move |event, point| {
            if let Some(app) = weak.upgrade().filter(|a| a.instance(&key_, &instance)) {
                app.terminal_menu(&key_, event, point);
            }
        }));
        let click = Rc::new(RefCell::new(ClickState::default()));
        let weak = Rc::downgrade(self);
        let key_ = key.to_string();
        let instance = term.cleanup_cancellation();
        let state = click.clone();
        term.on_primary_press(Rc::new(move |event, point| {
            let Some(app) = weak.upgrade().filter(|a| a.instance(&key_, &instance)) else {
                return;
            };
            {
                let mut state = state.borrow_mut();
                state.scope.advance();
                state.released = false;
                state.context = None;
                state.next = None;
                if event.button() != 1 || event.state().contains(gdk::ModifierType::CONTROL_MASK) {
                    return;
                }
                if state.pending {
                    state.next = Some(point);
                    return;
                }
            }
            app.capture_primary_click(&key_, state.clone(), point);
        }));
        let weak = Rc::downgrade(self);
        let key_ = key.to_string();
        let instance = term.cleanup_cancellation();
        term.on_clean_click(Rc::new(move |_, _| {
            if let Some(app) = weak.upgrade().filter(|a| a.instance(&key_, &instance)) {
                let (context, ticket) = {
                    let mut state = click.borrow_mut();
                    state.released = true;
                    (state.context.take(), state.scope.ticket())
                };
                if let Some(context) = context {
                    app.select_click(&key_, context, ticket);
                }
            }
        }));
        let weak = Rc::downgrade(self);
        let key_ = key.to_string();
        let instance = term.cleanup_cancellation();
        term.on_link_event(Rc::new(move |url, event, point, control| {
            if let Some(app) = weak.upgrade().filter(|a| a.instance(&key_, &instance)) {
                app.link_terminal(&key_, url, event, point, control);
            }
        }));
    }
    fn capture_primary_click(
        self: &Rc<Self>,
        key: &str,
        state: Rc<RefCell<ClickState>>,
        point: (u16, u16),
    ) {
        let Some(instance) = self
            .term_for_action(key)
            .as_ref()
            .map(TermView::cleanup_cancellation)
        else {
            return;
        };
        let ticket = {
            let mut state = state.borrow_mut();
            state.pending = true;
            state.scope.ticket()
        };
        let tmux = self.tmux.clone();
        let tty = self
            .term_for_action(key)
            .as_ref()
            .and_then(TermView::child_tty);
        let closed = self.closed.clone();
        let cancel = instance.clone();
        let current = ticket.clone();
        let weak = Rc::downgrade(self);
        let key = key.to_string();
        let instance = instance.clone();
        let state = state.clone();
        self.jobs.spawn(
            move || {
                ui::menu::capture_click(&tmux, tty.as_deref(), point, || {
                    !ticket.current()
                        || closed.load(Ordering::Acquire)
                        || cancel.load(Ordering::Acquire)
                })
            },
            move |result| {
                state.borrow_mut().pending = false;
                let next = state.borrow_mut().next.take();
                if let Some(point) = next {
                    if let Some(app) = weak.upgrade().filter(|a| a.instance(&key, &instance)) {
                        app.capture_primary_click(&key, state.clone(), point);
                    }
                    return;
                }
                let Some(app) = weak
                    .upgrade()
                    .filter(|a| a.instance(&key, &instance) && current.current())
                else {
                    return;
                };
                match result {
                    Ok(context) => {
                        let released = state.borrow().released;
                        if released {
                            app.select_click(&key, context, current);
                        } else {
                            state.borrow_mut().context = Some(context);
                        }
                    }
                    Err(e) => app.status.set_text(&e),
                }
            },
        );
    }
    fn select_click(self: &Rc<Self>, key: &str, context: Context, ticket: snippets::Ticket) {
        let Some((tty, instance)) = self
            .term_for_action(key)
            .as_ref()
            .map(|t| (t.child_tty(), t.cleanup_cancellation()))
        else {
            return;
        };
        let tmux = self.tmux.clone();
        let closed = self.closed.clone();
        let cancel = instance.clone();
        let weak = Rc::downgrade(self);
        let key = key.to_string();
        self.jobs.spawn(
            move || {
                ui::menu::select_captured_click(&tmux, &context, tty.as_deref(), || {
                    !ticket.current()
                        || closed.load(Ordering::Acquire)
                        || cancel.load(Ordering::Acquire)
                })
            },
            move |result| {
                if let Some(app) = weak.upgrade().filter(|a| a.instance(&key, &instance))
                    && let Err(e) = result
                {
                    app.status.set_text(&e);
                }
            },
        );
    }
    pub(super) fn install_clipboard_watch(self: &Rc<Self>) -> glib::SourceId {
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(
            Duration::from_millis(crate::clipboard_bridge::POLL_MS),
            move || {
                let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                    return glib::ControlFlow::Break;
                };
                if !app.writable() || !app.presence_visible.get() || app.t16.polling.replace(true) {
                    return glib::ControlFlow::Continue;
                }
                let tmux = app.tmux.clone();
                let bridge = app.t16.bridge.clone();
                let closed = app.closed.clone();
                let weak = Rc::downgrade(&app);
                app.jobs.spawn(
                    move || {
                        if closed.load(Ordering::Acquire) {
                            return None;
                        }
                        bridge
                            .lock()
                            .ok()
                            .and_then(|mut b| b.read_next(&tmux).ok().flatten())
                    },
                    move |text| {
                        if let Some(app) = weak.upgrade() {
                            app.t16.polling.set(false);
                            if app.writable()
                                && let Some(text) = text
                            {
                                app.t16.clipboard.copy(Target::Clipboard, &text);
                            }
                        }
                    },
                );
                glib::ControlFlow::Continue
            },
        )
    }
    pub(super) fn shutdown_snippets(&self) {
        self.t16.dialog_epoch.fetch_add(1, Ordering::AcqRel);
        self.t16.loading.set(false);
        if let Some(dialog) = self.t16.dialog.borrow_mut().take() {
            dialog.shutdown();
        }
    }
    pub(super) fn shutdown_t16(&self) {
        self.shutdown_snippets();
        self.t16.menu_epoch.fetch_add(1, Ordering::AcqRel);
        self.t16.menu_pending.borrow_mut().take();
        self.t16.menu_loading.set(false);
        self.t16.clipboard.invalidate();
        if let Some(notify) = &self.t16.notify {
            notify.cancel_pending();
        }
        if let Some(menu) = self.t16.menu.borrow_mut().take() {
            menu.popdown();
            for child in menu.children() {
                menu.remove(&child);
            }
        }
        if let Some(popover) = self.t16.link.borrow_mut().take() {
            popover.popdown();
            if let Some(child) = popover.child() {
                popover.remove(&child);
            }
        }
    }
    fn snippet_log(self: &Rc<Self>, event: snippets::LogEvent, count: usize) {
        if self.cfg.mode() == RunMode::Shadow {
            return;
        }
        let path = self
            .cfg
            .sandbox_temp()
            .map(|p| p.join("cc-app-snip.log"))
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/cc-app-snip.log"));
        let guard = self.guard.clone();
        let closed = self.closed.clone();
        self.jobs.spawn(
            move || {
                if !closed.load(Ordering::Acquire) {
                    let _ = snippets::log_operation(&guard, &path, event, count);
                }
            },
            |_| {},
        );
    }
    fn open_snippets(self: &Rc<Self>) {
        self.snippet_log(snippets::LogEvent::Open, 0);
        if self.t16.dialog.borrow().is_some() {
            self.close_modal();
            return;
        }
        if self.t16.loading.replace(true) {
            return;
        }
        let epoch = self.t16.dialog_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let state = self.state.clone();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || state.read_snippets(),
            move |result| {
                let Some(app) = weak.upgrade().filter(|a| {
                    a.writable() && a.t16.dialog_epoch.load(Ordering::Acquire) == epoch
                }) else {
                    return;
                };
                app.t16.loading.set(false);
                let items = match result {
                    Ok(items) => items,
                    Err(e) => {
                        app.status.set_text(&format!("Snippets: {e:?}"));
                        return;
                    }
                };
                let weak = Rc::downgrade(&app);
                let save: snippets::Save = Rc::new(move |expected, next, ticket, done| {
                    let Some(app) = weak.upgrade().filter(|a| a.writable() && ticket.current())
                    else {
                        done(Err("Dialogo cerrado".into()));
                        return;
                    };
                    app.snippet_log(snippets::LogEvent::Save, next.len());
                    let state = app.state.clone();
                    let closed = app.closed.clone();
                    app.jobs.spawn(
                        move || {
                            state
                                .write_snippets_when(&expected, &next, || {
                                    ticket.current() && !closed.load(Ordering::Acquire)
                                })
                                .map_err(|e| format!("{e:?}"))
                                .and_then(|ok| {
                                    if ok {
                                        Ok(())
                                    } else {
                                        Err("Snippets cambiaron; vuelve a abrir el editor".into())
                                    }
                                })
                        },
                        done,
                    );
                });
                let weak = Rc::downgrade(&app);
                let send: snippets::SendSnippet = Rc::new(move |body, ticket, done| {
                    let Some(app) = weak.upgrade().filter(|a| a.writable() && ticket.current())
                    else {
                        done(Err("Dialogo cerrado".into()));
                        return;
                    };
                    app.snippet_log(snippets::LogEvent::Send, body.chars().count());
                    let Some(key) = app.action_session() else {
                        done(Err("No hay terminal activa".into()));
                        return;
                    };
                    let Some((tty, instance)) = app
                        .term_for_action(&key)
                        .as_ref()
                        .map(|t| (t.child_tty(), t.cleanup_cancellation()))
                    else {
                        done(Err("No hay terminal activa".into()));
                        return;
                    };
                    let tmux = app.tmux.clone();
                    let closed = app.closed.clone();
                    app.jobs.spawn(
                        move || {
                            let session =
                                crate::clipboard_bridge::term_session(&tmux, tty.as_deref())
                                    .map_err(|e| e.to_string())?
                                    .ok_or("No hay sesion tmux activa")?;
                            snippets::paste_snippet_when(&session, &body, &tmux, || {
                                ticket.current()
                                    && !closed.load(Ordering::Acquire)
                                    && !instance.load(Ordering::Acquire)
                            })
                            .map_err(|e| e.to_string())?;
                            Ok(session)
                        },
                        done,
                    );
                });
                let weak = Rc::downgrade(&app);
                let dialog = snippets::Dialog::new(
                    items,
                    app.english,
                    save,
                    send,
                    Rc::new(move || {
                        if let Some(app) = weak.upgrade() {
                            app.close_modal();
                        }
                    }),
                );
                app.mount_modal(&dialog.frame, ui::switcher::Kind::Help, None);
                *app.t16.dialog.borrow_mut() = Some(dialog.clone());
                dialog.search.grab_focus();
            },
        );
    }
    pub(super) fn defer_copy_key(
        self: &Rc<Self>,
        event: &gdk::EventKey,
        key: Option<&str>,
    ) -> bool {
        use gdk::keys::constants as k;
        let Some(key) = key else {
            return false;
        };
        if [
            k::Control_L,
            k::Control_R,
            k::Shift_L,
            k::Shift_R,
            k::Alt_L,
            k::Alt_R,
            k::Super_L,
            k::Super_R,
        ]
        .iter()
        .any(|k| **k == *event.keyval())
        {
            return false;
        }
        let Some(term) = self.term_for_action(key).as_ref().cloned() else {
            return false;
        };
        let pending = term.key_pending();
        if !pending && !term.clipboard().take_selection_release() {
            return false;
        }
        if let Err(e) = term.queue_key(event, glib::monotonic_time() as f64 / 1_000_000.) {
            self.status.set_text(&e.to_string());
            return true;
        }
        if pending {
            return true;
        }
        let instance = term.cleanup_cancellation();
        let cancel = instance.clone();
        let closed = self.closed.clone();
        let tmux = self.tmux.clone();
        let tty = term.child_tty();
        let weak = Rc::downgrade(self);
        let key = key.to_string();
        self.jobs.spawn(
            move || {
                crate::clipboard_bridge::exit_copy_mode(&tmux, tty.as_deref(), || {
                    closed.load(Ordering::Acquire) || cancel.load(Ordering::Acquire)
                })
            },
            move |result| {
                let Some(app) = weak.upgrade().filter(|a| a.instance(&key, &instance)) else {
                    return;
                };
                let term = app.term_for_action(&key).as_ref().cloned();
                if let Some(term) = term {
                    match result {
                        Ok(context) => {
                            for ((event, now), context) in term.finish_keys(context) {
                                if !app.instance(&key, &instance) {
                                    break;
                                }
                                let mut input = ui::keys::KeyInput::new(*event.keyval());
                                input.control =
                                    event.state().contains(gdk::ModifierType::CONTROL_MASK);
                                input.shift = event.state().contains(gdk::ModifierType::SHIFT_MASK);
                                input.terminal = true;
                                input.has_term = true;
                                input.selection = term.selection_text().is_some();
                                if !app.execute_key_context(input, Some(&key), now, context) {
                                    term.forward_key(&event);
                                }
                            }
                        }
                        Err(e) => {
                            term.finish_keys(false);
                            app.status.set_text(&e.to_string());
                        }
                    }
                }
            },
        );
        true
    }
    fn paste_terminal(self: &Rc<Self>, key: &str) {
        let Some((tty, instance)) = self
            .term_for_action(key)
            .as_ref()
            .map(|t| (t.child_tty(), t.cleanup_cancellation()))
        else {
            return;
        };
        let tmux = self.tmux.clone();
        let closed = self.closed.clone();
        let cancel = instance.clone();
        let weak = Rc::downgrade(self);
        let key = key.to_string();
        self.jobs.spawn(
            move || {
                crate::clipboard_bridge::exit_copy_mode(&tmux, tty.as_deref(), || {
                    closed.load(Ordering::Acquire) || cancel.load(Ordering::Acquire)
                })
            },
            move |result| {
                if let Some(app) = weak.upgrade().filter(|a| a.instance(&key, &instance)) {
                    match result {
                        Ok(_) => {
                            if let Some(term) = app.term_for_action(&key).as_ref() {
                                term.paste_clipboard(false);
                            }
                        }
                        Err(e) => app.status.set_text(&e.to_string()),
                    }
                }
            },
        );
    }
    fn copy_terminal(self: &Rc<Self>, key: &str, preferred: Option<String>) {
        let Some((tty, instance, clipboard, text)) = self.term_for_action(key).as_ref().map(|t| {
            (
                t.child_tty(),
                t.cleanup_cancellation(),
                t.clipboard(),
                t.selection_text(),
            )
        }) else {
            return;
        };
        if let Some(text) = text {
            clipboard.copy_selection(&text, glib::monotonic_time() as f64 / 1_000_000.);
            if let Some(term) = self.term_for_action(key).as_ref() {
                term.clear_selection();
            }
            self.feedback("Seleccion copiada", "Texto al portapapeles");
            return;
        }
        let tmux = self.tmux.clone();
        let closed = self.closed.clone();
        let cancel = instance.clone();
        let weak = Rc::downgrade(self);
        let key = key.to_string();
        self.jobs.spawn(
            move || {
                let session = crate::clipboard_bridge::term_session(&tmux, tty.as_deref())?;
                let pane = crate::clipboard_bridge::copy_mode_pane(
                    &tmux,
                    session.as_deref(),
                    preferred.as_deref(),
                )?;
                match pane {
                    Some(pane) => {
                        crate::clipboard_bridge::copy_tmux_selection(&tmux, &pane, || {
                            closed.load(Ordering::Acquire) || cancel.load(Ordering::Acquire)
                        })
                        .map(Some)
                    }
                    None => Ok(None),
                }
            },
            move |result| {
                if let Some(app) = weak.upgrade().filter(|a| a.instance(&key, &instance)) {
                    match result {
                        Ok(Some(text)) => {
                            if clipboard
                                .copy_selection(&text, glib::monotonic_time() as f64 / 1_000_000.)
                            {
                                app.feedback("Seleccion copiada", "Texto al portapapeles");
                            }
                        }
                        Ok(None) => app.feedback(
                            if clipboard
                                .recent_selection(glib::monotonic_time() as f64 / 1_000_000.)
                            {
                                "Ya copiado"
                            } else {
                                "Sin seleccion"
                            },
                            "Selecciona texto en la terminal",
                        ),
                        Err(e) => app.status.set_text(&e.to_string()),
                    }
                }
            },
        );
    }
    fn feedback(self: &Rc<Self>, title: &str, body: &str) {
        self.status.set_text(&format!("{title}: {body}"));
        // The original app owns notifyd's /notify port in Live; private modes only show their owned status.
        let Some(notify) = self.t16.notify.clone() else {
            return;
        };
        let closed = self.closed.clone();
        let payload = ui::menu::notification_payload(title, body);
        self.jobs.spawn(
            move || {
                if !closed.load(Ordering::Acquire) {
                    let _ = notify.post("/notify", &payload, Duration::from_secs(3));
                }
            },
            |_| {},
        );
    }
    fn reply_terminal(self: &Rc<Self>, key: &str, format: Option<&'static str>) {
        let Some((tty, instance, clipboard)) = self
            .term_for_action(key)
            .as_ref()
            .map(|t| (t.child_tty(), t.cleanup_cancellation(), t.clipboard()))
        else {
            return;
        };
        let tmux = self.tmux.clone();
        let dash = self.dash.clone();
        let closed = self.closed.clone();
        let cancel = instance.clone();
        let weak = Rc::downgrade(self);
        let key = key.to_string();
        self.jobs.spawn(
            move || {
                if closed.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
                    return Err("Operacion cancelada".into());
                }
                let session = crate::clipboard_bridge::term_session(&tmux, tty.as_deref())
                    .map_err(|e| e.to_string())?
                    .ok_or("No hay sesion tmux")?;
                if closed.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
                    return Err("Operacion cancelada".into());
                }
                if let Some(format) = format {
                    let (status, res) = dash
                        .post(
                            "/export",
                            &json!({"session":session,"format":format}),
                            Duration::from_secs(45),
                        )
                        .map_err(|e| format!("{e:?}"))?;
                    if !(200..300).contains(&status) {
                        return Err(res.to_string());
                    }
                    Ok((
                        false,
                        res.get("path").and_then(Value::as_str).unwrap_or("").into(),
                        String::new(),
                    ))
                } else {
                    let items = dash
                        .get("/state", Duration::from_secs(3))
                        .map_err(|e| format!("{e:?}"))?;
                    let (text, project) = ui::menu::reply_from_state(&items, &session)
                        .ok_or("Esta sesion no tiene respuesta capturada")?;
                    Ok((true, text, project))
                }
            },
            move |result: Result<(bool, String, String), String>| {
                if let Some(app) = weak.upgrade().filter(|a| {
                    a.instance(&key, &instance) && a.action_session().as_deref() == Some(&key)
                }) {
                    match result {
                        Ok((true, text, project)) => {
                            if clipboard.copy(Target::Clipboard, &text) {
                                app.feedback(
                                    "Respuesta copiada",
                                    &format!("{} caracteres ({project})", text.chars().count()),
                                );
                            }
                        }
                        Ok((false, path, _)) => app.feedback("Respuesta guardada", &path),
                        Err(e) => app.feedback("No se pudo completar", &e),
                    }
                }
            },
        );
    }
    pub(super) fn terminal_menu(
        self: &Rc<Self>,
        key: &str,
        event: &gdk::EventButton,
        point: (u16, u16),
    ) {
        let Some((tty, instance)) = self
            .term_for_action(key)
            .as_ref()
            .map(|t| (t.child_tty(), t.cleanup_cancellation()))
        else {
            return;
        };
        if !self.instance(key, &instance) {
            return;
        }
        let epoch = self.t16.menu_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        if self.t16.menu_loading.replace(true) {
            *self.t16.menu_pending.borrow_mut() =
                Some((key.into(), event.clone(), point, instance));
            return;
        }
        let token = self.t16.menu_epoch.clone();
        let tmux = self.tmux.clone();
        let closed = self.closed.clone();
        let cancel = instance.clone();
        let weak = Rc::downgrade(self);
        let key = key.to_string();
        let event = event.clone();
        self.jobs.spawn(
            move || {
                ui::menu::read_context(&tmux, tty.as_deref(), point, || {
                    closed.load(Ordering::Acquire)
                        || cancel.load(Ordering::Acquire)
                        || token.load(Ordering::Acquire) != epoch
                })
            },
            move |result| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                app.t16.menu_loading.set(false);
                let pending = app.t16.menu_pending.borrow_mut().take();
                if let Some((key, event, point, instance)) = pending {
                    if app.instance(&key, &instance) {
                        app.terminal_menu(&key, &event, point);
                    }
                    return;
                }
                if !app.instance(&key, &instance)
                    || app.t16.menu_epoch.load(Ordering::Acquire) != epoch
                {
                    return;
                }
                let ctx = match result {
                    Ok(ctx) => Rc::new(ctx),
                    Err(e) => {
                        app.status.set_text(&e);
                        return;
                    }
                };
                if let Some(menu) = app.t16.menu.borrow_mut().take() {
                    menu.popdown();
                    for child in menu.children() {
                        menu.remove(&child);
                    }
                }
                let menu = gtk::Menu::new();
                for row in ui::menu::terminal_rows(
                    app.english,
                    ctx.session.as_deref(),
                    ctx.pane.as_deref(),
                    ctx.npanes,
                    true,
                ) {
                    match row {
                        MenuRow::Separator => menu.append(&gtk::SeparatorMenuItem::new()),
                        MenuRow::Marks => {
                            if let Some(session) = &ctx.session {
                                let weak = Rc::downgrade(&app);
                                let key = key.clone();
                                let instance = instance.clone();
                                app.append_mark_menu_when(
                                    &menu,
                                    session,
                                    ctx.pane.as_deref(),
                                    Rc::new(move || {
                                        weak.upgrade().is_some_and(|a| {
                                            a.instance(&key, &instance)
                                                && a.action_session().as_deref() == Some(&key)
                                                && a.t16.menu_epoch.load(Ordering::Acquire) == epoch
                                        })
                                    }),
                                );
                            }
                        }
                        MenuRow::Action {
                            action,
                            label,
                            enabled,
                        } => {
                            let item = gtk::MenuItem::with_label(&label);
                            item.set_sensitive(enabled);
                            if ui::menu::is_destructive(action) {
                                item.style_context().add_class("cc-danger");
                            }
                            if action == MenuAction::ExistingSessions {
                                let submenu = gtk::Menu::new();
                                for session in &ctx.sessions {
                                    let child = gtk::MenuItem::with_label(session);
                                    let weak = Rc::downgrade(&app);
                                    let key = key.clone();
                                    let instance = instance.clone();
                                    let session = session.clone();
                                    child.connect_activate(move |_| {
                                        if let Some(app) = weak.upgrade().filter(|a| {
                                            a.instance(&key, &instance)
                                                && a.t16.menu_epoch.load(Ordering::Acquire) == epoch
                                        }) {
                                            app.open_existing(&session, &session, true);
                                        }
                                    });
                                    submenu.append(&child);
                                }
                                item.set_submenu(Some(&submenu));
                            } else {
                                let weak = Rc::downgrade(&app);
                                let key = key.clone();
                                let instance = instance.clone();
                                let ctx = ctx.clone();
                                item.connect_activate(move |_| {
                                    if let Some(app) = weak.upgrade().filter(|a| {
                                        a.instance(&key, &instance)
                                            && a.action_session().as_deref() == Some(&key)
                                            && a.t16.menu_epoch.load(Ordering::Acquire) == epoch
                                    }) {
                                        app.menu_action(&key, &ctx, action);
                                    }
                                });
                            }
                            menu.append(&item);
                        }
                    }
                }
                menu.show_all();
                menu.popup_at_pointer(Some(&event));
                *app.t16.menu.borrow_mut() = Some(menu);
            },
        );
    }
    fn menu_action(self: &Rc<Self>, key: &str, ctx: &Context, action: MenuAction) {
        let Some((tty, instance)) = self
            .term_for_action(key)
            .as_ref()
            .map(|t| (t.child_tty(), t.cleanup_cancellation()))
        else {
            return;
        };
        let tmux = self.tmux.clone();
        let closed = self.closed.clone();
        let cancel = instance.clone();
        let context = ctx.clone();
        let weak = Rc::downgrade(self);
        let key = key.to_string();
        let ctx = ctx.clone();
        let epoch = self.t16.menu_epoch.load(Ordering::Acquire);
        self.jobs.spawn(
            move || {
                ui::menu::context_current(&tmux, &context, tty.as_deref(), || {
                    closed.load(Ordering::Acquire) || cancel.load(Ordering::Acquire)
                })
            },
            move |valid| {
                if let Some(app) = weak.upgrade().filter(|a| {
                    a.instance(&key, &instance)
                        && a.action_session().as_deref() == Some(&key)
                        && a.t16.menu_epoch.load(Ordering::Acquire) == epoch
                }) {
                    if valid {
                        app.perform_menu_action(&key, &ctx, action);
                    } else {
                        app.status
                            .set_text("El destino del menu cambio; abre el menu de nuevo");
                    }
                }
            },
        );
    }
    fn perform_menu_action(self: &Rc<Self>, key: &str, ctx: &Context, action: MenuAction) {
        match action {
            MenuAction::Copy => self.copy_terminal(key, ctx.copy_pane.clone()),
            MenuAction::Paste => self.paste_terminal(key),
            MenuAction::CopyReply => self.reply_terminal(key, None),
            MenuAction::ExportTxt => self.reply_terminal(key, Some("txt")),
            MenuAction::ExportPdf => self.reply_terminal(key, Some("pdf")),
            MenuAction::Split { side, ssh } => {
                if let Some(session) = &ctx.session {
                    let session = session.clone();
                    let pane = ctx.pane.clone();
                    let stamp = ctx.stamp.clone();
                    let tmux = self.tmux.clone();
                    let closed = self.closed.clone();
                    let instance = self
                        .term_for_action(key)
                        .as_ref()
                        .map(TermView::cleanup_cancellation);
                    let weak = Rc::downgrade(self);
                    self.jobs.spawn(
                        move || {
                            ui::app_commands::execute_split_at_expected(
                                &tmux,
                                &session,
                                side,
                                ssh,
                                pane.as_deref(),
                                stamp.as_deref(),
                                || {
                                    closed.load(Ordering::Acquire)
                                        || instance
                                            .as_ref()
                                            .is_none_or(|i| i.load(Ordering::Acquire))
                                },
                            )
                        },
                        move |res| {
                            if let Some(app) = weak.upgrade()
                                && let Err(e) = res
                            {
                                app.status.set_text(&e.to_string());
                            }
                        },
                    );
                }
            }
            MenuAction::ToggleWindow => {
                if let Some(term) = self.term_for_action(key).as_ref()
                    && let Err(e) = term.feed(b"\x02l")
                {
                    self.status.set_text(&format!("{e:?}"));
                }
            }
            MenuAction::CloseSplit => {
                if let (Some(session), Some(pane)) = (&ctx.session, &ctx.pane) {
                    self.close_split(session, pane);
                }
            }
            MenuAction::Rename => self.rename_tab(key),
            MenuAction::CloseTab => self.close_tab(key, true),
            MenuAction::ExistingSessions => {}
            _ => {
                let (name, args) = match action {
                    MenuAction::ToggleWindow => ("toggle_window", json!({})),
                    MenuAction::StartAI => (
                        "start_ai_here",
                        json!({"session":ctx.session,"pane":ctx.pane}),
                    ),
                    MenuAction::NewLocal => ("new_local_tab", json!({})),
                    MenuAction::NewXterm => ("open_xterm_tab", json!({"session":"local"})),
                    MenuAction::OpenProject => ("open_wizard", json!({})),
                    _ => return,
                };
                if let Err(e) = self.handlers.invoke(name, &args) {
                    self.status.set_text(&e.to_string());
                }
            }
        }
    }
    fn open_link(self: &Rc<Self>, key: &str, url: &str, intent: OpenIntent, point: (u16, u16)) {
        let Some(instance) = self
            .term_for_action(key)
            .as_ref()
            .map(TermView::cleanup_cancellation)
        else {
            return;
        };
        let key = key.to_string();
        let cancel = instance.clone();
        let url = url.to_string();
        let tty = self
            .term_for_action(&key)
            .as_ref()
            .and_then(TermView::child_tty);
        let tmux = self.tmux.clone();
        let closed = self.closed.clone();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || {
                let cancelled = || closed.load(Ordering::Acquire) || cancel.load(Ordering::Acquire);
                let url = complete_link(&tmux, tty.as_deref(), point, &url, cancelled);
                let plan = if ui::menu::relative_file_reference(&url) {
                    let context = ui::menu::capture_click(&tmux, tty.as_deref(), point, cancelled)?;
                    let pane = context
                        .pane
                        .as_deref()
                        .ok_or("No se encuentra el panel del archivo")?;
                    let cwd = tmux
                        .read(&["display-message", "-p", "-t", pane, "#{pane_current_path}"])
                        .map_err(|e| format!("{e:?}"))?;
                    if !cwd.ok()
                        || !ui::menu::context_current(&tmux, &context, tty.as_deref(), cancelled)
                    {
                        return Err("El panel del archivo cambió".into());
                    }
                    ui::menu::open_plan_in(
                        &url,
                        intent,
                        std::path::Path::new(cwd.stdout.trim_end_matches(['\r', '\n'])),
                    )?
                } else {
                    ui::menu::open_plan(&url, intent)?
                };
                ui::menu::launch(
                    &plan,
                    || !closed.load(Ordering::Acquire) && !cancel.load(Ordering::Acquire),
                    &crate::proc::run,
                    &crate::proc::spawn_detached,
                )
            },
            move |res| {
                if let Some(app) = weak.upgrade().filter(|a| a.instance(&key, &instance))
                    && let Err(e) = res
                {
                    app.status.set_text(&e);
                }
            },
        );
    }
    fn link_terminal(
        self: &Rc<Self>,
        key: &str,
        url: &str,
        event: &gdk::EventButton,
        point: (u16, u16),
        control: bool,
    ) {
        if control {
            self.open_link(
                key,
                url,
                if ui::menu::clean_local_path(url).is_some()
                    || ui::menu::relative_file_reference(url)
                {
                    OpenIntent::Reveal
                } else {
                    OpenIntent::Open
                },
                point,
            );
            return;
        }
        if let Some(old) = self.t16.link.borrow_mut().take() {
            old.popdown();
            if let Some(child) = old.child() {
                old.remove(&child);
            }
        }
        let Some(term) = self.term_for_action(key).as_ref().cloned() else {
            return;
        };
        let instance = term.cleanup_cancellation();
        let popover = gtk::Popover::new(Some(term.widget()));
        let (x, y) = event.position();
        popover.set_pointing_to(&gdk::Rectangle::new(x as i32, y as i32, 1, 1));
        popover.set_position(gtk::PositionType::Bottom);
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
        box_.set_margin_start(8);
        box_.set_margin_end(8);
        box_.set_margin_top(8);
        box_.set_margin_bottom(8);
        let disp = url.strip_prefix("file://").unwrap_or(url);
        let home = self.cfg.home().display().to_string();
        let shortened = if let Some(tail) = disp.strip_prefix(&home) {
            format!("~{tail}")
        } else {
            disp.to_string()
        };
        let shortened = shortened.chars().take(80).collect::<String>();
        let label = gtk::Label::new(None);
        label.set_xalign(0.);
        label.set_markup(&format!(
            "<span size=\"small\" font_family=\"monospace\">{}</span>",
            glib::markup_escape_text(&shortened)
        ));
        label.set_ellipsize(pango::EllipsizeMode::Middle);
        label.set_max_width_chars(46);
        box_.pack_start(&label, false, false, 0);
        box_.pack_start(
            &gtk::Separator::new(gtk::Orientation::Horizontal),
            false,
            false,
            2,
        );
        let local = ui::menu::clean_local_path(url).is_some()
            || url.starts_with('/')
            || url.starts_with('~')
            || ui::menu::relative_file_reference(url);
        let color = self
            .applied_theme
            .borrow()
            .as_ref()
            .and_then(|t| t.values.get("dim"))
            .and_then(Value::as_str)
            .unwrap_or("#AAAAAA")
            .to_string();
        for (icon, label, intent) in ui::menu::link_rows(local, self.english) {
            let button = gtk::Button::new();
            button.set_relief(gtk::ReliefStyle::None);
            button.style_context().add_class("link-action");
            button.set_size_request(-1, 36);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.set_margin_top(6);
            row.set_margin_bottom(6);
            row.set_margin_start(8);
            row.set_margin_end(8);
            row.pack_start(&ui::icons::image(icon, 15, &color), false, false, 0);
            let text = gtk::Label::new(Some(label));
            text.set_xalign(0.);
            row.pack_start(&text, true, true, 0);
            button.add(&row);
            let weak = Rc::downgrade(self);
            let key = key.to_string();
            let instance = instance.clone();
            let url = url.to_string();
            let pop = popover.downgrade();
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade().filter(|a| a.instance(&key, &instance)) {
                    if let Some(pop) = pop.upgrade() {
                        pop.popdown();
                    }
                    if let Some(intent) = intent {
                        app.open_link(&key, &url, intent, point);
                    } else if let Some(term) = app.term_for_action(&key).as_ref() {
                        term.clipboard().copy(
                            Target::Clipboard,
                            url.strip_prefix("file://").unwrap_or(&url),
                        );
                    }
                }
            });
            box_.pack_start(&button, false, false, 0);
        }
        popover.add(&box_);
        popover.show_all();
        popover.popup();
        *self.t16.link.borrow_mut() = Some(popover);
        let _ = point;
    }
}

/// A link the agent printed across two lines is seen half at a time in the window grid.
/// Completes it from the clicked pane alone (`capture-pane -J` joins tmux soft wraps);
/// on any failure the clicked fragment is kept, so opening never gets worse.
fn complete_link(
    tmux: &impl ui::clipboard::TmuxIo,
    tty: Option<&str>,
    point: (u16, u16),
    url: &str,
    cancelled: impl Fn() -> bool,
) -> String {
    let attempt = || -> Option<String> {
        let context = ui::menu::capture_click(tmux, tty, point, &cancelled).ok()?;
        let pane = context.pane.as_deref()?;
        let text = tmux
            .read(&["capture-pane", "-p", "-J", "-t", pane])
            .ok()
            .filter(|out| out.ok())?
            .stdout;
        let cwd = tmux
            .read(&["display-message", "-p", "-t", pane, "#{pane_current_path}"])
            .ok()
            .filter(|out| out.ok())
            .map(|out| out.stdout.trim_end_matches(['\r', '\n']).to_owned())
            .unwrap_or_default();
        let home = std::env::var("HOME").unwrap_or_default();
        crate::term::links::complete_from_pane(&text, url, |body| {
            let path = if let Some(rest) = body.strip_prefix("~/") {
                std::path::Path::new(&home).join(rest)
            } else {
                std::path::Path::new(&cwd).join(body)
            };
            path.exists()
        })
    };
    attempt().unwrap_or_else(|| url.to_owned())
}
