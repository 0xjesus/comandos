//! Snippet text is data. JSON documents retain their original metadata.
use serde_json::{Value, json};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub name: String,
    pub tags: Vec<String>,
    pub body: String,
}
fn score(item: &Snippet, query: &str) -> u8 {
    if item.name.to_lowercase().contains(query) {
        3
    } else if item.tags.join(" ").to_lowercase().contains(query) {
        2
    } else if item.body.to_lowercase().contains(query) {
        1
    } else {
        0
    }
}
pub fn filter_snippets(items: &[Snippet], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    let mut rows = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let s = if q.is_empty() { 1 } else { score(item, &q) };
            (s > 0).then_some((i, s))
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|(_, s)| std::cmp::Reverse(*s));
    rows.into_iter().map(|(i, _)| i).collect()
}
fn snippet(doc: &Value) -> Snippet {
    Snippet {
        name: doc.get("name").and_then(Value::as_str).unwrap_or("").into(),
        tags: doc
            .get("tags")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        body: doc.get("body").and_then(Value::as_str).unwrap_or("").into(),
    }
}
fn updated(doc: &Value) -> String {
    let v = doc.get("updated_at").unwrap_or(&Value::Null);
    if !comandos_core::json::truthy(v) {
        "0".into()
    } else {
        comandos_core::focus::integer_string(v).unwrap_or_else(|_| "0".into())
    }
}
fn integer_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let an = a.starts_with('-');
    let bn = b.starts_with('-');
    if an != bn {
        return bn.cmp(&an);
    }
    let aa = a.trim_start_matches('-');
    let bb = b.trim_start_matches('-');
    let cmp = aa.len().cmp(&bb.len()).then_with(|| aa.cmp(bb));
    if an { cmp.reverse() } else { cmp }
}
pub fn filter_documents(items: &[Value], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    let mut rows = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let s = if q.is_empty() {
                1
            } else {
                score(&snippet(item), &q)
            };
            (s > 0).then_some((i, s, updated(item)))
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| integer_cmp(&b.2, &a.2)));
    rows.into_iter().map(|(i, _, _)| i).collect()
}
pub fn edit_document(
    items: &[Value],
    id: Option<&str>,
    name: &str,
    tags: &str,
    body: &str,
    now: i64,
    new_id: &str,
) -> Result<Vec<Value>, String> {
    let name = name.trim();
    if name.is_empty() || body.trim().is_empty() {
        return Err("Nombre y cuerpo no pueden quedar vacios".into());
    }
    let tags = tags
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut next = items.to_vec();
    if let Some(id) = id {
        let item = next
            .iter_mut()
            .find(|v| v.get("id").and_then(Value::as_str) == Some(id))
            .ok_or("El snippet ya no existe")?;
        let obj = item.as_object_mut().ok_or("Snippet invalido")?;
        obj.insert("name".into(), json!(name));
        obj.insert("tags".into(), json!(tags));
        obj.insert("body".into(), json!(body));
        obj.insert("updated_at".into(), json!(now));
    } else {
        next.insert(
            0,
            json!({"id":new_id,"name":name,"tags":tags,"body":body,"updated_at":now}),
        );
    }
    Ok(next)
}
pub fn delete_document(items: &[Value], id: &str) -> Vec<Value> {
    items
        .iter()
        .filter(|v| v.get("id").and_then(Value::as_str) != Some(id))
        .cloned()
        .collect()
}

