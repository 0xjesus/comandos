//! Chat view of the current session. The header (session and pane chips) is native;
//! the messages and the composer are a web page (`chat_page`) drawn from the HTML the
//! server renders for each message, like the phone: markdown, highlighted code, tables,
//! copy and edit in place, a card for approvals, files and pasted images. It is its
//! own page of the terminal stack, so nothing that re-shows the tabs or the mosaic can
//! draw tmux under it. It opens and closes only with the tab strip's chat button; the
//! wheel always scrolls tmux. The timer only fetches while the view is visible; an
//! unchanged transcript costs one tiny answer from the server.
use super::*;
use crate::ui::chat_page;
use webkit2gtk::WebViewExt;

/// One message: role (`a` answer, `u` prompt, `t` tool calls), its text and its HTML.
type Message = (String, String, String);

#[derive(Default)]
pub(super) struct Owned {
    on: Cell<bool>,
    view: RefCell<Option<View>>,
    /// `session|pane` the list shows.
    session: RefCell<String>,
    /// Pane chosen for the chat ("" = the window's active pane).
    pane: RefCell<String>,
    /// Pane the server answered for.
    answered: RefCell<String>,
    token: RefCell<String>,
    busy: Cell<bool>,
    chips: RefCell<String>,
    /// The transcript as last received; older turns stay folded until asked for.
    messages: RefCell<Vec<Message>>,
    expanded: Cell<bool>,
    /// Seconds since the transcript moved, as the server reported it.
    age: Cell<u64>,
    /// What the agent waits on (approval card), as last sent to the page.
    ask: RefCell<Value>,
    /// Scripts for the page before it finished loading.
    queue: RefCell<Vec<String>>,
    ready: Cell<bool>,
}
struct View {
    stack: gtk::Stack,
    web: webkit2gtk::WebView,
    title: gtk::Label,
    chips: gtk::Box,
    toggle: gtk::Button,
}

/// Where the collapsed view starts: the last agent answer (and what followed it), or
/// the last prompt while no answer exists yet.
fn cut(messages: &[Message]) -> usize {
    messages
        .iter()
        .rposition(|(r, _, _)| r == "a")
        .or_else(|| messages.iter().rposition(|(r, _, _)| r == "u"))
        .unwrap_or(0)
}

/// Agents whose transcript the chat can show.
pub(super) fn is_agent(command: &str) -> bool {
    matches!(command, "claude" | "codex")
}

/// Chip text for a pane: its number and what runs there.
fn pane_label(n: usize, command: &str) -> String {
    let what = match command {
        "claude" => "Claude".to_owned(),
        "codex" => "Codex".to_owned(),
        c if ui::overlays::is_shell(c) => "Terminal".to_owned(),
        c => c.to_owned(),
    };
    format!("{n} · {what}")
}

/// Text and HTML on the clipboard (HTML pastes with formatting in docs and mail).
fn copy_to_clipboard(plain: &str, html: Option<&str>) {
    if !gtk::is_initialized_main_thread() || plain.is_empty() {
        return;
    }
    let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
    match html {
        None => clipboard.set_text(plain),
        Some(html) => {
            let entry = |name: &str, info: u32| gtk::TargetEntry::new(name, gtk::TargetFlags::empty(), info);
            let targets = [
                entry("text/html", 0),
                entry("UTF8_STRING", 1),
                entry("text/plain;charset=utf-8", 1),
                entry("text/plain", 1),
                entry("STRING", 1),
                entry("TEXT", 1),
            ];
            let (plain, html) = (plain.to_owned(), html.to_owned());
            clipboard.set_with_data(&targets, move |_, data, info| {
                if info == 0 {
                    data.set(&gdk::Atom::intern("text/html"), 8, html.as_bytes());
                } else {
                    data.set_text(&plain);
                }
            });
        }
    }
    clipboard.store();
}

fn open_url(url: &str) {
    if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
        let _ = gtk::show_uri_on_window(None::<&gtk::Window>, url, gtk::current_event_time());
    }
}

