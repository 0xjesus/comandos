//! Conexiones de la tira y del gesto compartido con la aplicación propietaria.
use super::*;
impl App {
    pub(super) fn apply_theme_layers(
        &self,
        theme: &crate::theme::ThemeTokens,
        style: &str,
    ) -> Result<(), glib::Error> {
        let layers = ui::theme_layers::ThemeLayers::new(theme, style)?;
        self.theme_layers.borrow_mut().take();
        layers.install();
        *self.theme_layers.borrow_mut() = Some(layers);
        *self.applied_theme.borrow_mut() = Some(theme.clone());
        *self.applied_button_style.borrow_mut() = Some(style.to_string());
        Ok(())
    }
    pub(super) fn paint_theme(&self, theme: &crate::theme::ThemeTokens, style: &str) {
        if let Err(error) = self.apply_theme_layers(theme, style) {
            self.status.set_text(&format!("Tema: {error}"));
            return;
        }
        self.header.paint(theme);
        if let Err(error) = self.t17.shelf.paint(theme) {
            self.status.set_text(&format!("Shelf theme: {error}"));
        }
        let dim = theme
            .values
            .get("dim")
            .and_then(Value::as_str)
            .unwrap_or("#AAAAAA");
        for row in [
            self.tab_layout.start(),
            self.tab_layout.actions(),
            self.tab_layout.navigation(),
        ] {
            for child in row.children() {
                if let Ok(button) = child.downcast::<gtk::Button>() {
                    let widget_name = button.widget_name();
                    let name = match widget_name.as_str() {
                        "tab-cycle-prev" => "chevron-left",
                        "tab-cycle-next" => "chevron-right",
                        "tabs-layout-rows" => "rows",
                        "left-panel-toggle" => "panel-left",
                        "noise-toggle" if self.focus.noise_hidden.get() => "eye-off",
                        "noise-toggle" => "eye",
                        other => other,
                    };
                    let pixels = if name.starts_with("chevron-") {
                        20
                    } else if matches!(name, "rows" | "panel-left" | "columns" | "eye" | "eye-off") {
                        18
                    } else {
                        16
                    };
                    let color = if (name == "rows" && self.tab_layout.rows())
                        || (name == "columns" && self.focus.enabled.get())
                        || name == "eye-off"
                    {
                        theme
                            .values
                            .get("brand")
                            .and_then(Value::as_str)
                            .unwrap_or(dim)
                    } else {
                        dim
                    };
                    button.set_image(Some(&ui::icons::image(name, pixels, color)));
                }
            }
        }
        self.paint_left_button(self.t18.state.borrow().left_hidden);
        for tab in self
            .labels
            .borrow()
            .values()
            .chain(self.group_labels.borrow().values())
        {
            if let Some(close) = &tab.close {
                close.set_image(Some(&ui::icons::image("close", 12, dim)));
            }
        }
    }
    pub(super) fn apply_tabs_layout(self: &Rc<Self>, mode: &str, persist: bool) {
        let mode = if mode == "rows" { "rows" } else { "row" };
        self.tab_layout.set_rows(mode == "rows");
        for row in [
            self.tab_layout.start(),
            self.tab_layout.actions(),
            self.tab_layout.navigation(),
        ] {
            for child in row.children() {
                match child.widget_name().as_str() {
                    "tab-cycle-prev" | "tab-cycle-next" => child.set_visible(true),
                    "tabs-layout-rows" => {
                        if let Ok(button) = child.downcast::<gtk::Button>() {
                            let color = self
                                .applied_theme
                                .borrow()
                                .as_ref()
                                .and_then(|t| {
                                    t.values.get(if mode == "rows" { "brand" } else { "dim" })
                                })
                                .and_then(Value::as_str)
                                .unwrap_or("#AAAAAA")
                                .to_string();
                            button.set_image(Some(&ui::icons::image("rows", 18, &color)));
                            button.set_tooltip_text(Some(match (mode, self.english) {
                                ("rows", true) => "One row",
                                ("rows", false) => "Una fila",
                                (_, true) => "Several rows",
                                _ => "Varias filas",
                            }));
                        }
                    }
                    _ => (),
                }
            }
        }
        if persist && self.writable() {
            let dash = self.dash.clone();
            let closed = self.closed.clone();
            let ticket = self.t18.scope.ticket();
            self.jobs.spawn(
                move || {
                    if closed.load(Ordering::Acquire) || !ticket.current() {
                        return Err(crate::dash_client::DashError::Cancelled);
                    }
                    dash.post(
                        "/prefs-set",
                        &json!({"tabs_layout":mode}),
                        Duration::from_secs(5),
                    )
                },
                |result| {
                    if let Err(error) = result {
                        eprintln!("tabs_layout: {error:?}");
                    }
                },
            );
        }
    }
    pub(super) fn device_id(&self) -> String {
        ui::presence::device_id(glib::host_name().as_str())
    }
    pub(super) fn restore_device_focus(self: &Rc<Self>) {
        if self.focus_restoring.replace(true) {
            return;
        }
        let path = format!("/workspace/client?deviceId={}", self.device_id());
        let dash = self.dash.clone();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || dash.get(&path, Duration::from_secs(3)),
            move |result| {
                if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) {
                    let key = result.ok().and_then(|v| {
                        v.get("activeTabId")
                            .unwrap_or(&Value::Null)
                            .as_str()
                            .map(str::to_string)
                    });
                    if !app.interacted.get()
                        && let Some(key) = key.as_deref()
                    {
                        app.select(key);
                    }
                    *app.saved_focus.borrow_mut() = key;
                    app.focus_ready.set(true);
                }
            },
        );
    }
    pub(super) fn save_device_focus(self: &Rc<Self>, key: &str) {
        if !self.writable()
            || !self.focus_ready.get()
            || self.saved_focus.borrow().as_deref() == Some(key)
            || !self
                .focus_retry
                .borrow()
                .allows(key, std::time::Instant::now())
        {
            return;
        }
        *self.saved_focus.borrow_mut() = Some(key.into());
        let key = key.to_string();
        let Some(body) = ui::presence::Presence::focus_payload(
            &self.device_id(),
            &key,
            self.focus_ready.get(),
            self.cfg.mode(),
        ) else {
            return;
        };
        let dash = self.dash.clone();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || dash.post("/workspace/client", &body, Duration::from_secs(3)),
            move |result| {
                if let Some(app) = weak.upgrade()
                    && app.saved_focus.borrow().as_deref() == Some(&key)
                {
                    if matches!(result, Ok((200, _))) {
                        app.focus_retry.borrow_mut().succeeded(&key);
                    } else {
                        app.focus_retry
                            .borrow_mut()
                            .failed(&key, std::time::Instant::now());
                        app.saved_focus.borrow_mut().take();
                    }
                }
            },
        );
    }
    pub(super) fn install_pane_position(self: &Rc<Self>) {
        let state = self.state.clone();
        let weak = Rc::downgrade(self);
        let ticket = self.t18.pane_scope.ticket();
        self.jobs.spawn(
            move || state.read_ui_document("app-pane-position.json"),
            move |saved| {
                if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire))
                    && ticket.current()
                {
                    match saved {
                        Ok(doc) => {
                            *app.t18.pane_doc.borrow_mut() = doc;
                            app.pane_saved.set(Some(0));
                            app.initialize_pane(app.paned.allocated_width());
                        }
                        Err(error) => app.status.set_text(&format!("Pane position: {error:?}")),
                    }
                }
            },
        );
        let weak = Rc::downgrade(self);
        self.paned.connect_size_allocate(move |_, a| {
            if let Some(app) = weak.upgrade() {
                app.initialize_pane(a.width());
            }
        });
        let weak = Rc::downgrade(self);
        self.paned.connect_position_notify(move |paned| {
            if let Some(app) = weak.upgrade().filter(|a| {
                a.writable()
                    && a.pane_initialized.get()
                    && !a.t18.state.borrow().left_hidden
                    && a.t18.left.is_visible()
                    && !a.t18.mosaic.borrow().on
            }) {
                app.pane_saved.set(Some(paned.position()));
                app.queue_ui_save("app-pane-position.json", 250);
            }
        });
        let weak = Rc::downgrade(self);
        self.window.connect_window_state_event(move |_, event| {
            if event.changed_mask().contains(gdk::WindowState::MAXIMIZED)
                && let Some(app) = weak.upgrade().filter(|a| a.pane_initialized.get())
            {
                app.queue_ui_save("app-pane-position.json", 250);
            }
            glib::Propagation::Proceed
        });
    }
    /// The pane document as it should be saved now: the left panel width and
    /// whether the window is maximized, over whatever else `old` holds.
    pub(super) fn pane_document(&self, old: &Value) -> Value {
        let mut doc = if old.is_object() {
            old.clone()
        } else {
            json!({})
        };
        if let Some(map) = doc.as_object_mut() {
            map.insert(
                "position".into(),
                json!(self.pane_saved.get().unwrap_or(self.paned.position())),
            );
            map.insert("maximized".into(), json!(self.window.is_maximized()));
        }
        doc
    }
    fn initialize_pane(&self, width: i32) {
        if self.pane_initialized.get() || self.pane_saved.get().is_none() {
            return;
        }
        let position = ui::side::pane_initial_position(width, &self.t18.pane_doc.borrow());
        if let Some(position) = position {
            self.pane_initialized.set(true);
            self.pane_saved.set(Some(position));
            self.paned.set_position(position);
        }
    }
    pub(super) fn sync_strip(self: &Rc<Self>) {
        let doc = self.workspace_doc.borrow().clone();
        let order = self.registry.borrow().ordered_keys();
        let mut used = BTreeSet::new();
        let mut entries = Vec::new();
        let mut group_keys = BTreeSet::new();
        if let Some(groups) = doc.get("groups").and_then(Value::as_array) {
            for group in groups {
                let ids =
                    comandos_core::workspace::tab_ids(group.get("tree").unwrap_or(&Value::Null))
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|key| self.labels.borrow().contains_key(key))
                        .collect::<Vec<_>>();
                if ids.is_empty() {
                    continue;
                }
                used.extend(ids.iter().cloned());
                if ids.len() == 1 {
                    if let Some(key) = ids.first()
                        && let Some(tab) = self.labels.borrow().get(key)
                    {
                        entries.push((key.clone(), tab.widget()));
                    }
                } else if let Some(gid) = group.get("id").and_then(Value::as_str) {
                    let key = format!("group:{gid}");
                    group_keys.insert(key.clone());
                    let label = ids
                        .first()
                        .and_then(|key| {
                            self.labels
                                .borrow()
                                .get(key)
                                .map(|tab| tab.text.text().to_string())
                        })
                        .unwrap_or_else(|| gid.into());
                    let text = format!("{label}  {} tabs", ids.len());
                    if !self.group_labels.borrow().contains_key(&key) {
                        let tab = ui::tab_label::TabLabel::new(&text, true, true);
                        self.install_drag(&key, tab.name.upcast_ref());
                        let weak = Rc::downgrade(self);
                        let k = ids.first().cloned().unwrap_or_default();
                        tab.name.connect_button_press_event(move |_, event| {
                            if event.button() == 1
                                && let Some(app) = weak.upgrade()
                            {
                                app.select(&k);
                            }
                            glib::Propagation::Proceed
                        });
                        if let Some(close) = &tab.close {
                            let weak = Rc::downgrade(self);
                            let group = gid.to_string();
                            close.connect_clicked(move |_| {
                                if let Some(app) = weak.upgrade() {
                                    app.close_group(&group);
                                }
                            });
                        }
                        self.group_labels.borrow_mut().insert(key.clone(), tab);
                    }
                    if let Some(tab) = self.group_labels.borrow().get(&key) {
                        tab.set_text(&text);
                        entries.push((key, tab.widget()));
                    }
                }
            }
        }
        for key in order {
            if !used.contains(&key)
                && let Some(tab) = self.labels.borrow().get(&key)
            {
                entries.push((key, tab.widget()));
            }
        }
        self.group_labels
            .borrow_mut()
            .retain(|key, _| group_keys.contains(key));
        // Local is the first card even when the saved notebook places it last.
        let local_group = doc
            .get("groups")
            .and_then(Value::as_array)
            .and_then(|groups| {
                groups
                    .iter()
                    .find(|group| {
                        comandos_core::workspace::tab_ids(&group["tree"])
                            .is_ok_and(|ids| ids.iter().any(|id| id == "local"))
                    })
                    .and_then(|group| group["id"].as_str())
                    .map(|id| format!("group:{id}"))
            });
        entries.sort_by_key(|(key, _)| key != "local" && Some(key) != local_group.as_ref());
        self.tab_layout.set_items(entries);
        self.paint_favorites();
        if let Some(key) = self.current_session() {
            self.paint_selected(&key);
        }
    }
    pub(super) fn paint_selected(&self, key: &str) {
        let doc = self.workspace_doc.borrow();
        let owner = doc
            .get("groups")
            .and_then(Value::as_array)
            .and_then(|groups| {
                groups.iter().find(|group| {
                    comandos_core::workspace::tab_ids(group.get("tree").unwrap_or(&Value::Null))
                        .is_ok_and(|ids| ids.len() > 1 && ids.iter().any(|k| k == key))
                })
            })
            .and_then(|g| g.get("id").unwrap_or(&Value::Null).as_str())
            .map(|id| format!("group:{id}"));
        self.tab_layout.set_current(owner.as_deref().unwrap_or(key));
    }
    pub(super) fn layer_point(&self, root: (f64, f64)) -> (f64, f64) {
        if let Some(window) = self.window.window() {
            let (_, x, y) = window.origin();
            let offset = self
                .drag_layer
                .widget()
                .translate_coordinates(&self.window, 0, 0)
                .unwrap_or((0, 0));
            crate::workspace_view::root_to_layer(
                root,
                (x as f64, y as f64),
                (offset.0 as f64, offset.1 as f64),
            )
        } else {
            (-1., -1.)
        }
    }
    pub(super) fn drag_motion(self: &Rc<Self>, root: (f64, f64)) -> bool {
        let was = self.drag_layer.active();
        let point = self.layer_point(root);
        if !self.drag_layer.0.gesture.borrow_mut().motion(root, point) {
            return false;
        }
        if !was {
            let source = self.drag_layer.0.gesture.borrow().source.clone();
            let page = self.drag_layer.0.gesture.borrow().press_page;
            self.workspace.widget().set_current_page(page);
            self.dragging.set(true);
            if let Some(source) = source {
                *self.drag_layer.0.source_label.borrow_mut() = self
                    .tab_layout
                    .entries()
                    .into_iter()
                    .find(|(key, _)| key == &source)
                    .map(|(_, label)| label);
                *self.drag_layer.0.text.borrow_mut() = self
                    .labels
                    .borrow()
                    .get(&source)
                    .map(|tab| tab.text.text().to_string())
                    .unwrap_or_else(|| source.trim_start_matches("group:").into());
            }
            if let (Some(seat), Some(window)) = (
                gdk::Display::default().and_then(|d| d.default_seat()),
                self.window.window(),
            ) {
                seat.grab(
                    &window,
                    gdk::SeatCapabilities::POINTER,
                    false,
                    None,
                    None,
                    None,
                );
            }
            self.drag_layer.widget().show();
        }
        let target = self.drag_target(point);
        self.drag_layer
            .0
            .gesture
            .borrow_mut()
            .docking(target.is_some());
        *self.drag_layer.0.target.borrow_mut() = target;
        self.drag_layer.widget().queue_draw();
        true
    }
    pub(super) fn measured_layout(&self) -> Value {
        let entries = self
            .tab_layout
            .entries()
            .into_iter()
            .filter_map(|(key, w)| {
                w.is_mapped()
                    .then(|| self.drag_layer.rect(&w).map(|r| json!([key, r])))
                    .flatten()
            })
            .collect::<Vec<_>>();
        let strip_h = entries
            .iter()
            .filter_map(|e| e.get(1).and_then(Value::as_array))
            .filter_map(|r| Some(r.get(1)?.as_f64()? + r.get(3)?.as_f64()?))
            .reduce(f64::max)
            .map_or(0., |h| h + 6.);
        let width = self.drag_layer.widget().allocated_width().max(1) as f64;
        let current = self
            .workspace
            .widget()
            .current_page()
            .and_then(|n| self.workspace.widget().nth_page(Some(n)));
        let area = current
            .as_ref()
            .and_then(|w| self.drag_layer.rect(w))
            .unwrap_or([0., strip_h, width, 1.]);
        let group = current
            .as_ref()
            .and_then(|w| self.workspace.group_of_page(w));
        let ids = group
            .as_deref()
            .and_then(|gid| {
                self.workspace_doc
                    .borrow()
                    .get("groups")
                    .and_then(Value::as_array)
                    .and_then(|groups| {
                        groups
                            .iter()
                            .find(|g| g.get("id").unwrap_or(&Value::Null) == gid)
                    })
                    .cloned()
            })
            .and_then(|g| {
                comandos_core::workspace::tab_ids(g.get("tree").unwrap_or(&Value::Null)).ok()
            })
            .unwrap_or_default();
        let mut leaves = serde_json::Map::new();
        for key in &ids {
            if let Some(widget) = self.workspace.leaf(key)
                && widget.is_mapped()
                && let Some(rect) = self.drag_layer.rect(&widget)
            {
                leaves.insert(key.clone(), json!(rect));
            }
        }
        json!({"strip":[0.,0.,width,strip_h],"entries":entries,"area":area,"leaves":leaves,"active":group,"activeTabs":ids})
    }
    pub(super) fn drag_target(&self, point: (f64, f64)) -> Option<Value> {
        let layout = self.measured_layout();
        let source = self.drag_layer.0.gesture.borrow().source.clone()?;
        self.drag_layer.0.trays.borrow_mut().clear();
        let strip_bottom = layout
            .pointer("/strip/3")
            .and_then(Value::as_f64)
            .unwrap_or(0.);
        if source != "local" && !source.starts_with("group:") && point.1 >= strip_bottom + 30. {
            let trays = crate::workspace_view::tray_rects(
                self.drag_layer.widget().allocated_width() as f64,
                self.drag_layer.widget().allocated_height() as f64,
            );
            let mark = crate::workspace_view::tray_at(&trays, point.0, point.1, strip_bottom);
            *self.drag_layer.0.trays.borrow_mut() = trays.clone();
            if let Some(mark) = mark
                && let Some((_, rect)) = trays.iter().find(|(m, _)| m == &mark)
            {
                return Some(json!({"kind":"mark","mark":mark,"rect":rect}));
            }
        }
        let moved = if let Some(group) = source.strip_prefix("group:") {
            self.workspace_doc
                .borrow()
                .get("groups")
                .and_then(Value::as_array)
                .and_then(|groups| {
                    groups
                        .iter()
                        .find(|g| g.get("id").unwrap_or(&Value::Null) == group)
                })
                .and_then(|g| {
                    comandos_core::workspace::tab_ids(g.get("tree").unwrap_or(&Value::Null)).ok()
                })
                .unwrap_or_default()
                .into_iter()
                .collect()
        } else {
            BTreeSet::from([source])
        };
        let hit = crate::workspace_view::dock_hit(&layout, point.0, point.1, &moved)?;
        let mut target = hit.to_json();
        if let crate::workspace_view::DockHit::Bar { index } = hit {
            let entries = layout.get("entries").unwrap_or(&Value::Null).as_array()?;
            let key = entries
                .get(index)
                .and_then(|e| e.get(0))
                .and_then(Value::as_str);
            let groups = self.workspace_doc.borrow();
            let groups = groups.get("groups").and_then(Value::as_array)?;
            let position = key
                .and_then(|key| {
                    groups.iter().position(|g| {
                        if let Some(gid) = key.strip_prefix("group:") {
                            g.get("id").unwrap_or(&Value::Null) == gid
                        } else {
                            comandos_core::workspace::tab_ids(g.get("tree").unwrap_or(&Value::Null))
                                .is_ok_and(|ids| ids.iter().any(|id| id == key))
                        }
                    })
                })
                .unwrap_or(groups.len());
            let x = entries
                .get(index)
                .and_then(|e| e.pointer("/1/0"))
                .and_then(Value::as_f64)
                .or_else(|| {
                    entries.last().and_then(|e| {
                        Some(e.pointer("/1/0")?.as_f64()? + e.pointer("/1/2")?.as_f64()?)
                    })
                })
                .unwrap_or(0.);
            let width = self
                .drag_layer
                .0
                .source_label
                .borrow()
                .as_ref()
                .map(|label| label.allocated_width())
                .filter(|w| *w > 12)
                .unwrap_or(96) as f64;
            if let Some(object) = target.as_object_mut() {
                object.insert("docIndex".into(), json!(position));
                object.insert("slot".into(), json!(true));
                object.insert(
                    "rect".into(),
                    json!([x - width / 2., 2., width, (strip_bottom - 8.).max(8.)]),
                );
            }
        }
        Some(target)
    }
    pub(super) fn cancel_drag(self: &Rc<Self>) {
        let page = self.drag_layer.0.gesture.borrow_mut().cancel();
        self.end_drag();
        if let Some(page) = page {
            self.workspace.widget().set_current_page(Some(page));
        }
        let pending = self.pending_workspace.borrow_mut().take();
        if let Some(doc) = pending {
            self.apply_workspace(&doc);
        }
        self.flush_resize();
    }
    fn end_drag(self: &Rc<Self>) {
        if let Some(seat) = gdk::Display::default().and_then(|d| d.default_seat()) {
            seat.ungrab();
        }
        self.drag_layer.clear();
        self.dragging.set(false);
    }
    pub(super) fn drop_drag(self: &Rc<Self>, target: Option<Value>) {
        let page = self.drag_layer.0.gesture.borrow().press_page;
        let source = self.drag_layer.0.gesture.borrow_mut().finish();
        self.end_drag();
        if target
            .as_ref()
            .is_none_or(|t| t.get("kind").unwrap_or(&Value::Null) == "mark")
        {
            self.workspace.widget().set_current_page(page);
        }
        if let (Some(target), Some(source)) = (target, source) {
            if target.get("kind").unwrap_or(&Value::Null) == "mark" {
                if let Some(mark) = target.get("mark").unwrap_or(&Value::Null).as_str() {
                    self.set_mark(&source, mark);
                }
            } else {
                let document = self.workspace_doc.borrow();
                let changed = if target.get("kind").unwrap_or(&Value::Null) == "bar" {
                    comandos_core::workspace::layout::detach_tab(
                        &document,
                        &source,
                        target.get("docIndex").unwrap_or(&Value::Null),
                    )
                } else {
                    comandos_core::workspace::layout::move_tab(
                        &document,
                        &source,
                        target
                            .get("target")
                            .unwrap_or(&Value::Null)
                            .as_str()
                            .unwrap_or(""),
                        target
                            .get("edge")
                            .unwrap_or(&Value::Null)
                            .as_str()
                            .unwrap_or(""),
                    )
                };
                drop(document);
                if let Ok(document) = changed {
                    let focus = (!source.starts_with("group:")).then_some(source);
                    self.commit_workspace(document, focus);
                }
            }
        }
        if !self.posting.get() {
            let pending = self.pending_workspace.borrow_mut().take();
            if let Some(pending) = pending {
                self.apply_workspace(&pending);
            }
        }
        self.flush_resize();
    }
    pub(super) fn drag_tick(self: &Rc<Self>) {
        if !self.drag_layer.active() {
            return;
        }
        let point = self.drag_layer.0.gesture.borrow().pointer;
        if let (Some(seat), Some(window)) = (
            gdk::Display::default().and_then(|d| d.default_seat()),
            self.window.window(),
        ) && let Some(pointer) = seat.pointer()
        {
            let (_, _, _, mask) = window.device_position(&pointer);
            if !mask.contains(gdk::ModifierType::BUTTON1_MASK) {
                self.drop_drag(self.drag_target(point));
                return;
            }
        }
        let layout = self.measured_layout();
        let strip_h = layout
            .pointer("/strip/3")
            .and_then(Value::as_f64)
            .unwrap_or(0.);
        let step = if point.1 >= 0. && point.1 <= strip_h + 12. {
            crate::workspace_view::strip_edge_step(
                point.0,
                self.drag_layer.widget().allocated_width() as f64,
            )
        } else {
            0
        };
        if step != 0 {
            self.tab_layout.scroll_by(step as f64 * 120.);
        } else {
            let over = layout
                .get("entries")
                .unwrap_or(&Value::Null)
                .as_array()
                .and_then(|entries| {
                    entries
                        .iter()
                        .find(|entry| {
                            entry.get(1).and_then(Value::as_array).is_some_and(|r| {
                                r.first()
                                    .and_then(Value::as_f64)
                                    .is_some_and(|x| point.0 >= x)
                                    && r.get(1)
                                        .and_then(Value::as_f64)
                                        .is_some_and(|y| point.1 >= y)
                                    && r.get(2).and_then(Value::as_f64).is_some_and(|w| {
                                        point.0
                                            <= r.first().and_then(Value::as_f64).unwrap_or(0.) + w
                                    })
                                    && r.get(3).and_then(Value::as_f64).is_some_and(|h| {
                                        point.1
                                            <= r.get(1).and_then(Value::as_f64).unwrap_or(0.) + h
                                    })
                            })
                        })
                        .and_then(|e| e.get(0))
                        .and_then(Value::as_str)
                })
                .map(ToString::to_string);
            if let Some(over) =
                over.filter(|key| self.drag_layer.0.gesture.borrow().source.as_ref() != Some(key))
            {
                if self.drag_layer.0.dwell.borrow().as_ref() == Some(&over) {
                    if let Some(gid) = over.strip_prefix("group:") {
                        self.workspace.select_group(gid);
                    } else {
                        self.select(&over);
                    }
                    self.drag_layer.0.dwell.borrow_mut().take();
                } else {
                    *self.drag_layer.0.dwell.borrow_mut() = Some(over);
                }
            } else {
                self.drag_layer.0.dwell.borrow_mut().take();
            }
        }
        *self.drag_layer.0.target.borrow_mut() = self.drag_target(point);
        self.drag_layer.widget().queue_draw();
    }
    pub(super) fn set_mark(self: &Rc<Self>, key: &str, mark: &str) {
        self.set_work_mark("session", key, &json!(mark));
    }
    pub(super) fn tab_menu(self: &Rc<Self>, key: &str, event: &gdk::EventButton) {
        self.mark_menu(key, None, event);
    }
    pub(super) fn sort_menu(self: &Rc<Self>, button: Option<&gtk::Widget>) {
        let menu = gtk::Menu::new();
        for (by, label) in [
            ("fav", "★ Favoritas primero"),
            ("recent", "⏱ Actividad reciente"),
            ("need", "▶ Te necesita primero"),
            ("alpha", "A→Z"),
        ] {
            let item = gtk::MenuItem::with_label(label);
            let weak = Rc::downgrade(self);
            item.connect_activate(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.sort_tabs(Some(by));
                }
            });
            menu.append(&item);
        }
        if self.previous_order.borrow().is_some() {
            let undo = gtk::MenuItem::with_label("↶ Deshacer");
            let weak = Rc::downgrade(self);
            undo.connect_activate(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.sort_tabs(None);
                }
            });
            menu.append(&undo);
        }
        menu.show_all();
        if let Some(button) = button {
            menu.popup_at_widget(
                button,
                gdk::Gravity::SouthWest,
                gdk::Gravity::NorthWest,
                None,
            );
        } else {
            menu.popup_at_pointer(None);
        }
    }
    pub(super) fn sort_tabs(self: &Rc<Self>, by: Option<&str>) {
        if self.writable() && self.cfg.dash_url().is_some() {
            if self.posting.replace(true) {
                return;
            }
            let body = by.map_or_else(
                || json!({"restore":self.previous_order.borrow().clone().unwrap_or_default()}),
                |by| json!({"by":by}),
            );
            let undo = by.is_none();
            let dash = self.dash.clone();
            let weak = Rc::downgrade(self);
            self.jobs.spawn(
                move || {
                    let result = dash.post("/workspace/sort", &body, Duration::from_secs(8));
                    let current = if matches!(result, Ok((200, _))) {
                        dash.get("/workspace", Duration::from_secs(3)).ok()
                    } else {
                        None
                    };
                    (result, current)
                },
                move |(result, current)| {
                    if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire))
                    {
                        app.posting.set(false);
                        app.resize_queue.borrow_mut().complete(false);
                        match result {
                            Ok((200, payload)) => {
                                *app.previous_order.borrow_mut() = if undo {
                                    None
                                } else {
                                    payload.get("previous").and_then(Value::as_array).map(|a| {
                                        a.iter()
                                            .filter_map(Value::as_str)
                                            .map(str::to_string)
                                            .collect()
                                    })
                                };
                                if let Some(current) = current {
                                    app.apply_workspace(&current);
                                }
                            }
                            result => app.status.set_text(&format!("Ordenar: {result:?}")),
                        }
                        let pending = app.pending_workspace.borrow_mut().take();
                        if let Some(pending) = pending
                            && pending
                                .get("revision")
                                .and_then(Value::as_u64)
                                .is_some_and(|r| r > app.revision.get())
                        {
                            app.apply_workspace(&pending);
                        }
                        app.flush_resize();
                    }
                },
            );
            return;
        }
        let doc = self.workspace_doc.borrow().clone();
        let before = doc
            .get("groups")
            .and_then(Value::as_array)
            .map(|groups| {
                groups
                    .iter()
                    .filter_map(|g| {
                        g.get("id")
                            .unwrap_or(&Value::Null)
                            .as_str()
                            .map(ToString::to_string)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let changed = if let Some(by) = by {
            let favorites = self.registry.borrow().favorite_keys();
            let mut info = serde_json::Map::new();
            for (key, tab) in self.labels.borrow().iter() {
                let state = self
                    .state_items
                    .borrow()
                    .get(key)
                    .cloned()
                    .unwrap_or(Value::Null);
                info.insert(key.clone(),json!({"label":tab.text.text().as_str(),"fav":favorites.contains(key),"need":comandos_core::work_marks::ai_status(state.get("status").unwrap_or(&Value::Null).as_str().unwrap_or(""))=="need","activeAt":state.get("activeAt").or_else(||state.get("active_at")).cloned().unwrap_or(json!(0))}));
            }
            comandos_core::workspace::layout::sort_groups(&doc, by, &json!(info))
        } else {
            comandos_core::workspace::layout::restore_order(
                &doc,
                self.previous_order.borrow().as_deref().unwrap_or(&[]),
            )
        };
        if let Ok(doc) = changed {
            *self.previous_order.borrow_mut() = by.map(|_| before);
            self.commit_workspace(doc, None);
        }
    }
    pub(super) fn quick_terminal(self: &Rc<Self>) {
        if !self.writable() || self.quick_busy.replace(true) {
            return;
        }
        if self.cfg.mode() == RunMode::Sandbox && self.cfg.dash_url().is_none() {
            self.quick_busy.set(false);
            self.new_terminal_in(true);
            return;
        }
        let dash = self.dash.clone();
        let quick = self.quick.clone();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || {
                quick.request(
                    || {
                        let mut bytes = [0u8; 12];
                        getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
                        Ok(format!(
                            "gtk-{}",
                            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
                        ))
                    },
                    |body| {
                        dash.post("/terminal/quick", body, Duration::from_secs(30))
                            .map_err(|e| format!("{e:?}"))
                            .and_then(|(status, value)| {
                                if status == 200 {
                                    Ok(value)
                                } else {
                                    Err(value
                                        .get("error")
                                        .and_then(Value::as_str)
                                        .unwrap_or("No se creó la terminal")
                                        .into())
                                }
                            })
                    },
                )
            },
            move |result| {
                if let Some(app) = weak.upgrade() {
                    app.quick_busy.set(false);
                    match result {
                        Ok(result) => {
                            if let Some(key) = result.get("tabId").and_then(Value::as_str) {
                                app.open_tab(
                                    key,
                                    "claude",
                                    result.get("label").and_then(Value::as_str).unwrap_or(key),
                                    true,
                                );
                            }
                        }
                        Err(e) => app.status.set_text(&e),
                    }
                }
            },
        );
    }
    pub(super) fn close_split(self: &Rc<Self>, session: &str, pane: &str) {
        if !self.writable() {
            return;
        }
        let session = session.to_string();
        let pane = pane.to_string();
        let dash = self.dash.clone();
        let weak = Rc::downgrade(self);
        let socket = self.tmux.socket_path().to_path_buf();
        self.jobs.spawn(move||crate::agent_stop::PaneCloseIntent::prepare(&socket,&session,&pane,|body|pane_post(&dash,body)),move|prepared|{if let Some(app)=weak.upgrade().filter(|a|a.writable()){match prepared{Ok(token)=>{if ui::confirm::confirm_must_answer(Some(app.window.upcast_ref()),&format!("¿Cerrar el split «{}»?",token.title),"Termina el proceso de ese panel. Se guarda una copia antes; los demás paneles siguen abiertos.","Cerrar","Cancelar")!=ui::confirm::ConfirmAnswer::Yes{return;}let dash=app.dash.clone();let closed=app.closed.clone();let weak=Rc::downgrade(&app);let socket=app.tmux.socket_path().to_path_buf();let mode=app.cfg.mode();app.jobs.spawn(move||token.finish(&socket,true,closed.load(Ordering::Acquire),mode,|body|pane_post(&dash,body)),move|result|{if let (Some(app),Err(error))=(weak.upgrade(),result){app.status.set_text(&error);}});},Err(error)=>app.status.set_text(&error)}}});
    }
    pub(super) fn close_group(self: &Rc<Self>, group: &str) {
        if !self.writable() {
            return;
        }
        let group = group.to_string();
        let path = format!(
            "/workspace/close-group?groupId={}",
            ui::webview::encode_query(&group)
        );
        let dash = self.dash.clone();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(move||dash.get(&path,Duration::from_secs(3)),move|result|{let Some(app)=weak.upgrade().filter(|a|a.writable())else{return;};match result{Ok(preview)=>{let members=preview.get("members").and_then(Value::as_array).cloned().unwrap_or_default();let n=members.iter().filter(|m|m.get("kept").and_then(Value::as_bool)!=Some(true)).count();let lines=members.iter().map(|m|format!("• {}{}",m.get("label").and_then(Value::as_str).unwrap_or(""),if m.get("kept").and_then(Value::as_bool)==Some(true){" — se queda abierta"}else{""})).collect::<Vec<_>>().join("\n");if n==0||ui::confirm::confirm_must_answer(Some(app.window.upcast_ref()),&format!("¿Cerrar {n} pestañas de este grupo?"),&format!("{lines}\n\nLas sesiones tmux siguen vivas en Recientes."),"Cerrar","Cancelar")!=ui::confirm::ConfirmAnswer::Yes{return;}let mut random=[0u8;12];if getrandom::fill(&mut random).is_err(){return;}let payload=json!({"requestId":random.iter().map(|b|format!("{b:02x}")).collect::<String>(),"groupId":group,"expectedRevision":preview.get("revision"),"members":members});let dash=app.dash.clone();let weak=Rc::downgrade(&app);app.jobs.spawn(move||dash.post("/workspace/close-group",&payload,Duration::from_secs(15)),move|result|{if let Some(app)=weak.upgrade(){match result{Ok((200,result))if result.get("ok").and_then(Value::as_bool)==Some(true)=>{},result=>app.status.set_text(&format!("Cerrar grupo: {result:?}"))}}});},Err(error)=>app.status.set_text(&format!("Cerrar grupo: {error:?}"))}});
    }
}
fn pane_post(dash: &DashClient, body: &Value) -> Result<Value, String> {
    dash.post("/terminal-panes", body, Duration::from_secs(15))
        .map_err(|e| format!("{e:?}"))
        .and_then(|(status, value)| {
            if status == 200 {
                Ok(value)
            } else {
                Err(value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("No se cerró")
                    .into())
            }
        })
}