use super::clipboard::{ClipboardError, TmuxIo};
const IDENTITY: &str = "#{pid}|#{session_id}|#{s/[$]//:session_id}|#{pane_id}";
pub fn paste_snippet(
    session: &str,
    text: &str,
    tmux: &crate::tmux::TmuxCtl,
) -> Result<(), ClipboardError> {
    paste_snippet_when(session, text, tmux, || true)
}
pub fn paste_snippet_when(
    session: &str,
    text: &str,
    tmux: &impl TmuxIo,
    allowed: impl Fn() -> bool,
) -> Result<(), ClipboardError> {
    if tmux.mode() == crate::config::RunMode::Shadow {
        return Err(ClipboardError::Shadow);
    }
    if !allowed() {
        return Err(ClipboardError::Cancelled);
    }
    if session.is_empty()
        || session.len() > 80
        || !session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(ClipboardError::Tmux("Sesion invalida".into()));
    }
    let socket = tmux.socket().to_path_buf();
    let session_target = format!("={session}");
    let target = format!("={session}:");
    let error = |e: crate::tmux::TmuxError| ClipboardError::Tmux(format!("{e:?}"));
    let has = tmux
        .read(&["has-session", "-t", &session_target])
        .map_err(error)?;
    if !has.ok() {
        return Err(ClipboardError::Tmux(format!(
            "No hay sesion tmux '{session}'"
        )));
    }
    let captured = tmux
        .read(&["display-message", "-p", "-t", &target, IDENTITY])
        .map_err(error)?;
    if !captured.ok() || captured.stdout.trim().is_empty() {
        return Err(ClipboardError::DestinationChanged);
    }
    let recheck = || -> Result<(), ClipboardError> {
        if !allowed() {
            return Err(ClipboardError::Cancelled);
        }
        if tmux.socket() != socket {
            return Err(ClipboardError::DestinationChanged);
        }
        let now = tmux
            .read(&["display-message", "-p", "-t", &target, IDENTITY])
            .map_err(error)?;
        if !allowed() {
            return Err(ClipboardError::Cancelled);
        }
        if tmux.socket() != socket || !now.ok() || now.stdout.trim() != captured.stdout.trim() {
            Err(ClipboardError::DestinationChanged)
        } else {
            Ok(())
        }
    };
    let mut random = [0u8; 8];
    getrandom::fill(&mut random)
        .map_err(|_| ClipboardError::Tmux("No se pudo crear el buffer".into()))?;
    let buffer = format!(
        "comandos-snip-{}",
        random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    recheck()?;
    let loaded = tmux
        .mutate(&["load-buffer", "-b", &buffer, "-"], Some(text.as_bytes()))
        .map_err(error)?;
    // A failed load did not prove ownership; never delete a pre-existing buffer.
    if !loaded.ok() {
        return Err(ClipboardError::Tmux(if loaded.stderr.trim().is_empty() {
            "load-buffer fallo".into()
        } else {
            loaded.stderr.trim().into()
        }));
    }
    let result = recheck()
        .and_then(|()| {
            tmux.mutate(
                &["paste-buffer", "-p", "-d", "-b", &buffer, "-t", &target],
                None,
            )
            .map_err(error)
        })
        .and_then(|out| {
            if out.ok() {
                Ok(())
            } else {
                Err(ClipboardError::Tmux(if out.stderr.trim().is_empty() {
                    "paste-buffer fallo".into()
                } else {
                    out.stderr.trim().into()
                }))
            }
        });
    if result.is_err() {
        // Cleanup may outlive a UI cancellation, but never crosses server identity.
        if tmux.socket() == socket
            && tmux
                .read(&["display-message", "-p", "-t", &target, IDENTITY])
                .is_ok_and(|out| out.ok() && out.stdout.trim() == captured.stdout.trim())
            && tmux.socket() == socket
        {
            let _ = tmux.mutate(&["delete-buffer", "-b", &buffer], None);
        }
    }
    result
}

