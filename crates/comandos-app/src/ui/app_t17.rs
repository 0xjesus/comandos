//! Pane-local overlays and shelf consumers. All workers return through T8 Jobs.
use super::*;
use ui::accounts::{self, AccountFlow, AccountStage, Dialog};
pub(super) struct Owned {
    pub account: RefCell<Option<Rc<Dialog>>>,
    pub shelf: ui::extensions::Shelf,
    pub boxes: RefCell<BTreeMap<String, Rc<ui::overlays::PaneOverlay>>>,
    pub models: RefCell<Value>,
    pub models_busy: Cell<bool>,
    pub scope: ui::snippets::Scope,
    pub wizard: ui::snippets::Scope,
    pub sources: RefCell<Vec<glib::SourceId>>,
    pub signals: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
    pub prefix: Cell<(u32, gdk::ModifierType)>,
}
impl Default for Owned {
    fn default() -> Self {
        Self {
            account: RefCell::new(None),
            shelf: ui::extensions::Shelf::new(),
            boxes: RefCell::new(BTreeMap::new()),
            models: RefCell::new(Value::Null),
            models_busy: Cell::new(false),
            scope: ui::snippets::Scope::default(),
            wizard: ui::snippets::Scope::default(),
            sources: RefCell::new(vec![]),
            signals: RefCell::new(vec![]),
            prefix: Cell::new((*gdk::keys::constants::b, gdk::ModifierType::CONTROL_MASK)),
        }
    }
}
impl App {
    pub(super) fn install_t17(self: &Rc<Self>) {
        self.install_shelf();
        self.install_pane_overlays();
    }
    pub(super) fn shutdown_t17(&self) {
        self.close_account();
        self.t17.shelf.cancel();
        self.t17.scope.close();
        self.t17.wizard.close();
        for source in self.t17.sources.borrow_mut().drain(..) {
            source.remove();
        }
        for (object, id) in self.t17.signals.borrow_mut().drain(..) {
            object.disconnect(id);
        }
        for owner in self.t17.boxes.borrow().values() {
            owner.cancel();
        }
        self.t17.boxes.borrow_mut().clear();
    }
    pub(super) fn close_pane_consumers(&self, key: &str) {
        let account = self
            .t17
            .account
            .borrow()
            .as_ref()
            .is_some_and(|a| a.session == key);
        if account {
            self.close_account();
        }
        let extension = self.t17.shelf.extension.borrow().as_ref().is_some_and(|v| {
            v.target
                .as_ref()
                .is_some_and(|(session, _, _)| session == key)
        });
        let opening = self
            .t17
            .shelf
            .opening
            .borrow()
            .as_ref()
            .is_some_and(|(session, _)| session == key);
        if extension || opening {
            self.t17.shelf.close_extension();
        }
        self.t17.wizard.advance();
    }
    fn close_account(&self) {
        if let Some(dialog) = self.t17.account.borrow_mut().take() {
            dialog.cancel();
            dialog.pop.popdown();
        }
    }
    fn account_current(&self, dialog: &Dialog) -> bool {
        dialog.scope.ticket().current()
            && self.owns_term_instance(&dialog.session, &dialog.instance)
    }
    pub(super) fn account_popover(
        self: &Rc<Self>,
        button: &gtk::Button,
        session: &str,
        pane: &str,
        harness: &str,
        current: &str,
    ) {
        if !matches!(harness, "claude" | "codex" | "grok") || !ui::app_commands::valid_pane(pane) {
            return;
        }
        let Some(instance) = self
            .term_for_action(session)
            .as_ref()
            .map(TermView::cleanup_cancellation)
        else {
            return;
        };
        self.close_account();
        let dialog = Dialog::new(
            button,
            session,
            pane,
            harness,
            instance,
            self.dash.isolated(),
            self.english,
        );
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(&dialog);
        dialog.add.connect_clicked(move |_| {
            if let (Some(app), Some(dialog)) = (weak.upgrade(), d.upgrade()) {
                app.account_action(&dialog, "", false);
            }
        });
        dialog.add.set_sensitive(false);
        dialog.pop.show_all();
        dialog.pop.popup();
        *self.t17.account.borrow_mut() = Some(dialog.clone());
        let path = format!("/accounts?harness={}", ui::webview::encode_query(harness));
        let ticket = dialog.scope.ticket();
        let work_ticket = ticket.clone();
        let instance = dialog.instance.clone();
        let tmux = self.tmux.clone();
        let session = session.to_string();
        let pane = pane.to_string();
        let client = dialog.client.clone();
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(&dialog);
        let current = current.to_string();
        self.jobs.spawn(
            move || -> Result<(String, Value), String> {
                if !work_ticket.current() || instance.load(Ordering::Acquire) {
                    return Err("Account view closed".into());
                }
                let identity = accounts::pane_identity_when(&tmux, &session, &pane, None, || {
                    work_ticket.current() && !instance.load(Ordering::Acquire)
                })?;
                let data = client
                    .get(&path, Duration::from_secs(8))
                    .map_err(|_| "No se pudieron leer las cuentas".to_string())?;
                accounts::pane_identity_when(&tmux, &session, &pane, Some(&identity), || {
                    work_ticket.current() && !instance.load(Ordering::Acquire)
                })?;
                Ok((identity, data))
            },
            move |result| {
                let (Some(app), Some(dialog)) = (weak.upgrade(), d.upgrade()) else {
                    return;
                };
                if !ticket.current() || !app.account_current(&dialog) {
                    return;
                }
                match result {
                    Ok((identity, data)) => {
                        *dialog.identity.borrow_mut() = Some(identity);
                        let weak = Rc::downgrade(&app);
                        let d = Rc::downgrade(&dialog);
                        dialog.fill(
                            &data,
                            &current,
                            app.english,
                            app.writable(),
                            Rc::new(move |alias, ready| {
                                if let (Some(app), Some(dialog)) = (weak.upgrade(), d.upgrade()) {
                                    app.account_action(&dialog, &alias, ready);
                                }
                            }),
                        );
                    }
                    Err(error) => dialog.fail(&error, false),
                }
            },
        );
    }
    fn account_action(self: &Rc<Self>, dialog: &Rc<Dialog>, alias: &str, switch: bool) {
        if !self.writable() || !self.account_current(dialog) || dialog.busy.get() {
            return;
        }
        let Some(identity) = dialog.identity.borrow().clone() else {
            return;
        };
        dialog.scope.advance();
        dialog.set_busy(true, true);
        dialog.note.style_context().remove_class("acct-err");
        dialog.note.set_text(if switch {
            if self.english {
                "Moving account…"
            } else {
                "Pasando de cuenta…"
            }
        } else if self.english {
            "Opening login…"
        } else {
            "Abriendo login…"
        });
        let ticket = dialog.scope.ticket();
        let work_ticket = ticket.clone();
        let instance = dialog.instance.clone();
        let tmux = self.tmux.clone();
        let session = dialog.session.clone();
        let pane = dialog.pane.clone();
        let alias = alias.to_string();
        let payload = if switch {
            json!({"session":session,"pane":pane,"alias":alias})
        } else {
            json!({"provider":dialog.harness,"alias":alias})
        };
        let path = if switch {
            "/account/switch"
        } else {
            "/account/add"
        };
        let client = dialog.client.clone();
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(dialog);
        self.jobs.spawn(
            move || -> Result<Value, String> {
                if !work_ticket.current() || instance.load(Ordering::Acquire) {
                    return Err("Account view closed".to_string());
                }
                accounts::pane_identity_when(&tmux, &session, &pane, Some(&identity), || {
                    work_ticket.current() && !instance.load(Ordering::Acquire)
                })?;
                let (status, data) = client
                    .post(path, &payload, Duration::from_secs(20))
                    .map_err(|_| "No se pudo completar la petición de cuenta".to_string())?;
                if !(200..300).contains(&status) || data.get("ok") == Some(&json!(false)) {
                    // The server's reason («el panel tiene una operación pendiente…») is
                    // what tells the user why; the generic line hid it (13-oct).
                    return Err(accounts::request_error(&data));
                }
                accounts::pane_identity_when(&tmux, &session, &pane, Some(&identity), || {
                    work_ticket.current() && !instance.load(Ordering::Acquire)
                })?;
                Ok(data)
            },
            move |result| {
                let (Some(app), Some(dialog)) = (weak.upgrade(), d.upgrade()) else {
                    return;
                };
                if !ticket.current() || !app.writable() || !app.account_current(&dialog) {
                    return;
                }
                match result {
                    Err(error) => dialog.fail(&error, true),
                    Ok(data) if !switch => {
                        let Some(session) = data
                            .get("session")
                            .and_then(Value::as_str)
                            .filter(|s| crate::tab_actions::valid_session(s))
                        else {
                            dialog.fail("Login sin sesión válida", true);
                            return;
                        };
                        let label = format!(
                            "Login {} · {}",
                            dialog.harness,
                            data.get("alias")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                        );
                        dialog.pop.popdown();
                        app.open_tab(session, "claude", &label, true);
                    }
                    Ok(data) => {
                        let Some(id) =
                            data.get("operationId").and_then(Value::as_str).filter(|s| {
                                !s.is_empty()
                                    && s.len() <= 128
                                    && s.bytes().all(|b| {
                                        b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')
                                    })
                            })
                        else {
                            dialog.fail("Cambio sin operación válida", true);
                            return;
                        };
                        let key = format!("{}|{}", dialog.session, dialog.pane);
                        if data.get("operationKey").and_then(Value::as_str) != Some(key.as_str()) {
                            dialog.fail("La operación no corresponde a este pane", true);
                            return;
                        }
                        *dialog.flow.borrow_mut() = Some(AccountFlow {
                            operation_id: id.into(),
                            alias,
                            stage: AccountStage::Switching,
                            attempts: 0,
                        });
                        app.account_poll(&dialog);
                    }
                }
            },
        );
    }
    fn account_poll(self: &Rc<Self>, dialog: &Rc<Dialog>) {
        if !self.account_current(dialog) || !self.writable() {
            dialog.cancel();
            return;
        }
        let next = dialog
            .flow
            .borrow_mut()
            .as_mut()
            .and_then(AccountFlow::next_poll);
        let Some(delay) = next else {
            let awaiting = dialog
                .flow
                .borrow()
                .as_ref()
                .filter(|f| f.stage == AccountStage::AwaitingLogin)
                .map(|f| accounts::awaiting(&f.alias, self.english));
            dialog.fail(
                awaiting.as_deref().unwrap_or(if self.english {
                    "No confirmation in 3 minutes; check the pane's terminal."
                } else {
                    "Sin confirmación en 3 minutos; revisa la terminal del pane."
                }),
                true,
            );
            return;
        };
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(dialog);
        let ticket = dialog.scope.ticket();
        let source = glib::timeout_add_local_once(delay, move || {
            let (Some(app), Some(dialog)) = (weak.upgrade(), d.upgrade()) else {
                return;
            };
            dialog.timer.borrow_mut().take();
            if !ticket.current() || !app.account_current(&dialog) || !app.writable() {
                return;
            }
            app.account_poll_request(&dialog);
        });
        if let Some(old) = dialog.timer.borrow_mut().replace(source) {
            old.remove();
        }
    }
    fn account_poll_request(self: &Rc<Self>, dialog: &Rc<Dialog>) {
        let Some(identity) = dialog.identity.borrow().clone() else {
            return;
        };
        let Some(flow) = dialog.flow.borrow().clone() else {
            return;
        };
        let operation_key = format!("{}|{}", dialog.session, dialog.pane);
        let path = format!(
            "/model/status?operationKey={}&operationId={}",
            ui::webview::encode_query(&operation_key),
            ui::webview::encode_query(&flow.operation_id)
        );
        let client = dialog.client.clone();
        let tmux = self.tmux.clone();
        let session = dialog.session.clone();
        let pane = dialog.pane.clone();
        let ticket = dialog.scope.ticket();
        let work_ticket = ticket.clone();
        let instance = dialog.instance.clone();
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(dialog);
        self.jobs.spawn(move || {
            if !work_ticket.current()||instance.load(Ordering::Acquire){return None;}
            if accounts::pane_identity_when(&tmux,&session,&pane,Some(&identity),||work_ticket.current()&&!instance.load(Ordering::Acquire)).is_err(){return Some(Err(()));}
            let result=client.get(&path,Duration::from_secs(8));
            if accounts::pane_identity_when(&tmux,&session,&pane,Some(&identity),||work_ticket.current()&&!instance.load(Ordering::Acquire)).is_err(){return Some(Err(()));}
            Some(Ok(result.ok()))
        },move |result| {
            let (Some(app),Some(dialog))=(weak.upgrade(),d.upgrade()) else{return;};
            if !ticket.current()||!app.account_current(&dialog)||!app.writable(){return;}
            match result {
                Some(Err(()))=>{dialog.fail("El pane cambió durante la operación",false);dialog.cancel();return;},
                Some(Ok(Some(data)))=>{
                    let mut state=dialog.flow.borrow_mut();let Some(state)=state.as_mut().filter(|f|f.alias==flow.alias&&f.operation_id==flow.operation_id) else{return;};
                    if state.adopt_response(&flow.operation_id,&data) {
                        if state.stage==AccountStage::Complete {
                            dialog.note.set_text(&if app.english {format!("Done: this pane now continues the same conversation on {}.",state.alias)}else{format!("Listo: este pane ya sigue la misma conversación en {}.",state.alias)});
                            let d=Rc::downgrade(&dialog);let ticket=dialog.scope.ticket();
                            *dialog.timer.borrow_mut()=Some(glib::timeout_add_local_once(Duration::from_millis(2200),move ||{if let Some(dialog)=d.upgrade(){dialog.timer.borrow_mut().take();if ticket.current(){dialog.pop.popdown();}}}));return;
                        }
                        if state.stage==AccountStage::Failed {dialog.fail("El cambio no se confirmó",true);return;}
                        let note=accounts::stage_note(&data,&state.alias,app.english);if !note.is_empty(){dialog.note.set_text(&note);}
                    }
                },_=>{}
            }
            app.account_poll(&dialog);
        });
    }
}
