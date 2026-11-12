pub use crate::workspace_view::WorkspaceView;
use gtk::prelude::*;
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};
type Resize = Rc<dyn Fn(&[crate::workspace_resize::ResizeUpdate])>;
type Header = Rc<dyn Fn(&str) -> gtk::Widget>;
type PendingResize =
    std::collections::BTreeMap<(String, Vec<usize>), crate::workspace_resize::ResizeUpdate>;
#[derive(Default)]
struct RatioState {
    ratio: Cell<f64>,
    size: Cell<i32>,
    programmatic: Cell<bool>,
}
pub struct GtkWorkspace {
    root: gtk::Notebook,
    pages: RefCell<HashMap<String, gtk::Widget>>,
    focus: Rc<RefCell<Option<String>>>,
    document: RefCell<Value>,
    nodes: RefCell<HashMap<String, gtk::Widget>>,
    resize: Rc<RefCell<Option<Resize>>>,
    applying: Rc<Cell<bool>>,
    header: RefCell<Option<Header>>,
    group_pages: RefCell<Vec<(String, gtk::Widget)>>,
    ratios: RefCell<HashMap<String, Rc<RatioState>>>,
    pending_resize: Rc<RefCell<PendingResize>>,
    resize_timer: Rc<RefCell<Option<glib::SourceId>>>,
}
impl GtkWorkspace {
    pub fn new() -> Self {
        let root = gtk::Notebook::new();
        root.set_show_tabs(false);
        root.set_show_border(false);
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.style_context().add_class("workspace-root");
        Self {
            root,
            pages: RefCell::new(HashMap::new()),
            focus: Rc::new(RefCell::new(None)),
            document: RefCell::new(Value::Null),
            nodes: RefCell::new(HashMap::new()),
            resize: Rc::new(RefCell::new(None)),
            applying: Rc::new(Cell::new(false)),
            header: RefCell::new(None),
            group_pages: RefCell::new(Vec::new()),
            ratios: RefCell::new(HashMap::new()),
            pending_resize: Rc::new(RefCell::new(Default::default())),
            resize_timer: Rc::new(RefCell::new(None)),
        }
    }
    pub fn shutdown(&self) {
        if let Some(source) = self.resize_timer.borrow_mut().take() {
            source.remove();
        }
        self.pending_resize.borrow_mut().clear();
        self.resize.borrow_mut().take();
    }
    pub fn widget(&self) -> &gtk::Notebook {
        &self.root
    }
    pub fn is_applying(&self) -> bool {
        self.applying.get()
    }
    /// Registered tab keys survive reconstruction of the group's GTK container.
    pub fn page_key(&self, page: &gtk::Widget) -> Option<String> {
        self.pages
            .borrow()
            .iter()
            .filter(|(_, widget)| *widget == page || widget.is_ancestor(page))
            .map(|(key, _)| key.clone())
            .min()
    }
    pub fn page_index(&self, key: &str) -> Option<u32> {
        let pages = self.pages.borrow();
        let widget = pages.get(key)?;
        if !widget.is_ancestor(&self.root) {
            return None;
        }
        let mut child = widget.clone();
        while let Some(parent) = child.parent() {
            if parent == self.root.clone().upcast::<gtk::Widget>() {
                break;
            }
            child = parent;
        }
        self.root.page_num(&child)
    }
    pub fn focus_page(&self, page: &gtk::Widget) {
        let index = self.root.page_num(page);
        let key = self
            .focused()
            .filter(|key| self.page_index(key) == index)
            .or_else(|| self.page_key(page));
        if let Some(key) = key {
            self.select(&key);
        }
    }
    pub fn resize_pending(&self) -> bool {
        !self.pending_resize.borrow().is_empty() || self.resize_timer.borrow().is_some()
    }
    pub fn on_header(&self, callback: Header) {
        *self.header.borrow_mut() = Some(callback);
    }
    pub fn group_of_page(&self, page: &gtk::Widget) -> Option<String> {
        self.group_pages
            .borrow()
            .iter()
            .find(|(_, w)| w == page)
            .map(|(id, _)| id.clone())
    }
    pub fn select_group(&self, id: &str) -> bool {
        if let Some((_, page)) = self.group_pages.borrow().iter().find(|(gid, _)| gid == id)
            && let Some(index) = self.root.page_num(page)
        {
            self.root.set_current_page(Some(index));
            true
        } else {
            false
        }
    }
    pub fn leaf(&self, key: &str) -> Option<gtk::Widget> {
        let widget = self.pages.borrow().get(key)?.clone();
        let parent = widget
            .parent()
            .filter(|p| p.style_context().has_class("ws-leaf"));
        Some(parent.unwrap_or(widget))
    }
    pub fn diagnostic_splits(&self) -> Vec<(String, Value, gtk::Widget)> {
        fn walk(
            group: &str,
            node: &Value,
            widget: &gtk::Widget,
            out: &mut Vec<(String, Value, gtk::Widget)>,
        ) {
            let Some(identity) = crate::workspace_view::resize_update(group, node, 0.) else {
                return;
            };
            let Some(paned) = widget.downcast_ref::<gtk::Paned>() else {
                return;
            };
            out.push((
                format!("split:{group}:{}", serde_json::json!(identity.path)),
                serde_json::json!({"group":group,"path":identity.path,"axis":node.get("axis")}),
                widget.clone(),
            ));
            if let Some(child) = paned.child1() {
                walk(group, &node["first"], &child, out);
            }
            if let Some(child) = paned.child2() {
                walk(group, &node["second"], &child, out);
            }
        }
        let present = self.pages.borrow().keys().cloned().collect();
        let doc = self.document.borrow();
        let mut out = Vec::new();
        for (group, widget) in self.group_pages.borrow().iter() {
            if let Some(source) = doc
                .get("groups")
                .and_then(Value::as_array)
                .and_then(|groups| {
                    groups
                        .iter()
                        .find(|source| source.get("id").and_then(Value::as_str) == Some(group))
                })
            {
                let projected = crate::workspace_view::prune_for_view(&source["tree"], &present);
                walk(group, &projected, widget, &mut out);
            }
        }
        out
    }
    pub fn on_resize(&self, callback: Resize) {
        *self.resize.borrow_mut() = Some(callback);
    }
    pub fn register_tab(&self, key: impl Into<String>, widget: &gtk::Widget) {
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        let key = key.into();
        let focus = self.focus.clone();
        let focus_key = key.clone();
        let root = self.root.downgrade();
        widget.connect_focus_in_event(move |widget, _| {
            if let Some(root) = root.upgrade() {
                paint_leaf_focus(root.upcast_ref(), widget);
            }
            *focus.borrow_mut() = Some(focus_key.clone());
            glib::Propagation::Proceed
        });
        let replaced = self.pages.borrow_mut().insert(key, widget.clone());
        // A new key changes its group's signature by itself; only a replaced
        // widget can still sit inside a cached node.
        if replaced.is_some_and(|old| &old != widget) {
            self.nodes.borrow_mut().clear();
        }
    }
    /// Child terminals inside an overlay report focus to this existing workspace owner.
    pub fn note_focus(&self, key: &str) {
        if let Some(widget) = self.pages.borrow().get(key) {
            paint_leaf_focus(self.root.upcast_ref(), widget);
            *self.focus.borrow_mut() = Some(key.into());
        }
    }
    pub fn refresh(&self) {
        let doc = self.document.borrow().clone();
        *self.document.borrow_mut() = Value::Null;
        self.apply(&doc);
    }
    pub fn remove_tab(&self, key: &str) {
        self.applying.set(true);
        let removed = self.pages.borrow_mut().remove(key);
        // The other groups keep their cached nodes, so only this tab's page changes.
        let doc = self.document.borrow().clone();
        *self.document.borrow_mut() = Value::Null;
        self.apply(&doc);
        if let Some(widget) = removed {
            detach(&widget);
        }
        self.applying.set(false);
    }
    pub fn apply(&self, doc: &Value) {
        if self.document.borrow().eq(doc) {
            return;
        }
        if comandos_core::workspace::validate_document(doc).is_err() {
            return;
        }
        self.applying.set(true);
        let mut used = std::collections::HashSet::new();
        let present = self.pages.borrow().keys().cloned().collect();
        if let Some(groups) = doc["groups"].as_array() {
            for group in groups {
                collect_keys(
                    group["id"].as_str().unwrap_or("group"),
                    &crate::workspace_view::prune_for_view(&group["tree"], &present),
                    Vec::new(),
                    &mut used,
                );
            }
        }
        self.nodes.borrow_mut().retain(|key, _| used.contains(key));
        self.ratios.borrow_mut().retain(|key, _| used.contains(key));
        let group_shapes = |doc: &Value| {
            doc.get("groups").and_then(Value::as_array).map(|groups| {
                groups
                    .iter()
                    .map(|group| {
                        (
                            group["id"].clone(),
                            signature(&crate::workspace_view::prune_for_view(
                                &group["tree"],
                                &present,
                            )),
                        )
                    })
                    .collect::<Vec<_>>()
            })
        };
        let same_shapes = group_shapes(doc) == group_shapes(&self.document.borrow());
        let root: gtk::Widget = self.root.clone().upcast();
        let shown = self
            .root
            .current_page()
            .and_then(|index| self.root.nth_page(Some(index)))
            .and_then(|page| self.group_of_page(&page));
        self.group_pages.borrow_mut().clear();
        let mut wanted: Vec<(gtk::Widget, String)> = Vec::new();
        if let Some(groups) = doc["groups"].as_array() {
            for group in groups {
                let group_id = group["id"].as_str().unwrap_or("group");
                let present = self.pages.borrow().keys().cloned().collect();
                let tree = crate::workspace_view::prune_for_view(&group["tree"], &present);
                if tree.is_null() {
                    continue;
                }
                let node = self.build_node(group_id, &tree, Vec::new());
                self.group_pages
                    .borrow_mut()
                    .push((group_id.into(), node.clone()));
                wanted.push((node, group_id.into()));
            }
        }
        let documented = doc
            .get("groups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|g| comandos_core::workspace::tab_ids(&g["tree"]).unwrap_or_default())
            .collect::<std::collections::BTreeSet<_>>();
        let mut loose = self
            .pages
            .borrow()
            .iter()
            .filter(|(key, _)| !documented.contains(*key))
            .map(|(key, widget)| (self.root.page_num(widget), key.clone(), widget.clone()))
            .collect::<Vec<_>>();
        loose.sort_by_key(|(index, key, _)| (index.is_none(), *index, key.clone()));
        wanted.extend(loose.into_iter().map(|(_, key, widget)| (widget, key)));
        // Pages change one by one. Emptying the notebook made each following
        // page the current one in turn: every terminal was mapped and reread.
        for (widget, label) in &wanted {
            if widget.parent().as_ref() != Some(&root) {
                detach(widget);
                self.root
                    .append_page(widget, Some(&gtk::Label::new(Some(label))));
            }
        }
        self.root.show_all();
        if let Some(index) = shown
            .and_then(|id| {
                self.group_pages
                    .borrow()
                    .iter()
                    .find(|(group, _)| *group == id)
                    .map(|(_, node)| node.clone())
            })
            .and_then(|node| self.root.page_num(&node))
            && self.root.current_page() != Some(index)
        {
            self.root.set_current_page(Some(index));
        }
        for child in self.root.children() {
            if !wanted.iter().any(|(widget, _)| *widget == child) {
                self.root.remove(&child);
            }
        }
        for (index, (widget, _)) in wanted.iter().enumerate() {
            if self.root.page_num(widget) != Some(index as u32) {
                self.root.reorder_child(widget, Some(index as u32));
            }
        }
        *self.document.borrow_mut() = doc.clone();
        self.applying.set(false);
        self.restore_focus_after_apply(doc, same_shapes);
    }
    fn shown_index(&self) -> Option<u32> {
        self.root.current_page()
    }
    /// A terminal of the page on screen.
    fn shown_key(&self) -> Option<String> {
        let page = self.root.nth_page(Some(self.shown_index()?))?;
        self.page_key(&page)
    }
    fn restore_focus_after_apply(&self, doc: &Value, same_shapes: bool) {
        let previous = self.focused();
        // Metadata refreshes must leave focus with an open popover or modal.
        // Structural rebuilds and removed terminals still restore valid focus.
        if same_shapes
            && previous
                .as_deref()
                .is_some_and(|key| self.page_index(key).is_some())
        {
            return;
        }
        // The page on screen wins: focus remembered on another page must not
        // move the view when some other tab was the one that changed.
        let current = self.shown_index();
        let target = previous
            .filter(|key| current.is_some() && self.page_index(key) == current)
            .or_else(|| self.shown_key())
            .or_else(|| {
                doc.get("groups")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .flat_map(|group| {
                        comandos_core::workspace::tab_ids(&group["tree"]).unwrap_or_default()
                    })
                    .find(|key| self.pages.borrow().contains_key(key))
            });
        *self.focus.borrow_mut() = None;
        if let Some(target) = target {
            self.select(&target);
        }
    }
    pub fn select(&self, key: &str) -> bool {
        let pages = self.pages.borrow();
        let Some(widget) = pages.get(key) else {
            return false;
        };
        if !widget.is_ancestor(&self.root) {
            return false;
        }
        let mut child = widget.clone();
        while let Some(parent) = child.parent() {
            if parent == self.root.clone().upcast::<gtk::Widget>() {
                break;
            }
            child = parent;
        }
        if let Some(index) = self.root.page_num(&child)
            && self.root.current_page() != Some(index)
        {
            self.root.set_current_page(Some(index));
        }
        widget.grab_focus();
        *self.focus.borrow_mut() = Some(key.into());
        for (id, page) in pages.iter() {
            if let Some(leaf) = page
                .parent()
                .filter(|p| p.style_context().has_class("ws-leaf"))
            {
                if id == key {
                    leaf.style_context().add_class("focused");
                } else {
                    leaf.style_context().remove_class("focused");
                }
            }
        }
        true
    }
    pub fn commit(&self, doc: &Value, focus: Option<&str>) {
        self.apply(doc);
        if let Some(key) = focus {
            self.select(key);
        }
    }
    pub fn focused(&self) -> Option<String> {
        self.focus.borrow().clone()
    }
    fn build_node(&self, group: &str, node: &Value, path: Vec<usize>) -> gtk::Widget {
        let cache_key = format!("{group}:{path:?}:{}", signature(node));
        let cached = self.nodes.borrow().get(&cache_key).cloned();
        if let Some(widget) = cached {
            if let Some(paned) = widget.downcast_ref::<gtk::Paned>() {
                if let Some(state) = self.ratios.borrow().get(&cache_key) {
                    state
                        .ratio
                        .set(node["ratio"].as_f64().unwrap_or(0.5).clamp(0.1, 0.9));
                    state.programmatic.set(true);
                }
                let size = if paned.orientation() == gtk::Orientation::Horizontal {
                    paned.allocated_width()
                } else {
                    paned.allocated_height()
                };
                if size > 1 {
                    paned.set_position(
                        (f64::from(size) * node.get("ratio").and_then(Value::as_f64).unwrap_or(0.5))
                            .round() as i32,
                    );
                }
                if let Some(state) = self.ratios.borrow().get(&cache_key) {
                    state.programmatic.set(false);
                }
                // Recurse to apply ratios of cached descendants without reparenting their leaves.
                let mut first_path = path.clone();
                first_path.push(0);
                let mut second_path = path.clone();
                second_path.push(1);
                self.build_node(group, &node["first"], first_path);
                self.build_node(group, &node["second"], second_path);
            }
            return widget;
        }
        let widget = match node["type"].as_str() {
            Some("tab") => {
                let key = node["tabId"].as_str().unwrap_or("");
                let widget = self
                    .pages
                    .borrow()
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| gtk::Label::new(Some(key)).upcast());
                if path.is_empty() {
                    // Already a page of the notebook: taking it out would show its neighbour.
                    if widget.parent().as_ref() != Some(self.root.upcast_ref::<gtk::Widget>()) {
                        detach(&widget);
                    }
                    widget
                } else {
                    detach(&widget);
                    let leaf = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    leaf.style_context().add_class("ws-leaf");
                    if let Some(header) = self.header.borrow().as_ref() {
                        let header = header(key);
                        leaf.pack_start(&header, false, false, 0);
                    }
                    leaf.pack_start(&widget, true, true, 0);
                    leaf.upcast()
                }
            }
            Some("split") => {
                let orientation = if node["axis"] == "y" {
                    gtk::Orientation::Vertical
                } else {
                    gtk::Orientation::Horizontal
                };
                let paned = gtk::Paned::new(orientation);
                paned.set_wide_handle(true);
                paned.set_hexpand(true);
                paned.set_vexpand(true);
                let mut first_path = path.clone();
                first_path.push(0);
                let mut second_path = path.clone();
                second_path.push(1);
                let first = self.build_node(group, &node["first"], first_path);
                let second = self.build_node(group, &node["second"], second_path);
                detach(&first);
                detach(&second);
                paned.pack1(&first, true, false);
                paned.pack2(&second, true, false);
                let state = Rc::new(RatioState {
                    ratio: Cell::new(node["ratio"].as_f64().unwrap_or(0.5).clamp(0.1, 0.9)),
                    ..Default::default()
                });
                self.ratios
                    .borrow_mut()
                    .insert(cache_key.clone(), state.clone());
                paned.connect_size_allocate({
                    let state = state.clone();
                    move |paned, allocation| {
                        let size = if orientation == gtk::Orientation::Horizontal {
                            allocation.width()
                        } else {
                            allocation.height()
                        };
                        if size > 1 && size != state.size.get() {
                            state.programmatic.set(true);
                            state.size.set(size);
                            paned
                                .set_position((f64::from(size) * state.ratio.get()).round() as i32);
                            state.programmatic.set(false);
                        }
                    }
                });
                let callback = self.resize.clone();
                let applying = self.applying.clone();
                let pending = self.pending_resize.clone();
                let timer = self.resize_timer.clone();
                if let Some(identity) =
                    crate::workspace_view::resize_update(group, node, state.ratio.get())
                {
                    paned.connect_position_notify(move |paned| {
                        let size = if orientation == gtk::Orientation::Horizontal {
                            paned.allocated_width()
                        } else {
                            paned.allocated_height()
                        };
                        if applying.get()
                            || state.programmatic.get()
                            || size <= 1
                            || size != state.size.get()
                        {
                            return;
                        }
                        let ratio = (f64::from(paned.position()) / f64::from(size)).clamp(0.1, 0.9);
                        if (ratio - state.ratio.get()).abs() < 0.005 {
                            return;
                        }
                        state.ratio.set(ratio);
                        let mut update = identity.clone();
                        update.ratio = ratio;
                        pending
                            .borrow_mut()
                            .insert((update.group.clone(), update.path.clone()), update);
                        if let Some(source) = timer.borrow_mut().take() {
                            source.remove();
                        }
                        let pending = pending.clone();
                        let callback = callback.clone();
                        let weak_timer = Rc::downgrade(&timer);
                        *timer.borrow_mut() = Some(glib::timeout_add_local_once(
                            std::time::Duration::from_millis(400),
                            move || {
                                if let Some(timer) = weak_timer.upgrade() {
                                    timer.borrow_mut().take();
                                }
                                let updates = std::mem::take(&mut *pending.borrow_mut());
                                let updates = updates.into_values().collect::<Vec<_>>();
                                if let Some(callback) = callback.borrow().as_ref() {
                                    callback(&updates);
                                }
                            },
                        ));
                    });
                }
                paned.upcast()
            }
            _ => gtk::Label::new(None).upcast(),
        };
        self.nodes.borrow_mut().insert(cache_key, widget.clone());
        widget
    }
}
impl Default for GtkWorkspace {
    fn default() -> Self {
        Self::new()
    }
}
fn detach(widget: &gtk::Widget) {
    if let Some(parent) = widget
        .parent()
        .and_then(|p| p.downcast::<gtk::Container>().ok())
    {
        parent.remove(widget);
    }
}

fn signature(node: &Value) -> Value {
    crate::workspace_view::view_signature(node)
}
fn collect_keys(
    group: &str,
    node: &Value,
    path: Vec<usize>,
    keys: &mut std::collections::HashSet<String>,
) {
    keys.insert(format!("{group}:{path:?}:{}", signature(node)));
    if node["type"] == "split" {
        let mut first = path.clone();
        first.push(0);
        let mut second = path;
        second.push(1);
        collect_keys(group, &node["first"], first, keys);
        collect_keys(group, &node["second"], second, keys);
    }
}

fn paint_leaf_focus(root: &gtk::Widget, focused: &gtk::Widget) {
    if root.style_context().has_class("ws-leaf") {
        if focused.is_ancestor(root) {
            root.style_context().add_class("focused");
        } else {
            root.style_context().remove_class("focused");
        }
    }
    if let Some(container) = root.downcast_ref::<gtk::Container>() {
        for child in container.children() {
            paint_leaf_focus(&child, focused);
        }
    }
}
