//! Chat view of the current session: its agent transcript as selectable bubbles, a
//! composer that types into the pane and quick keys. It replaces the terminal notebook
//! while on; the timer only fetches while it is visible, and an unchanged transcript
//! costs one tiny answer from the server.
use super::*;
use crate::ui::chat_markdown::{self, Block};

#[derive(Default)]
pub(super) struct Owned {
    on: Cell<bool>,
    view: RefCell<Option<View>>,
    session: RefCell<String>,
    token: RefCell<String>,
    busy: Cell<bool>,
    /// Terminal containers hidden by the chat, restored when it closes.
    hidden: RefCell<Vec<gtk::Widget>>,
    /// Messages currently drawn, one list child each; a refresh only redraws the tail
    /// that changed.
    shown: RefCell<Vec<(String, String)>>,
    /// The transcript as last received; older turns stay folded until asked for.
    messages: RefCell<Vec<(String, String)>>,
    expanded: Cell<bool>,
    /// Seconds since the transcript moved, as the server reported it.
    age: Cell<u64>,
}
struct View {
    root: gtk::Box,
    notebook: gtk::Notebook,
    list: gtk::Box,
    more: gtk::Button,
    typing: gtk::Box,
    send: gtk::Button,
    scroll: gtk::ScrolledWindow,
    title: gtk::Label,
    input: gtk::TextView,
    toggle: gtk::Button,
}

/// Where the collapsed view starts: the last agent answer (and what followed it), or
/// the last prompt while no answer exists yet.
fn cut(messages: &[(String, String)]) -> usize {
    messages
        .iter()
        .rposition(|(r, _)| r == "a")
        .or_else(|| messages.iter().rposition(|(r, _)| r == "u"))
        .unwrap_or(0)
}

fn text_label(markup: Option<&str>, plain: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    match markup {
        Some(m) => label.set_markup(m),
        None => label.set_text(plain),
    }
    label.set_line_wrap(true);
    label.set_line_wrap_mode(pango::WrapMode::WordChar);
    label.set_selectable(true);
    label.set_can_focus(false);
    label.set_xalign(0.);
    label.style_context().add_class(class);
    label
}

/// Markdown blocks as widgets: prose and headings are wrapping labels, lists hang
/// their text after the marker, quotes and code get their own panel, tables a grid.
fn render_blocks(column: &gtk::Box, blocks: &[Block], ctx: &Render, depth: usize) {
    for block in blocks {
        let widget: gtk::Widget = match block {
            Block::Para(markup) => {
                let label = text_label(Some(markup), "", "cv-text");
                label.set_max_width_chars(ctx.width);
                label.upcast()
            }
            Block::Heading(level, markup) => {
                let label = text_label(Some(markup), "", "cv-h");
                label.style_context().add_class(&format!("cv-h{}", (*level).min(4)));
                label.set_max_width_chars(ctx.width);
                label.upcast()
            }
            Block::Code { lang, body } => {
                let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
                panel.style_context().add_class("cv-codeblock");
                if !lang.is_empty() {
                    let name = gtk::Label::new(Some(lang));
                    name.style_context().add_class("cv-lang");
                    name.set_xalign(0.);
                    panel.pack_start(&name, false, false, 0);
                }
                let markup = chat_markdown::highlight(lang, body, &ctx.palette);
                let code = text_label(Some(&markup), "", "cv-code");
                code.set_max_width_chars(ctx.width);
                panel.pack_start(&code, false, false, 0);
                panel.upcast()
            }
            Block::Quote(inner) => {
                let quote = gtk::Box::new(gtk::Orientation::Vertical, 6);
                quote.style_context().add_class("cv-quote");
                render_blocks(&quote, inner, ctx, depth);
                quote.upcast()
            }
            Block::List { start, items } => {
                let list = gtk::Box::new(gtk::Orientation::Vertical, 3);
                list.style_context().add_class("cv-ul");
                let digits = start.map_or(0, |n| (n + items.len() as u64).to_string().len());
                for (index, item) in items.iter().enumerate() {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                    let mark = match (item.task, start) {
                        (Some(true), _) => "☑".to_owned(),
                        (Some(false), _) => "☐".to_owned(),
                        (None, Some(n)) => format!("{}.", n + index as u64),
                        (None, None) => match depth % 3 {
                            0 => "•",
                            1 => "◦",
                            _ => "▪",
                        }
                        .to_owned(),
                    };
                    let marker = gtk::Label::new(Some(&mark));
                    marker.style_context().add_class("cv-mark");
                    if item.task == Some(true) {
                        marker.style_context().add_class("cv-done");
                    }
                    marker.set_xalign(1.);
                    marker.set_yalign(0.);
                    marker.set_width_chars(if start.is_some() { digits as i32 + 1 } else { 2 });
                    let body = gtk::Box::new(gtk::Orientation::Vertical, 4);
                    render_blocks(&body, &item.blocks, ctx, depth + 1);
                    row.pack_start(&marker, false, false, 0);
                    row.pack_start(&body, true, true, 0);
                    list.pack_start(&row, false, false, 0);
                }
                list.upcast()
            }
            Block::Table { align, rows } => {
                let grid = gtk::Grid::new();
                grid.style_context().add_class("cv-table");
                grid.set_halign(gtk::Align::Start);
                let columns = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
                let max = (ctx.width as usize / columns).clamp(12, 60) as i32;
                for (y, row) in rows.iter().enumerate() {
                    for (x, cell) in row.iter().enumerate() {
                        let label = text_label(Some(cell), "", "cv-cell");
                        if y == 0 {
                            label.style_context().add_class("cv-th");
                        }
                        let xalign = match align.get(x) {
                            Some(chat_markdown::Align::Center) => 0.5,
                            Some(chat_markdown::Align::Right) => 1.,
                            _ => 0.,
                        };
                        label.set_xalign(xalign);
                        label.set_yalign(0.);
                        label.set_max_width_chars(max);
                        label.set_hexpand(true);
                        grid.attach(&label, x as i32, y as i32, 1, 1);
                    }
                }
                grid.upcast()
            }
            Block::Rule => {
                let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
                rule.style_context().add_class("cv-rule");
                rule.upcast()
            }
        };
        column.pack_start(&widget, false, false, 0);
    }
}

