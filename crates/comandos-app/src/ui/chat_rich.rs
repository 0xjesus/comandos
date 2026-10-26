//! Agent answers as rich text: one GtkTextView per message, so a selection runs across
//! the whole answer, the text can be edited in place, and a copy carries HTML plus
//! clean text. Built from `chat_markdown` blocks (whose inline text is our own small
//! Pango subset); tables and the code «Copiar» buttons are child widgets.
use crate::ui::chat_markdown::{self, Align, Block, CodePalette};
use gtk::prelude::*;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// Theme colors the tags need (CSS cannot reach a text tag).
#[derive(Clone, Debug)]
pub struct Colors {
    pub dim: String,
    pub faint: String,
    pub brand: String,
    pub code_bg: String,
    pub quote_bg: String,
}

/// What copy buttons and links call back into: `copy(plain, html)` and `open(url)`.
#[derive(Clone)]
pub struct Hooks {
    pub copy: Rc<dyn Fn(&str, Option<&str>)>,
    pub open: Rc<dyn Fn(&str)>,
}

#[derive(Clone)]
struct Tags {
    b: gtk::TextTag,
    i: gtk::TextTag,
    s: gtk::TextTag,
    sup: gtk::TextTag,
    sub: gtk::TextTag,
    code: gtk::TextTag,
    pre: gtk::TextTag,
    head: gtk::TextTag,
    quote: gtk::TextTag,
    dim: gtk::TextTag,
    mark: gtk::TextTag,
    done: gtk::TextTag,
    rule: gtk::TextTag,
    h: [gtk::TextTag; 6],
    item: gtk::TextTag,
    gap: gtk::TextTag,
}

struct State {
    tags: Tags,
    links: Vec<(gtk::TextTag, String)>,
    tables: Vec<(gtk::TextChildAnchor, Vec<Vec<String>>)>,
    margins: HashMap<(i32, i32), gtk::TextTag>,
}

/// A rendered message; clone freely (it is a handle).
#[derive(Clone)]
pub struct Rich {
    pub view: gtk::TextView,
    state: Rc<RefCell<State>>,
}

fn new_tag(table: &gtk::TextTagTable, f: impl FnOnce(&gtk::TextTag)) -> gtk::TextTag {
    let tag = gtk::TextTag::new(None);
    f(&tag);
    table.add(&tag);
    tag
}

fn tags(table: &gtk::TextTagTable, c: &Colors) -> Tags {
    let heading = |scale: f64| {
        new_tag(table, |t| {
            t.set_weight(700);
            t.set_scale(scale);
            t.set_pixels_above_lines(12);
            t.set_pixels_below_lines(8);
        })
    };
    let h = [heading(1.45), heading(1.28), heading(1.14), heading(1.05), heading(1.0), heading(1.0)];
    let b = new_tag(table, |t| t.set_weight(700));
    let i = new_tag(table, |t| t.set_style(pango::Style::Italic));
    let s = new_tag(table, |t| t.set_strikethrough(true));
    let sup = new_tag(table, |t| {
        t.set_rise(4 * pango::SCALE);
        t.set_scale(0.8);
    });
    let sub = new_tag(table, |t| {
        t.set_rise(-3 * pango::SCALE);
        t.set_scale(0.8);
    });
    let code = new_tag(table, |t| {
        t.set_family(Some("monospace"));
        t.set_background(Some(&c.code_bg));
        t.set_scale(0.95);
    });
    let pre = new_tag(table, |t| {
        t.set_family(Some("monospace"));
        t.set_paragraph_background(Some(&c.code_bg));
        t.set_scale(0.94);
        t.set_wrap_mode(gtk::WrapMode::Char);
        t.set_pixels_above_lines(0);
        t.set_pixels_below_lines(0);
    });
    let head = new_tag(table, |t| {
        t.set_paragraph_background(Some(&c.code_bg));
        t.set_foreground(Some(&c.faint));
        t.set_family(Some("monospace"));
        t.set_scale(0.82);
        t.set_pixels_above_lines(8);
        t.set_pixels_below_lines(2);
    });
    let quote = new_tag(table, |t| t.set_paragraph_background(Some(&c.quote_bg)));
    let dim = new_tag(table, |t| t.set_foreground(Some(&c.dim)));
    let mark = new_tag(table, |t| t.set_foreground(Some(&c.dim)));
    let done = new_tag(table, |t| t.set_foreground(Some(&c.brand)));
    let rule = new_tag(table, |t| {
        t.set_foreground(Some(&c.faint));
        t.set_justification(gtk::Justification::Center);
        t.set_pixels_above_lines(6);
        t.set_pixels_below_lines(10);
    });
    let item = new_tag(table, |t| t.set_pixels_below_lines(3));
    // Created last: wins over `item`/`pre` spacing on the line it marks.
    let gap = new_tag(table, |t| t.set_pixels_below_lines(14));
    Tags { b, i, s, sup, sub, code, pre, head, quote, dim, mark, done, rule, h, item, gap }
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let at = tag.find(&key)? + key.len();
    let end = tag[at..].find('"')? + at;
    Some(unescape(&tag[at..end]))
}

