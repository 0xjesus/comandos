//! Foco + repisa wiring: the shelf beside the tab area, the fixed terminal
//! width and tmux zoom so a tab with several agent panes shows one at a time.
use super::*;
use ui::focus::{self, Card, FocusShelf};

pub(super) struct Owned {
    pub row: gtk::Box,
    pub shelf: Rc<FocusShelf>,
    pub enabled: Cell<bool>,
    /// Sessions already zoomed once in this run; a manual unzoom is respected.
    pub zoomed: RefCell<BTreeSet<String>>,
    pub fit_idle: RefCell<Option<glib::SourceId>>,
    pub busy: Cell<bool>,
    /// Claude Code's gray tool noise is hidden in every terminal.
    pub noise_hidden: Cell<bool>,
}

impl Default for Owned {
    fn default() -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.style_context().add_class("fs-row");
        Self {
            row,
            shelf: FocusShelf::new(),
            // A view you turn on: the splits stay as they are by default.
            enabled: Cell::new(false),
            zoomed: RefCell::new(BTreeSet::new()),
            fit_idle: RefCell::new(None),
            busy: Cell::new(false),
            // Off by default: hiding rows made the gray text jump and split scroll
            // and selection between the app and tmux. The eye button turns it on.
            noise_hidden: Cell::new(false),
        }
    }
}