/// A pasted or dropped image (a `data:` URL) saved where the agent can read it.
fn save_image(data_url: &str) -> Option<std::path::PathBuf> {
    let (head, body) = data_url.strip_prefix("data:")?.split_once(',')?;
    let (mime, encoding) = head.split_once(';')?;
    if encoding != "base64" || !mime.starts_with("image/") {
        return None;
    }
    let ext = match mime {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    };
    let bytes = glib::base64_decode(body);
    if bytes.is_empty() {
        return None;
    }
    let dir = glib::user_cache_dir().join("comandos").join("chat-files");
    std::fs::create_dir_all(&dir).ok()?;
    let stamp = glib::DateTime::now_local().ok()?.format("%Y%m%d-%H%M%S").ok()?;
    let mut path = dir.join(format!("imagen-{stamp}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("imagen-{stamp}-{n}.{ext}"));
        n += 1;
    }
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

/// Header: the session's name and, in a split, one chip per pane.
fn build_head() -> (gtk::Box, gtk::Label, gtk::Box) {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.style_context().add_class("cv-head");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let title = gtk::Label::new(None);
    title.style_context().add_class("cv-title");
    title.set_xalign(0.);
    title.set_ellipsize(pango::EllipsizeMode::End);
    let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    chips.style_context().add_class("cv-chips");
    head.pack_start(&title, false, false, 0);
    head.pack_start(&chips, false, false, 0);
    bar.pack_start(&ui::column::Column::new(&head, 780, 32), true, true, 0);
    (bar, title, chips)
}

impl App {
    /// Adds the chat as the `chat` page of the terminal stack (`term` is the other).
    pub(super) fn install_chat(self: &Rc<Self>, stack: &gtk::Stack) {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.style_context().add_class("cv-root");
        let (bar, title, chips) = build_head();
        let weak = Rc::downgrade(self);
        let web = chat_page::view(move |message| {
            if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) {
                app.chat_message(&message);
            }
        });
        root.pack_start(&bar, false, false, 0);
        root.pack_start(&web, true, true, 0);
        root.show_all();
        stack.add_named(&root, "chat");
        let toggle = ui::icons::button("chat", 18, "Ver como chat");
        toggle.set_widget_name("chat");
        toggle.style_context().add_class("cc-key");
        let weak = Rc::downgrade(self);
        toggle.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                let on = !app.chat.on.get();
                if on {
                    app.chat.pane.replace(String::new());
                }
                app.chat_show(on);
            }
        });
        self.tab_layout.actions().pack_start(&toggle, false, false, 0);
        toggle.show();
        *self.chat.view.borrow_mut() = Some(View { stack: stack.clone(), web, title, chips, toggle });
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(1500), move || {
            let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                return glib::ControlFlow::Break;
            };
            app.chat_tick(false);
            glib::ControlFlow::Continue
        });
    }
    /// Runs `CV.<function>(arg)` in the page, or queues it until the page is ready.
    fn chat_js(&self, function: &str, arg: &Value) {
        let script = chat_page::call(function, arg);
        if !self.chat.ready.get() {
            self.chat.queue.borrow_mut().push(script);
            return;
        }
        if let Some(view) = &*self.chat.view.borrow() {
            view.web.evaluate_javascript(&script, None, None, None::<&gio::Cancellable>, |_| {});
        }
    }
    /// What the page asks for.
    fn chat_message(self: &Rc<Self>, message: &Value) {
        match message["type"].as_str().unwrap_or("") {
            "ready" => {
                self.chat.ready.set(true);
                let theme = chat_page::theme_vars(self.applied_theme.borrow().as_ref());
                self.chat_js("theme", &theme);
                let queued: Vec<String> = self.chat.queue.borrow_mut().drain(..).collect();
                if let Some(view) = &*self.chat.view.borrow() {
                    for script in queued {
                        view.web.evaluate_javascript(&script, None, None, None::<&gio::Cancellable>, |_| {});
                    }
                }
            }
            "send" => {
                if let Some(text) = message["text"].as_str().filter(|t| !t.trim().is_empty()) {
                    self.chat.age.set(0);
                    self.chat_post("/send", json!({"text": text}));
                }
            }
            "key" => {
                if let Some(key) = message["key"].as_str() {
                    self.chat_post("/key", json!({"key": key}));
                }
            }
            "more" => {
                self.chat.expanded.set(!self.chat.expanded.get());
                self.chat_draw(false, true);
            }
            "copy" => copy_to_clipboard(message["plain"].as_str().unwrap_or(""), message["html"].as_str()),
            "open" => open_url(message["url"].as_str().unwrap_or("")),
            "attach" => self.chat_attach(),
            "image" => {
                if let Some(path) = message["data"].as_str().and_then(save_image) {
                    let data = message["data"].clone();
                    self.chat_js("files", &json!([{"path": path, "thumb": data}]));
                }
            }
            _ => {}
        }
    }
    /// «+»: pick files; their paths go into the message (the agent reads them, and
    /// Claude takes image paths as images).
    fn chat_attach(self: &Rc<Self>) {
        let dialog = gtk::FileChooserNative::new(
            Some("Adjuntar archivos"),
            Some(&self.window),
            gtk::FileChooserAction::Open,
            Some("Adjuntar"),
            Some("Cancelar"),
        );
        dialog.set_select_multiple(true);
        if dialog.run() == gtk::ResponseType::Accept {
            let files: Vec<Value> = dialog
                .filenames()
                .into_iter()
                .map(|p| json!({"path": p.to_string_lossy()}))
                .collect();
            if !files.is_empty() {
                self.chat_js("files", &Value::Array(files));
            }
        }
        dialog.destroy();
    }
    fn chat_show(self: &Rc<Self>, on: bool) {
        self.chat.on.set(on);
        if let Some(view) = &*self.chat.view.borrow() {
            view.stack.set_visible_child_name(if on { "chat" } else { "term" });
            if on {
                view.web.grab_focus();
            }
            view.toggle.set_tooltip_text(Some(if on { "Ver terminal" } else { "Ver como chat" }));
            let ctx = view.toggle.style_context();
            if on { ctx.add_class("cv-on") } else { ctx.remove_class("cv-on") }
        }
        if !on {
            self.focus_current_term();
            return;
        }
        let theme = chat_page::theme_vars(self.applied_theme.borrow().as_ref());
        self.chat_js("theme", &theme);
        self.chat_js("focus", &Value::Null);
        self.chat.session.replace(String::new());
        self.chat_tick(true);
    }
    fn chat_title(&self, session: &str) -> String {
        let key = self.current_session().unwrap_or_default();
        self.labels
            .borrow()
            .get(&key)
            .map(|tab| tab.text.text().to_string())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| session.to_owned())
    }
    fn chat_target(&self) -> String {
        self.current_session()
            .and_then(|key| key.split(':').next().map(str::to_owned))
            .unwrap_or_default()
    }
    /// Pane chips when the session is split: one per pane, the chat's pane marked.
    fn chat_chips(self: &Rc<Self>) {
        let Some(key) = self.current_session() else { return };
        let records: Vec<(String, String, bool)> = {
            let boxes = self.t17.boxes.borrow();
            let Some(owner) = boxes.get(&key) else { return };
            let mut panes = owner.panes.borrow().clone();
            panes.sort_by_key(|p| (p.geometry.top, p.geometry.left));
            panes.into_iter().map(|p| (p.geometry.id, p.command, p.active)).collect()
        };
        let chosen = {
            let answered = self.chat.answered.borrow().clone();
            if answered.is_empty() { records.iter().find(|r| r.2).map(|r| r.0.clone()).unwrap_or_default() } else { answered }
        };
        let signature = format!("{chosen}|{records:?}");
        if *self.chat.chips.borrow() == signature {
            return;
        }
        self.chat.chips.replace(signature);
        let Some(view) = &*self.chat.view.borrow() else { return };
        for child in view.chips.children() {
            view.chips.remove(&child);
        }
        if records.len() < 2 {
            return;
        }
        for (n, (pane, command, _)) in records.iter().enumerate() {
            let chip = gtk::Button::with_label(&pane_label(n + 1, command));
            chip.style_context().add_class("cv-chip");
            if *pane == chosen {
                chip.style_context().add_class("on");
            }
            chip.set_can_focus(false);
            chip.set_tooltip_text(Some(if is_agent(command) { "Ver la conversación de este pane" } else { "Este pane no tiene un agente" }));
            let (weak, pane) = (Rc::downgrade(self), pane.clone());
            chip.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.chat.pane.replace(pane.clone());
                    app.chat_tick(true);
                }
            });
            view.chips.pack_start(&chip, false, false, 0);
        }
        view.chips.show_all();
    }
    fn chat_tick(self: &Rc<Self>, force: bool) {
        if !self.chat.on.get() || !self.window.is_active() && !force {
            return;
        }
        let session = self.chat_target();
        if session.is_empty() {
            return;
        }
        let pane = self.chat.pane.borrow().clone();
        let identity = format!("{session}|{pane}");
        if *self.chat.session.borrow() != identity {
            self.chat.session.replace(identity.clone());
            self.chat.token.replace(String::new());
            self.chat.answered.replace(String::new());
            self.chat.messages.borrow_mut().clear();
            self.chat.expanded.set(false);
            self.chat.ask.replace(Value::Null);
            if let Some(view) = &*self.chat.view.borrow() {
                view.title.set_text(&self.chat_title(&session));
            }
            self.chat_js("update", &json!({"reset": true, "messages": [], "note": "Cargando conversación…", "ask": null}));
        }
        self.chat_chips();
        if self.chat.busy.replace(true) {
            return;
        }
        let since = self.chat.token.borrow().clone();
        let dash = self.dash.clone();
        let body = json!({"session": session, "pane": pane, "since": since});
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || dash.post("/chat/transcript", &body, Duration::from_secs(5)),
            move |result| {
                let Some(app) = weak.upgrade() else { return };
                app.chat.busy.set(false);
                let Ok((_, v)) = result else { return };
                if *app.chat.session.borrow() != identity {
                    return;
                }
                app.chat.age.set(v["age"].as_u64().unwrap_or(u64::MAX));
                if let Some(pane) = v["pane"].as_str().filter(|p| p.starts_with('%')) {
                    app.chat.answered.replace(pane.to_owned());
                    app.chat_chips();
                }
                let ask = v.get("ask").cloned().unwrap_or(Value::Null);
                if *app.chat.ask.borrow() != ask {
                    app.chat.ask.replace(ask.clone());
                    app.chat_js("ask", &ask);
                }
                if v["unchanged"] == true {
                    app.chat_js("busy", &json!(app.chat_busy()));
                    return;
                }
                app.chat.token.replace(v["token"].as_str().unwrap_or("").into());
                if v["agent"].is_null() {
                    app.chat.messages.borrow_mut().clear();
                    let note = if app.chat.chips.borrow().contains("Terminal") {
                        "Este pane es una terminal sin Claude ni Codex. Elige otro pane arriba o vuelve a la terminal."
                    } else {
                        "Sin conversación de Claude o Codex en esta sesión. Usa la terminal."
                    };
                    app.chat_js("update", &json!({"reset": true, "messages": [], "note": note}));
                    return;
                }
                let empty = Vec::new();
                *app.chat.messages.borrow_mut() = v["messages"]
                    .as_array()
                    .unwrap_or(&empty)
                    .iter()
                    .map(|m| {
                        let text = |k: &str| m[k].as_str().unwrap_or("").to_owned();
                        (m["r"].as_str().unwrap_or("t").to_owned(), text("t"), text("h"))
                    })
                    .collect();
                app.chat_draw(since.is_empty(), false);
            },
        );
    }
    /// «Working» while the transcript moved in the last 90 s and does not end in an answer.
    fn chat_busy(&self) -> bool {
        self.chat.age.get() < 90 && self.chat.messages.borrow().last().is_some_and(|(r, _, _)| r != "a")
    }
    /// Sends the visible slice (the last answer, or everything when expanded); the page
    /// redraws only from the first message that changed and waits while one is edited.
    fn chat_draw(self: &Rc<Self>, first: bool, keep_scroll: bool) {
        let state = {
            let messages = self.chat.messages.borrow();
            let folded = cut(&messages);
            let from = if self.chat.expanded.get() { 0 } else { folded };
            let slice: Vec<Value> = messages[from..].iter().map(|(r, t, h)| json!({"r": r, "t": t, "h": h})).collect();
            let note = slice.is_empty().then_some("Sin conversación todavía. Escribe abajo o vuelve a la terminal.");
            json!({
                "messages": slice,
                "folded": folded,
                "expanded": self.chat.expanded.get(),
                "busy": self.chat_busy(),
                "first": first,
                "keepScroll": keep_scroll,
                "note": note,
            })
        };
        self.chat_js("update", &state);
    }
    fn chat_post(self: &Rc<Self>, path: &'static str, mut body: Value) {
        let session = self.chat_target();
        if session.is_empty() || !self.writable() {
            return;
        }
        body["session"] = session.into();
        let pane = {
            let answered = self.chat.answered.borrow().clone();
            if answered.is_empty() { self.chat.pane.borrow().clone() } else { answered }
        };
        body["pane"] = pane.into();
        let dash = self.dash.clone();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || dash.post(path, &body, Duration::from_secs(5)),
            move |result| {
                let Some(app) = weak.upgrade() else { return };
                if let Err(error) = result {
                    app.status.set_text(&format!("Chat: {error:?}"));
                }
                if path == "/send" {
                    app.chat_js("sent", &Value::Null);
                }
                app.chat_tick(true);
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapsed_view_starts_at_the_last_answer() {
        let m = |r: &str| (r.to_owned(), String::new(), String::new());
        assert_eq!(cut(&[m("u"), m("a"), m("u"), m("t"), m("a"), m("t")]), 4);
        assert_eq!(cut(&[m("t"), m("u"), m("t")]), 1);
        assert_eq!(cut(&[]), 0);
    }

    #[test]
    fn chips_name_what_runs_in_each_pane() {
        assert_eq!(pane_label(1, "claude"), "1 · Claude");
        assert_eq!(pane_label(2, "zsh"), "2 · Terminal");
        assert_eq!(pane_label(3, "htop"), "3 · htop");
    }

    #[test]
    fn pasted_images_are_saved_and_other_data_is_refused() {
        // 1×1 transparent PNG.
        let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";
        let path = save_image(png).expect("saved");
        assert!(path.extension().is_some_and(|e| e == "png"));
        assert!(std::fs::read(&path).unwrap().starts_with(&[0x89, b'P', b'N', b'G']));
        std::fs::remove_file(path).unwrap();
        assert!(save_image("data:text/plain;base64,aG9sYQ==").is_none());
        assert!(save_image("hola").is_none());
    }
}
