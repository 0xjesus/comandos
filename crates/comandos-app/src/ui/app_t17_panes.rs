use super::*;
use ui::overlays::{self, Drag, PaneOverlay, PaneWidget, ResizeWish};
impl App {
    pub(super) fn install_pane_overlays(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.t17.sources.borrow_mut().push(glib::timeout_add_local(
            Duration::from_secs(2),
            move || {
                let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                    return glib::ControlFlow::Break;
                };
                if app.presence_visible.get() {
                    app.refresh_pane_models();
                }
                glib::ControlFlow::Continue
            },
        ));
        for notebook in [&self.notebook, self.workspace.widget()] {
            let weak = Rc::downgrade(self);
            let id = notebook.connect_switch_page(move |_, _, _| {
                if let Some(app) = weak.upgrade() {
                    app.refresh_visible_geometry();
                }
            });
            self.t17
                .signals
                .borrow_mut()
                .push((notebook.clone().upcast(), id));
        }
        let tmux = self.tmux.clone();
        let weak = Rc::downgrade(self);
        let ticket = self.t17.scope.ticket();
        self.jobs.spawn(
            move || tmux.read(&["show-options", "-gv", "prefix"]),
            move |result| {
                if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire))
                    && ticket.current()
                {
                    let raw = result
                        .ok()
                        .filter(|out| out.ok())
                        .map(|out| out.stdout.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "C-b".into());
                    let mut name = raw.as_str();
                    let mut mask = gdk::ModifierType::empty();
                    while name.len() > 2 && name.as_bytes().get(1) == Some(&b'-') {
                        let bit = match name.as_bytes().first() {
                            Some(b'C') => gdk::ModifierType::CONTROL_MASK,
                            Some(b'M') => gdk::ModifierType::MOD1_MASK,
                            Some(b'S') => gdk::ModifierType::SHIFT_MASK,
                            _ => break,
                        };
                        mask |= bit;
                        name = &name[2..];
                    }
                    let key = gdk::keys::Key::from_name(name);
                    if key != gdk::keys::constants::VoidSymbol {
                        app.t17.prefix.set((*key.to_lower(), mask));
                    }
                }
            },
        );
    }
    pub(super) fn attach_pane_overlay(
        self: &Rc<Self>,
        key: &str,
        term: &TermView,
    ) -> Rc<PaneOverlay> {
        self.attach_pane_overlay_at(key, term, key)
    }
    pub(super) fn attach_pane_overlay_at(
        self: &Rc<Self>,
        key: &str,
        term: &TermView,
        view_id: &str,
    ) -> Rc<PaneOverlay> {
        let owner = PaneOverlay::new(key, term);
        let weak_owner = Rc::downgrade(&owner);
        owner.widget.connect_local("draw", true, move |values| {
            if let Some(owner) = weak_owner.upgrade()
                && let Some(cr) = values.get(1).and_then(|v| v.get::<cairo::Context>().ok())
            {
                owner.draw(&cr);
            }
            Some(false.to_value())
        });
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(&owner);
        term.widget().connect_size_allocate(move |_, _| {
            if let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade()) {
                owner.reposition(app.pane_focused(&owner));
                app.pane_geo_soon(&owner);
            }
        });
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(&owner);
        term.widget().connect_button_release_event(move |_, _| {
            if let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade()) {
                app.pane_geo_soon(&owner);
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(&owner);
        term.widget().connect_button_press_event(move |_, event| {
            if let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade()) {
                let (x, y) = event.position();
                let geom = owner.term.cell_geometry();
                let col = ((x - geom.origin_x) / geom.cell_w).floor();
                let row = ((y - geom.origin_y) / geom.cell_h).floor();
                let selected = owner
                    .panes
                    .borrow()
                    .iter()
                    .find(|p| {
                        f64::from(p.geometry.left) <= col
                            && col < f64::from(p.geometry.left) + f64::from(p.geometry.width)
                            && f64::from(p.geometry.top) - 1.0 <= row
                            && row < f64::from(p.geometry.top) + f64::from(p.geometry.height)
                    })
                    .map(|p| p.geometry.id.clone());
                if let Some(selected) = selected {
                    for p in owner.panes.borrow_mut().iter_mut() {
                        p.active = p.geometry.id == selected;
                    }
                    owner.reposition(app.pane_focused(&owner));
                }
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(&owner);
        let key = key.to_string();
        term.widget().connect_focus_in_event(move |_, _| {
            if let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade()) {
                if !app.auxiliary_current(&owner) {
                    app.t18.focused.borrow_mut().take();
                    app.workspace.note_focus(&key);
                }
                owner.reposition(true);
                app.refresh_visible_geometry();
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(&owner);
        let hover_owner = Rc::downgrade(&owner);
        term.widget().connect_motion_notify_event(move |_, event| {
            if let Some(owner) = hover_owner.upgrade() {
                owner.hover_at(Some(event.position()));
            }
            glib::Propagation::Proceed
        });
        let hover_owner = Rc::downgrade(&owner);
        term.widget().connect_leave_notify_event(move |_, _| {
            if let Some(owner) = hover_owner.upgrade() {
                owner.hover_at(None);
            }
            glib::Propagation::Proceed
        });
        let focus_owner = Rc::downgrade(&owner);
        term.widget().connect_focus_out_event(move |_, _| {
            if let Some(owner) = focus_owner.upgrade() {
                owner.reposition(false);
            }
            glib::Propagation::Proceed
        });
        let epoch = std::time::Instant::now();
        term.on_key_observer(Rc::new(move |event| {
            if let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade()) {
                let now = epoch.elapsed().as_secs_f64();
                let (key, mask) = app.t17.prefix.get();
                let mods = event.state()
                    & (gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::MOD1_MASK);
                if *event.keyval().to_lower() == key && mods == mask {
                    owner.prefix_until.set(now + 1.5);
                } else if now < owner.prefix_until.get() {
                    owner.prefix_until.set(now + 0.6);
                    if let Some(source) = owner.prefix_timer.borrow_mut().take() {
                        source.remove();
                    }
                    let weak = Rc::downgrade(&app);
                    let d = Rc::downgrade(&owner);
                    *owner.prefix_timer.borrow_mut() = Some(glib::timeout_add_local_once(
                        Duration::from_millis(60),
                        move || {
                            if let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade()) {
                                owner.prefix_timer.borrow_mut().take();
                                app.pane_geo_soon(&owner);
                            }
                        },
                    ));
                }
            }
        }));
        let d = Rc::downgrade(&owner);
        term.widget().connect_destroy(move |_| {
            if let Some(owner) = d.upgrade() {
                owner.cancel();
            }
        });
        if let Some(old) = self
            .t17
            .boxes
            .borrow_mut()
            .insert(view_id.to_string(), owner.clone())
        {
            old.cancel();
        }
        owner
    }
    fn pane_current(&self, owner: &PaneOverlay) -> bool {
        !self.closed.load(Ordering::Acquire)
            && owner.scope.ticket().current()
            && !owner.term.cleanup_cancellation().load(Ordering::Acquire)
            && (self.auxiliary_current(owner)
                || self.terms.borrow().get(&owner.session).is_some_and(|t| {
                    Arc::ptr_eq(
                        &t.cleanup_cancellation(),
                        &owner.term.cleanup_cancellation(),
                    )
                }))
    }
    fn pane_visible(&self, owner: &PaneOverlay) -> bool {
        if !self.presence_visible.get() {
            return false;
        }
        if self.auxiliary_current(owner) {
            return self.auxiliary_visible(owner);
        }
        self.active_notebook()
            .current_page()
            .and_then(|i| self.active_notebook().nth_page(Some(i)))
            .is_some_and(|page| {
                page == owner.widget.clone().upcast::<gtk::Widget>()
                    || owner.widget.is_ancestor(&page)
            })
    }
    fn pane_focused(&self, owner: &PaneOverlay) -> bool {
        if self.auxiliary_current(owner) {
            self.auxiliary_focused(owner)
        } else {
            self.current_session().as_deref() == Some(owner.session.as_str())
        }
    }
    fn refresh_visible_geometry(self: &Rc<Self>) {
        for owner in self.t17.boxes.borrow().values() {
            if self.pane_visible(owner) {
                self.pane_geo_soon(owner);
            }
        }
    }
    fn refresh_pane_models(self: &Rc<Self>) {
        self.refresh_visible_geometry();
        if self.t17.models_busy.replace(true) {
            return;
        }
        let state = self.state.clone();
        let ticket = self.t17.scope.ticket();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || state.read_pane_document("app-tab-models.json"),
            move |result| {
                let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                    return;
                };
                app.t17.models_busy.set(false);
                if !ticket.current() {
                    return;
                }
                match result {
                    Ok(models) => {
                        *app.t17.models.borrow_mut() = models;
                        app.update_tab_model_labels();
                        for owner in app.t17.boxes.borrow().values() {
                            if app.pane_visible(owner) {
                                app.place_pane_pills(owner);
                            }
                        }
                    }
                    Err(error) => app.status.set_text(&format!("Pane models: {error:?}")),
                }
            },
        );
    }
    fn update_tab_model_labels(&self) {
        let models = self.t17.models.borrow();
        for (session, tab) in self
            .labels
            .borrow()
            .iter()
            .filter(|(key, _)| key.as_str() != "local")
        {
            let panes = models
                .get(session.split(':').next().unwrap_or(session))
                .and_then(|m| m.get("panes"))
                .and_then(Value::as_array);
            let known = panes
                .into_iter()
                .flatten()
                .filter_map(|p| p.get("model").and_then(Value::as_str))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            let text = known.first().map_or_else(String::new, |first| {
                format!(
                    "{first}{}",
                    if known.len() > 1 {
                        format!(" +{}", known.len() - 1)
                    } else {
                        String::new()
                    }
                )
            });
            for widget in tab.row.children() {
                if widget.style_context().has_class("tab-model")
                    && let Some(label) = widget.downcast_ref::<gtk::Label>()
                    && label.text() != text
                {
                    label.set_text(&text);
                }
            }
        }
    }
    pub(super) fn pane_geo_soon(self: &Rc<Self>, owner: &Rc<PaneOverlay>) {
        if !self.pane_current(owner) || !self.pane_visible(owner) || owner.timer.borrow().is_some()
        {
            return;
        }
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(owner);
        *owner.timer.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_millis(16),
            move || {
                if let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade()) {
                    owner.timer.borrow_mut().take();
                    let next = owner.gate.borrow_mut().request();
                    if let Some(ticket) = next {
                        app.request_pane_geometry(&owner, ticket);
                    }
                }
            },
        ));
    }
    fn request_pane_geometry(self: &Rc<Self>, owner: &Rc<PaneOverlay>, serial: u64) {
        if !self.pane_current(owner) || !self.pane_visible(owner) {
            owner.gate.borrow_mut().pause();
            return;
        }
        let session = owner.session.clone();
        let tmux = self.tmux.clone();
        let instance = owner.term.cleanup_cancellation();
        let ticket = owner.scope.ticket();
        let work_ticket = ticket.clone();
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(owner);
        self.jobs.spawn(
            move || {
                overlays::read_geometry_when(&tmux, &session, || {
                    work_ticket.current() && !instance.load(Ordering::Acquire)
                })
            },
            move |result| {
                let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade()) else {
                    return;
                };
                let (publish, next) = owner.gate.borrow_mut().finish(serial);
                if !ticket.current() || !app.pane_current(&owner) || !app.pane_visible(&owner) {
                    owner.gate.borrow_mut().pause();
                    return;
                }
                if publish {
                    match result {
                        Ok(panes) => {
                            *owner.panes.borrow_mut() = panes;
                            app.sync_pane_grips(&owner);
                            app.place_pane_pills(&owner);
                        }
                        Err(error) => {
                            app.status.set_text(&error);
                            owner.panes.borrow_mut().clear();
                            app.sync_pane_grips(&owner);
                            app.place_pane_pills(&owner);
                        }
                    }
                }
                if let Some(next) = next {
                    app.request_pane_geometry(&owner, next);
                }
            },
        );
    }
    fn place_pane_pills(self: &Rc<Self>, owner: &Rc<PaneOverlay>) {
        let models = self.t17.models.borrow();
        let info = models.get(owner.session.split(':').next().unwrap_or(&owner.session));
        let panes = info
            .and_then(|v| v.get("panes"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let theme = self.applied_theme.borrow();
        let values = theme.as_ref().map(|t| &t.values);
        let color = |key, fallback: &str| {
            values
                .and_then(|v| v.get(key))
                .and_then(Value::as_str)
                .unwrap_or(fallback)
                .to_string()
        };
        let fg = color("fg", "#ECECEC");
        let brand = color("brand", "#8B7CFF");
        let line = color("line", "#223044");
        let bg = color("bg", "#0A0D13");
        *owner.colors.borrow_mut() = (line, brand.clone(), bg);
        owner.reposition(self.pane_focused(owner));
        let geometry = owner
            .panes
            .borrow()
            .iter()
            .map(|p| (p.geometry.id.clone(), p.command.clone()))
            .collect::<Vec<_>>();
        let title = self
            .registry
            .borrow()
            .to_json()
            .get(&owner.session)
            .and_then(Value::as_str)
            .unwrap_or(&owner.session)
            .to_string();
        let signature = format!(
            "{title}|{}|{:?}|{fg}|{brand}",
            panes
                .iter()
                .take(6)
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join(","),
            geometry
        );
        if *owner.signature.borrow() == signature {
            return;
        }
        *owner.signature.borrow_mut() = signature;
        // Take the list first: removing a widget re-enters `reposition`,
        // which borrows `pills` again (panic on account switch, 13-oct).
        let old = std::mem::take(&mut *owner.pills.borrow_mut());
        for pill in old {
            owner.widget.remove(&pill.widget);
        }
        owner.ai.borrow_mut().clear();
        owner.ai_frame.borrow_mut().clear();
        let mut placed = BTreeSet::new();
        for pane in panes.iter().take(6) {
            let id = pane.get("pane").and_then(Value::as_str).unwrap_or_default();
            let records = owner.panes.borrow();
            let Some(record) = records.iter().find(|r| r.geometry.id == id) else {
                continue;
            };
            if overlays::is_shell(&record.command) {
                continue;
            }
            drop(records);
            let Some(row) = owner.layout.borrow().rows.get(id).copied() else {
                continue;
            };
            let card = pane_card(pane, &title, &fg);
            if owner.session != "local" {
                let ai = gtk::Image::new();
                card.pack_start(&ai, false, false, 0);
                card.reorder_child(&ai, 0);
                owner.ai.borrow_mut().push(ai);
            }
            card.set_halign(gtk::Align::Start);
            card.set_valign(gtk::Align::Start);
            owner.widget.add_overlay(&card);
            owner.widget.set_overlay_pass_through(&card, true);
            card.show_all();
            // Natural width of the card's text, before reposition stretches it.
            let card_text = card.preferred_width().1;
            owner.pills.borrow_mut().push(PaneWidget {
                pane: id.into(),
                kind: "card",
                widget: card.upcast(),
                width: card_text,
            });
            let keys = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            keys.style_context().add_class("pane-card-keys");
            let harness = pane
                .get("harness")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if matches!(harness, "claude" | "codex" | "grok") {
                let alias = pane
                    .get("hAcct")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty() && *s != "unknown")
                    .unwrap_or("…");
                let button = card_button(
                    "user",
                    &format!(
                        "{}: {alias}",
                        if self.english { "Account" } else { "Cuenta" }
                    ),
                    if self.english {
                        "Move this pane to another account, same conversation"
                    } else {
                        "Pasar este pane a otra cuenta sin perder la conversación"
                    },
                    &fg,
                );
                let weak = Rc::downgrade(self);
                let d = Rc::downgrade(owner);
                let id = id.to_string();
                let harness = harness.to_string();
                let alias = alias.to_string();
                button.connect_clicked(move |b| {
                    if let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade())
                        && app.pane_current(&owner)
                    {
                        owner.term.widget().grab_focus();
                        app.account_popover(b, &owner.session, &id, &harness, &alias);
                    }
                });
                keys.pack_start(&button, false, false, 0);
            }
            let button = self.extension_button(owner, id, harness, &fg);
            keys.pack_start(&button, false, false, 0);
            keys.set_halign(gtk::Align::Start);
            keys.set_valign(gtk::Align::Start);
            owner.widget.add_overlay(&keys);
            keys.show_all();
            let width = keys.preferred_width().1;
            owner.pills.borrow_mut().push(PaneWidget {
                pane: id.into(),
                kind: "keys",
                widget: keys.upcast(),
                width,
            });
            placed.insert(id.to_string());
            let _ = row;
        }
        for record in owner.panes.borrow().iter() {
            if placed.contains(&record.geometry.id)
                || !overlays::is_shell(&record.command)
                || record.geometry.width < 24
            {
                continue;
            }
            let pill = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            pill.style_context().add_class("pane-pill");
            pill.style_context().add_class("pp-shell");
            pill.set_halign(gtk::Align::Start);
            pill.set_valign(gtk::Align::Start);
            let button = gtk::Button::new();
            let contents = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            contents.pack_start(&svg_image("layers", 13, &brand), false, false, 0);
            let label = gtk::Label::new(None);
            label.set_markup(&format!(
                "<span foreground=\"{fg}\" weight=\"bold\">{}</span>",
                if self.english {
                    "Start AI here"
                } else {
                    "Iniciar IA aquí"
                }
            ));
            contents.pack_start(&label, false, false, 0);
            button.add(&contents);
            button.set_relief(gtk::ReliefStyle::None);
            button.style_context().add_class("pill-btn");
            button.set_sensitive(self.writable());
            button.set_tooltip_text(Some(if self.english {
                "Pick CLI, engine and account: starts in THIS terminal (Ctrl+Shift+A)"
            } else {
                "Elige CLI, motor y cuenta: arranca en ESTA terminal (Ctrl+Shift+A)"
            }));
            let weak = Rc::downgrade(self);
            let d = Rc::downgrade(owner);
            let pane = record.geometry.id.clone();
            button.connect_clicked(move |_| {
                if let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade())
                    && app.pane_current(&owner)
                {
                    owner.term.widget().grab_focus();
                    app.start_ai_in_pane(&owner.session, &pane);
                }
            });
            pill.pack_start(&button, false, false, 0);
            pill.pack_start(
                &self.extension_button(owner, &record.geometry.id, "", &fg),
                false,
                false,
                0,
            );
            owner.widget.add_overlay(&pill);
            pill.show_all();
            owner.pills.borrow_mut().push(PaneWidget {
                pane: record.geometry.id.clone(),
                kind: "shell",
                widget: pill.upcast(),
                width: 0,
            });
        }
        owner.reposition(self.pane_focused(owner));
    }
    fn extension_button(
        self: &Rc<Self>,
        owner: &Rc<PaneOverlay>,
        pane: &str,
        harness: &str,
        fg: &str,
    ) -> gtk::Button {
        let button = card_button(
            "ia-spark",
            "MCPs · Skills",
            if self.english {
                "This pane's extensions"
            } else {
                "Extensiones de este panel"
            },
            fg,
        );
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(owner);
        let pane = pane.to_string();
        let harness = harness.to_string();
        button.connect_clicked(move |_| {
            if let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade())
                && app.pane_current(&owner)
            {
                owner.term.widget().grab_focus();
                app.open_extension(&owner.session, &pane, &harness);
            }
        });
        button
    }
    pub(super) fn paint_pane_indicators(&self, seconds: f64) {
        if !self.presence_visible.get() {
            return;
        }
        for owner in self
            .t17
            .boxes
            .borrow()
            .values()
            .filter(|owner| self.pane_visible(owner))
        {
            let state = self
                .state_items
                .borrow()
                .get(&owner.session)
                .and_then(|v| v.get("status"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            owner.paint_ai(
                &state,
                if ui::header::animations_enabled() {
                    seconds
                } else {
                    0.0
                },
                self.english,
            );
        }
    }
    fn sync_pane_grips(self: &Rc<Self>, owner: &Rc<PaneOverlay>) {
        let previous = owner
            .layout
            .borrow()
            .gutters
            .iter()
            .map(|g| (g.pane.clone(), g.orientation.clone()))
            .collect::<Vec<_>>();
        owner.reposition(self.pane_focused(owner));
        let gutters = owner.layout.borrow().gutters.clone();
        let next = gutters
            .iter()
            .map(|g| (g.pane.clone(), g.orientation.clone()))
            .collect::<Vec<_>>();
        if previous == next && owner.grips.borrow().len() == gutters.len() {
            return;
        }
        owner.drag.borrow_mut().take();
        owner.wanted.borrow_mut().take();
        owner.hot.set(None);
        let old = std::mem::take(&mut *owner.grips.borrow_mut());
        for grip in old {
            owner.widget.remove(&grip);
        }
        for (index, gutter) in gutters.iter().enumerate() {
            let grip = gtk::EventBox::new();
            grip.set_visible_window(false);
            grip.set_halign(gtk::Align::Start);
            grip.set_valign(gtk::Align::Start);
            grip.add_events(
                gdk::EventMask::BUTTON_PRESS_MASK
                    | gdk::EventMask::BUTTON_RELEASE_MASK
                    | gdk::EventMask::POINTER_MOTION_MASK
                    | gdk::EventMask::ENTER_NOTIFY_MASK
                    | gdk::EventMask::LEAVE_NOTIFY_MASK,
            );
            let d = Rc::downgrade(owner);
            grip.connect_enter_notify_event(move |w, _| {
                if let Some(owner) = d.upgrade() {
                    owner.hot.set(Some(index));
                    owner.cursor(w, "grab");
                    owner.widget.queue_draw();
                }
                glib::Propagation::Stop
            });
            let d = Rc::downgrade(owner);
            grip.connect_leave_notify_event(move |_, _| {
                if let Some(owner) = d.upgrade()
                    && owner.drag.borrow().is_none()
                {
                    owner.hot.set(None);
                    owner.widget.queue_draw();
                }
                glib::Propagation::Stop
            });
            let weak = Rc::downgrade(self);
            let d = Rc::downgrade(owner);
            let pane = gutter.pane.clone();
            let orientation = gutter.orientation.clone();
            grip.connect_button_press_event(move |w, event| {
                if event.button() != 1 {
                    return glib::Propagation::Proceed;
                }
                if let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade())
                    && app.writable()
                    && app.pane_current(&owner)
                {
                    let panes = owner.geometries();
                    let neighbor =
                        overlays::gutter_neighbor(&panes, &pane, &orientation).map(str::to_string);
                    if event.event_type() == gdk::EventType::DoubleButtonPress {
                        owner.drag.borrow_mut().take();
                        if let Some(size) = overlays::gutter_half(&panes, &pane, &orientation) {
                            *owner.wanted.borrow_mut() = Some(ResizeWish {
                                pane: pane.clone(),
                                orientation: orientation.clone(),
                                neighbor,
                                size,
                            });
                            app.send_gutter_resize(&owner);
                        }
                    } else if event.event_type() == gdk::EventType::ButtonPress {
                        owner.sent.borrow_mut().take();
                        *owner.drag.borrow_mut() = Some(Drag {
                            pane: pane.clone(),
                            orientation: orientation.clone(),
                            neighbor,
                        });
                        owner.cursor(w, "grabbing");
                    }
                    owner.hot.set(Some(index));
                    owner.widget.queue_draw();
                }
                glib::Propagation::Stop
            });
            let weak = Rc::downgrade(self);
            let d = Rc::downgrade(owner);
            grip.connect_motion_notify_event(move |w, event| {
                if let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade())
                    && let Some(drag) = owner.drag.borrow().clone()
                {
                    let geom = owner.term.cell_geometry();
                    let (x, y) = event.position();
                    let x = f64::from(w.margin_start()) + x - geom.origin_x;
                    let y = f64::from(w.margin_top()) + y - geom.origin_y;
                    if geom.cell_w >= 4.0 && geom.cell_h >= 6.0 {
                        let cell = if drag.orientation == "v" {
                            (x / geom.cell_w).floor()
                        } else {
                            (y / geom.cell_h).floor()
                        };
                        if let Some(size) = overlays::gutter_target(
                            &owner.geometries(),
                            &drag.pane,
                            &drag.orientation,
                            cell,
                        )
                        .and_then(|v| v.as_f64())
                        {
                            *owner.wanted.borrow_mut() = Some(ResizeWish {
                                pane: drag.pane,
                                orientation: drag.orientation,
                                neighbor: drag.neighbor,
                                size: size as u16,
                            });
                            app.send_gutter_resize(&owner);
                        }
                    }
                }
                glib::Propagation::Stop
            });
            let weak = Rc::downgrade(self);
            let d = Rc::downgrade(owner);
            grip.connect_button_release_event(move |w, event| {
                if event.button() != 1 {
                    return glib::Propagation::Proceed;
                }
                if let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade()) {
                    owner.drag.borrow_mut().take();
                    app.send_gutter_resize(&owner);
                    owner.cursor(w, "grab");
                    let (x, y) = event.position();
                    owner.hot.set(
                        (x >= 0.0
                            && y >= 0.0
                            && x < f64::from(w.allocated_width())
                            && y < f64::from(w.allocated_height()))
                        .then_some(index),
                    );
                    owner.widget.queue_draw();
                }
                glib::Propagation::Stop
            });
            owner.widget.add_overlay(&grip);
            grip.show();
            owner.grips.borrow_mut().push(grip);
        }
        owner.reposition(self.pane_focused(owner));
    }
    fn send_gutter_resize(self: &Rc<Self>, owner: &Rc<PaneOverlay>) {
        if !self.writable() || !self.pane_current(owner) || owner.resize_busy.get() {
            return;
        }
        let Some(wish) = owner
            .wanted
            .borrow()
            .clone()
            .filter(|wish| Some(wish) != owner.sent.borrow().as_ref())
        else {
            return;
        };
        let Some(neighbor) = wish.neighbor.clone() else {
            return;
        };
        owner.resize_busy.set(true);
        *owner.sent.borrow_mut() = Some(wish.clone());
        let ticket = owner.scope.ticket();
        let work_ticket = ticket.clone();
        let instance = owner.term.cleanup_cancellation();
        let tmux = self.tmux.clone();
        let session = owner.session.clone();
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(owner);
        self.jobs.spawn(
            move || {
                overlays::resize_neighbor_when(
                    &tmux,
                    &session,
                    &wish.pane,
                    &wish.orientation,
                    &neighbor,
                    wish.size,
                    || work_ticket.current() && !instance.load(Ordering::Acquire),
                )
            },
            move |result| {
                let (Some(app), Some(owner)) = (weak.upgrade(), d.upgrade()) else {
                    return;
                };
                owner.resize_busy.set(false);
                if !ticket.current() || !app.pane_current(&owner) {
                    return;
                }
                if let Err(error) = result {
                    app.status.set_text(&error);
                    owner.wanted.borrow_mut().take();
                }
                app.pane_geo_soon(&owner);
                app.send_gutter_resize(&owner);
            },
        );
    }
}
fn svg_image(name: &str, size: i32, color: &str) -> gtk::Widget {
    let svg = match name {
        "layers" => include_str!("../../../../dash/icons/layers.svg"),
        "user" => include_str!("../../../../dash/icons/user.svg"),
        "ia-spark" => include_str!("../../../../dash/icons/ia-spark.svg"),
        "anthropic" => include_str!("../../../../dash/icons/anthropic.svg"),
        "openai" => include_str!("../../../../dash/icons/openai.svg"),
        "grok" => include_str!("../../../../dash/icons/grok.svg"),
        _ => return gtk::Label::new(Some("⬤")).upcast(),
    };
    if let Ok(loader) = gdk_pixbuf::PixbufLoader::with_type("svg") {
        loader.set_size(size, size);
        if loader
            .write(svg.replace("currentColor", color).as_bytes())
            .is_ok()
            && loader.close().is_ok()
        {
            return gtk::Image::from_pixbuf(loader.pixbuf().as_ref()).upcast();
        }
    }
    gtk::Label::new(Some("⬤")).upcast()
}
/// A pane header as the app draws it, `width` px wide, for the `header_shot` bench:
/// the same card, the same buttons and the same fitting as `place_pane_pills`.
pub fn header_preview(pane: &Value, title: &str, account: &str, width: i32) -> gtk::Widget {
    let fg = "#ECECEC";
    let area = gtk::Overlay::new();
    let floor = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    floor.set_size_request(width, 34);
    area.add(&floor);
    let card = pane_card(pane, title, fg);
    card.set_halign(gtk::Align::Start);
    card.set_valign(gtk::Align::Start);
    card.show_all();
    let card_text = card.preferred_width().1;
    card.set_size_request(width, 30);
    area.add_overlay(&card);
    let keys = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    keys.style_context().add_class("pane-card-keys");
    keys.pack_start(&card_button("user", &format!("Cuenta: {account}"), "", fg), false, false, 0);
    keys.pack_start(&card_button("ia-spark", "MCPs · Skills", "", fg), false, false, 0);
    keys.set_halign(gtk::Align::Start);
    keys.set_valign(gtk::Align::Start);
    keys.show_all();
    let full = keys.preferred_width().1;
    area.add_overlay(&keys);
    match overlays::fit_keys(keys.upcast_ref(), width, card_text, full) {
        Some(used) => {
            keys.set_margin_start((width - used - 8).max(0));
            keys.set_margin_top(4);
        }
        None => keys.hide(),
    }
    area.upcast()
}
fn card_button(icon: &str, text: &str, tooltip: &str, fg: &str) -> gtk::Button {
    let button = gtk::Button::new();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    content.pack_start(&svg_image(icon, 12, fg), false, false, 0);
    content.pack_start(&gtk::Label::new(Some(text)), false, false, 0);
    button.add(&content);
    button.set_relief(gtk::ReliefStyle::None);
    button.style_context().add_class("pill-btn");
    button.set_tooltip_text(Some(tooltip));
    button
}
fn pane_card(p: &Value, title: &str, fg: &str) -> gtk::Box {
    let text = |name| p.get(name).and_then(Value::as_str).unwrap_or_default();
    let harness = if text("harness").is_empty() {
        "?"
    } else {
        text("harness")
    };
    let motor = if text("motor").is_empty() {
        "?"
    } else {
        text("motor")
    };
    let state = if text("state").is_empty() {
        "detecting"
    } else {
        text("state")
    };
    let known = matches!(
        motor,
        "claude" | "codex" | "grok" | "opencode" | "gemini" | "acp" | "agy"
    );
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for class in [
        "pane-pill",
        "pane-card",
        &format!("pp-{}", if known { motor } else { "x" }),
    ] {
        card.style_context().add_class(class);
    }
    card.set_valign(gtk::Align::Start);
    let name = gtk::Label::new(None);
    name.set_markup(&format!(
        "<span foreground=\"{fg}\" weight=\"bold\">{}</span>",
        glib::markup_escape_text(title)
    ));
    name.set_ellipsize(pango::EllipsizeMode::End);
    name.set_max_width_chars(18);
    card.pack_start(&name, false, false, 0);
    let sep = gtk::Separator::new(gtk::Orientation::Vertical);
    sep.set_valign(gtk::Align::Center);
    card.pack_start(&sep, false, false, 0);
    if motor != harness
        && let Some(icon) = match harness {
            "claude" => Some("anthropic"),
            "codex" => Some("openai"),
            "grok" => Some("grok"),
            "acp" | "opencode" => Some("layers"),
            "agy" => Some("gemini"),
            _ => None,
        }
    {
        card.pack_start(&svg_image(icon, 12, "#8A8F98"), false, false, 0);
    }
    let (letter, color) = match motor {
        "claude" => ("A".to_string(), "#d97757"),
        "codex" => ("O".into(), "#10a37f"),
        "grok" => ("G".into(), "#fb923c"),
        "opencode" => ("OC".into(), "#f0abfc"),
        "gemini" => ("G".into(), "#4285f4"),
        "agy" => ("AG".into(), "#93c5fd"),
        "acp" => ("AC".into(), "#67e8f9"),
        _ => (
            motor.chars().take(2).collect::<String>().to_uppercase(),
            "#56627a",
        ),
    };
    let badge = gtk::EventBox::new();
    badge.style_context().add_class("pp-logo");
    badge
        .style_context()
        .add_class(&format!("pp-logo-{}", if known { motor } else { "x" }));
    let provider = gtk::CssProvider::new();
    if provider
        .load_from_data(format!(".pp-logo{{background-color:{color};}}").as_bytes())
        .is_ok()
    {
        badge
            .style_context()
            .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    let glyph = gtk::Label::new(Some(&letter));
    glyph.style_context().add_class("pp-logo-glyph");
    glyph.set_halign(gtk::Align::Center);
    glyph.set_valign(gtk::Align::Center);
    badge.add(&glyph);
    badge.set_valign(gtk::Align::Center);
    card.pack_start(&badge, false, false, 0);
    let model = gtk::Label::new(None);
    let esc = glib::markup_escape_text;
    model.set_markup(&match state {"changing"=>format!("<span foreground=\"{fg}\" weight=\"bold\">{}</span>",esc(&format!("→ {}",if text("target").is_empty(){"…"}else{text("target")}))),"verified"=>format!("<span foreground=\"{fg}\" weight=\"bold\" font_family=\"Ubuntu Sans Mono\">{}</span>{}",esc(text("model")),if text("effort").is_empty(){String::new()}else{format!("<span foreground=\"#8A8F98\"> {}</span>",esc(text("effort")))}),_=>"<span foreground=\"#8A8F98\">…</span>".into()});
    card.pack_start(&model, false, false, 0);
    let (state_color, symbol) = match state {
        "verified" => ("#4ADE80", "✓"),
        "changing" => ("#FBBF24", "⟳"),
        _ => ("#9CA3AF", "◌"),
    };
    let label = gtk::Label::new(None);
    label.set_markup(&format!(
        "<span foreground=\"{state_color}\" weight=\"bold\">{symbol}</span>"
    ));
    card.pack_start(&label, false, false, 0);
    let mut tooltip = format!(
        "{harness} → {motor} · {}",
        if text("model").is_empty() {
            "?"
        } else {
            text("model")
        }
    );
    for field in ["effort", "tierSym"] {
        if !text(field).is_empty() {
            tooltip.push_str(&format!(" · {}", text(field)));
        }
    }
    let ha = text("hAcct");
    let ma = text("mAcct");
    if !ha.is_empty() && ha != "unknown" {
        tooltip.push_str(&format!(
            " · cuenta {ha}{}",
            if ma.is_empty() || ma == "unknown" || ma == ha {
                String::new()
            } else {
                format!("→{ma}")
            }
        ));
    }
    if !text("pane").is_empty() {
        tooltip.push_str(&format!(" · pane {}", text("pane")));
    }
    tooltip.push_str(match state {
        "verified" => " · verificado en el proceso vivo",
        "changing" => " · cambio de motor en curso",
        "detecting" => " · detectando, aun sin confirmar",
        _ => "",
    });
    card.set_tooltip_text(Some(&tooltip));
    card
}