struct Render {
    palette: chat_markdown::CodePalette,
    width: i32,
}

/// One message as a widget: a bubble with the agent's markdown rendered; tool calls are
/// a slim line.
fn bubble(role: &str, text: &str, palette: &chat_markdown::CodePalette) -> gtk::Widget {
    if role == "t" {
        let label = text_label(None, text, "cv-tool");
        label.set_max_width_chars(140);
        return label.upcast();
    }
    let me = role == "u";
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.style_context().add_class(if me { "cv-me" } else { "cv-ai" });
    column.set_halign(if me { gtk::Align::End } else { gtk::Align::Fill });
    let ctx = Render {
        palette: palette.clone(),
        width: if me { 72 } else { 110 },
    };
    render_blocks(&column, &chat_markdown::parse(text), &ctx, 0);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.style_context().add_class(if me { "cv-row-me" } else { "cv-row-ai" });
    if me {
        row.pack_end(&column, false, false, 0);
    } else {
        row.pack_start(&column, true, true, 0);
    }
    row.upcast()
}

/// Code colors from a theme: its ANSI palette, like the terminal.
fn code_palette(theme: Option<&crate::theme::ThemeTokens>) -> chat_markdown::CodePalette {
    let fallback;
    let theme = match theme {
        Some(theme) => theme,
        None => {
            fallback = crate::theme::desktop_theme("noche", &Value::Null);
            match &fallback {
                Some(theme) => theme,
                None => return chat_markdown::CodePalette::from_ansi(&[], "#7f848e", "#d4d4d4", "#0b0f17"),
            }
        }
    };
    let token = |name: &str, fallback: &'static str| {
        theme.values.get(name).and_then(Value::as_str).unwrap_or(fallback).to_owned()
    };
    // Code blocks sit on the theme background (`.cv-codeblock`).
    chat_markdown::CodePalette::from_ansi(&theme.ansi, &token("faint", "#7f848e"), &token("text", "#d4d4d4"), &token("bg", "#0b0f17"))
}