struct Builder<'a> {
    buffer: gtk::TextBuffer,
    table: gtk::TextTagTable,
    state: State,
    colors: &'a Colors,
    palette: &'a CodePalette,
    hooks: &'a Hooks,
    anchors: Vec<(gtk::TextChildAnchor, gtk::Widget)>,
    fg: HashMap<String, gtk::TextTag>,
}

#[derive(Clone, Copy)]
struct Ctx {
    margin: i32,
    depth: usize,
    quote: bool,
}

impl Builder<'_> {
    fn offset(&self) -> i32 {
        self.buffer.end_iter().offset()
    }
    fn push(&mut self, text: &str, tags: &[gtk::TextTag]) {
        if text.is_empty() {
            return;
        }
        let start = self.offset();
        let mut end = self.buffer.end_iter();
        self.buffer.insert(&mut end, text);
        self.apply(start, tags);
    }
    fn apply(&self, start: i32, tags: &[gtk::TextTag]) {
        let s = self.buffer.iter_at_offset(start);
        let e = self.buffer.end_iter();
        for tag in tags {
            self.buffer.apply_tag(tag, &s, &e);
        }
    }
    /// A paragraph-level tag for this left margin (and hanging indent).
    fn margin(&mut self, left: i32, indent: i32) -> gtk::TextTag {
        if let Some(tag) = self.state.margins.get(&(left, indent)) {
            return tag.clone();
        }
        let tag = new_tag(&self.table, |t| {
            t.set_left_margin(left);
            t.set_indent(indent);
        });
        self.state.margins.insert((left, indent), tag.clone());
        tag
    }
    fn color(&mut self, color: &str) -> gtk::TextTag {
        if let Some(tag) = self.fg.get(color) {
            return tag.clone();
        }
        let tag = new_tag(&self.table, |t| t.set_foreground(Some(color)));
        self.fg.insert(color.to_owned(), tag.clone());
        tag
    }
    fn link(&mut self, url: Option<String>) -> Option<gtk::TextTag> {
        let url = url?;
        let brand = self.colors.brand.clone();
        let tag = new_tag(&self.table, |t| {
            t.set_foreground(Some(&brand));
            t.set_underline(pango::Underline::Single);
        });
        self.state.links.push((tag.clone(), url));
        Some(tag)
    }
    /// Our Pango subset (`<b> <i> <s> <sup> <sub> <a href> <span …>`) as tagged runs.
    fn markup(&mut self, markup: &str, base: &[gtk::TextTag]) {
        let mut stack: Vec<Option<gtk::TextTag>> = Vec::new();
        let mut rest = markup;
        while !rest.is_empty() {
            if let Some(open) = rest.strip_prefix('<')
                && let Some(close) = open.find('>')
            {
                let tag = &open[..close];
                rest = &open[close + 1..];
                if tag.starts_with('/') {
                    stack.pop();
                    continue;
                }
                let name = tag.split_whitespace().next().unwrap_or("");
                let found = match name {
                    "b" => Some(self.state.tags.b.clone()),
                    "i" => Some(self.state.tags.i.clone()),
                    "s" => Some(self.state.tags.s.clone()),
                    "sup" => Some(self.state.tags.sup.clone()),
                    "sub" => Some(self.state.tags.sub.clone()),
                    "a" => self.link(attr(tag, "href")),
                    "span" if attr(tag, "font_family").is_some() => Some(self.state.tags.code.clone()),
                    "span" => attr(tag, "foreground").map(|c| self.color(&c)),
                    _ => None,
                };
                stack.push(found);
                continue;
            }
            let skip = rest.chars().next().map_or(1, char::len_utf8);
            let end = rest[skip..].find('<').map_or(rest.len(), |n| n + skip);
            let text = unescape(&rest[..end]);
            let mut tags = base.to_vec();
            tags.extend(stack.iter().flatten().cloned());
            self.push(&text, &tags);
            rest = &rest[end..];
        }
    }
    /// Closes the line just written and gives its paragraph extra space below.
    fn gap(&mut self) {
        let end = self.buffer.end_iter();
        let mut start = end.clone();
        if start.backward_char() {
            start.set_line_offset(0);
            let gap = self.state.tags.gap.clone();
            self.buffer.apply_tag(&gap, &start, &end);
        }
    }
    fn line(&mut self, markup: &str, line_tags: &[gtk::TextTag], inline: &[gtk::TextTag]) {
        let start = self.offset();
        self.markup(markup, inline);
        self.push("\n", &[]);
        self.apply(start, line_tags);
    }
    fn blocks(&mut self, blocks: &[Block], ctx: Ctx) {
        for block in blocks {
            self.block(block, ctx);
        }
    }
    fn inline_base(&self, ctx: Ctx) -> Vec<gtk::TextTag> {
        if ctx.quote { vec![self.state.tags.dim.clone()] } else { Vec::new() }
    }
    fn block(&mut self, block: &Block, ctx: Ctx) {
        let base = self.inline_base(ctx);
        match block {
            Block::Para(markup) => {
                let margin = self.margin(ctx.margin, 0);
                self.line(markup, &[margin], &base);
                self.gap();
            }
            Block::Heading(level, markup) => {
                let margin = self.margin(ctx.margin, 0);
                let tag = self.state.tags.h[usize::from((*level).clamp(1, 6)) - 1].clone();
                self.line(markup, &[margin, tag], &base);
            }
            Block::Code { lang, body } => self.code(lang, body, ctx),
            Block::Quote(inner) => {
                let start = self.offset();
                self.blocks(inner, Ctx { margin: ctx.margin + 16, quote: true, ..ctx });
                let quote = self.state.tags.quote.clone();
                self.apply(start, &[quote]);
            }
            Block::List { start, items } => {
                let digits = start.map_or(0, |n| (n + items.len() as u64).to_string().len() as i32);
                let hang = if start.is_some() { 12 + 8 * digits } else { 18 };
                for (index, item) in items.iter().enumerate() {
                    let marker = match (item.task, start) {
                        (Some(true), _) => "☑".to_owned(),
                        (Some(false), _) => "☐".to_owned(),
                        (None, Some(n)) => format!("{}.", n + index as u64),
                        (None, None) => ["•", "◦", "▪"][ctx.depth % 3].to_owned(),
                    };
                    let mark = if item.task == Some(true) { self.state.tags.done.clone() } else { self.state.tags.mark.clone() };
                    let begin = self.offset();
                    self.push(&format!("{marker} "), &[mark]);
                    let mut rest = item.blocks.as_slice();
                    if let Some((Block::Para(first), tail)) = rest.split_first() {
                        self.markup(first, &base);
                        rest = tail;
                    }
                    self.push("\n", &[]);
                    let margin = self.margin(ctx.margin + hang, -hang);
                    let tight = self.state.tags.item.clone();
                    self.apply(begin, &[margin, tight]);
                    self.blocks(rest, Ctx { margin: ctx.margin + hang, depth: ctx.depth + 1, ..ctx });
                }
                self.gap();
            }
            Block::Table { align, rows } => {
                let start = self.offset();
                let mut end = self.buffer.end_iter();
                if let Some(anchor) = self.buffer.create_child_anchor(&mut end) {
                    self.anchors.push((anchor.clone(), table_widget(align, rows)));
                    self.state.tables.push((anchor, rows.clone()));
                }
                self.push("\n", &[]);
                let margin = self.margin(ctx.margin, 0);
                self.apply(start, &[margin]);
                self.gap();
            }
            Block::Rule => {
                let rule = self.state.tags.rule.clone();
                self.line("· · ·", &[rule], &[]);
            }
        }
    }
    fn code(&mut self, lang: &str, body: &str, ctx: Ctx) {
        let start = self.offset();
        let label = if lang.is_empty() { "código" } else { lang };
        self.push(&format!("  {label}   "), &[]);
        let mut end = self.buffer.end_iter();
        if let Some(anchor) = self.buffer.create_child_anchor(&mut end) {
            self.anchors.push((anchor, copy_button(body, &self.hooks.copy)));
        }
        self.push("\n", &[]);
        let head = self.state.tags.head.clone();
        let margin = self.margin(ctx.margin, 0);
        self.apply(start, &[margin.clone(), head]);
        let start = self.offset();
        let markup = chat_markdown::highlight(lang, body, self.palette);
        self.markup(&markup, &[]);
        self.push("\n", &[]);
        let pre = self.state.tags.pre.clone();
        let inner = self.margin(ctx.margin + 12, 0);
        self.apply(start, &[inner, pre]);
        self.gap();
    }
}

