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
    pub reader_layout: RefCell<Option<glib::SourceId>>,
    pub reader_signals: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
    pub mosaic_loading: Cell<bool>,
    pub debug_loading: Cell<bool>,
    pub zoom_closing: Cell<bool>,
    pub cells: RefCell<Vec<(String, gtk::Box)>>,
    pub cell_taps: RefCell<BTreeMap<String, glib::SourceId>>,
    pub cell_sizes: RefCell<BTreeMap<String, (i32, i32)>>,
    pub cell_grown: RefCell<BTreeMap<String, bool>>,
    pub state: RefCell<SideState>,
    /// The left panel: the dashboard WebKit view, full height.
    pub left: gtk::Box,
    pub column: gtk::Box,
    pub reader_paned: gtk::Paned,
    pub reader_host: gtk::Box,
    pub reader: RefCell<Option<ui::webview::OwnedPage>>,
    pub reader_open: Cell<bool>,
    pub reader_terminal: Cell<bool>,
    pub reader_scope: ui::snippets::Scope,
    pub clients: RefCell<OwnedViews<Client>>,
    pub focused: RefCell<Option<String>>,
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
    pub pane_save: RefCell<Option<glib::SourceId>>,
    pub layout_scope: ui::snippets::Scope,
    pub pane_scope: ui::snippets::Scope,
    pub fx: RefCell<Option<LeftFx>>,
    pub snapshot: RefCell<Option<Snapshot>>,
    pub signals: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
    pub sources: RefCell<Vec<glib::SourceId>>,
}
impl Default for Owned {
    fn default() -> Self {
        let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
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
            reader_layout: RefCell::new(None),
            reader_signals: RefCell::new(vec![]),
            mosaic_loading: Cell::new(false),
            debug_loading: Cell::new(false),
            zoom_closing: Cell::new(false),
            cells: RefCell::new(vec![]),
            cell_taps: RefCell::new(BTreeMap::new()),
            cell_sizes: RefCell::new(BTreeMap::new()),
            cell_grown: RefCell::new(BTreeMap::new()),
            state: RefCell::new(SideState::default()),
            left,
            column,
            reader_paned,
            reader_host,
            reader: RefCell::new(None),
            reader_open: Cell::new(false),
            reader_terminal: Cell::new(false),
            reader_scope: ui::snippets::Scope::default(),
            clients: RefCell::new(OwnedViews::new(53)),
            focused: RefCell::new(None),
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
            pane_save: RefCell::new(None),
            layout_scope: ui::snippets::Scope::default(),
            pane_scope: ui::snippets::Scope::default(),
            fx: RefCell::new(None),
            snapshot: RefCell::new(None),
            signals: RefCell::new(vec![]),
            sources: RefCell::new(vec![]),
        }
    }
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
        let widget = self
            .t18
            .clients
            .borrow()
            .get(id)
            .map(|c| c.overlay.widget.clone());
        if let Some(widget) = widget
            && let Some(parent) = widget
                .parent()
                .and_then(|p| p.downcast::<gtk::Container>().ok())
        {
            parent.remove(&widget);
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
        self.t18.mosaic_scope.close();
        self.t18.reader_scope.close();
        self.t18.layout_scope.close();
        self.t18.pane_scope.close();
        for slot in [
            &self.t18.fill,
            &self.t18.layout_save,
            &self.t18.pane_save,
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
        if let Some(source) = self.t18.reader_layout.borrow_mut().take() {
            source.remove();
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
            control_argv: None,
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
        let instance = term.cleanup_cancellation();
        term.widget().connect_focus_in_event(move |_, _| {
            if let Some(app) = weak.upgrade()
                && !app.closed.load(Ordering::Acquire)
                && !instance.load(Ordering::Acquire)
            {
                app.interacted.set(true);
                *app.t18.focused.borrow_mut() = Some(view.clone());
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
}
