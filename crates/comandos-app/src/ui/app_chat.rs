//! Chat view of the current session: its agent transcript as bubbles, a composer that
//! types into the pane and quick keys. It is its own page of the terminal stack, so
//! nothing that re-shows the tabs or the mosaic can draw tmux under it. Answers are
//! rich text (`chat_rich`): select across a whole answer, copy with formatting, edit in
//! place and copy the edit (the agent never sees it). Scrolling up over an agent pane
//! opens this view on that pane; «Historial de tmux» hands the wheel back to tmux.
//! The timer only fetches while the view is visible; an unchanged transcript costs
//! one tiny answer from the server.
use super::*;
use crate::ui::chat_markdown::{self, Block};
use crate::ui::chat_rich::{self, Colors, Hooks, Rich};

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
    /// Panes whose wheel goes to tmux again (after «Historial de tmux»).
    raw: RefCell<std::collections::BTreeSet<String>>,
    token: RefCell<String>,
    busy: Cell<bool>,
    /// Bubbles being edited: the list is not redrawn under them.
    editing: Cell<usize>,
    pinned: Cell<bool>,
    chips: RefCell<String>,
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
    stack: gtk::Stack,
    list: gtk::Box,
    more: gtk::Button,
    typing: gtk::Box,
    send: gtk::Button,
    scroll: gtk::ScrolledWindow,
    title: gtk::Label,
    chips: gtk::Box,
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