fn copy_button(body: &str, copy: &Rc<dyn Fn(&str, Option<&str>)>) -> gtk::Widget {
    let button = gtk::Button::with_label("Copiar");
    button.style_context().add_class("cv-copy");
    button.set_tooltip_text(Some("Copiar este código"));
    button.set_can_focus(false);
    let (body, copy) = (body.to_owned(), copy.clone());
    button.connect_clicked(move |button| {
        copy(&body, None);
        flash(button, "Copiar");
    });
    button.upcast()
}

/// «Copiado ✓» for a moment on a button, then its own label again.
pub fn flash(button: &gtk::Button, label: &'static str) {
    button.set_label("Copiado ✓");
    let weak = button.downgrade();
    glib::timeout_add_local_once(std::time::Duration::from_millis(1400), move || {
        if let Some(button) = weak.upgrade() {
            button.set_label(label);
        }
    });
}

fn table_widget(align: &[Align], rows: &[Vec<String>]) -> gtk::Widget {
    let grid = gtk::Grid::new();
    grid.style_context().add_class("cv-table");
    let columns = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let max = (110 / columns).clamp(12, 60) as i32;
    for (y, row) in rows.iter().enumerate() {
        for (x, cell) in row.iter().enumerate() {
            let label = gtk::Label::new(None);
            label.set_markup(cell);
            label.set_line_wrap(true);
            label.set_line_wrap_mode(pango::WrapMode::WordChar);
            label.set_selectable(true);
            label.set_can_focus(false);
            label.style_context().add_class("cv-cell");
            if y == 0 {
                label.style_context().add_class("cv-th");
            }
            label.set_xalign(match align.get(x) {
                Some(Align::Center) => 0.5,
                Some(Align::Right) => 1.,
                _ => 0.,
            });
            label.set_yalign(0.);
            label.set_max_width_chars(max);
            // A wrapping label asks for almost no width inside a text view: give it its text.
            label.set_width_chars((cell_plain(cell).chars().count() as i32 + 2).clamp(3, max));
            grid.attach(&label, x as i32, y as i32, 1, 1);
        }
    }
    grid.upcast()
}

