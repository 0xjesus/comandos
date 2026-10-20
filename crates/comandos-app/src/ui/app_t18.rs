//! T18 owns auxiliary clients and views while App keeps workspace/session selection.
use super::*;
use ui::{
    mosaic::{ManagedView, MosaicState, OwnedViews},
    side::SideState,
};
pub(super) struct Client {
    pub session: String,
    pub term: TermView,
    pub overlay: Rc<ui::overlays::PaneOverlay>,
    pub kind: &'static str,
}
impl ManagedView for Client {
    fn shutdown(&self) {
        self.overlay.cancel();
        self.term.shutdown();
    }
}
pub(super) struct WindowView {
    pub window: gtk::Window,
    pub page: Option<ui::webview::OwnedPage>,
    pub client: Option<String>,
    pub signals: Vec<(glib::Object, glib::SignalHandlerId)>,
    pub closing: Rc<Cell<bool>>,
    pub scope: Rc<ui::snippets::Scope>,
}
impl Drop for WindowView {
    fn drop(&mut self) {
        self.scope.close();
        if let Some(page) = &self.page {
            page.cancel();
        }
        for (object, id) in self.signals.drain(..) {
            object.disconnect(id);
        }
        if !self.closing.replace(true) {
            self.window.close();
        }
    }
}
pub(super) struct LeftFx {
    pub area: gtk::DrawingArea,
    pub tick: Option<gtk::TickCallbackId>,
}
impl Drop for LeftFx {
    fn drop(&mut self) {
        if let Some(tick) = self.tick.take() {
            tick.remove();
        }
        if let Some(parent) = self
            .area
            .parent()
            .and_then(|p| p.downcast::<gtk::Container>().ok())
        {
            parent.remove(&self.area);
        }
    }
}
type Snapshot = (gdk_pixbuf::Pixbuf, i32, i32, i32, i32);
pub(super) struct Owned {
    pub relay: RefCell<Option<ui::signals::SignalRelay>>,
    pub requests: RefCell<BTreeMap<&'static str, Rc<ui::snippets::Scope>>>,
    pub reader_layout: RefCell<Option<glib::SourceId>>,
    pub reader_signals: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
    pub reveal: RefCell<Option<glib::SourceId>>,
    pub mosaic_loading: Cell<bool>,
    pub side_loading: Cell<bool>,
    pub debug_loading: Cell<bool>,
    pub zoom_closing: Cell<bool>,
    pub cells: RefCell<Vec<(String, gtk::Box)>>,
    pub cell_taps: RefCell<BTreeMap<String, glib::SourceId>>,
    pub cell_sizes: RefCell<BTreeMap<String, (i32, i32)>>,
    pub cell_grown: RefCell<BTreeMap<String, bool>>,
    pub state: RefCell<SideState>,
    pub side: gtk::Paned,
    pub host: gtk::Box,
    pub head_event: gtk::EventBox,
    pub head: gtk::Box,
    pub tabs: gtk::Box,
    pub tabs_scroll: gtk::ScrolledWindow,
    pub previous: gtk::Button,
    pub next: gtk::Button,
    pub toggle: gtk::Button,
    pub column: gtk::Box,
    pub reader_paned: gtk::Paned,
    pub reader_host: gtk::Box,
    pub reader: RefCell<Option<ui::webview::OwnedPage>>,
    pub reader_open: Cell<bool>,
    pub reader_terminal: Cell<bool>,
    pub reader_scope: ui::snippets::Scope,
    pub clients: RefCell<OwnedViews<Client>>,
    pub focused: RefCell<Option<String>>,
    pub side_current: RefCell<Option<String>>,
    pub side_scope: ui::snippets::Scope,
    pub mosaic: RefCell<MosaicState>,
    pub mosaic_box: RefCell<Option<gtk::Box>>,
    pub mosaic_scope: ui::snippets::Scope,
    pub mosaic_position: Cell<i32>,
    pub fill: RefCell<Option<glib::SourceId>>,
    pub cell_ticks: RefCell<BTreeMap<String, gtk::TickCallbackId>>,
    pub windows: RefCell<BTreeMap<String, WindowView>>,
    pub web_tabs: RefCell<BTreeMap<String, ui::webview::OwnedPage>>,
    pub layout_busy: Cell<bool>,
    pub pane_busy: Cell<bool>,
    pub scope: ui::snippets::Scope,
    pub layout_doc: RefCell<Value>,
    pub pane_doc: RefCell<Value>,
    pub layout_save: RefCell<Option<glib::SourceId>>,
    pub share_save: RefCell<Option<glib::SourceId>>,
    pub pane_save: RefCell<Option<glib::SourceId>>,
    pub layout_scope: ui::snippets::Scope,
    pub pane_scope: ui::snippets::Scope,
    pub drag: Cell<Option<(f64, i32)>>,
    pub queued: RefCell<Option<glib::SourceId>>,
    pub shown: RefCell<String>,
    pub fx: RefCell<Option<LeftFx>>,
    pub snapshot: RefCell<Option<Snapshot>>,
    pub signals: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
    pub sources: RefCell<Vec<glib::SourceId>>,
}
impl Default for Owned {
    fn default() -> Self {
        let side = gtk::Paned::new(gtk::Orientation::Vertical);
        side.set_wide_handle(false);
        let host = gtk::Box::new(gtk::Orientation::Vertical, 0);
        host.style_context().add_class("cc-side-term");
        host.set_no_show_all(true);
        host.hide();
        side.pack2(&host, true, false);
        let head_event = gtk::EventBox::new();
        head_event.style_context().add_class("side-head");
        head_event.add_events(
            gdk::EventMask::BUTTON_PRESS_MASK
                | gdk::EventMask::BUTTON_RELEASE_MASK
                | gdk::EventMask::POINTER_MOTION_MASK,
        );
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        head.set_size_request(-1, 40);
        head_event.add(&head);
        host.pack_start(&head_event, false, false, 0);
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let tabs_scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        tabs_scroll.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
        tabs_scroll.set_propagate_natural_height(true);
        tabs_scroll.add(&tabs);
        let previous = side_button("‹", &["side-arr"], "Terminales anteriores");
        let next = side_button("›", &["side-arr"], "Más terminales");
        for button in [&previous, &next] {
            button.set_valign(gtk::Align::Center);
            button.set_no_show_all(true);
        }
        let plus = side_button("+", &["side-plus"], "Nueva terminal");
        plus.set_widget_name("side-plus");
        plus.set_valign(gtk::Align::Center);
        let toggle = side_button("", &["side-tog"], "");
        toggle.set_always_show_image(true);
        toggle.set_no_show_all(true);
        head.pack_start(&previous, false, false, 0);
        head.pack_start(&tabs_scroll, true, true, 0);
        head.pack_start(&next, false, false, 0);
        head.pack_start(&plus, false, false, 0);
        head.pack_end(&toggle, false, false, 0);
        // The host deliberately skips window.show_all(); reveal its header
        // children now so controls remain available when the host is shown.
        head_event.show_all();
        let reader_paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        reader_paned.style_context().add_class("cc-paned");
        reader_paned.set_wide_handle(true);
        let reader_host = gtk::Box::new(gtk::Orientation::Vertical, 0);
        reader_host.set_no_show_all(true);
        reader_host.hide();
        reader_paned.pack2(&reader_host, true, false);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.pack_start(&reader_paned, true, true, 0);
        Self {
            relay: RefCell::new(None),
            requests: RefCell::new(BTreeMap::new()),
            reader_layout: RefCell::new(None),
            reader_signals: RefCell::new(vec![]),
            reveal: RefCell::new(None),
            mosaic_loading: Cell::new(false),
            side_loading: Cell::new(false),
            debug_loading: Cell::new(false),
            zoom_closing: Cell::new(false),
            cells: RefCell::new(vec![]),
            cell_taps: RefCell::new(BTreeMap::new()),
            cell_sizes: RefCell::new(BTreeMap::new()),
            cell_grown: RefCell::new(BTreeMap::new()),
            state: RefCell::new(SideState::default()),
            side,
            host,
            head_event,
            head,
            tabs,
            tabs_scroll,
            previous,
            next,
            toggle,
            column,
            reader_paned,
            reader_host,
            reader: RefCell::new(None),
            reader_open: Cell::new(false),
            reader_terminal: Cell::new(false),
            reader_scope: ui::snippets::Scope::default(),
            clients: RefCell::new(OwnedViews::new(53)),
            focused: RefCell::new(None),
            side_current: RefCell::new(None),
            side_scope: ui::snippets::Scope::default(),
            mosaic: RefCell::new(MosaicState::default()),
            mosaic_box: RefCell::new(None),
            mosaic_scope: ui::snippets::Scope::default(),
            mosaic_position: Cell::new(700),
            fill: RefCell::new(None),
            cell_ticks: RefCell::new(BTreeMap::new()),
            windows: RefCell::new(BTreeMap::new()),
            web_tabs: RefCell::new(BTreeMap::new()),
            layout_busy: Cell::new(false),
            pane_busy: Cell::new(false),
            scope: ui::snippets::Scope::default(),
            layout_doc: RefCell::new(Value::Null),
            pane_doc: RefCell::new(Value::Null),
            layout_save: RefCell::new(None),
            share_save: RefCell::new(None),
            pane_save: RefCell::new(None),
            layout_scope: ui::snippets::Scope::default(),
            pane_scope: ui::snippets::Scope::default(),
            drag: Cell::new(None),
            queued: RefCell::new(None),
            shown: RefCell::new(String::new()),
            fx: RefCell::new(None),
            snapshot: RefCell::new(None),
            signals: RefCell::new(vec![]),
            sources: RefCell::new(vec![]),
        }
    }
}
pub(super) fn side_button(label: &str, classes: &[&str], tip: &str) -> gtk::Button {
    let button = if label.is_empty() {
        gtk::Button::new()
    } else {
        gtk::Button::with_label(label)
    };
    button.set_relief(gtk::ReliefStyle::None);
    button.set_can_focus(false);
    for name in std::iter::once("side-btn").chain(classes.iter().copied()) {
        button.style_context().add_class(name);
    }
    button.set_tooltip_text(Some(tip));
    button.connect_realize(|b| {
        if let Some(window) = b.event_window() {
            window.set_cursor(gdk::Cursor::from_name(&b.display(), "pointer").as_ref());
        }
    });
    button
}
impl App {
    pub(super) fn action_session(&self) -> Option<String> {
        self.t18
            .focused
            .borrow()
            .as_deref()
            .and_then(|id| {
                self.t18
                    .clients
                    .borrow()
                    .get(id)
                    .filter(|c| {
                        c.overlay.widget.is_mapped()
                            && !c.term.cleanup_cancellation().load(Ordering::Acquire)
                    })
                    .map(|c| c.session.clone())
            })
            .or_else(|| self.current_session())
    }
    pub(super) fn term_for_action(&self, key: &str) -> Option<TermView> {
        if let Some(view) = self.t18.focused.borrow().as_deref()
            && let Some(client) = self.t18.clients.borrow().get(view)
            && client.session == key
            && client.overlay.widget.is_mapped()
            && !client.term.cleanup_cancellation().load(Ordering::Acquire)
        {
            return Some(client.term.clone());
        }
        self.terms.borrow().get(key).cloned()
    }
    pub(super) fn owns_term_instance(&self, session: &str, instance: &Arc<AtomicBool>) -> bool {
        !self.closed.load(Ordering::Acquire)
            && !instance.load(Ordering::Acquire)
            && (self
                .terms
                .borrow()
                .get(session)
                .is_some_and(|t| Arc::ptr_eq(&t.cleanup_cancellation(), instance))
                || self.t18.clients.borrow().values().any(|c| {
                    c.session == session && Arc::ptr_eq(&c.term.cleanup_cancellation(), instance)
                }))
    }
    pub(super) fn auxiliary_current(&self, owner: &ui::overlays::PaneOverlay) -> bool {
        self.t18.clients.borrow().values().any(|c| {
            Arc::ptr_eq(
                &c.term.cleanup_cancellation(),
                &owner.term.cleanup_cancellation(),
            ) && c.session == owner.session
        })
    }
    pub(super) fn auxiliary_visible(&self, owner: &ui::overlays::PaneOverlay) -> bool {
        self.auxiliary_current(owner) && owner.widget.is_mapped()
    }
    pub(super) fn auxiliary_focused(&self, owner: &ui::overlays::PaneOverlay) -> bool {
        self.t18
            .focused
            .borrow()
            .as_deref()
            .and_then(|id| {
                self.t18.clients.borrow().get(id).map(|c| {
                    Arc::ptr_eq(
                        &c.term.cleanup_cancellation(),
                        &owner.term.cleanup_cancellation(),
                    )
                })
            })
            .unwrap_or(false)
    }
    pub(super) fn close_aux(&self, id: &str) {
        let client = self
            .t18
            .clients
            .borrow()
            .get(id)
            .map(|c| (c.overlay.widget.clone(), c.kind));
        if let Some((widget, kind)) = client {
            if let Some(parent) = widget
                .parent()
                .and_then(|p| p.downcast::<gtk::Container>().ok())
            {
                parent.remove(&widget);
            }
            if kind == "side" && self.t18.side_current.borrow().as_deref() == Some(id) {
                self.t18.side_current.borrow_mut().take();
                self.t18.state.borrow_mut().collapsed = true;
            }
        }
        self.t17.boxes.borrow_mut().remove(id);
        self.t18.clients.borrow_mut().remove(id);
        if self.t18.focused.borrow().as_deref() == Some(id) {
            self.t18.focused.borrow_mut().take();
        }
    }
    pub(super) fn close_window(&self, name: &str) {
        if name == "zoom" {
            self.t18.zoom_closing.set(false);
            self.t18.mosaic.borrow_mut().unzoom();
        }
        if let Some(scope) = self.t18.requests.borrow_mut().remove(name) {
            scope.close();
        }
        if let Some(tick) = self
            .t18
            .cell_ticks
            .borrow_mut()
            .remove(&format!("window:{name}"))
        {
            tick.remove();
        }
        let old = self.t18.windows.borrow_mut().remove(name);
        if let Some(window) = old
            && let Some(client) = &window.client
        {
            self.close_aux(client);
        }
    }
    pub(super) fn shutdown_t18(&self) {
        self.t18.scope.close();
        self.t18.side_scope.close();
        self.t18.mosaic_scope.close();
        self.t18.reader_scope.close();
        self.t18.layout_scope.close();
        self.t18.pane_scope.close();
        for slot in [
            &self.t18.fill,
            &self.t18.layout_save,
            &self.t18.share_save,
            &self.t18.pane_save,
            &self.t18.queued,
        ] {
            if let Some(id) = slot.borrow_mut().take() {
                id.remove();
            }
        }
        for (_, tick) in std::mem::take(&mut *self.t18.cell_ticks.borrow_mut()) {
            tick.remove();
        }
        let keys = self
            .t18
            .clients
            .borrow()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            self.close_aux(&key);
        }
        self.t18.windows.borrow_mut().clear();
        self.t18.web_tabs.borrow_mut().clear();
        self.t18.reader.borrow_mut().take();
        self.t18.fx.borrow_mut().take();
        self.t18.snapshot.borrow_mut().take();
        for scope in self.t18.requests.borrow().values() {
            scope.close();
        }
        for slot in [&self.t18.reveal, &self.t18.reader_layout] {
            if let Some(source) = slot.borrow_mut().take() {
                source.remove();
            }
        }
        for (_, source) in std::mem::take(&mut *self.t18.cell_taps.borrow_mut()) {
            source.remove();
        }
        for (object, id) in self.t18.reader_signals.borrow_mut().drain(..) {
            object.disconnect(id);
        }
        for source in self.t18.sources.borrow_mut().drain(..) {
            source.remove();
        }
        for (object, id) in self.t18.signals.borrow_mut().drain(..) {
            object.disconnect(id);
        }
        self.t18.relay.borrow_mut().take();
    }
    pub(super) fn aux_terminal(
        self: &Rc<Self>,
        session: &str,
        id: &str,
        kind: &'static str,
        scrollback: usize,
    ) -> Result<Rc<ui::overlays::PaneOverlay>, String> {
        if self.closed.load(Ordering::Acquire) || !crate::tab_actions::valid_session(session) {
            return Err("Auxiliary session unavailable".into());
        }
        let home = self.private_home();
        let sandbox = self.cfg.mode() == RunMode::Sandbox;
        let palette = self
            .applied_theme
            .borrow()
            .as_ref()
            .map(theme_palette)
            .ok_or("Theme unavailable")?;
        let term = TermView::new(TermOptions {
            argv: if kind == "mosaic" {
                self.tmux.viewer_attach_argv(session)
            } else {
                self.tmux.attach_argv(session)
            },
            cwd: home.clone(),
            session: Some(session.into()),
            palette,
            scrollback,
            preferences: {
                let mut prefs = self.preferences.borrow().clone();
                if kind == "mosaic" {
                    if !prefs.is_object() {
                        prefs = json!({});
                    }
                    if let Some(map) = prefs.as_object_mut() {
                        map.insert("cursor_blink".into(), json!(false));
                    }
                }
                prefs
            },
            mode: self.cfg.mode(),
            clear_env: sandbox,
            environment: if sandbox {
                vec![
                    ("HOME".into(), home.display().to_string()),
                    ("PATH".into(), "/usr/bin:/bin".into()),
                    ("SHELL".into(), "/bin/sh".into()),
                    ("TERM".into(), "xterm-256color".into()),
                ]
            } else {
                vec![]
            },
            tmux_size: None,
            shadow_tmux: (self.cfg.mode() == RunMode::Shadow).then(|| self.tmux.clone()),
            before_spawn: None,
            on_title: None,
            on_exit: None,
            on_bell: None,
            on_link: None,
            on_ssh_scroll: None,
        })
        .map_err(|e| format!("{e:?}"))?;
        term.set_respawn(kind != "mosaic");
        self.attach_term_keys(session, &term);
        self.attach_term_t16(session, &term);
        let overlay = if kind == "mosaic" {
            ui::overlays::PaneOverlay::new(session, &term)
        } else {
            self.attach_pane_overlay_at(session, &term, id)
        };
        self.t18.clients.borrow_mut().insert(
            id.into(),
            Client {
                session: session.into(),
                term: term.clone(),
                overlay: overlay.clone(),
                kind,
            },
        )?;
        let weak = Rc::downgrade(self);
        let view = id.to_string();
        let key = session.to_string();
        let instance = term.cleanup_cancellation();
        term.widget().connect_focus_in_event(move |_, _| {
            if let Some(app) = weak.upgrade()
                && !app.closed.load(Ordering::Acquire)
                && !instance.load(Ordering::Acquire)
            {
                app.interacted.set(true);
                *app.t18.focused.borrow_mut() = Some(view.clone());
                app.side_focus_js(&key, kind == "side");
            }
            glib::Propagation::Proceed
        });
        Ok(overlay)
    }
    pub(super) fn quiet_js(&self, code: &str) {
        use webkit2gtk::WebViewExt;
        if self.writable() {
            self.webview.evaluate_javascript(
                &format!("try{{{code}}}catch(e){{}}"),
                None,
                None,
                Some(&self.protocol_cancellable),
                |_| {},
            );
        }
    }
    fn side_focus_js(&self, session: &str, side: bool) {
        self.quiet_js(&format!(
            "window.sidebarTermFocused&&sidebarTermFocused({})",
            json!(if side { session } else { "" })
        ));
    }
}