impl App {
    pub(super) fn install_focus(self: &Rc<Self>) {
        self.focus.shelf.set_language(self.english);
        let weak = Rc::downgrade(self);
        self.focus.shelf.connect_card(Rc::new(move |card: &Card| {
            if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) {
                app.focus_open(card);
            }
        }));
        let weak = Rc::downgrade(self);
        self.focus.shelf.connect_show_all(Rc::new(move || {
            if let Some(app) = weak.upgrade() {
                app.focus_set_enabled(false);
            }
        }));
        // The switch lives with the other strip buttons: nothing beside the
        // terminals takes their width.
        let toggle = ui::icons::button("columns", 18, "Foco: una terminal grande y los demás paneles como tarjetas");
        toggle.style_context().add_class("cc-key");
        toggle.style_context().add_class("cc-key-focus");
        let weak = Rc::downgrade(self);
        toggle.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                let on = !app.focus.enabled.get();
                app.focus_set_enabled(on);
            }
        });
        self.tab_layout.actions().pack_start(&toggle, false, false, 0);
        toggle.show();
        let noise = ui::icons::button(
            "eye",
            18,
            "Texto gris de Claude Code (herramientas, archivos leídos): visible. Clic para ocultarlo",
        );
        noise.set_widget_name("noise-toggle");
        noise.style_context().add_class("cc-key");
        let weak = Rc::downgrade(self);
        noise.connect_clicked(move |button| {
            if let Some(app) = weak.upgrade() {
                let hidden = !app.focus.noise_hidden.get();
                app.focus.noise_hidden.set(hidden);
                for term in app.terms.borrow().values() {
                    term.set_hide_noise(hidden);
                }
                let tip = if hidden {
                    "Texto gris de Claude Code (herramientas, archivos leídos): oculto. Clic para verlo"
                } else {
                    "Texto gris de Claude Code (herramientas, archivos leídos): visible. Clic para ocultarlo"
                };
                button.set_tooltip_text(Some(tip));
                let theme = app.applied_theme.borrow().clone();
                let style = app.applied_button_style.borrow().clone();
                if let (Some(theme), Some(style)) = (theme, style) {
                    app.paint_theme(&theme, &style);
                }
            }
        });
        self.tab_layout.actions().pack_start(&noise, false, false, 0);
        // Hidden while the gray-text filter is retired (see `set_hide_noise`).
        noise.set_no_show_all(true);
        noise.hide();
        crate::term::lag::watch_main_loop();
        let weak = Rc::downgrade(self);
        self.notebook.connect_switch_page(move |_, _, _| {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) {
                    app.focus_refresh();
                }
            });
        });
        let weak = Rc::downgrade(self);
        self.focus.row.connect_size_allocate(move |_, _| {
            if let Some(app) = weak.upgrade() {
                app.focus_fit_soon();
            }
        });
    }

    fn focus_set_enabled(self: &Rc<Self>, on: bool) {
        self.focus.enabled.set(on);
        let theme = self.applied_theme.borrow().clone();
        let style = self.applied_button_style.borrow().clone();
        if let (Some(theme), Some(style)) = (theme, style) {
            self.paint_theme(&theme, &style);
        }
        let key = self.current_session();
        if let Some(key) = key.as_deref() {
            self.focus.zoomed.borrow_mut().remove(key);
            if !on {
                self.focus_unzoom(key);
            }
        }
        self.focus_refresh();
    }

    /// The tab area changed size: re-snap the terminal width after this
    /// allocation (never resize from inside it).
    fn focus_fit_soon(self: &Rc<Self>) {
        if self.focus.fit_idle.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        *self.focus.fit_idle.borrow_mut() = Some(glib::idle_add_local_once(move || {
            if let Some(app) = weak.upgrade() {
                app.focus.fit_idle.borrow_mut().take();
                app.focus_fit();
            }
        }));
    }

    fn focus_term(&self) -> Option<TermView> {
        let key = self.current_session()?;
        self.terms.borrow().get(&key).cloned()
    }

    /// Shows the shelf at the width that leaves the terminal on a column step.
    fn focus_fit(self: &Rc<Self>) {
        let shelf = &self.focus.shelf.widget;
        let usable = self.focus.enabled.get()
            && !self.t18.mosaic.borrow().on
            && self.focus_term().is_some();
        let Some(term) = self.focus_term().filter(|_| usable) else {
            shelf.set_visible(false);
            return;
        };
        let avail = f64::from(self.focus.row.allocated_width());
        let (cell_w, padding) = term.cell_layout();
        // Whatever the tab area adds around the terminal counts as chrome too;
        // several tabs docked side by side are not one terminal to snap.
        let area = self.focus.row.allocated_width()
            - if shelf.is_visible() { shelf.allocated_width() } else { 0 };
        let around = if term.widget().allocated_width() > 1 && shelf.is_visible() {
            f64::from((area - term.widget().allocated_width()).max(0))
        } else {
            0.
        };
        let around = if around > 160. { 0. } else { around };
        match focus::snap(avail, cell_w, padding + around) {
            Some((_, term_px)) => {
                let width = focus::shelf_width(avail, term_px) as i32;
                if shelf.width_request() != width {
                    shelf.set_size_request(width, -1);
                }
                if !shelf.is_visible() {
                    shelf.set_visible(true);
                }
            }
            None => shelf.set_visible(false),
        }
    }

    /// New feed or another tab: refresh the cards and zoom a fresh tab.
    pub(super) fn focus_refresh(self: &Rc<Self>) {
        self.focus_fit();
        if !self.focus.enabled.get() {
            return;
        }
        let Some(key) = self.current_session() else {
            return;
        };
        let active = self.t17.boxes.borrow().get(&key).and_then(|owner| {
            owner
                .panes
                .borrow()
                .iter()
                .find(|p| p.active)
                .map(|p| p.geometry.id.clone())
        });
        let feed = self.feed.feed.borrow();
        let Some(feed) = feed.as_ref() else {
            return;
        };
        let registry = self.registry.borrow();
        let label = |session: &str| {
            registry
                .records()
                .find(|r| r.key == session)
                .map(|r| r.label.clone())
                .unwrap_or_default()
        };
        let cards = focus::cards(feed, &key, active.as_deref(), &label, self.english);
        let panes = feed
            .sessions
            .get(&key)
            .map_or(1, |e| e.pane_list.len().max(1));
        self.focus.shelf.render(&cards, panes);
        drop(registry);
        // Busy with another tab: the next feed or tab switch tries again.
        if panes > 1 && !self.focus.busy.get() && !self.focus.zoomed.borrow().contains(&key) {
            self.focus.zoomed.borrow_mut().insert(key.clone());
            self.focus_zoom(&key, panes, None);
        }
    }

    /// Zooms the tab's window when every pane in it is an agent we can show
    /// as a card; a split with a shell stays as the user built it.
    /// `select` first makes that pane the active one (keeping the zoom).
    fn focus_zoom(self: &Rc<Self>, key: &str, agents: usize, select: Option<String>) {
        if self.focus.busy.replace(true) || !self.writable() {
            self.focus.busy.set(false);
            return;
        }
        let tmux = self.tmux.clone();
        let target = format!("={key}:");
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || -> Result<(), String> {
                if let Some(pane) = &select {
                    tmux.mutate(&["select-pane", "-Z", "-t", pane])
                        .map_err(|e| format!("{e:?}"))?;
                }
                let out = tmux
                    .read(&[
                        "list-panes",
                        "-t",
                        &target,
                        "-F",
                        "#{pane_id} #{pane_active} #{window_zoomed_flag}",
                    ])
                    .map_err(|e| format!("{e:?}"))?;
                let rows: Vec<Vec<&str>> =
                    out.stdout.lines().map(|l| l.split_whitespace().collect()).collect();
                let zoomed = rows.iter().any(|r| r.get(2) == Some(&"1"));
                let active = rows
                    .iter()
                    .find(|r| r.get(1) == Some(&"1"))
                    .and_then(|r| r.first().copied());
                if rows.len() > 1 && rows.len() <= agents && !zoomed && let Some(active) = active {
                    tmux.mutate(&["resize-pane", "-Z", "-t", active])
                        .map_err(|e| format!("{e:?}"))?;
                }
                Ok(())
            },
            move |result| {
                let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                    return;
                };
                app.focus.busy.set(false);
                if let Err(error) = result {
                    app.status.set_text(&format!("Foco: {error}"));
                }
                app.refresh_visible_geometry();
            },
        );
    }

    fn focus_unzoom(self: &Rc<Self>, key: &str) {
        if !self.writable() {
            return;
        }
        let tmux = self.tmux.clone();
        let target = format!("={key}:");
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || {
                let zoomed = tmux
                    .read(&["display-message", "-p", "-t", &target, "#{window_zoomed_flag}"])
                    .is_ok_and(|o| o.stdout.trim() == "1");
                if zoomed {
                    let _ = tmux.mutate(&["resize-pane", "-Z", "-t", &target]);
                }
            },
            move |()| {
                if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) {
                    app.refresh_visible_geometry();
                }
            },
        );
    }

    fn focus_open(self: &Rc<Self>, card: &Card) {
        match &card.pane {
            Some(pane) => {
                let agents = self
                    .feed
                    .feed
                    .borrow()
                    .as_ref()
                    .and_then(|f| f.sessions.get(&card.session).map(|e| e.pane_list.len()))
                    .unwrap_or(1);
                self.focus.zoomed.borrow_mut().insert(card.session.clone());
                self.focus_zoom(&card.session, agents, Some(pane.clone()));
                if let Some(term) = self.terms.borrow().get(&card.session) {
                    term.widget().grab_focus();
                }
            }
            None => self.select(&card.session),
        }
    }
}