impl Rich {
    pub fn new(text: &str, colors: &Colors, palette: &CodePalette, hooks: &Hooks) -> Rich {
        let table = gtk::TextTagTable::new();
        let tags = tags(&table, colors);
        let buffer = gtk::TextBuffer::new(Some(&table));
        let mut builder = Builder {
            buffer: buffer.clone(),
            table,
            state: State { tags, links: Vec::new(), tables: Vec::new(), margins: HashMap::new() },
            colors,
            palette,
            hooks,
            anchors: Vec::new(),
            fg: HashMap::new(),
        };
        builder.blocks(&chat_markdown::parse(text), Ctx { margin: 0, depth: 0, quote: false });
        // No empty last line under the answer.
        let mut end = buffer.end_iter();
        let mut last = end.clone();
        if last.backward_char() && last.char() == Some('\n') {
            buffer.delete(&mut last, &mut end);
        }
        // Nor the paragraph gap: the buttons under the answer sit close to it.
        let (end, mut line) = (buffer.end_iter(), buffer.end_iter());
        line.set_line_offset(0);
        buffer.remove_tag(&builder.state.tags.gap, &line, &end);
        let view = gtk::TextView::with_buffer(&buffer);
        view.style_context().add_class("cv-rich");
        view.set_editable(false);
        view.set_cursor_visible(false);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        // Air between wrapped lines: long paragraphs read like a page, not a log.
        view.set_pixels_inside_wrap(4);
        view.set_accepts_tab(false);
        view.set_hexpand(true);
        for (anchor, widget) in builder.anchors.drain(..) {
            view.add_child_at_anchor(&widget, &anchor);
            widget.show_all();
        }
        let state = Rc::new(RefCell::new(builder.state));
        let rich = Rich { view, state };
        rich.connect_links(hooks.open.clone());
        rich
    }