/// The message list exactly as the chat view draws it, for off-screen checks.
pub fn preview(messages: &[(String, String)], theme: Option<&crate::theme::ThemeTokens>) -> gtk::Widget {
    let palette = code_palette(theme);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.style_context().add_class("cv-root");
    let feed = gtk::Box::new(gtk::Orientation::Vertical, 12);
    feed.style_context().add_class("cv-feed");
    let list = gtk::Box::new(gtk::Orientation::Vertical, 10);
    list.style_context().add_class("cv-list");
    for (role, text) in messages {
        list.pack_start(&bubble(role, text, &palette), false, false, 0);
    }
    feed.pack_start(&list, false, false, 0);
    root.pack_start(&feed, true, true, 0);
    root.upcast()
}

impl App {
    pub(super) fn install_chat(self: &Rc<Self>, terminals: &gtk::Box, notebook: &gtk::Notebook) {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.style_context().add_class("cv-root");
        root.set_no_show_all(true);
        let title = gtk::Label::new(None);
        title.style_context().add_class("cv-title");
        title.set_xalign(0.);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 10);
        list.style_context().add_class("cv-list");
        let more = gtk::Button::with_label("");
        more.style_context().add_class("cv-more");
        more.set_halign(gtk::Align::Center);
        more.set_no_show_all(true);
        let weak = Rc::downgrade(self);
        more.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.chat.expanded.set(!app.chat.expanded.get());
                app.chat_draw(false);
            }
        });
        let typing = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        typing.style_context().add_class("cv-typing");
        typing.set_halign(gtk::Align::Start);
        let spinner = gtk::Spinner::new();
        spinner.start();
        typing.pack_start(&spinner, false, false, 0);
        typing.pack_start(&gtk::Label::new(Some("Trabajando…")), false, false, 0);
        typing.set_no_show_all(true);
        spinner.show();
        typing.children().iter().for_each(|c| c.show());
        let feed = gtk::Box::new(gtk::Orientation::Vertical, 12);
        feed.style_context().add_class("cv-feed");
        feed.pack_start(&more, false, false, 0);
        feed.pack_start(&list, false, false, 0);
        feed.pack_start(&typing, false, false, 0);
        let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.add(&feed);
        let keys = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        keys.style_context().add_class("cv-keys");
        for (label, key) in [("Esc", "Escape"), ("↑", "Up"), ("↓", "Down"), ("1", "1"), ("2", "2"), ("Tab", "Tab")] {
            let button = gtk::Button::with_label(label);
            button.style_context().add_class("cv-key");
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.chat_post("/key", json!({"key": key}));
                }
            });
            keys.pack_start(&button, false, false, 0);
        }
        let input = gtk::TextView::new();
        input.set_wrap_mode(gtk::WrapMode::WordChar);
        input.style_context().add_class("cv-input");
        input.set_accepts_tab(false);
        let send = gtk::Button::with_label("Enviar ↵");
        send.style_context().add_class("cv-send");
        let compose = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        compose.style_context().add_class("cv-compose");
        compose.pack_start(&input, true, true, 0);
        compose.pack_start(&send, false, false, 0);
        root.pack_start(&title, false, false, 0);
        root.pack_start(&scroll, true, true, 0);
        root.pack_start(&keys, false, false, 0);
        root.pack_start(&compose, false, false, 0);
        terminals.pack_start(&root, true, true, 0);
        let weak = Rc::downgrade(self);
        send.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.chat_send();
            }
        });
        let weak = Rc::downgrade(self);
        input.connect_key_press_event(move |_, event| {
            let enter = matches!(event.keyval(), gdk::keys::constants::Return | gdk::keys::constants::KP_Enter);
            if enter && !event.state().contains(gdk::ModifierType::SHIFT_MASK) {
                if let Some(app) = weak.upgrade() {
                    app.chat_send();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        let toggle = ui::icons::button("chat", 18, "Ver como chat");
        toggle.set_widget_name("chat");
        toggle.style_context().add_class("cc-key");
        let weak = Rc::downgrade(self);
        toggle.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.chat_show(!app.chat.on.get());
            }
        });
        self.tab_layout.actions().pack_start(&toggle, false, false, 0);
        toggle.show();
        *self.chat.view.borrow_mut() = Some(View {
            root,
            notebook: notebook.clone(),
            list,
            more,
            typing,
            send,
            scroll,
            title,
            input,
            toggle,
        });
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(1500), move || {
            let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                return glib::ControlFlow::Break;
            };
            app.chat_tick(false);
            glib::ControlFlow::Continue
        });
    }
    fn chat_show(self: &Rc<Self>, on: bool) {
        self.chat.on.set(on);
        if let Some(view) = &*self.chat.view.borrow() {
            if on {
                self.chat_cover(view);
                // `show_all` is a no-op on a widget marked `no_show_all`.
                view.root.set_no_show_all(false);
                view.root.show_all();
                view.input.grab_focus();
            } else {
                view.root.hide();
                view.root.set_no_show_all(true);
                for widget in self.chat.hidden.borrow_mut().drain(..) {
                    widget.show();
                }
            }
            view.toggle.set_tooltip_text(Some(if on { "Ver terminal" } else { "Ver como chat" }));
            let ctx = view.toggle.style_context();
            if on { ctx.add_class("cv-on") } else { ctx.remove_class("cv-on") }
        }
        self.chat.session.replace(String::new());
        self.chat_tick(true);
    }
    /// Hides whichever terminal container is showing (tabs or the pane mosaic).
    fn chat_cover(&self, view: &View) {
        let workspace: gtk::Widget = self.workspace.widget().clone().upcast();
        for widget in [view.notebook.clone().upcast::<gtk::Widget>(), workspace] {
            if widget.is_visible() {
                widget.hide();
                self.chat.hidden.borrow_mut().push(widget);
            }
        }
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
    fn chat_tick(self: &Rc<Self>, force: bool) {
        if !self.chat.on.get() || !self.window.is_active() && !force {
            return;
        }
        if let Some(view) = &*self.chat.view.borrow() {
            self.chat_cover(view);
        }
        let session = self.chat_target();
        if session.is_empty() {
            return;
        }
        if *self.chat.session.borrow() != session {
            self.chat.session.replace(session.clone());
            self.chat.token.replace(String::new());
            if let Some(view) = &*self.chat.view.borrow() {
                view.title.set_text(&self.chat_title(&session));
                for child in view.list.children() {
                    view.list.remove(&child);
                }
                self.chat.shown.borrow_mut().clear();
                self.chat.messages.borrow_mut().clear();
                self.chat.expanded.set(false);
                view.more.hide();
                view.typing.hide();
                let loading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                loading.style_context().add_class("cv-loading");
                loading.set_halign(gtk::Align::Center);
                let spinner = gtk::Spinner::new();
                spinner.start();
                loading.pack_start(&spinner, false, false, 0);
                loading.pack_start(&gtk::Label::new(Some("Cargando conversación…")), false, false, 0);
                view.list.pack_start(&loading, false, false, 0);
                view.list.show_all();
            }
        }
        if self.chat.busy.replace(true) {
            return;
        }
        let since = self.chat.token.borrow().clone();
        let dash = self.dash.clone();
        let body = json!({"session": session, "since": since});
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || dash.post("/chat/transcript", &body, Duration::from_secs(5)),
            move |result| {
                let Some(app) = weak.upgrade() else { return };
                app.chat.busy.set(false);
                let Ok((_, v)) = result else { return };
                if *app.chat.session.borrow() != session {
                    return;
                }
                app.chat.age.set(v["age"].as_u64().unwrap_or(u64::MAX));
                if v["unchanged"] == true {
                    app.chat_typing();
                    return;
                }
                app.chat.token.replace(v["token"].as_str().unwrap_or("").into());
                if v["agent"].is_null() {
                    app.chat.messages.borrow_mut().clear();
                    app.chat.shown.borrow_mut().clear();
                    if let Some(view) = &*app.chat.view.borrow() {
                        for child in view.list.children() {
                            view.list.remove(&child);
                        }
                        view.list.pack_start(&bubble("t", "Sin conversación de Claude o Codex en esta sesión. Usa la terminal.", &app.chat_palette()), false, false, 0);
                        view.list.show_all();
                    }
                    return;
                }
                let empty = Vec::new();
                *app.chat.messages.borrow_mut() = v["messages"]
                    .as_array()
                    .unwrap_or(&empty)
                    .iter()
                    .map(|m| (m["r"].as_str().unwrap_or("t").to_owned(), m["t"].as_str().unwrap_or("").to_owned()))
                    .collect();
                app.chat_draw(since.is_empty());
            },
        );
    }
    /// Draws the visible slice (the last answer, or everything when expanded), rebuilding
    /// only from the first message that changed.
    fn chat_palette(&self) -> chat_markdown::CodePalette {
        code_palette(self.applied_theme.borrow().as_ref())
    }
    fn chat_draw(&self, first: bool) {
        let palette = self.chat_palette();
        let Some(view) = &*self.chat.view.borrow() else { return };
        let adj = view.scroll.vadjustment();
        let from_bottom = adj.upper() - adj.value();
        let pinned = first || adj.upper() - adj.value() - adj.page_size() < 120.;
        let messages = self.chat.messages.borrow();
        let folded = cut(&messages);
        let from = if self.chat.expanded.get() { 0 } else { folded };
        let slice = &messages[from..];
        let children = view.list.children();
        let mut shown = self.chat.shown.borrow_mut();
        let keep = if children.len() == shown.len() {
            shown.iter().zip(slice).take_while(|(a, b)| a == b).count()
        } else {
            0
        };
        for child in children.iter().skip(keep) {
            view.list.remove(child);
        }
        if slice.is_empty() {
            view.list.pack_start(&bubble("t", "Sin conversación todavía. Escribe abajo o vuelve a la terminal.", &palette), false, false, 0);
            shown.clear();
        } else {
            for (role, text) in &slice[keep..] {
                view.list.pack_start(&bubble(role, text, &palette), false, false, 0);
            }
            *shown = slice.to_vec();
        }
        view.list.show_all();
        view.more.set_label(&match (self.chat.expanded.get(), folded) {
            (true, _) => "Ocultar mensajes anteriores".to_owned(),
            (false, 1) => "Ver 1 mensaje anterior".to_owned(),
            (false, n) => format!("Ver {n} mensajes anteriores"),
        });
        view.more.set_visible(folded > 0);
        drop(shown);
        drop(messages);
        self.chat_typing();
        let scroll = view.scroll.clone();
        let expanded = self.chat.expanded.get();
        glib::idle_add_local_once(move || {
            let adj = scroll.vadjustment();
            if pinned {
                adj.set_value(adj.upper() - adj.page_size());
            } else if expanded {
                adj.set_value(adj.upper() - from_bottom);
            }
        });
    }
    /// «Working» while the transcript moved in the last 90 s and does not end in an answer.
    fn chat_typing(&self) {
        let Some(view) = &*self.chat.view.borrow() else { return };
        let busy = self.chat.age.get() < 90 && self.chat.messages.borrow().last().is_some_and(|(r, _)| r != "a");
        view.typing.set_visible(busy);
    }
    fn chat_send(self: &Rc<Self>) {
        let text = {
            let Some(view) = &*self.chat.view.borrow() else { return };
            let buffer = view.input.buffer().expect("text view buffer");
            let (start, end) = buffer.bounds();
            let text = buffer.text(&start, &end, false).map(|t| t.to_string()).unwrap_or_default();
            if text.trim().is_empty() {
                return;
            }
            buffer.set_text("");
            let pending = bubble("u", &text, &self.chat_palette());
            pending.set_opacity(0.6);
            view.list.pack_start(&pending, false, false, 0);
            // The pending bubble breaks the list↔shown pairing, so the next draw rebuilds.
            view.list.show_all();
            view.typing.show();
            view.send.set_sensitive(false);
            view.send.set_label("Enviando…");
            let adj = view.scroll.vadjustment();
            glib::idle_add_local_once(move || adj.set_value(adj.upper() - adj.page_size()));
            text
        };
        self.chat.age.set(0);
        self.chat_post("/send", json!({"text": text}));
    }
    fn chat_post(self: &Rc<Self>, path: &'static str, mut body: Value) {
        let session = self.chat_target();
        if session.is_empty() || !self.writable() {
            return;
        }
        body["session"] = session.into();
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
                    if let Some(view) = &*app.chat.view.borrow() {
                        view.send.set_sensitive(true);
                        view.send.set_label("Enviar ↵");
                    }
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
        let m = |r: &str| (r.to_owned(), String::new());
        assert_eq!(cut(&[m("u"), m("a"), m("u"), m("t"), m("a"), m("t")]), 4);
        assert_eq!(cut(&[m("t"), m("u"), m("t")]), 1);
        assert_eq!(cut(&[]), 0);
    }
}
