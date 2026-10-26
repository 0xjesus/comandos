use crate::{
    config::{self, AppConfig, RunMode},
    dash_client::DashClient,
    guard::WriteGuard,
    ipc::{self, IpcConsumer, IpcKind, IpcRequest},
    jobs::Jobs,
    poll::{PollUpdate, Poller},
    restore::{self, RestoreCoordinator, RestoreResult},
    snapshot,
    state_files::StateFiles,
    tabs::{TabKind, TabRecord, TabRegistry},
    term::{
        engine::Palette,
        view::{TermOptions, TermView},
    },
    tmux::TmuxCtl,
    ui,
};
use gtk::prelude::*;
use nix::fcntl::Flock;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    process::ExitCode,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

type Handler = ui::app_commands::Handler;
pub struct App {
    pub cfg: AppConfig,
    pub tmux: TmuxCtl,
    pub guard: WriteGuard,
    pub jobs: Jobs,
    pub dash: DashClient,
    pub window: gtk::ApplicationWindow,
    english: bool,
    t16: app_t16::Owned,
    chat: app_chat::Owned,
    t17: app_t17::Owned,
    t18: app_t18::Owned,
    modal_overlay: gtk::Overlay,
    modal: RefCell<Option<app_t15::Modal>>,
    modal_generation: Cell<u64>,
    modal_idle: RefCell<Option<glib::SourceId>>,
    cleanup: Arc<dyn crate::agent_stop::CleanupService>,
    pub paned: gtk::Paned,
    pub notebook: gtk::Notebook,
    pub webview: webkit2gtk::WebView,
    handlers: ui::app_commands::Registry,
    protocol_signals: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
    protocol_cancellable: gio::Cancellable,
    interaction_controllers: RefCell<Vec<gtk::EventController>>,
    presence: RefCell<ui::presence::Presence>,
    presence_visible: Cell<bool>,
    bridge_queue: RefCell<ui::bridge::BridgeQueue>,
    mru_pages: RefCell<Vec<String>>,
    state: StateFiles,
    registry: RefCell<TabRegistry>,
    strip: RefCell<ui::tabstrip::TabStripNotebook>,
    terms: RefCell<BTreeMap<String, TermView>>,
    labels: RefCell<BTreeMap<String, ui::tab_label::TabLabel>>,
    group_labels: RefCell<BTreeMap<String, ui::tab_label::TabLabel>>,
    tab_layout: ui::tabstrip::TabStripLayout,
    drag_layer: ui::drag::DragLayer,
    previous_order: RefCell<Option<Vec<String>>>,
    state_items: RefCell<BTreeMap<String, Value>>,
    marks: RefCell<BTreeMap<String, Value>>,
    work_marks: RefCell<ui::marks::Marks>,
    marks_fetching: Cell<bool>,
    header: ui::header::Header,
    popovers: RefCell<BTreeMap<String, (gtk::Popover, webkit2gtk::WebView)>>,
    favorite_pending: RefCell<BTreeMap<String, bool>>,
    favorite_queue: RefCell<std::collections::VecDeque<String>>,
    favorite_posting: Cell<bool>,
    quick: Arc<crate::tab_actions::QuickTerminal>,
    quick_busy: Cell<bool>,
    focus_ready: Cell<bool>,
    focus_restoring: Cell<bool>,
    interacted: Cell<bool>,
    saved_focus: RefCell<Option<String>>,
    focus_retry: RefCell<ui::presence::FocusRetry>,
    pane_initialized: Cell<bool>,
    pane_saved: Cell<Option<i32>>,
    theme_layers: RefCell<Option<ui::theme_layers::ThemeLayers>>,
    applied_theme: RefCell<Option<crate::theme::ThemeTokens>>,
    applied_button_style: RefCell<Option<String>>,
    toolbar: gtk::Box,
    workspace: ui::workspace::GtkWorkspace,
    restore: RefCell<RestoreCoordinator>,
    startup_valid: Cell<bool>,
    prefs_received: Cell<bool>,
    state_received: Cell<bool>,
    marks_received: Cell<bool>,
    dashboard_observation: RefCell<Option<Rc<RefCell<ui::webview::LoadObservation>>>>,
    workspace_doc: RefCell<Value>,
    pending_workspace: RefCell<Option<Value>>,
    resize_queue: RefCell<crate::workspace_resize::ResizeQueue>,
    revision: Cell<u64>,
    posting: Cell<bool>,
    dragging: Cell<bool>,
    preferences: RefCell<Value>,
    generation: Arc<AtomicU64>,
    snapshot_busy: Arc<AtomicBool>,
    state_epoch: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
    ipc_pending: RefCell<BTreeSet<std::path::PathBuf>>,
    ipc_reading: Cell<bool>,
    ipc_seen: RefCell<Vec<IpcRequest>>,
    status: gtk::Label,
}
struct AppRuntime {
    app: Rc<App>,
    poller: Poller,
    sources: Vec<glib::SourceId>,
    monitors: Vec<gio::FileMonitor>,
    _lock: Flock<std::fs::File>,
}
impl AppRuntime {
    fn shutdown(self) {
        self.app.closed.store(true, Ordering::Release);
        self.app.shutdown_protocol();
        self.app.shutdown_header();
        self.app.shutdown_t16();
        self.app.shutdown_t18();
        self.app.shutdown_t17();
        self.app.close_modal();
        for source in self.sources {
            source.remove();
        }
        for monitor in self.monitors {
            monitor.cancel();
        }
        self.poller.stop();
        self.app.dash.cancel_pending();
        self.app.cancel_drag();
        self.app.workspace.shutdown();
        self.app.resize_queue.borrow_mut().cancel();
        self.app.theme_layers.borrow_mut().take();
        self.app.jobs.shutdown(Duration::from_secs(6));
        for term in self.app.terms.borrow().values() {
            term.shutdown();
        }
    }
}
impl App {
    fn install_foundation_handlers(self: &Rc<Self>) {
        let themes = crate::theme::themes_from_file(Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/themes.json"
        ))));
        if let Some(theme) = crate::theme::desktop_theme(crate::theme::DEFAULT_THEME, &themes)
            && let Err(error) = self.apply_theme_layers(&theme, "sutil")
        {
            self.status.set_text(&format!("Tema: {error}"));
        }
        let plus = ui::icons::button("plus", 16, "Nueva sesión");
        let weak = Rc::downgrade(self);
        plus.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade()
                && let Err(error) = app.handlers.invoke("T14.wizard", &json!({}))
            {
                app.status.set_text(&error.to_string());
            }
        });
        let weak = Rc::downgrade(self);
        plus.connect_button_press_event(move |button, event| {
            if event.button() == 3
                && let Some(app) = weak.upgrade().filter(|a| a.writable())
            {
                app.new_tab_menu(button);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        plus.style_context().add_class("cc-key");
        plus.style_context().add_class("cc-key-plus");
        self.tab_layout.actions().pack_start(&plus, false, false, 0);
        // Quick terminal and sort buttons are retired (11-oct): «+» is the only way to
        // create, and its dialog has «Solo terminal».
        let rows = ui::icons::button("rows", 18, "Varias filas");
        rows.set_widget_name("tabs-layout-rows");
        rows.style_context().add_class("cc-key");
        rows.style_context().add_class("cc-key-rows");
        let weak = Rc::downgrade(self);
        rows.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade().filter(|a| a.writable()) {
                let mode = if app.tab_layout.rows() { "row" } else { "rows" };
                app.apply_tabs_layout(mode, true);
            }
        });
        self.tab_layout.actions().pack_start(&rows, false, false, 0);
        let left = ui::icons::button("panel-left", 18, "Mostrar/ocultar el panel izquierdo");
        left.style_context().add_class("tabnav");
        left.set_widget_name("left-panel-toggle");
        let weak = Rc::downgrade(self);
        left.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                let hidden = !app.t18.state.borrow().left_hidden;
                app.left_panel_set(hidden);
            }
        });
        self.tab_layout.start().pack_start(&left, false, false, 0);
        for (glyph, delta) in [("‹", -1), ("›", 1)] {
            let button = ui::icons::button(
                if delta < 0 {
                    "chevron-left"
                } else {
                    "chevron-right"
                },
                20,
                glyph,
            );
            button.set_widget_name(if delta < 0 {
                "tab-cycle-prev"
            } else {
                "tab-cycle-next"
            });
            button.style_context().add_class("tab-cycle");
            button.set_no_show_all(true);
            button.show();
            button.style_context().add_class("tabnav");
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.cycle_navigation_page(delta);
                }
            });
            button.set_size_request(36, 30);
            self.tab_layout
                .navigation()
                .pack_start(&button, false, false, 0);
        }
        self.tab_layout.widget().show_all();
        let weak = Rc::downgrade(self);
        self.workspace.on_header(Rc::new(move |key| {
            let header = gtk::EventBox::new();
            header.style_context().add_class("ws-leaf-head");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let grip = gtk::Label::new(Some("⠿"));
            grip.style_context().add_class("dim-label");
            row.pack_start(&grip, false, false, 0);
            let label = weak
                .upgrade()
                .and_then(|a| {
                    a.labels
                        .borrow()
                        .get(key)
                        .map(|tab| tab.text.text().to_string())
                })
                .unwrap_or_else(|| key.into());
            let name = gtk::Label::new(Some(&label));
            name.set_xalign(0.);
            name.set_ellipsize(pango::EllipsizeMode::End);
            row.pack_start(&name, true, true, 0);
            header.add(&row);
            if let Some(app) = weak.upgrade() {
                app.install_drag(key, header.upcast_ref());
                let w = Rc::downgrade(&app);
                let key_for_focus = key.to_string();
                header.connect_button_press_event(move |_, _| {
                    if let Some(app) = w.upgrade() {
                        app.select(&key_for_focus);
                    }
                    glib::Propagation::Proceed
                });
            }
            header.upcast()
        }));
        self.install_pane_position();
        self.window.add_events(
            gdk::EventMask::POINTER_MOTION_MASK
                | gdk::EventMask::BUTTON_RELEASE_MASK
                | gdk::EventMask::KEY_PRESS_MASK,
        );
        let weak = Rc::downgrade(self);
        self.window.connect_motion_notify_event(move |_, e| {
            if let Some(app) = weak.upgrade()
                && app.drag_layer.active()
            {
                app.drag_motion(e.root());
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        let weak = Rc::downgrade(self);
        self.window.connect_button_release_event(move |_, e| {
            if e.button() == 1
                && let Some(app) = weak.upgrade()
                && app.drag_layer.active()
            {
                app.drop_drag(app.drag_target(app.layer_point(e.root())));
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        let weak = Rc::downgrade(self);
        self.window.connect_key_press_event(move |_, e| {
            if e.keyval() == gdk::keys::constants::Escape
                && let Some(app) = weak.upgrade()
                && app.drag_layer.active()
            {
                app.cancel_drag();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }

    fn new_terminal(self: &Rc<Self>) {
        self.new_terminal_in(false);
    }
    fn new_terminal_in(self: &Rc<Self>, fresh: bool) {
        if !self.writable() {
            return;
        }
        let mut bytes = [0u8; 8];
        if getrandom::fill(&mut bytes).is_err() {
            return;
        }
        let key = format!(
            "term-{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let backend = restore::CancelableTmux {
            tmux: self.tmux.clone(),
            cancelled: self.closed.clone(),
        };
        let home = self.private_home();
        let guard = self.guard.clone();
        let selected = self.current_session();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || {
                let cwd = if fresh {
                    let now: chrono::DateTime<chrono::Utc> = std::time::SystemTime::now().into();
                    guard
                        .reserve_quick_directory(
                            &home.join("codebase/0xJesus/Terminal"),
                            now.fixed_offset(),
                        )
                        .map_err(|e| format!("{e:?}"))?
                        .display()
                        .to_string()
                } else {
                    selected
                        .as_deref()
                        .and_then(|session| {
                            backend
                                .tmux
                                .read(&[
                                    "display-message",
                                    "-p",
                                    "-t",
                                    &format!("={session}:"),
                                    "#{pane_current_path}",
                                ])
                                .ok()
                        })
                        .filter(|out| out.ok())
                        .map(|out| out.stdout.trim().to_string())
                        .unwrap_or_else(|| home.display().to_string())
                };
                let label = std::path::Path::new(&cwd)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Terminal");
                let plan = restore::RestorePlan::build(
                    &json!({key.clone():label}),
                    &json!({key:{"cwd":cwd,"agent":""}}),
                    &BTreeSet::new(),
                    backend.tmux.mode(),
                )
                .map_err(|e| format!("{e:?}"))?;
                Ok::<_, String>(restore::execute(&plan, &backend, &SandboxScopes, &home))
            },
            move |result| {
                if let Some(app) = weak.upgrade().filter(|a| a.writable()) {
                    match result {
                        Ok(result) => {
                            for tab in result.tabs {
                                app.add_tab(
                                    &tab.key,
                                    &tab.label,
                                    tab.attached,
                                    tab.error.as_deref(),
                                );
                                app.select(&tab.key);
                            }
                            app.persist();
                        }
                        Err(e) => app.status.set_text(&e),
                    }
                }
            },
        );
    }

    fn reorder_tab(self: &Rc<Self>, key: &str, index: usize) {
        if !self.writable() {
            return;
        }
        if !self.workspace_doc.borrow().is_null() {
            let changed = comandos_core::workspace::layout::move_tab_group(
                &self.workspace_doc.borrow(),
                key,
                &json!(index),
            );
            if let Ok(Some(doc)) = changed {
                self.commit_workspace(doc, Some(key.into()));
            }
        } else {
            self.registry.borrow_mut().reorder(key, index);
            self.strip.borrow().reorder(key, index as u32);
            if let Some(label) = self.labels.borrow().get(key) {
                self.toolbar.reorder_child(&label.item, index as i32);
            }
            self.persist();
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cfg: AppConfig,
        tmux: TmuxCtl,
        guard: WriteGuard,
        jobs: Jobs,
        dash: DashClient,
        window: gtk::ApplicationWindow,
        paned: gtk::Paned,
        notebook: gtk::Notebook,
        webview: webkit2gtk::WebView,
    ) -> Rc<Self> {
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let tab_layout = ui::tabstrip::TabStripLayout::new(&toolbar);
        let drag_layer = ui::drag::DragLayer::new();
        let status = gtk::Label::new(None);
        status.set_no_show_all(true);
        status.connect_label_notify(|label| label.set_visible(!label.text().is_empty()));
        let workspace = ui::workspace::GtkWorkspace::new();
        let state = StateFiles::new(cfg.clone(), guard.clone());
        let strip = RefCell::new(ui::tabstrip::TabStripNotebook::new(notebook.clone()));
        let english =
            config::ui_lang(cfg.hooks_dir(), std::env::var("LANG").ok().as_deref()) == "en";
        let closed = Arc::new(AtomicBool::new(false));
        let t16 = app_t16::Owned::new(cfg.mode(), closed.clone());
        Rc::new(Self {
            english,
            t16,
            chat: app_chat::Owned::default(),
            t17: app_t17::Owned::default(),
            t18: app_t18::Owned::default(),
            modal_overlay: gtk::Overlay::new(),
            modal: RefCell::new(None),
            modal_generation: Cell::new(0),
            modal_idle: RefCell::new(None),
            cleanup: Arc::new(crate::agent_stop::NativeCleanup),
            cfg,
            tmux,
            guard,
            jobs,
            dash,
            window: window.clone(),
            paned,
            notebook,
            webview,
            toolbar,
            status,
            workspace,
            state,
            strip,
            registry: RefCell::new(
                TabRegistry::from_json(&json!({})).unwrap_or_else(|_| unreachable!()),
            ),
            handlers: ui::app_commands::Registry::default(),
            protocol_signals: RefCell::new(Vec::new()),
            protocol_cancellable: gio::Cancellable::new(),
            interaction_controllers: RefCell::new(Vec::new()),
            presence: RefCell::new(ui::presence::Presence::default()),
            presence_visible: Cell::new(true),
            bridge_queue: RefCell::new(ui::bridge::BridgeQueue::default()),
            mru_pages: RefCell::new(Vec::new()),
            terms: RefCell::new(BTreeMap::new()),
            labels: RefCell::new(BTreeMap::new()),
            group_labels: RefCell::new(BTreeMap::new()),
            tab_layout,
            drag_layer,
            previous_order: RefCell::new(None),
            state_items: RefCell::new(BTreeMap::new()),
            marks: RefCell::new(BTreeMap::new()),
            work_marks: RefCell::new(ui::marks::Marks::default()),
            marks_fetching: Cell::new(false),
            header: ui::header::Header::new(&window, english),
            popovers: RefCell::new(BTreeMap::new()),
            favorite_pending: RefCell::new(BTreeMap::new()),
            favorite_queue: RefCell::new(std::collections::VecDeque::new()),
            favorite_posting: Cell::new(false),
            quick: Arc::new(crate::tab_actions::QuickTerminal::default()),
            quick_busy: Cell::new(false),
            focus_ready: Cell::new(false),
            focus_restoring: Cell::new(false),
            interacted: Cell::new(false),
            saved_focus: RefCell::new(None),
            focus_retry: RefCell::new(ui::presence::FocusRetry::default()),
            pane_initialized: Cell::new(false),
            pane_saved: Cell::new(None),
            theme_layers: RefCell::new(None),
            applied_theme: RefCell::new(None),
            applied_button_style: RefCell::new(None),
            restore: RefCell::new(RestoreCoordinator::default()),
            startup_valid: Cell::new(false),
            prefs_received: Cell::new(false),
            state_received: Cell::new(false),
            marks_received: Cell::new(false),
            dashboard_observation: RefCell::new(None),
            workspace_doc: RefCell::new(Value::Null),
            pending_workspace: RefCell::new(None),
            resize_queue: RefCell::new(crate::workspace_resize::ResizeQueue::default()),
            revision: Cell::new(0),
            posting: Cell::new(false),
            dragging: Cell::new(false),
            preferences: RefCell::new(json!({})),
            generation: Arc::new(AtomicU64::new(0)),
            snapshot_busy: Arc::new(AtomicBool::new(false)),
            state_epoch: Arc::new(AtomicU64::new(0)),
            closed,
            ipc_pending: RefCell::new(BTreeSet::new()),
            ipc_reading: Cell::new(false),
            ipc_seen: RefCell::new(Vec::new()),
        })
    }
    pub fn install_handler(&self, name: &'static str, callback: Handler) {
        self.handlers.install(name, callback);
    }
    pub fn dispatch(self: &Rc<Self>, message: &Value) -> bool {
        if self.closed.load(Ordering::Acquire) || !self.restore.borrow().ready() {
            return false;
        }
        if let Err(error) = self.dispatch_message(message) {
            self.status.set_text(&error.to_string());
        }
        // A consumed command, including a reported error, must not be replayed by IPC.
        true
    }
    fn writable(&self) -> bool {
        self.cfg.writes_allowed()
            && self.startup_valid.get()
            && self.restore.borrow().ready()
            && !self.closed.load(Ordering::Acquire)
    }
    fn persist(&self) {
        if !self.writable() {
            return;
        }
        let epoch = self.state_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let current = self.state_epoch.clone();
        let state = self.state.clone();
        let value = self.registry.borrow().to_json();
        let closed = self.closed.clone();
        self.jobs.spawn(
            move || {
                if !closed.load(Ordering::Acquire) {
                    state
                        .write_tabs_when(&value, || {
                            !closed.load(Ordering::Acquire)
                                && current.load(Ordering::Acquire) == epoch
                        })
                        .map_err(|e| format!("{e:?}"))
                } else {
                    Ok(())
                }
            },
            |result| {
                if let Err(e) = result {
                    eprintln!("save tabs: {e}");
                }
            },
        );
    }
    fn startup(self: &Rc<Self>) {
        self.restore.borrow_mut().begin();
        let state = self.state.clone();
        let tmux = self.tmux.clone();
        let home = self.private_home();
        let mode = self.cfg.mode();
        let cancel = self.closed.clone();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || {
                let raw = state.read("app-tabs.json").map_err(|e| format!("{e:?}"))?;
                let mut tabs = if raw.is_null() {
                    json!({})
                } else {
                    TabRegistry::from_json(&raw)
                        .map_err(|e| format!("{e:?}"))?
                        .to_json()
                };
                if tabs.as_object().is_none_or(|o| o.is_empty()) {
                    tabs = json!({"local":"Local"});
                }
                let layouts = state.read_session_snapshot();
                let legacy = state
                    .read("app-tabs-snapshot.json")
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({}));
                let mut snapshots = legacy;
                if let (Some(dest), Some(src)) = (
                    snapshots.as_object_mut(),
                    layouts.get("sessions").unwrap_or(&Value::Null).as_object(),
                ) {
                    dest.extend(src.clone());
                }
                let present = tmux
                    .read(&["list-sessions", "-F", "#{session_name}"])
                    .ok()
                    .filter(|o| o.ok())
                    .map(|o| o.stdout.lines().map(ToString::to_string).collect())
                    .unwrap_or_default();
                let plan = restore::RestorePlan::build(&tabs, &snapshots, &present, mode)
                    .map_err(|e| format!("{e:?}"))?;
                let backend = restore::CancelableTmux {
                    tmux,
                    cancelled: cancel,
                };
                // Sandbox never launches agents, even if a copied fixture includes saved identities.
                let launcher: &dyn restore::ScopeLauncher = if mode == RunMode::Sandbox {
                    &SandboxScopes
                } else {
                    &restore::PaneScopes
                };
                let layout = if mode == RunMode::Sandbox {
                    state.read("app-layout.json").ok()
                } else {
                    None
                };
                let fixture = if mode == RunMode::Sandbox {
                    home.parent()
                        .and_then(|root| std::fs::read(root.join("dashboard-fixture.json")).ok())
                        .and_then(|bytes| comandos_core::json::workspace_loads_bytes(&bytes))
                } else {
                    None
                };
                Ok::<_, String>((
                    restore::execute(&plan, &backend, launcher, &home),
                    layout,
                    fixture,
                ))
            },
            move |result| {
                let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                    return;
                };
                match result {
                    Ok((result, layout, fixture)) => {
                        app.startup_valid.set(true);
                        for tab in result.tabs {
                            app.add_tab(
                                &tab.key,
                                &tab.label,
                                tab.attached && app.cfg.mode() != RunMode::Shadow,
                                tab.error
                                    .as_deref()
                                    .or_else(|| tab.ambiguity.first().map(String::as_str)),
                            );
                        }
                        app.restore.borrow_mut().finish(result.result);
                        if app.cfg.dash_url().is_none()
                            && let Some(layout) = layout
                        {
                            app.apply_workspace(&layout);
                        }
                        if app.cfg.mode() == RunMode::Sandbox
                            && app.cfg.dash_url().is_none()
                            && let Some(fixture) = fixture
                        {
                            for (key, kind) in [("prefs", 0), ("state", 1), ("marks", 2)] {
                                if let Some(value) = fixture.get(key) {
                                    app.poll(match kind {
                                        0 => PollUpdate::Prefs {
                                            value: value.clone(),
                                            favorite_generation: 0,
                                        },
                                        1 => PollUpdate::State(value.clone()),
                                        _ => PollUpdate::Marks(value.clone()),
                                    });
                                }
                            }
                        }
                    }
                    Err(error) => {
                        app.status.set_text(&error);
                        app.restore.borrow_mut().finish(RestoreResult::Failed);
                    }
                }
                let pending = app.pending_workspace.borrow_mut().take();
                if let Some(doc) = pending {
                    app.apply_workspace(&doc);
                }
                app.persist();
                app.drain_ipc();
            },
        );
    }
    fn private_home(&self) -> std::path::PathBuf {
        self.cfg
            .sandbox_root()
            .map(|r| r.join("home"))
            .unwrap_or_else(|| self.cfg.home().to_path_buf())
    }
    fn mount_web_terminal(self: &Rc<Self>, key: &str, slot: &gtk::Box) {
        if self.cfg.mode() != RunMode::Live {
            slot.add(&gtk::Label::new(Some(
                "Web terminal requires an explicitly configured live backend",
            )));
            return;
        }
        let Some(base) = self.cfg.dash_url().map(str::to_owned) else {
            slot.add(&gtk::Label::new(Some(
                "Native web terminal requires a configured dashboard URL",
            )));
            return;
        };
        let session = key.strip_prefix("xterm-").unwrap_or("local").to_string();
        if !ui::app_commands::valid_xterm_session(&session) {
            slot.add(&gtk::Label::new(Some("Invalid web terminal session")));
            return;
        }
        let cancelled = self.closed.clone();
        let dash = self.dash.clone();
        let weak_slot = slot.downgrade();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || {
                if cancelled.load(Ordering::Acquire) {
                    return Err("webterm request cancelled".into());
                }
                ui::webview::native_terminal_backend(RunMode::Live, &base)?;
                if cancelled.load(Ordering::Acquire) {
                    return Err("webterm request cancelled".into());
                }
                let token = dash
                    .get("/webterm-token", Duration::from_secs(2))
                    .map_err(|e| format!("{e:?}"))?;
                let token = token.get("token").and_then(Value::as_str).unwrap_or("");
                ui::webview::terminal_uri(&base, &session, token, "noche")
            },
            move |result| {
                let (Some(app), Some(slot)) = (weak.upgrade(), weak_slot.upgrade()) else {
                    return;
                };
                if app.closed.load(Ordering::Acquire) {
                    return;
                }
                match result {
                    Ok(uri) => match ui::webview::terminal_view(&app.cfg, &uri) {
                        Ok(view) => {
                            slot.add(&view);
                            slot.show_all();
                        }
                        Err(e) => app.status.set_text(&format!("{e:?}")),
                    },
                    Err(e) => {
                        slot.add(&gtk::Label::new(Some(&e)));
                        slot.show_all();
                    }
                }
            },
        );
    }
    fn current_session(&self) -> Option<String> {
        self.workspace.focused().or_else(|| {
            let current = self.notebook.current_page()?;
            self.registry
                .borrow()
                .ordered_keys()
                .into_iter()
                .find(|key| {
                    self.terms.borrow().get(key).is_some_and(|_| {
                        self.t17.boxes.borrow().get(key).is_some_and(|owner| {
                            self.notebook.page_num(&owner.widget) == Some(current)
                        })
                    })
                })
        })
    }
    fn terminal(self: &Rc<Self>, key: &str) -> Result<TermView, String> {
        let weak = Rc::downgrade(self);
        let ended_key = key.to_string();
        let home = self.private_home();
        let sandbox = self.cfg.mode() == RunMode::Sandbox;
        let term = TermView::new(TermOptions {
            argv: self.tmux.attach_argv(key),
            cwd: home.clone(),
            session: Some(key.into()),
            palette: crate::theme::desktop_theme(
                self.preferences
                    .borrow()
                    .get("theme")
                    .and_then(Value::as_str)
                    .unwrap_or(crate::theme::DEFAULT_THEME),
                &crate::theme::themes_from_file(Some(include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../config/themes.json"
                )))),
            )
            .map(|theme| theme_palette(&theme))
            .unwrap_or_else(|| {
                Palette::xterm_default(
                    [204, 204, 204],
                    [26, 26, 26],
                    [204, 204, 204],
                    [26, 26, 26],
                    [81, 82, 87],
                )
            }),
            scrollback: 10000,
            preferences: self.preferences.borrow().clone(),
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
            on_exit: Some(Rc::new(move |_| {
                if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) {
                    let tmux = app.tmux.clone();
                    let key = ended_key.clone();
                    let weak = Rc::downgrade(&app);
                    app.jobs.spawn(
                        move || {
                            let alive = tmux
                                .read(&["has-session", "-t", &format!("={key}")])
                                .is_ok_and(|o| o.ok());
                            (key, alive)
                        },
                        move |(key, alive)| {
                            if !alive && let Some(app) = weak.upgrade() {
                                if let Some(term) = app.terms.borrow().get(&key) {
                                    term.show_finished(
                                        "[sesión terminada — cierra esta pestaña con la x]",
                                    );
                                }
                                if let Some(label) = app.labels.borrow().get(&key) {
                                    label.item.style_context().add_class("closed");
                                    label.set_state("dead");
                                }
                            }
                        },
                    );
                }
            })),
            on_bell: None,
            on_link: None,
            on_ssh_scroll: None,
        })
        .map_err(|e| format!("{e:?}"))?;
        self.attach_term_keys(key, &term);
        self.attach_term_t16(key, &term);
        Ok(term)
    }
    fn add_tab(self: &Rc<Self>, key: &str, label: &str, attach: bool, error: Option<&str>) {
        if let Some(url) = key.strip_prefix("web:") {
            if let Err(error) = self.open_web_tab(url, label) {
                self.status.set_text(&error.to_string());
            }
            return;
        }
        if self.labels.borrow().contains_key(key) {
            return;
        }
        self.registry.borrow_mut().insert(TabRecord {
            key: key.into(),
            label: label.into(),
            favorite: false,
            kind: if key == "local" {
                TabKind::Local
            } else if key.starts_with("xterm-") {
                TabKind::Web
            } else {
                TabKind::Session
            },
        });
        let child: gtk::Widget = if key.starts_with("xterm-") {
            let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
            self.mount_web_terminal(key, &slot);
            slot.upcast()
        } else if attach {
            match self.terminal(key) {
                Ok(term) => {
                    let widget = self.attach_pane_overlay(key, &term).widget.clone().upcast();
                    self.terms.borrow_mut().insert(key.into(), term);
                    widget
                }
                Err(e) => {
                    self.status.set_text(&e);
                    gtk::Label::new(Some(&e)).upcast()
                }
            }
        } else {
            let button = gtk::Button::with_label(
                error.unwrap_or("Select to attach existing session read-only"),
            );
            let weak = Rc::downgrade(self);
            let k = key.to_string();
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.attach_selected(&k);
                }
            });
            button.upcast()
        };
        child.set_widget_name(&format!("tab-{key}"));
        self.workspace.register_tab(key, &child);
        let tab = ui::tab_label::TabLabel::new(label, key == "local", key != "local");
        tab.set_language(self.english);
        self.install_drag(key, tab.name.upcast_ref());
        self.install_drag(key, tab.item.upcast_ref());
        let weak = Rc::downgrade(self);
        let k = key.to_string();
        tab.item.connect_button_press_event(move |_, event| {
            if event.button() == 1
                && let Some(app) = weak.upgrade()
            {
                app.select(&k);
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        let k = key.to_string();
        tab.name.connect_button_press_event(move |_, event| {
            if let Some(app) = weak.upgrade() {
                if event.button() == 1 {
                    if event.event_type() == gdk::EventType::DoubleButtonPress {
                        app.rename_tab(&k);
                        return glib::Propagation::Stop;
                    }
                    app.select(&k);
                } else if event.button() == 3 {
                    app.tab_menu(&k, event);
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        let k = key.to_string();
        tab.dot.add_events(gdk::EventMask::BUTTON_PRESS_MASK);
        tab.dot.connect_button_press_event(move |_, event| {
            if event.button() == 1
                && let Some(app) = weak.upgrade().filter(|a| a.writable())
            {
                app.mark_menu(&k, None, event);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        if let Some(button) = &tab.favorite {
            let weak = Rc::downgrade(self);
            let k = key.to_string();
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.toggle_favorite(&k);
                }
            });
        }
        if let Some(button) = &tab.close {
            let weak = Rc::downgrade(self);
            let k = key.to_string();
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.close_tab(&k, true);
                }
            });
        }
        if let Some(button) = &tab.suggest {
            let weak = Rc::downgrade(self);
            let k = key.to_string();
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.set_mark(&k, "resolved");
                }
            });
        }
        self.strip
            .borrow_mut()
            .insert(key.into(), &child, &gtk::Label::new(Some(label)).upcast());
        self.labels.borrow_mut().insert(key.into(), tab);
        self.sync_strip();
        self.notebook.show_all();
        if !self.workspace_doc.borrow().is_null() {
            self.workspace.refresh();
            self.notebook.hide();
        }
        self.remember_current_navigation_page();
    }
    fn attach_selected(self: &Rc<Self>, key: &str) {
        if self.cfg.mode() != RunMode::Shadow || self.terms.borrow().contains_key(key) {
            return;
        }
        let tmux = self.tmux.clone();
        let key = key.to_string();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || {
                let exists = tmux
                    .read(&["has-session", "-t", &format!("={key}")])
                    .is_ok_and(|o| o.ok());
                (key, exists)
            },
            move |(key, exists)| {
                let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                    return;
                };
                if !exists {
                    app.status.set_text("Session unavailable");
                    return;
                }
                match app.terminal(&key) {
                    Ok(term) => {
                        let widget: gtk::Widget =
                            app.attach_pane_overlay(&key, &term).widget.clone().upcast();
                        app.strip.borrow_mut().remove(&key);
                        app.workspace.register_tab(&key, &widget);
                        let label = app
                            .registry
                            .borrow()
                            .to_json()
                            .get(&key)
                            .unwrap_or(&Value::Null)
                            .as_str()
                            .unwrap_or(&key)
                            .to_string();
                        app.strip.borrow_mut().insert(
                            key.clone(),
                            &widget,
                            &gtk::Label::new(Some(&label)).upcast(),
                        );

                        app.terms.borrow_mut().insert(key.clone(), term);
                        app.workspace.refresh();
                        app.notebook.show_all();
                        app.select(&key);
                    }
                    Err(e) => app.status.set_text(&e),
                }
            },
        );
    }
    fn select(&self, key: &str) {
        self.paint_selected(key);
        if self.strip.borrow().focus(key) {
            if let Some(term) = self.terms.borrow().get(key) {
                term.widget().grab_focus();
            }
        } else {
            self.workspace.select(key);
        }
    }
    fn rename_tab(self: &Rc<Self>, key: &str) {
        if !self.writable() {
            return;
        }
        let tab = self.labels.borrow().get(key).cloned();
        if let Some(tab) = tab {
            let weak = Rc::downgrade(self);
            let k = key.to_string();
            tab.begin_rename(Rc::new(move |text| {
                if let Some(app) = weak.upgrade().filter(|a| a.writable()) {
                    app.registry.borrow_mut().rename(&k, &text);
                    if let Some(tab) = app.labels.borrow().get(&k) {
                        tab.set_text(&text);
                    }
                    app.sync_strip();
                    app.persist();
                }
            }));
        }
    }
    fn toggle_favorite(self: &Rc<Self>, key: &str) {
        self.queue_favorite(key);
    }
    fn paint_favorites(&self) {
        let favorites = self.registry.borrow().favorite_keys();
        for (key, tab) in self.labels.borrow().iter() {
            tab.set_favorite(favorites.contains(key));
        }
    }
    fn close_tab(self: &Rc<Self>, key: &str, confirm: bool) {
        if self.t18.web_tabs.borrow().contains_key(key) {
            self.close_web_tab(key);
            return;
        }
        if !self.writable() {
            return;
        }
        if confirm
            && ui::confirm::confirm_must_answer(
                Some(self.window.upcast_ref()),
                "Close tab?",
                "The session remains running.",
                "Close",
                "Keep",
            ) != ui::confirm::ConfirmAnswer::Yes
        {
            return;
        }
        let label = self
            .registry
            .borrow()
            .to_json()
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or(key)
            .to_string();
        let key_for_archive = key.to_string();
        let state = self.state.clone();
        let tmux = self.tmux.clone();
        let cancelled = self.closed.clone();
        self.jobs.spawn(move||{
            if cancelled.load(Ordering::Acquire){return Ok(());}
            let target=format!("={key_for_archive}:");
            let cwd=tmux.read(&["display-message","-p","-t",&target,"#{pane_current_path}"]).ok().filter(|o|o.ok()).map(|o|o.stdout.trim().to_string()).unwrap_or_default();
            let command=tmux.read(&["display-message","-p","-t",&target,"#{pane_current_command}"]).ok().filter(|o|o.ok()).map(|o|o.stdout.trim().to_string()).unwrap_or_default();
            let agent=if command=="node"{"codex"}else if command=="cc-acp"{"acp"}else if matches!(command.as_str(),"claude"|"grok"|"opencode"|"gemini"|"agy"|"aider"|"acp"){command.as_str()}else{"claude"};
            state.archive_tab(&json!({"session":key_for_archive,"label":label,"cwd":cwd,"agent":agent,"reason":"closed","ts":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0,|d|d.as_secs())})).map_err(|e|format!("{e:?}"))?;
            if !cancelled.load(Ordering::Acquire)&& let Ok(Some(owned))=tmux.idle_scratch(&key_for_archive){tmux.kill_owned_session(owned).map_err(|e|format!("{e:?}"))?;}
            Ok::<_,String>(())
        },|result|{if let Err(e)=result{eprintln!("close tab: {e}");}});
        let removed_term = self.terms.borrow_mut().remove(key);
        if let Some(term) = removed_term {
            self.close_pane_consumers(key);
            if let Some(owner) = self.t17.boxes.borrow_mut().remove(key) {
                owner.cancel();
            }
            term.shutdown();
        }
        self.strip.borrow_mut().remove(key);
        self.workspace.remove_tab(key);
        self.forget_navigation_page(key);
        if let Some(row) = self.labels.borrow_mut().remove(key)
            && let Some(parent) = row
                .item
                .parent()
                .and_then(|p| p.downcast::<gtk::Container>().ok())
        {
            parent.remove(&row.item);
        }
        self.registry.borrow_mut().archive(key, "closed");
        self.sync_strip();
        self.persist();
    }
    fn poll(self: &Rc<Self>, update: PollUpdate) {
        match update {
            PollUpdate::Prefs {
                value,
                favorite_generation,
            } => {
                self.prefs_received.set(true);
                if crate::poll::live_pref_snapshot(&self.preferences.borrow())
                    != crate::poll::live_pref_snapshot(&value)
                    || self.preferences.borrow().get("button_style") != value.get("button_style")
                {
                    *self.preferences.borrow_mut() = value.clone();
                    self.apply_tabs_layout(
                        value
                            .get("tabs_layout")
                            .and_then(Value::as_str)
                            .unwrap_or("row"),
                        false,
                    );
                    let themes = crate::theme::themes_from_file(Some(include_bytes!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../config/themes.json"
                    ))));
                    let theme = crate::theme::desktop_theme(
                        value
                            .get("theme")
                            .and_then(Value::as_str)
                            .unwrap_or(crate::theme::DEFAULT_THEME),
                        &themes,
                    );
                    if let Some(theme) = theme.as_ref() {
                        self.drag_layer.colors(
                            theme
                                .values
                                .get("bar")
                                .and_then(Value::as_str)
                                .unwrap_or("#161B22"),
                            theme
                                .values
                                .get("brand")
                                .and_then(Value::as_str)
                                .unwrap_or("#8B7CF6"),
                        );
                        self.paint_theme(
                            theme,
                            value
                                .get("button_style")
                                .and_then(Value::as_str)
                                .unwrap_or("sutil"),
                        );
                    }
                    for term in self.terms.borrow().values() {
                        term.set_preferences(&value);
                        if let Some(theme) = theme.as_ref() {
                            term.set_palette(theme_palette(theme));
                        }
                    }
                    let auxiliary = self
                        .t18
                        .clients
                        .borrow()
                        .values()
                        .map(|c| (c.kind, c.term.clone()))
                        .collect::<Vec<_>>();
                    for (kind, term) in auxiliary {
                        let mut prefs = value.clone();
                        if kind == "mosaic"
                            && let Some(map) = prefs.as_object_mut()
                        {
                            map.insert("cursor_blink".into(), json!(false));
                        }
                        term.set_preferences(&prefs);
                        if let Some(theme) = theme.as_ref() {
                            term.set_palette(theme_palette(theme));
                        }
                    }
                }
                if let Some(f) = value.get("favorites") {
                    let favorites = self.merge_pending_favorites(f);
                    self.registry
                        .borrow_mut()
                        .apply_favorites(&favorites, favorite_generation);
                    self.paint_favorites();
                }
            }
            PollUpdate::Workspace(value) => {
                if self.apply_workspace(&value) {
                    self.flush_resize();
                }
            }
            PollUpdate::Notices { badge, .. } => self.header.set_badge(badge),
            PollUpdate::State(value) => {
                self.state_received.set(true);
                for item in value
                    .as_array()
                    .or_else(|| {
                        value
                            .get("sessions")
                            .or_else(|| value.get("items"))
                            .and_then(Value::as_array)
                    })
                    .into_iter()
                    .flatten()
                {
                    if let Some(key) = item.get("session").unwrap_or(&Value::Null).as_str()
                        && let Some(row) = self.labels.borrow().get(key)
                    {
                        row.set_state(
                            item.get("status")
                                .unwrap_or(&Value::Null)
                                .as_str()
                                .unwrap_or(""),
                        );
                        self.state_items
                            .borrow_mut()
                            .insert(key.into(), item.clone());
                    }
                }
            }
            PollUpdate::Marks(value) => {
                self.marks_received.set(true);
                // La lectura heredada alimenta restore; la adopción visual usa GET propio con generación.
                if self.work_marks.borrow().generation() == 0 {
                    self.work_marks.borrow_mut().adopt_poll(&value, 0);
                    self.paint_marks();
                }
            }
        }
    }
    fn apply_workspace(self: &Rc<Self>, value: &Value) -> bool {
        if !self.restore.borrow().ready() || self.posting.get() || self.dragging.get() {
            *self.pending_workspace.borrow_mut() = Some(value.clone());
            return false;
        }
        let doc = value.get("document").unwrap_or(value);
        if comandos_core::workspace::validate_document(doc).is_err() {
            return false;
        }
        self.revision.set(
            value
                .get("revision")
                .unwrap_or(&Value::Null)
                .as_u64()
                .unwrap_or(0),
        );
        self.workspace.apply(doc);
        *self.workspace_doc.borrow_mut() = doc.clone();
        self.remember_current_navigation_page();
        self.sync_strip();
        self.notebook.hide();
        self.workspace.widget().show_all();
        self.restore_device_focus();
        self.resize_queue.borrow_mut().authority_received();
        true
    }
    pub(super) fn flush_resize(self: &Rc<Self>) {
        if self.posting.get()
            || self.dragging.get()
            || !self.restore.borrow().ready()
            || self.closed.load(Ordering::Acquire)
            || (self.cfg.mode() != RunMode::Shadow && !self.writable())
        {
            return;
        }
        let publication = self
            .resize_queue
            .borrow_mut()
            .prepare(&self.workspace_doc.borrow(), false);
        if let Some(publication) = publication {
            if publication.rejected > 0 {
                self.status
                    .set_text("Layout changed; obsolete divider movement discarded");
            }
            if publication.changed {
                self.commit_workspace(publication.document, None);
            }
        }
    }
    fn commit_workspace(self: &Rc<Self>, doc: Value, focus: Option<String>) {
        if self.posting.get() || !self.restore.borrow().ready() {
            return;
        }
        if self.cfg.mode() == RunMode::Shadow {
            self.workspace.commit(&doc, focus.as_deref());
            self.resize_queue.borrow_mut().complete(true);
            self.resize_queue.borrow_mut().authority_received();
            return;
        }
        if !self.writable() {
            return;
        }
        if self.cfg.mode() == RunMode::Sandbox && self.cfg.dash_url().is_none() {
            let revision = self.revision.get().saturating_add(1);
            let value = json!({"document":doc,"revision":revision});
            self.resize_queue.borrow_mut().complete(true);
            self.apply_workspace(&value);
            if let Some(focus) = focus.as_deref() {
                self.select(focus);
            }
            let state = self.state.clone();
            let closed = self.closed.clone();
            self.jobs.spawn(
                move || {
                    if !closed.load(Ordering::Acquire) {
                        state.write("app-layout.json", &value)
                    } else {
                        Ok(())
                    }
                },
                |result| {
                    if let Err(e) = result {
                        eprintln!("workspace privado: {e:?}");
                    }
                },
            );
            return;
        }
        let mut bytes = [0u8; 12];
        if getrandom::fill(&mut bytes).is_err() {
            self.resize_queue.borrow_mut().complete(false);
            self.status.set_text("No request identity available");
            return;
        }
        let request_id = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        self.posting.set(true);
        let dash = self.dash.clone();
        let revision = self.revision.get();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || {
                dash.post(
                    "/workspace",
                    &json!({"requestId":request_id,"expectedRevision":revision,"document":doc}),
                    Duration::from_secs(5),
                )
            },
            move |response| {
                let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                    return;
                };
                app.posting.set(false);
                let mut authority_received = false;
                match response {
                    Ok((status, value)) => {
                        let current = if status == 409 {
                            value.get("current")
                        } else if (200..300).contains(&status) {
                            Some(&value)
                        } else {
                            None
                        };
                        // Un conflicto conserva la última intención y la rebasa sobre current.
                        let valid_authority = current.is_some_and(|v| {
                            comandos_core::workspace::validate_document(
                                v.get("document").unwrap_or(v),
                            )
                            .is_ok()
                        });
                        app.resize_queue
                            .borrow_mut()
                            .complete((200..300).contains(&status) && valid_authority);
                        if let Some(current) = current {
                            authority_received = app.apply_workspace(current);
                            if let Some(focus) = focus.as_deref() {
                                app.workspace.select(focus);
                            }
                        }
                        if status == 409 {
                            app.status.set_text(
                                "Another device changed the layout; showing current layout",
                            );
                        } else if !(200..300).contains(&status) {
                            app.status.set_text(&format!("Workspace HTTP {status}"));
                        }
                    }
                    Err(e) => {
                        app.resize_queue.borrow_mut().complete(false);
                        app.status.set_text(&format!("Workspace: {e:?}"));
                    }
                }
                // A queued poll may be older than the authoritative POST response.
                let pending = app.pending_workspace.borrow_mut().take();
                if let Some(pending) = pending
                    && pending
                        .get("revision")
                        .unwrap_or(&Value::Null)
                        .as_u64()
                        .is_some_and(|r| r > app.revision.get())
                {
                    authority_received |= app.apply_workspace(&pending);
                }
                if authority_received {
                    app.flush_resize();
                }
            },
        );
    }
    fn install_drag(self: &Rc<Self>, key: &str, source: &gtk::Widget) {
        source.add_events(
            gdk::EventMask::BUTTON_PRESS_MASK
                | gdk::EventMask::BUTTON_RELEASE_MASK
                | gdk::EventMask::POINTER_MOTION_MASK,
        );
        let weak = Rc::downgrade(self);
        let k = key.to_string();
        source.connect_button_press_event(move |_, event| {
            if event.button() == 1
                && event.event_type() == gdk::EventType::ButtonPress
                && let Some(app) = weak
                    .upgrade()
                    .filter(|a| !a.workspace_doc.borrow().is_null() && !a.posting.get())
            {
                let same_press = {
                    let gesture = app.drag_layer.0.gesture.borrow();
                    gesture.phase == crate::workspace_view::DragPhase::Pressed
                        && gesture.press_root == event.root()
                };
                if !same_press {
                    app.drag_layer.0.gesture.borrow_mut().press(
                        k.clone(),
                        event.root(),
                        app.workspace.widget().current_page(),
                    );
                }
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        source.connect_motion_notify_event(move |_, event| {
            if let Some(app) = weak.upgrade()
                && event.state().contains(gdk::ModifierType::BUTTON1_MASK)
                && app.drag_motion(event.root())
            {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        let weak = Rc::downgrade(self);
        source.connect_button_release_event(move |_, event| {
            if event.button() == 1
                && let Some(app) = weak.upgrade()
                && app.drag_layer.active()
            {
                let point = app.layer_point(event.root());
                app.drop_drag(app.drag_target(point));
                glib::Propagation::Stop
            } else {
                if let Some(app) = weak.upgrade() {
                    app.drag_layer.0.gesture.borrow_mut().finish();
                }
                glib::Propagation::Proceed
            }
        });
    }
    fn snapshot(self: &Rc<Self>, tabs_only: bool) {
        if !self.writable() || self.snapshot_busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let epoch = self.state_epoch.load(Ordering::Acquire);
        let current = self.state_epoch.clone();
        let keys = self.registry.borrow().ordered_keys();
        let state = self.state.clone();
        let tmux = self.tmux.clone();
        let home = self.private_home();
        let closed = self.closed.clone();
        let busy = self.snapshot_busy.clone();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || {
                let result = (|| {
                    let old=state.read_session_snapshot();
                    let inspector = comandos_runtime::pane_snapshot::PaneInspector::new(
                        &home,
                        std::path::Path::new("/proc"),
                    )
                    .map_err(|_| "process metadata unavailable".to_string())?;
                    let mut sessions = serde_json::Map::new();
                    let mut tabs = serde_json::Map::new();
                    let keys: Vec<_> = keys.into_iter()
                        .filter(|k| k.as_str() == "local" || k.starts_with("term-"))
                        .collect();
                    let allowed = || !closed.load(Ordering::Acquire)
                        && current.load(Ordering::Acquire) == epoch;
                    let mut batch = snapshot::capture_sessions_when(&tmux, &keys, &inspector, &allowed).ok();
                    for key in &keys {
                        if closed.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        let captured = match batch.as_mut() {
                            Some(values) => values.remove(key),
                            None if allowed() => snapshot::capture_session(&tmux, key, &inspector).ok(),
                            None => return Ok(()),
                        }
                            .map(|v| {
                                snapshot::carry_pane_keys(
                                    snapshot::carry_resume_ids(
                                        v,
                                        old.get("sessions")
                                            .and_then(|s| s.get(key))
                                            .unwrap_or(&Value::Null),
                                    ),
                                    old.get("sessions")
                                        .and_then(|s| s.get(key))
                                        .unwrap_or(&Value::Null),
                                )
                            })
                            .or_else(|| {
                                old.get("sessions")
                                    .unwrap_or(&Value::Null)
                                    .get(key)
                                    .cloned()
                            });
                        if let Some(captured) = captured {
                            if let Some(pane) = captured
                                .get("windows")
                                .unwrap_or(&Value::Null)
                                .as_array()
                                .and_then(|w| w.iter().find(|w| w["active"] == true))
                                .and_then(|w| w["panes"].as_array())
                                .and_then(|p| p.iter().find(|p| p["active"] == true))
                            {
                                tabs.insert(key.clone(), pane.clone());
                            }
                            sessions.insert(key.clone(), captured);
                        }
                    }
                    if closed.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    if tabs_only {
                        state
                            .write_snapshot_when(
                                "app-tabs-snapshot.json",
                                &Value::Object(tabs),
                                || {
                                    !closed.load(Ordering::Acquire)
                                        && current.load(Ordering::Acquire) == epoch
                                },
                            )
                            .map_err(|e| format!("{e:?}"))?;
                    } else {
                        let snapshot = json!({"version":2,"sessions":sessions,"saved_at":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0,|d|d.as_secs())});
                        if comandos_core::workspace::snapshot::check_snapshot(&snapshot)
                            != comandos_core::workspace::snapshot::Snapshot::Valid
                        {
                            return Err("refusing incomplete snapshot".into());
                        }
                        state
                            .write_snapshot_when("app-sessions-v2.json", &snapshot, || {
                                !closed.load(Ordering::Acquire)
                                    && current.load(Ordering::Acquire) == epoch
                            })
                            .map_err(|e| format!("{e:?}"))?;
                    }
                    Ok(())
                })();
                busy.store(false, Ordering::Release);
                result
            },
            move |result: Result<(), String>| {
                if let (Some(a), Err(e)) = (weak.upgrade(), result) {
                    a.status.set_text(&format!("Snapshot: {e}"));
                }
            },
        );
    }
    fn drain_ipc(self: &Rc<Self>) {
        if !self.writable() || self.ipc_reading.replace(true) {
            return;
        }
        let mut paths = std::mem::take(&mut *self.ipc_pending.borrow_mut());
        // SQL queues have no file notification; the existing 100 ms deadline polls them.
        for name in [
            "app-focus.json",
            "app-tab-open.json",
            "app-tab-close.json",
            "app-command.json",
        ] {
            paths.insert(self.cfg.hooks_dir().join(name));
        }
        let home = self.cfg.home().to_path_buf();
        let live = self.cfg.mode() == RunMode::Live;
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || {
                let paths = paths.into_iter().collect::<Vec<_>>();
                (if live {
                    ipc::read_requests_domain_live(&home, &paths)
                } else {
                    ipc::read_requests_domain(&home, &paths)
                })
                .unwrap_or_default()
            },
            move |requests| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                app.ipc_reading.set(false);
                for request in requests {
                    if app.ipc_seen.borrow().contains(&request) {
                        continue;
                    }
                    if app.handle_ipc(&request) {
                        let consumer = IpcConsumer::new(app.cfg.mode(), app.guard.clone());
                        let request_for_consume = request.clone();
                        let home = app.cfg.home().to_path_buf();
                        app.jobs.spawn(
                            move || consumer.consume_domain(&home, &request_for_consume),
                            |result| {
                                if let Err(e) = result {
                                    eprintln!("IPC consume: {e:?}");
                                }
                            },
                        );
                        let mut seen = app.ipc_seen.borrow_mut();
                        seen.push(request);
                        if seen.len() > 64 {
                            seen.remove(0);
                        }
                    }
                }
            },
        );
    }
    fn handle_ipc(self: &Rc<Self>, request: &IpcRequest) -> bool {
        if !self.writable() {
            return false;
        }
        let key = request
            .payload
            .get("session")
            .unwrap_or(&Value::Null)
            .as_str()
            .unwrap_or("");
        match request.kind {
            IpcKind::TabClose => {
                let keys = request
                    .payload
                    .get("sessions")
                    .unwrap_or(&Value::Null)
                    .as_array()
                    .map(|v| v.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                    .unwrap_or_else(|| vec![key]);
                for key in keys {
                    self.close_tab(key, false);
                }
                true
            }
            IpcKind::Focus | IpcKind::TabOpen if !key.is_empty() => {
                if self.labels.borrow().contains_key(key) {
                    if request.kind == IpcKind::Focus {
                        self.select(key);
                        self.window.present();
                    }
                } else {
                    self.open_existing(
                        key,
                        request
                            .payload
                            .get("label")
                            .unwrap_or(&Value::Null)
                            .as_str()
                            .unwrap_or(key),
                        request.kind == IpcKind::Focus,
                    );
                }
                true
            }
            IpcKind::Command => self.dispatch(&request.payload),
            _ => false,
        }
    }
    fn open_existing(self: &Rc<Self>, key: &str, label: &str, focus: bool) {
        self.open_tab(key, "claude", label, focus);
    }
    fn open_tab(self: &Rc<Self>, key: &str, window: &str, label: &str, focus: bool) {
        if !self.writable() || !crate::tab_actions::valid_session(key) {
            return;
        }
        let window = window.to_string();
        let previous = self.current_session();
        let tmux = self.tmux.clone();
        let key = key.to_string();
        let label = label.to_string();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || {
                let exists = tmux
                    .read(&["has-session", "-t", &format!("={key}")])
                    .is_ok_and(|o| o.ok());
                if exists {
                    let _ = crate::tab_actions::select_window(&tmux, &key, &window);
                }
                (key, label, exists)
            },
            move |(key, label, exists)| {
                if let Some(app) = weak.upgrade().filter(|a| a.writable()) {
                    app.add_tab(
                        &key,
                        &label,
                        exists,
                        (!exists).then_some("Session unavailable"),
                    );
                    if focus {
                        app.select(&key);
                    } else if let Some(previous) = previous.as_deref() {
                        app.select(previous);
                    }
                    app.persist();
                }
            },
        );
    }
}
// Copied sandbox state may contain real agent identities. Keep those panels as shells.
struct SandboxScopes;
impl restore::ScopeLauncher for SandboxScopes {
    fn resume(&self, _: &Value, _: &std::path::Path) -> Option<String> {
        None
    }
    fn argv(&self, _: &str, _: &str) -> Result<Vec<String>, String> {
        Ok(vec!["/bin/true".into()])
    }
}
pub fn run(args: &[String], default_live: bool) -> ExitCode {
    let cfg = match config::parse_args(args, default_live, &|k| std::env::var(k).ok()) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("comandos-app: {e}");
            return ExitCode::from(2);
        }
    };
    let display = std::env::var("DISPLAY").unwrap_or_default();
    let guard = WriteGuard::from_config(&cfg, &display);
    let lock = match ui::window::instance_lock(&cfg, &guard, &display) {
        Ok(lock) => lock,
        Err(ui::window::InstanceError::Contended) => {
            ui::window::activate_existing(&cfg, &crate::proc::run);
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("comandos-app: instance lock: {error:?}");
            return ExitCode::from(2);
        }
    };
    if cfg.mode() == RunMode::Sandbox
        && let Err(error) = guard.create_dir_all(cfg.hooks_dir(), 0o700)
    {
        eprintln!("comandos-app: private state directory: {error:?}");
        return ExitCode::from(2);
    }
    gdk::set_allowed_backends("x11");
    glib::set_prgname(Some(cfg.wm_class()));
    if let Err(e) = gtk::init() {
        eprintln!("comandos-app: GTK: {e}");
        return ExitCode::from(2);
    }
    let application = gtk::Application::new(None, gio::ApplicationFlags::NON_UNIQUE);
    let lock = Rc::new(RefCell::new(Some(lock)));
    application.connect_activate(move |application| {
        let Some(lock) = lock.borrow_mut().take() else {
            return;
        };
        let Ok(tmux) = TmuxCtl::from_config(&cfg, &|k| std::env::var(k).ok()) else {
            application.quit();
            return;
        };
        if let Err(e) = tmux.prepare_socket_dir(&guard) {
            eprintln!("comandos-app: {e:?}");
            application.quit();
            return;
        }
        let Ok(dash) = DashClient::new(cfg.dash_url(), cfg.mode()) else {
            application.quit();
            return;
        };
        let window = ui::window::create(application, &cfg);
        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.style_context().add_class("cc-paned");
        paned.set_wide_handle(false);
        let notebook = gtk::Notebook::new();
        notebook.set_hexpand(true);
        notebook.set_vexpand(true);
        let observation =
            crate::layout_dump::enabled(std::env::var("COMANDOS_APP_LAYOUT_DUMP").ok().as_deref())
                .then(|| Rc::new(RefCell::new(ui::webview::LoadObservation::default())));
        let webview = match ui::webview::create_observed(&cfg, observation.clone()) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("comandos-app: WebKit: {e:?}");
                application.quit();
                return;
            }
        };
        let app = App::new(
            cfg.clone(),
            tmux,
            guard.clone(),
            Jobs::new(2, "app"),
            dash.clone(),
            window.clone(),
            paned.clone(),
            notebook.clone(),
            webview.clone(),
        );
        *app.dashboard_observation.borrow_mut() = observation;
        let terminals = gtk::Box::new(gtk::Orientation::Vertical, 0);
        terminals.pack_start(app.tab_layout.widget(), false, false, 0);
        // The tabs or the mosaic are the `term` page; the chat is its own page, so
        // whatever re-shows the notebook or the mosaic never draws tmux under the chat.
        let term_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        term_page.pack_start(&notebook, true, true, 0);
        term_page.pack_start(app.workspace.widget(), true, true, 0);
        let pages = gtk::Stack::new();
        pages.set_transition_type(gtk::StackTransitionType::None);
        pages.set_hhomogeneous(false);
        pages.set_vhomogeneous(false);
        pages.add_named(&term_page, "term");
        app.install_chat(&pages);
        pages.set_visible_child_name("term");
        terminals.pack_start(&pages, true, true, 0);
        terminals.pack_start(&app.status, false, false, 0);
        app.t18.side.pack1(&webview, true, false);
        paned.pack1(&app.t18.side, true, false);
        let overlay = gtk::Overlay::new();
        overlay.add(&terminals);
        overlay.add_overlay(app.drag_layer.widget());
        overlay.set_overlay_pass_through(app.drag_layer.widget(), true);
        app.t17.shelf.paned.pack1(&overlay, true, false);
        app.t18
            .reader_paned
            .pack1(&app.t17.shelf.paned, true, false);
        paned.pack2(&app.t18.column, true, false);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let header_event = gtk::EventBox::new();
        header_event.set_above_child(false);
        header_event.set_visible_window(false);
        header_event.add(&app.header.bar);
        header_event.add_events(gdk::EventMask::BUTTON_PRESS_MASK);
        let weak = window.downgrade();
        header_event.connect_button_press_event(move |_, event| {
            if event.button() != 1 {
                return glib::Propagation::Proceed;
            }
            let Some(window) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if event.event_type() == gdk::EventType::DoubleButtonPress {
                if window.is_maximized() {
                    window.unmaximize();
                } else {
                    window.maximize();
                }
                return glib::Propagation::Stop;
            }
            if event.event_type() == gdk::EventType::ButtonPress {
                let (x, y) = event.root();
                window.begin_move_drag(1, x as i32, y as i32, event.time());
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        content.pack_start(&header_event, false, false, 0);
        content.pack_start(&paned, true, true, 0);
        app.modal_overlay.add(&content);
        window.add(&app.modal_overlay);
        app.install_foundation_handlers();
        ui::app_commands::install(&app);
        ui::header::install(&app);
        app.install_keys();
        app.install_auxiliary_views();
        app.install_t16();
        app.install_t17();
        let bridge_source = ui::bridge::install(&app);
        let weak = Rc::downgrade(&app);
        app.workspace.on_resize(Rc::new(move |updates| {
            if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) {
                let rejected = app
                    .resize_queue
                    .borrow_mut()
                    .record_visual(&app.workspace_doc.borrow(), updates);
                if rejected > 0 {
                    app.status
                        .set_text("Layout changed; obsolete divider movement discarded");
                }
                app.flush_resize();
            }
        }));
        let (poller, rx) = Poller::start(dash, app.generation.clone());
        let weak = Rc::downgrade(&app);
        let poll = glib::timeout_add_local(Duration::from_millis(100), move || {
            if let Some(app) = weak.upgrade() {
                let mut batch = crate::poll::PollBatch::default();
                for update in rx.try_iter().take(256) {
                    batch.push(update);
                }
                for update in batch.drain() {
                    app.poll(update);
                }
                app.drain_ipc();
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
        let mut sources = vec![
            poll,
            bridge_source,
            ui::presence::install(&app),
            app.install_clipboard_watch(),
        ];
        if let Some(source) = app.install_layout_diagnostic() {
            sources.push(source);
        }
        let weak = Rc::downgrade(&app);
        sources.push(glib::timeout_add_local(
            Duration::from_millis(260),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.drag_tick();
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            },
        ));
        let epoch = std::time::Instant::now();
        let weak = Rc::downgrade(&app);
        sources.push(glib::timeout_add_local(
            Duration::from_millis(80),
            move || {
                if let Some(app) = weak.upgrade() {
                    if app.presence_visible.get() && ui::header::animations_enabled() {
                        for tab in app.labels.borrow().values() {
                            tab.paint(epoch.elapsed().as_secs_f64());
                        }
                    }
                    app.paint_pane_indicators(epoch.elapsed().as_secs_f64());
                    if let Some(key) = app.current_session() {
                        app.paint_selected(&key);
                        app.save_device_focus(&key);
                    }
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            },
        ));
        let weak = Rc::downgrade(&app);
        sources.push(glib::timeout_add_local(Duration::from_secs(5), move || {
            if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) {
                if app.presence_visible.get() {
                    app.fetch_marks();
                }
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        }));
        for (seconds, tabs_only) in [(5, false), (30, true)] {
            let weak = Rc::downgrade(&app);
            sources.push(glib::timeout_add_local(
                Duration::from_secs(seconds),
                move || {
                    if let Some(app) = weak.upgrade() {
                        app.snapshot(tabs_only);
                        glib::ControlFlow::Continue
                    } else {
                        glib::ControlFlow::Break
                    }
                },
            ));
        }
        let mut monitors = Vec::new();
        let file = gio::File::for_path(cfg.hooks_dir());
        if let Ok(monitor) =
            file.monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            let weak = Rc::downgrade(&app);
            monitor.connect_changed(move |_, file, _, event| {
                if matches!(
                    event,
                    gio::FileMonitorEvent::Created
                        | gio::FileMonitorEvent::ChangesDoneHint
                        | gio::FileMonitorEvent::MovedIn
                ) && let (Some(app), Some(path)) = (weak.upgrade(), file.path())
                    && matches!(
                        path.file_name().and_then(|s| s.to_str()),
                        Some(
                            "app-focus.json"
                                | "app-tab-open.json"
                                | "app-tab-close.json"
                                | "app-command.json"
                        )
                    )
                {
                    app.ipc_pending.borrow_mut().insert(path);
                    app.drain_ipc();
                }
            });
            monitors.push(monitor);
        }
        let runtime = Rc::new(RefCell::new(Some(AppRuntime {
            app: app.clone(),
            poller,
            sources,
            monitors,
            _lock: lock,
        })));
        let shutdown_runtime = Rc::downgrade(&runtime);
        application.connect_shutdown(move |_| {
            if let Some(runtime) = shutdown_runtime.upgrade()
                && let Some(runtime) = runtime.borrow_mut().take()
            {
                runtime.shutdown();
            }
        });
        let close_runtime = runtime.clone();
        window.connect_delete_event(move |_, _| {
            if let Some(runtime) = close_runtime.borrow_mut().take() {
                runtime.shutdown();
            }
            glib::Propagation::Proceed
        });
        let destroy_runtime = runtime.clone();
        window.connect_destroy(move |_| {
            if let Some(runtime) = destroy_runtime.borrow_mut().take() {
                runtime.shutdown();
            }
        });
        window.show_all();
        app.workspace.widget().hide();
        app.startup();
    });
    // AppConfig already consumed our arguments. GApplication's parser must not
    // see --mode, --tmux-socket or --dash-url again and reject a valid launch.
    application.run_with_args::<&str>(&[]);
    ExitCode::SUCCESS
}

fn theme_palette(theme: &crate::theme::ThemeTokens) -> Palette {
    let color = |name: &str, fallback: [u8; 3]| {
        theme
            .values
            .get(name)
            .and_then(Value::as_str)
            .and_then(parse_color)
            .unwrap_or(fallback)
    };
    let bg = color("bg", [26, 26, 26]);
    let mut palette = Palette::xterm_default(
        color("fg", [204, 204, 204]),
        bg,
        color("text", [204, 204, 204]),
        bg,
        color("brand", [81, 82, 87]),
    );
    for (slot, hex) in palette.ansi.iter_mut().zip(theme.ansi.iter()) {
        if let Some(rgb) = parse_color(hex) {
            *slot = rgb;
        }
    }
    palette
}
fn parse_color(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(hex.get(..2)?, 16).ok()?,
        u8::from_str_radix(hex.get(2..4)?, 16).ok()?,
        u8::from_str_radix(hex.get(4..6)?, 16).ok()?,
    ])
}

#[path = "app_t14.rs"]
mod app_t14;
#[path = "app_t15.rs"]
mod app_t15;
#[path = "app_diagnostics.rs"]
mod diagnostics;
#[path = "app_foundation.rs"]
mod foundation;

#[path = "app_t13.rs"]
mod protocol;

#[path = "app_t16.rs"]
mod app_t16;
#[path = "app_t17.rs"]
mod app_t17;
#[path = "app_t17_panes.rs"]
mod app_t17_panes;
#[path = "app_t17_shelf.rs"]
mod app_t17_shelf;

#[path = "app_t18.rs"]
mod app_t18;
#[path = "app_t18_side.rs"]
mod app_t18_side;
#[path = "app_t18_views.rs"]
mod app_t18_views;
#[path = "app_chat.rs"]
mod app_chat;
pub use app_chat::preview as chat_preview;
pub use app_t17_panes::header_preview;