    fn connect_links(&self, open: Rc<dyn Fn(&str)>) {
        let state = Rc::downgrade(&self.state);
        self.view.connect_button_release_event(move |view, event| {
            if event.button() != 1 || view.is_editable() {
                return glib::Propagation::Proceed;
            }
            let Some(buffer) = view.buffer() else { return glib::Propagation::Proceed };
            if buffer.has_selection() {
                return glib::Propagation::Proceed;
            }
            let (x, y) = event.position();
            let (bx, by) = view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
            let (Some(iter), Some(state)) = (view.iter_at_location(bx, by), state.upgrade()) else {
                return glib::Propagation::Proceed;
            };
            let url = state.borrow().links.iter().find(|(tag, _)| iter.has_tag(tag)).map(|(_, u)| u.clone());
            match url {
                Some(url) => {
                    open(&url);
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            }
        });
        let state = Rc::downgrade(&self.state);
        self.view.connect_motion_notify_event(move |view, event| {
            let (x, y) = event.position();
            let (bx, by) = view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
            let over = state.upgrade().is_some_and(|state| {
                view.iter_at_location(bx, by)
                    .is_some_and(|iter| state.borrow().links.iter().any(|(tag, _)| iter.has_tag(tag)))
            });
            if let Some(window) = TextViewExt::window(view, gtk::TextWindowType::Text) {
                let name = if over && !view.is_editable() { "pointer" } else { "text" };
                window.set_cursor(gdk::Cursor::from_name(&view.display(), name).as_ref());
            }
            glib::Propagation::Proceed
        });
    }

    pub fn set_editing(&self, on: bool) {
        self.view.set_editable(on);
        self.view.set_cursor_visible(on);
        let ctx = self.view.style_context();
        if on {
            ctx.add_class("cv-editing");
            self.view.grab_focus();
            if let Some(buffer) = self.view.buffer() {
                buffer.place_cursor(&buffer.end_iter());
            }
        } else {
            ctx.remove_class("cv-editing");
        }
    }

    /// The message as it reads now (edits included): HTML for rich paste and clean text.
    pub fn export(&self) -> (String, String) {
        let Some(buffer) = self.view.buffer() else { return (String::new(), String::new()) };
        let state = self.state.borrow();
        let t = &state.tags;
        let mut lines: Vec<Line> = Vec::new();
        for n in 0..buffer.line_count() {
            let start = buffer.iter_at_line(n);
            if start.has_tag(&t.head) {
                // A code header separates two code blocks in a row.
                lines.push(Line { kind: Kind::Head, html: String::new(), plain: String::new(), gap: true });
                continue;
            }
            if let Some(anchor) = start.child_anchor()
                && let Some((_, rows)) = state.tables.iter().find(|(a, _)| *a == anchor)
            {
                lines.push(Line { kind: Kind::Table, html: table_html(rows), plain: table_plain(rows), gap: true });
                continue;
            }
            let mut end = start.clone();
            if !end.ends_line() {
                end.forward_to_line_end();
            }
            let kind = if start.has_tag(&t.pre) {
                Kind::Code
            } else if start.has_tag(&t.rule) {
                Kind::Rule
            } else if let Some(level) = t.h.iter().position(|h| start.has_tag(h)) {
                Kind::Heading(level + 1)
            } else if start.has_tag(&t.quote) {
                Kind::Quote
            } else if start.has_tag(&t.item) {
                Kind::Item(left_margin(&state, &start))
            } else {
                Kind::Para
            };
            let (html, mut plain) = runs(&buffer, &start, &end, &state, kind == Kind::Code);
            if let Kind::Item(left) = kind {
                plain.insert_str(0, &" ".repeat((left.max(18) as usize - 18) / 6));
            }
            lines.push(Line { kind, html, plain, gap: start.has_tag(&t.gap) || start.has_tag(&t.h[0]) });
        }
        assemble(&lines)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Para,
    Heading(usize),
    Code,
    Quote,
    Item(i32),
    Rule,
    Table,
    Head,
}

struct Line {
    kind: Kind,
    html: String,
    plain: String,
    /// Last line of its paragraph (soft breaks keep the next line in the same one).
    gap: bool,
}

fn left_margin(state: &State, iter: &gtk::TextIter) -> i32 {
    state.margins.iter().filter(|(_, tag)| iter.has_tag(*tag)).map(|((left, _), _)| *left).max().unwrap_or(0)
}

fn esc(text: &str) -> String {
    chat_markdown::escape(text)
}

/// One line's text in runs of equal tags.
fn runs(buffer: &gtk::TextBuffer, start: &gtk::TextIter, end: &gtk::TextIter, state: &State, code: bool) -> (String, String) {
    let (mut html, mut plain) = (String::new(), String::new());
    let mut at = start.clone();
    while at < *end {
        let mut next = at.clone();
        if !next.forward_to_tag_toggle(None::<&gtk::TextTag>) || next > *end {
            next = end.clone();
        }
        let text = buffer.text(&at, &next, false).map(|t| t.to_string()).unwrap_or_default();
        let text = text.replace('\u{2028}', "\n").replace('\u{2009}', "");
        plain.push_str(&text);
        if code {
            html.push_str(&esc(&text));
        } else {
            let t = &state.tags;
            let link = state.links.iter().find(|(tag, _)| at.has_tag(tag)).map(|(_, u)| u.as_str());
            let mut open = String::new();
            let mut close = String::new();
            for (tag, name) in [(&t.b, "strong"), (&t.i, "em"), (&t.s, "s"), (&t.code, "code"), (&t.sup, "sup"), (&t.sub, "sub")] {
                if at.has_tag(tag) {
                    open.push_str(&format!("<{name}>"));
                    close.insert_str(0, &format!("</{name}>"));
                }
            }
            if let Some(url) = link.filter(|u| u.starts_with("http://") || u.starts_with("https://") || u.starts_with("mailto:")) {
                open.insert_str(0, &format!("<a href=\"{}\">", esc(url)));
                close.push_str("</a>");
            }
            html.push_str(&format!("{open}{}{close}", esc(&text).replace('\n', "<br>")));
        }
        at = next;
    }
    (html, plain)
}

fn cell_plain(markup: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in markup.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    unescape(&out).replace('\u{2009}', "")
}

fn table_plain(rows: &[Vec<String>]) -> String {
    rows.iter().map(|row| row.iter().map(|c| cell_plain(c)).collect::<Vec<_>>().join("\t")).collect::<Vec<_>>().join("\n")
}

fn table_html(rows: &[Vec<String>]) -> String {
    let mut out = String::from("<table>");
    for (y, row) in rows.iter().enumerate() {
        out.push_str("<tr>");
        let cell = if y == 0 { "th" } else { "td" };
        for c in row {
            out.push_str(&format!("<{cell}>{}</{cell}>", esc(&cell_plain(c))));
        }
        out.push_str("</tr>");
    }
    out.push_str("</table>");
    out
}

/// Lines back into one HTML fragment and one clean text.
fn assemble(lines: &[Line]) -> (String, String) {
    let mut html = String::from("<meta charset=\"utf-8\">");
    let mut plain = String::new();
    let mut i = 0;
    let mut prev: Option<Kind> = None;
    let mut prev_gap = true;
    while let Some(line) = lines.get(i) {
        let sep = match (prev, line.kind) {
            (None, _) => "",
            (Some(Kind::Para), Kind::Para) if !prev_gap => "\n",
            (Some(Kind::Code), Kind::Code) | (Some(Kind::Item(_)), Kind::Item(_)) | (Some(Kind::Quote), Kind::Quote) => "\n",
            _ => "\n\n",
        };
        if line.kind == Kind::Head {
            prev = prev.map(|_| Kind::Head);
            prev_gap = true;
            i += 1;
            continue;
        }
        match line.kind {
            Kind::Code => {
                let mut body = Vec::new();
                while let Some(l) = lines.get(i).filter(|l| l.kind == Kind::Code) {
                    body.push(l.plain.as_str());
                    i += 1;
                }
                let text = body.join("\n");
                html.push_str(&format!("<pre><code>{}</code></pre>", esc(&text)));
                plain.push_str(sep);
                plain.push_str(&text);
                prev = Some(Kind::Code);
                prev_gap = true;
                continue;
            }
            Kind::Heading(level) => html.push_str(&format!("<h{level}>{}</h{level}>", line.html)),
            Kind::Quote => html.push_str(&format!("<blockquote><p>{}</p></blockquote>", line.html)),
            Kind::Item(left) => html.push_str(&format!("<p style=\"margin:0 0 0 {left}px\">{}</p>", line.html)),
            Kind::Rule => html.push_str("<hr>"),
            Kind::Head | Kind::Table | Kind::Para => html.push_str(&if line.kind == Kind::Table { line.html.clone() } else { format!("<p>{}</p>", line.html) }),
        }
        if line.kind != Kind::Rule {
            plain.push_str(sep);
            plain.push_str(&line.plain);
        }
        prev = Some(line.kind);
        prev_gap = line.gap;
        i += 1;
    }
    (html, plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: Kind, plain: &str, gap: bool) -> Line {
        Line { kind, html: esc(plain), plain: plain.into(), gap }
    }

    #[test]
    fn markup_helpers_undo_our_own_escaping() {
        assert_eq!(unescape("a &lt;b&gt; &amp;amp; &quot;c&quot; &#39;d&#39;"), "a <b> &amp; \"c\" 'd'");
        assert_eq!(attr("a href=\"http://x/?a=1&amp;b=2\"", "href").as_deref(), Some("http://x/?a=1&b=2"));
        assert_eq!(attr("span foreground=\"#ff0000\"", "href"), None);
        assert_eq!(cell_plain("<b>a</b> <span font_family=\"monospace\">\u{2009}x &lt; y\u{2009}</span>"), "a x < y");
    }

    #[test]
    fn copies_keep_paragraphs_lists_and_separate_code_blocks() {
        let lines = [
            line(Kind::Heading(2), "Plan", true),
            line(Kind::Para, "uno", false),
            line(Kind::Para, "sigue", true),
            line(Kind::Item(20), "1. a", false),
            line(Kind::Item(38), "   ◦ b", false),
            line(Kind::Head, "", true),
            line(Kind::Code, "let a = 1;", false),
            line(Kind::Head, "", true),
            line(Kind::Code, "echo $HOME", false),
            line(Kind::Rule, "", true),
            line(Kind::Para, "fin", true),
        ];
        let (html, plain) = assemble(&lines);
        assert_eq!(plain, "Plan\n\nuno\nsigue\n\n1. a\n   ◦ b\n\nlet a = 1;\n\necho $HOME\n\nfin");
        assert_eq!(html.matches("<pre><code>").count(), 2, "{html}");
        assert!(html.starts_with("<meta charset=\"utf-8\"><h2>Plan</h2><p>uno</p><p>sigue</p>"), "{html}");
        assert!(html.contains("<hr><p>fin</p>"), "{html}");
    }

    #[test]
    fn table_copies_are_tab_separated_and_escaped() {
        let rows = vec![vec!["<b>A</b>".to_owned(), "B".to_owned()], vec!["1 &lt; 2".to_owned(), "x".to_owned()]];
        assert_eq!(table_plain(&rows), "A\tB\n1 < 2\tx");
        assert_eq!(table_html(&rows), "<table><tr><th>A</th><th>B</th></tr><tr><td>1 &lt; 2</td><td>x</td></tr></table>");
    }
}