/// The pane under a terminal cell (pane headers sit on the row above each pane).
fn pane_at(panes: &[ui::overlays::PaneRecord], col: u16, row: u16) -> Option<&ui::overlays::PaneRecord> {
    panes.iter().find(|p| {
        let g = &p.geometry;
        col >= g.left && col < g.left + g.width && row + 1 >= g.top && row < g.top + g.height
    })
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

/// A prompt the person wrote, as one selectable label (prompts are short).
fn prompt_markup(blocks: &[Block], out: &mut Vec<String>) {
    for block in blocks {
        match block {
            Block::Para(m) | Block::Heading(_, m) => out.push(m.clone()),
            Block::Code { body, .. } => out.push(chat_markdown::code_span(body)),
            Block::Quote(inner) => prompt_markup(inner, out),
            Block::List { items, .. } => {
                for item in items {
                    let mut inner = Vec::new();
                    prompt_markup(&item.blocks, &mut inner);
                    out.push(format!("• {}", inner.join("\n")));
                }
            }
            Block::Table { rows, .. } => out.extend(rows.iter().map(|r| r.join("  ·  "))),
            Block::Rule => out.push("—".into()),
        }
    }
}

/// What a bubble needs from its host (the live app, or an off-screen preview).
#[derive(Clone)]
struct Ctx {
    palette: chat_markdown::CodePalette,
    colors: Colors,
    hooks: Hooks,
    /// Called with `true` when a bubble starts being edited and `false` when it ends.
    editing: Rc<dyn Fn(bool)>,
    /// Typing on an answer goes to the composer.
    composer: Option<gtk::TextView>,
}

fn small_button(label: &str, class: &str, tip: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.style_context().add_class("cv-act");
    if !class.is_empty() {
        button.style_context().add_class(class);
    }
    button.set_tooltip_text(Some(tip));
    button.set_can_focus(false);
    button
}

/// An agent answer: rich text plus «Copiar» and «✎ Editar»; editing swaps the row to
/// «Copiar editado», «Listo» and «Deshacer».
fn answer(text: &str, ctx: &Ctx) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.style_context().add_class("cv-ai");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let rich = Rc::new(RefCell::new(Rich::new(text, &ctx.colors, &ctx.palette, &ctx.hooks)));
    holder.pack_start(&rich.borrow().view, false, false, 0);
    forward_typing(&rich.borrow(), ctx);
    column.pack_start(&holder, false, false, 0);
    let acts = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    acts.style_context().add_class("cv-acts");
    let copy = small_button("Copiar", "", "Copiar la respuesta con su formato");
    let edit = small_button("✎ Editar", "", "Editar el texto aquí para copiarlo; la IA no ve el cambio");
    let copy_edit = small_button("Copiar editado", "cv-primary", "Copiar tu versión con su formato");
    let done = small_button("Listo", "", "Terminar de editar");
    let undo = small_button("Deshacer", "", "Volver al texto original");
    for b in [&copy, &edit, &copy_edit, &done, &undo] {
        acts.pack_start(b, false, false, 0);
    }
    for b in [&copy_edit, &done, &undo] {
        b.set_no_show_all(true);
    }
    column.pack_start(&acts, false, false, 0);
    let mode = {
        let (copy, edit, copy_edit, done, undo) = (copy.clone(), edit.clone(), copy_edit.clone(), done.clone(), undo.clone());
        Rc::new(move |editing: bool| {
            copy.set_visible(!editing);
            edit.set_visible(!editing);
            copy_edit.set_visible(editing);
            done.set_visible(editing);
            undo.set_visible(editing);
        })
    };
    let copy_now = {
        let (rich, hooks) = (rich.clone(), ctx.hooks.clone());
        Rc::new(move || {
            let (html, plain) = rich.borrow().export();
            (hooks.copy)(&plain, Some(&html));
        })
    };
    let run = copy_now.clone();
    copy.connect_clicked(move |b| {
        run();
        chat_rich::flash(b, "Copiar");
    });
    let run = copy_now;
    copy_edit.connect_clicked(move |b| {
        run();
        chat_rich::flash(b, "Copiar editado");
    });
    {
        let (rich, mode, editing) = (rich.clone(), mode.clone(), ctx.editing.clone());
        edit.connect_clicked(move |_| {
            editing(true);
            mode(true);
            rich.borrow().set_editing(true);
        });
    }
    {
        let (rich, mode, editing) = (rich.clone(), mode.clone(), ctx.editing.clone());
        done.connect_clicked(move |_| {
            rich.borrow().set_editing(false);
            mode(false);
            editing(false);
        });
    }
    {
        let (rich, mode, ctx, holder, text) = (rich.clone(), mode, ctx.clone(), holder.clone(), text.to_owned());
        undo.connect_clicked(move |_| {
            let fresh = Rich::new(&text, &ctx.colors, &ctx.palette, &ctx.hooks);
            forward_typing(&fresh, &ctx);
            holder.remove(&rich.borrow().view);
            holder.pack_start(&fresh.view, false, false, 0);
            fresh.view.show_all();
            *rich.borrow_mut() = fresh;
            mode(false);
            (ctx.editing)(false);
        });
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.style_context().add_class("cv-row-ai");
    row.pack_start(&column, true, true, 0);
    row.upcast()
}

/// Letters typed while an answer (not being edited) has the focus go to the composer.
fn forward_typing(rich: &Rich, ctx: &Ctx) {
    let Some(input) = ctx.composer.clone() else { return };
    rich.view.connect_key_press_event(move |view, event| {
        let plain = !event.state().intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::MOD1_MASK);
        let typed = event.keyval().to_unicode().filter(|c| !c.is_control());
        match typed {
            Some(c) if plain && !view.is_editable() => {
                input.grab_focus();
                if let Some(buffer) = input.buffer() {
                    buffer.insert_at_cursor(&c.to_string());
                }
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
}

/// One message as a widget: answers are rich text, prompts a bubble, tool calls a slim line.
fn bubble(role: &str, text: &str, ctx: &Ctx) -> gtk::Widget {
    match role {
        "a" => answer(text, ctx),
        "u" => {
            let mut parts = Vec::new();
            prompt_markup(&chat_markdown::parse(text), &mut parts);
            let label = text_label(Some(&parts.join("\n\n")), "", "cv-text");
            label.set_max_width_chars(72);
            let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
            column.style_context().add_class("cv-me");
            column.pack_start(&label, false, false, 0);
            let copy = small_button("Copiar", "", "Copiar este mensaje");
            copy.style_context().add_class("cv-act-me");
            copy.set_halign(gtk::Align::End);
            let (hooks, raw) = (ctx.hooks.clone(), text.to_owned());
            copy.connect_clicked(move |b| {
                (hooks.copy)(&raw, None);
                chat_rich::flash(b, "Copiar");
            });
            let outer = gtk::Box::new(gtk::Orientation::Vertical, 2);
            outer.pack_start(&column, false, false, 0);
            outer.pack_start(&copy, false, false, 0);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.style_context().add_class("cv-row-me");
            row.pack_end(&outer, false, false, 0);
            row.upcast()
        }
        _ => {
            let label = text_label(None, text, "cv-tool");
            label.set_max_width_chars(140);
            label.upcast()
        }
    }
}

fn hex(c: &str) -> Option<[f64; 3]> {
    let h = c.strip_prefix('#')?;
    let byte = |i: usize| h.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()).map(f64::from);
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// `a` moved toward `b` by `k` (0–1), as `#rrggbb`.
fn mix(a: &str, b: &str, k: f64) -> String {
    match (hex(a), hex(b)) {
        (Some(a), Some(b)) => {
            let m = |i: usize| (a[i] + (b[i] - a[i]) * k).round().clamp(0., 255.) as u8;
            format!("#{:02x}{:02x}{:02x}", m(0), m(1), m(2))
        }
        _ => a.to_owned(),
    }
}

fn theme_or_default(theme: Option<&crate::theme::ThemeTokens>) -> Option<crate::theme::ThemeTokens> {
    theme.cloned().or_else(|| crate::theme::desktop_theme("noche", &Value::Null))
}

fn colors(theme: Option<&crate::theme::ThemeTokens>) -> Colors {
    let theme = theme_or_default(theme);
    let token = |name: &str, fallback: &str| {
        theme
            .as_ref()
            .and_then(|t| t.values.get(name))
            .and_then(Value::as_str)
            .unwrap_or(fallback)
            .to_owned()
    };
    let (bar, text, bg) = (token("bar", "#151a24"), token("text", "#d4d4d4"), token("bg", "#0b0f17"));
    Colors {
        dim: token("dim", "#9aa3b5"),
        faint: chat_markdown::CodePalette::from_ansi(&[], &token("faint", "#7f848e"), &text, &bg).comment,
        brand: token("brand", "#8b7cf6"),
        code_bg: bg,
        quote_bg: mix(&bar, &text, 0.05),
    }
}

/// Code colors from a theme: its ANSI palette, like the terminal.
fn code_palette(theme: Option<&crate::theme::ThemeTokens>) -> chat_markdown::CodePalette {
    let Some(theme) = theme_or_default(theme) else {
        return chat_markdown::CodePalette::from_ansi(&[], "#7f848e", "#d4d4d4", "#0b0f17");
    };
    let token = |name: &str, fallback: &'static str| {
        theme.values.get(name).and_then(Value::as_str).unwrap_or(fallback).to_owned()
    };
    // Code blocks sit on the theme background.
    chat_markdown::CodePalette::from_ansi(&theme.ansi, &token("faint", "#7f848e"), &token("text", "#d4d4d4"), &token("bg", "#0b0f17"))
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

fn preview_ctx(theme: Option<&crate::theme::ThemeTokens>) -> Ctx {
    Ctx {
        palette: code_palette(theme),
        colors: colors(theme),
        hooks: Hooks { copy: Rc::new(copy_to_clipboard), open: Rc::new(open_url) },
        editing: Rc::new(|_| {}),
        composer: None,
    }
}

/// The message list exactly as the chat view draws it, for off-screen checks.
pub fn preview(messages: &[(String, String)], theme: Option<&crate::theme::ThemeTokens>) -> gtk::Widget {
    let ctx = preview_ctx(theme);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.style_context().add_class("cv-root");
    let feed = gtk::Box::new(gtk::Orientation::Vertical, 12);
    feed.style_context().add_class("cv-feed");
    let list = gtk::Box::new(gtk::Orientation::Vertical, 14);
    list.style_context().add_class("cv-list");
    for (role, text) in messages {
        list.pack_start(&bubble(role, text, &ctx), false, false, 0);
    }
    feed.pack_start(&list, false, false, 0);
    root.pack_start(&feed, true, true, 0);
    root.upcast()
}

impl App {
    /// Adds the chat as the `chat` page of the terminal stack (`term` is the other).
    pub(super) fn install_chat(self: &Rc<Self>, stack: &gtk::Stack) {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.style_context().add_class("cv-root");
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.style_context().add_class("cv-head");
        let title = gtk::Label::new(None);
        title.style_context().add_class("cv-title");
        title.set_xalign(0.);
        title.set_ellipsize(pango::EllipsizeMode::End);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        chips.style_context().add_class("cv-chips");
        let raw = small_button("Historial de tmux", "", "Volver a la terminal; la rueda vuelve a recorrer el historial de tmux en este pane");
        let back = small_button("Terminal", "cv-primary", "Volver a la terminal");
        head.pack_start(&title, false, false, 0);
        head.pack_start(&chips, false, false, 0);
        head.pack_end(&back, false, false, 0);
        head.pack_end(&raw, false, false, 0);
        let weak = Rc::downgrade(self);
        back.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.chat_show(false);
            }
        });
        let weak = Rc::downgrade(self);
        raw.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                let pane = app.chat.answered.borrow().clone();
                if !pane.is_empty() {
                    app.chat.raw.borrow_mut().insert(pane);
                }
                app.chat_show(false);
            }
        });
        let list = gtk::Box::new(gtk::Orientation::Vertical, 14);
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
        feed.set_halign(gtk::Align::Center);
        feed.pack_start(&more, false, false, 0);
        feed.pack_start(&list, false, false, 0);
        feed.pack_start(&typing, false, false, 0);
        let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.add(&feed);
        // A readable column on wide windows.
        let column = feed.clone();
        scroll.connect_size_allocate(move |_, alloc| {
            let width = (alloc.width() - 40).clamp(300, 1100);
            if column.width_request() != width {
                let column = column.clone();
                glib::idle_add_local_once(move || column.set_size_request(width, -1));
            }
        });
        // Stay at the bottom while answers grow (rich text lays out in steps).
        let adj = scroll.vadjustment();
        let weak = Rc::downgrade(self);
        adj.connect_value_changed(move |adj| {
            if let Some(app) = weak.upgrade() {
                app.chat.pinned.set(adj.upper() - adj.value() - adj.page_size() < 60.);
            }
        });
        let weak = Rc::downgrade(self);
        adj.connect_changed(move |adj| {
            if let Some(app) = weak.upgrade()
                && app.chat.pinned.get()
            {
                adj.set_value(adj.upper() - adj.page_size());
            }
        });
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
        let paste = gtk::Button::with_label("Pegar");
        paste.style_context().add_class("cv-key");
        paste.set_tooltip_text(Some("Pegar el portapapeles en el mensaje"));
        let target = input.clone();
        paste.connect_clicked(move |_| {
            let target = target.clone();
            gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).request_text(move |_, text| {
                if let (Some(text), Some(buffer)) = (text, target.buffer()) {
                    buffer.insert_at_cursor(text);
                    target.grab_focus();
                }
            });
        });
        keys.pack_end(&paste, false, false, 0);
        let field = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        field.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        field.set_propagate_natural_height(true);
        field.set_max_content_height(200);
        field.add(&input);
        let send = gtk::Button::with_label("Enviar ↵");
        send.style_context().add_class("cv-send");
        let compose = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        compose.style_context().add_class("cv-compose");
        compose.pack_start(&field, true, true, 0);
        compose.pack_start(&send, false, false, 0);
        root.pack_start(&head, false, false, 0);
        root.pack_start(&scroll, true, true, 0);
        root.pack_start(&keys, false, false, 0);
        root.pack_start(&compose, false, false, 0);
        root.show_all();
        stack.add_named(&root, "chat");
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
                let on = !app.chat.on.get();
                if on {
                    // Opened by hand: the active pane, and the wheel opens the chat again.
                    app.chat.pane.replace(String::new());
                    app.chat.raw.borrow_mut().clear();
                }
                app.chat_show(on);
            }
        });
        self.tab_layout.actions().pack_start(&toggle, false, false, 0);
        toggle.show();
        *self.chat.view.borrow_mut() = Some(View {
            stack: stack.clone(),
            list,
            more,
            typing,
            send,
            scroll,
            title,
            chips,
            input,
            toggle,
        });
        self.chat.pinned.set(true);
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
            view.stack.set_visible_child_name(if on { "chat" } else { "term" });
            if on {
                view.input.grab_focus();
            }
            view.toggle.set_tooltip_text(Some(if on { "Ver terminal" } else { "Ver como chat" }));
            let ctx = view.toggle.style_context();
            if on { ctx.add_class("cv-on") } else { ctx.remove_class("cv-on") }
        }
        if !on {
            self.focus_current_term();
            return;
        }
        self.chat.session.replace(String::new());
        self.chat_tick(true);
    }
    /// The wheel went up over `key`'s terminal at a cell: over an agent pane, the chat
    /// opens on that pane instead of tmux's scrollback (which mixes widths and repeats
    /// redraws). Returns whether it took the wheel.
    pub(super) fn chat_wheel(self: &Rc<Self>, key: &str, col: u16, row: u16) -> bool {
        if self.chat.on.get() || self.chat.view.borrow().is_none() {
            return false;
        }
        let pane = {
            let boxes = self.t17.boxes.borrow();
            let Some(owner) = boxes.get(key) else { return false };
            let panes = owner.panes.borrow();
            match pane_at(&panes, col, row) {
                Some(p) if is_agent(&p.command) && !self.chat.raw.borrow().contains(&p.geometry.id) => p.geometry.id.clone(),
                _ => return false,
            }
        };
        if self.current_session().as_deref() != Some(key) {
            self.select(key);
        }
        self.chat.pane.replace(pane);
        self.chat_show(true);
        true
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
    fn chat_ctx(self: &Rc<Self>) -> Ctx {
        let theme = self.applied_theme.borrow();
        let weak = Rc::downgrade(self);
        Ctx {
            palette: code_palette(theme.as_ref()),
            colors: colors(theme.as_ref()),
            hooks: Hooks { copy: Rc::new(copy_to_clipboard), open: Rc::new(open_url) },
            editing: Rc::new(move |on| {
                if let Some(app) = weak.upgrade() {
                    let n = app.chat.editing.get();
                    app.chat.editing.set(if on { n + 1 } else { n.saturating_sub(1) });
                    if !on && app.chat.editing.get() == 0 {
                        app.chat_draw(false);
                    }
                }
            }),
            composer: self.chat.view.borrow().as_ref().map(|v| v.input.clone()),
        }
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
            self.chat.editing.set(0);
            self.chat.pinned.set(true);
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
                if v["unchanged"] == true {
                    app.chat_typing();
                    return;
                }
                app.chat.token.replace(v["token"].as_str().unwrap_or("").into());
                if v["agent"].is_null() {
                    app.chat.messages.borrow_mut().clear();
                    app.chat.shown.borrow_mut().clear();
                    let ctx = app.chat_ctx();
                    if let Some(view) = &*app.chat.view.borrow() {
                        for child in view.list.children() {
                            view.list.remove(&child);
                        }
                        let note = if app.chat.chips.borrow().contains("Terminal") {
                            "Este pane es una terminal sin Claude ni Codex. Elige otro pane arriba o vuelve a la terminal."
                        } else {
                            "Sin conversación de Claude o Codex en esta sesión. Usa la terminal."
                        };
                        view.list.pack_start(&bubble("t", note, &ctx), false, false, 0);
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
    /// only from the first message that changed. Waits while a bubble is being edited.
    fn chat_draw(self: &Rc<Self>, first: bool) {
        if self.chat.editing.get() > 0 {
            return;
        }
        let ctx = self.chat_ctx();
        let Some(view) = &*self.chat.view.borrow() else { return };
        let adj = view.scroll.vadjustment();
        let from_bottom = adj.upper() - adj.value();
        if first {
            self.chat.pinned.set(true);
        }
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
            view.list.pack_start(&bubble("t", "Sin conversación todavía. Escribe abajo o vuelve a la terminal.", &ctx), false, false, 0);
            shown.clear();
        } else {
            for (role, text) in &slice[keep..] {
                view.list.pack_start(&bubble(role, text, &ctx), false, false, 0);
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
        let expanded = self.chat.expanded.get() && !self.chat.pinned.get();
        let pinned = self.chat.pinned.get();
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
        let ctx = self.chat_ctx();
        let text = {
            let Some(view) = &*self.chat.view.borrow() else { return };
            let buffer = view.input.buffer().expect("text view buffer");
            let (start, end) = buffer.bounds();
            let text = buffer.text(&start, &end, false).map(|t| t.to_string()).unwrap_or_default();
            if text.trim().is_empty() {
                return;
            }
            buffer.set_text("");
            let pending = bubble("u", &text, &ctx);
            pending.set_opacity(0.6);
            view.list.pack_start(&pending, false, false, 0);
            // The pending bubble breaks the list↔shown pairing, so the next draw rebuilds.
            view.list.show_all();
            view.typing.show();
            view.send.set_sensitive(false);
            view.send.set_label("Enviando…");
            self.chat.pinned.set(true);
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

    fn record(id: &str, left: u16, top: u16, width: u16, height: u16, command: &str) -> ui::overlays::PaneRecord {
        ui::overlays::PaneRecord {
            geometry: ui::overlays::PaneGeometry { id: id.into(), left, top, width, height },
            command: command.into(),
            active: false,
        }
    }

    #[test]
    fn the_wheel_finds_the_pane_under_it_including_its_header_row() {
        let panes = [record("%1", 0, 1, 80, 30, "claude"), record("%2", 81, 1, 79, 30, "zsh")];
        assert_eq!(pane_at(&panes, 10, 0).map(|p| p.geometry.id.as_str()), Some("%1"));
        assert_eq!(pane_at(&panes, 79, 30).map(|p| p.geometry.id.as_str()), Some("%1"));
        assert_eq!(pane_at(&panes, 90, 5).map(|p| p.geometry.id.as_str()), Some("%2"));
        assert!(pane_at(&panes, 80, 5).is_none(), "the divider column belongs to no pane");
        assert!(is_agent("claude") && is_agent("codex") && !is_agent("zsh"));
    }

    #[test]
    fn chips_name_what_runs_in_each_pane() {
        assert_eq!(pane_label(1, "claude"), "1 · Claude");
        assert_eq!(pane_label(2, "zsh"), "2 · Terminal");
        assert_eq!(pane_label(3, "htop"), "3 · htop");
    }

    #[test]
    fn quote_tint_mixes_toward_the_text_color() {
        assert_eq!(mix("#000000", "#ffffff", 0.5), "#808080");
        assert_eq!(mix("#101010", "nope", 0.5), "#101010");
    }
}