use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
#[derive(Default)]
pub struct Scope {
    closed: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
}
#[derive(Clone)]
pub struct Ticket {
    closed: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    expected: u64,
}
impl Scope {
    pub fn advance(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.advance();
    }
    pub fn ticket(&self) -> Ticket {
        Ticket {
            closed: self.closed.clone(),
            generation: self.generation.clone(),
            expected: self.generation.load(Ordering::Acquire),
        }
    }
}
impl Ticket {
    pub fn current(&self) -> bool {
        !self.closed.load(Ordering::Acquire)
            && self.generation.load(Ordering::Acquire) == self.expected
    }
}
pub type Save = Rc<dyn Fn(Vec<Value>, Vec<Value>, Ticket, Box<dyn FnOnce(Result<(), String>)>)>;
pub type SendSnippet = Rc<dyn Fn(String, Ticket, Box<dyn FnOnce(Result<String, String>)>)>;
pub struct Dialog {
    pub frame: gtk::Frame,
    pub search: gtk::Entry,
    list: gtk::ListBox,
    right: gtk::Box,
    status: gtk::Label,
    items: RefCell<Vec<Value>>,
    filtered: RefCell<Vec<usize>>,
    selected: RefCell<Option<Value>>,
    rendering: Cell<bool>,
    busy: Cell<bool>,
    flash_epoch: Cell<u64>,
    flash_timer: RefCell<Option<glib::SourceId>>,
    pub scope: Scope,
    english: bool,
    save: Save,
    send: SendSnippet,
}
impl Dialog {
    pub fn new(
        items: Vec<Value>,
        english: bool,
        save: Save,
        send: SendSnippet,
        close: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        let frame = gtk::Frame::new(None);
        frame.set_size_request(720, 520);
        frame.set_halign(gtk::Align::Center);
        frame.set_valign(gtk::Align::Center);
        frame.style_context().add_class("cc-snip-dialog");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.add(&root);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.set_margin_top(8);
        header.set_margin_bottom(6);
        header.set_margin_start(12);
        header.set_margin_end(6);
        header.style_context().add_class("cc-snip-hdr");
        let title = gtk::Label::new(Some("SNIPPETS"));
        title.set_xalign(0.);
        title.style_context().add_class("cc-snip-title");
        header.pack_start(&title, true, true, 0);
        let button = super::icons::button(
            "close",
            14,
            if english {
                "Close (ESC)"
            } else {
                "Cerrar (ESC)"
            },
        );
        // The original dialog uses a plain button; toolbar sizing would enlarge
        // its header and move the search, list, and preview below it.
        button.style_context().remove_class("tabplus");
        button.style_context().add_class("cc-snip-close");
        button.set_relief(gtk::ReliefStyle::None);
        button.connect_clicked(move |_| close());
        header.pack_end(&button, false, false, 0);
        root.pack_start(&header, false, false, 0);
        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_position(260);
        root.pack_start(&paned, true, true, 0);
        let left = gtk::Box::new(gtk::Orientation::Vertical, 6);
        left.set_margin_top(8);
        left.set_margin_bottom(8);
        left.set_margin_start(8);
        left.set_margin_end(8);
        let search = gtk::Entry::new();
        search.set_placeholder_text(Some("buscar..."));
        left.pack_start(&search, false, false, 0);
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroll.set_vexpand(true);
        scroll.add(&list);
        left.pack_start(&scroll, true, true, 0);
        let new = gtk::Button::with_label("+ nuevo");
        left.pack_start(&new, false, false, 0);
        paned.pack1(&left, false, false);
        let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
        right.set_margin_top(8);
        right.set_margin_bottom(8);
        right.set_margin_start(8);
        right.set_margin_end(8);
        paned.pack2(&right, true, false);
        let status = gtk::Label::new(None);
        status.set_xalign(0.);
        status.set_margin_top(4);
        status.set_margin_bottom(6);
        status.set_margin_start(12);
        status.set_margin_end(12);
        status.style_context().add_class("cc-snip-status");
        root.pack_start(&status, false, false, 0);
        let dialog = Rc::new(Self {
            frame,
            search,
            list,
            right,
            status,
            items: RefCell::new(items),
            filtered: RefCell::new(vec![]),
            selected: RefCell::new(None),
            rendering: Cell::new(false),
            busy: Cell::new(false),
            flash_epoch: Cell::new(0),
            flash_timer: RefCell::new(None),
            scope: Scope::default(),
            english,
            save,
            send,
        });
        let weak = Rc::downgrade(&dialog);
        dialog.search.connect_changed(move |_| {
            if let Some(d) = weak.upgrade().filter(|d| !d.busy.get()) {
                let selected = d.list.selected_row().is_some();
                d.render_list();
                if selected {
                    d.selected.borrow_mut().take();
                    d.scope.advance();
                    d.render_preview();
                }
            }
        });
        let weak = Rc::downgrade(&dialog);
        dialog.list.connect_row_selected(move |_, row| {
            if let Some(d) = weak
                .upgrade()
                .filter(|d| !d.rendering.get() && !d.busy.get())
            {
                let item = row
                    .and_then(|r| usize::try_from(r.index()).ok())
                    .and_then(|i| d.filtered.borrow().get(i).copied())
                    .and_then(|i| d.items.borrow().get(i).cloned());
                *d.selected.borrow_mut() = item;
                d.scope.advance();
                d.render_preview();
            }
        });
        let weak = Rc::downgrade(&dialog);
        new.connect_clicked(move |_| {
            if let Some(d) = weak.upgrade().filter(|d| !d.busy.get()) {
                d.scope.advance();
                d.edit(None);
            }
        });
        let weak = Rc::downgrade(&dialog);
        dialog.frame.connect_destroy(move |_| {
            if let Some(d) = weak.upgrade() {
                d.shutdown();
            }
        });
        dialog.render_list();
        dialog.render_preview();
        dialog.frame.show_all();
        dialog
    }
    fn clear_right(&self) {
        for child in self.right.children() {
            self.right.remove(&child);
        }
    }
    fn render_list(&self) {
        self.rendering.set(true);
        for child in self.list.children() {
            self.list.remove(&child);
        }
        let items = self.items.borrow();
        let filtered = filter_documents(&items, self.search.text().as_str());
        for i in &filtered {
            if let Some(item) = items.get(*i) {
                let row = gtk::ListBoxRow::new();
                let box_ = gtk::Box::new(gtk::Orientation::Vertical, 2);
                box_.set_margin_top(4);
                box_.set_margin_bottom(4);
                box_.set_margin_start(6);
                box_.set_margin_end(6);
                let s = snippet(item);
                let name = gtk::Label::new(Some(&s.name));
                name.set_xalign(0.);
                box_.pack_start(&name, false, false, 0);
                if !s.tags.is_empty() {
                    let tags = gtk::Label::new(Some(&s.tags.join(" · ")));
                    tags.set_xalign(0.);
                    tags.style_context().add_class("dim-label");
                    box_.pack_start(&tags, false, false, 0);
                }
                row.add(&box_);
                self.list.add(&row);
            }
        }
        *self.filtered.borrow_mut() = filtered;
        self.list.show_all();
        self.rendering.set(false);
    }
    fn render_preview(self: &Rc<Self>) {
        self.clear_right();
        let item = self.selected.borrow().clone();
        let Some(item) = item else {
            let label = gtk::Label::new(Some("Elegí un snippet o creá uno nuevo."));
            label.set_xalign(0.);
            self.right.pack_start(&label, false, false, 0);
            self.right.show_all();
            return;
        };
        let s = snippet(&item);
        let name = gtk::Label::new(Some(&s.name));
        name.set_xalign(0.);
        name.style_context().add_class("title-4");
        self.right.pack_start(&name, false, false, 0);
        if !s.tags.is_empty() {
            let tags = gtk::Label::new(Some(&s.tags.join(" · ")));
            tags.set_xalign(0.);
            self.right.pack_start(&tags, false, false, 0);
        }
        let text = gtk::TextView::new();
        text.set_editable(false);
        text.set_monospace(true);
        text.set_wrap_mode(gtk::WrapMode::WordChar);
        if let Some(buffer) = text.buffer() {
            buffer.set_text(&s.body);
        }
        let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroll.set_vexpand(true);
        scroll.add(&text);
        self.right.pack_start(&scroll, true, true, 0);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for label in ["Send", "Edit", "Delete"] {
            let button = gtk::Button::with_label(label);
            let weak = Rc::downgrade(self);
            let item = item.clone();
            button.connect_clicked(move |_| {
                if let Some(d) = weak
                    .upgrade()
                    .filter(|d| !d.busy.get() && d.scope.ticket().current())
                {
                    match label {
                        "Edit" => {
                            d.scope.advance();
                            d.edit(Some(item.clone()));
                        }
                        "Delete" => d.delete(&item),
                        _ => d.send_item(&item),
                    }
                }
            });
            actions.pack_start(&button, false, false, 0);
        }
        self.right.pack_start(&actions, false, false, 0);
        self.right.show_all();
    }
    fn edit(self: &Rc<Self>, item: Option<Value>) {
        self.clear_right();
        let s = item.as_ref().map(snippet);
        let name = gtk::Entry::new();
        name.set_placeholder_text(Some("nombre"));
        name.set_text(s.as_ref().map(|s| s.name.as_str()).unwrap_or(""));
        self.right.pack_start(&name, false, false, 0);
        let tags = gtk::Entry::new();
        tags.set_placeholder_text(Some("tags coma-separados"));
        tags.set_text(&s.as_ref().map(|s| s.tags.join(", ")).unwrap_or_default());
        self.right.pack_start(&tags, false, false, 0);
        let text = gtk::TextView::new();
        text.set_monospace(true);
        text.set_wrap_mode(gtk::WrapMode::WordChar);
        if let Some(buffer) = text.buffer() {
            buffer.set_text(s.as_ref().map(|s| s.body.as_str()).unwrap_or(""));
        }
        let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroll.set_vexpand(true);
        scroll.add(&text);
        self.right.pack_start(&scroll, true, true, 0);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let save = gtk::Button::with_label("Save");
        let cancel = gtk::Button::with_label("Cancel");
        actions.pack_start(&save, false, false, 0);
        actions.pack_start(&cancel, false, false, 0);
        self.right.pack_start(&actions, false, false, 0);
        let focus_name = name.clone();
        let weak = Rc::downgrade(self);
        save.connect_clicked(move |_| {
            let Some(d) = weak
                .upgrade()
                .filter(|d| !d.busy.get() && d.scope.ticket().current())
            else {
                return;
            };
            let body = text
                .buffer()
                .and_then(|b| b.text(&b.start_iter(), &b.end_iter(), false))
                .map(|s| s.to_string())
                .unwrap_or_default();
            let mut random = [0u8; 8];
            if getrandom::fill(&mut random).is_err() {
                d.flash("No se pudo crear el snippet", true);
                return;
            }
            let new_id = random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let id = item
                .as_ref()
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str);
            let expected = d.items.borrow().clone();
            let next = edit_document(
                &expected,
                id,
                name.text().as_str(),
                tags.text().as_str(),
                &body,
                chrono::Utc::now().timestamp(),
                &new_id,
            );
            match next {
                Ok(next) => {
                    let selected = next
                        .iter()
                        .find(|v| {
                            v.get("id").and_then(Value::as_str) == Some(id.unwrap_or(&new_id))
                        })
                        .cloned();
                    d.persist(expected, next, selected);
                }
                Err(e) => d.flash(&e, true),
            }
        });
        let weak = Rc::downgrade(self);
        cancel.connect_clicked(move |_| {
            if let Some(d) = weak.upgrade().filter(|d| !d.busy.get()) {
                d.scope.advance();
                d.render_preview();
            }
        });
        self.right.show_all();
        focus_name.grab_focus();
    }
    fn persist(self: &Rc<Self>, expected: Vec<Value>, next: Vec<Value>, selected: Option<Value>) {
        if self.busy.replace(true) {
            return;
        }
        self.right.set_sensitive(false);
        self.search.set_sensitive(false);
        self.list.set_sensitive(false);
        let ticket = self.scope.ticket();
        let current = ticket.clone();
        let weak = Rc::downgrade(self);
        let saved = next.clone();
        (self.save)(
            expected,
            next,
            ticket,
            Box::new(move |result| {
                if let Some(d) = weak.upgrade().filter(|_| current.current()) {
                    d.busy.set(false);
                    d.right.set_sensitive(true);
                    d.search.set_sensitive(true);
                    d.list.set_sensitive(true);
                    match result {
                        Ok(()) => {
                            *d.items.borrow_mut() = saved;
                            *d.selected.borrow_mut() = selected;
                            d.render_list();
                            d.render_preview();
                        }
                        Err(e) => d.flash(&e, true),
                    }
                }
            }),
        );
    }
    fn delete(self: &Rc<Self>, item: &Value) {
        if let Some(id) = item.get("id").and_then(Value::as_str) {
            let expected = self.items.borrow().clone();
            let next = delete_document(&expected, id);
            self.persist(expected, next, None);
        }
    }
    fn send_item(self: &Rc<Self>, item: &Value) {
        if self.busy.replace(true) {
            return;
        }
        self.right.set_sensitive(false);
        self.search.set_sensitive(false);
        self.list.set_sensitive(false);
        let body = snippet(item).body;
        let ticket = self.scope.ticket();
        let current = ticket.clone();
        let weak = Rc::downgrade(self);
        (self.send)(
            body,
            ticket,
            Box::new(move |result| {
                if let Some(d) = weak.upgrade().filter(|_| current.current()) {
                    d.busy.set(false);
                    d.right.set_sensitive(true);
                    d.search.set_sensitive(true);
                    d.list.set_sensitive(true);
                    match result {
                        Ok(session) => d.flash(
                            &format!(
                                "{} {session}",
                                if d.english {
                                    "Pasted into"
                                } else {
                                    "Pegado en"
                                }
                            ),
                            false,
                        ),
                        Err(e) => d.flash(&e, true),
                    }
                }
            }),
        );
    }
    pub fn flash(self: &Rc<Self>, text: &str, error: bool) {
        if !self.scope.ticket().current() {
            return;
        }
        self.status.set_text(text);
        for class in ["cc-snip-status-ok", "cc-snip-status-err"] {
            self.status.style_context().remove_class(class);
        }
        self.status.style_context().add_class(if error {
            "cc-snip-status-err"
        } else {
            "cc-snip-status-ok"
        });
        if let Some(id) = self.flash_timer.borrow_mut().take() {
            id.remove();
        }
        let epoch = self.flash_epoch.get().wrapping_add(1);
        self.flash_epoch.set(epoch);
        let weak = Rc::downgrade(self);
        *self.flash_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            std::time::Duration::from_millis(2500),
            move || {
                if let Some(d) = weak
                    .upgrade()
                    .filter(|d| d.scope.ticket().current() && d.flash_epoch.get() == epoch)
                {
                    d.status.set_text("");
                    d.flash_timer.borrow_mut().take();
                }
            },
        ));
    }
    pub fn shutdown(&self) {
        self.scope.close();
        if let Some(id) = self.flash_timer.borrow_mut().take() {
            id.remove();
        }
    }
}
impl Drop for Dialog {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Clone, Copy)]
pub enum LogEvent {
    Open,
    Save,
    Send,
}
pub fn log_operation(
    guard: &crate::guard::WriteGuard,
    path: &std::path::Path,
    event: LogEvent,
    count: usize,
) -> Result<bool, crate::guard::GuardError> {
    let operation = match event {
        LogEvent::Open => "open",
        LogEvent::Save => "save",
        LogEvent::Send => "send",
    };
    guard.append_if_exists(
        path,
        format!(
            "[{}] snippets {operation} count={count}\n",
            chrono::Local::now().format("%H:%M:%S")
        )
        .as_bytes(),
    )
}
