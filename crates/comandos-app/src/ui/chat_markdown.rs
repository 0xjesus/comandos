//! Agent markdown for the desktop chat view: CommonMark (with tables, strikethrough
//! and task lists) parsed by `pulldown-cmark` into blocks whose text is escaped Pango
//! markup, plus a small syntax highlighter for fenced code. No GTK here: the chat
//! view turns these blocks into widgets.
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Pango markup of a paragraph; single newlines are kept as line breaks.
    Para(String),
    Heading(u8, String),
    Code { lang: String, body: String },
    Quote(Vec<Block>),
    List { start: Option<u64>, items: Vec<Item> },
    /// Row 0 is the header; every cell is Pango markup.
    Table { align: Vec<Align>, rows: Vec<Vec<String>> },
    Rule,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    /// `Some(done)` for `- [x]` / `- [ ]` items.
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Bare `http(s)://` addresses become links, without the punctuation that ends a sentence.
fn linkify(text: &str, out: &mut String) {
    let mut rest = text;
    while let Some(at) = rest.find("http://").into_iter().chain(rest.find("https://")).min() {
        out.push_str(&escape(&rest[..at]));
        let tail = &rest[at..];
        let mut end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`'))
            .unwrap_or(tail.len());
        while end > 0 && tail[..end].ends_with(['.', ',', ';', ':', '!', '?', ')', ']', '\'']) {
            // Keep a closing parenthesis that has its opening one inside the address.
            if tail[..end].ends_with(')') && tail[..end].matches('(').count() >= tail[..end].matches(')').count() {
                break;
            }
            end -= tail[..end].chars().next_back().map_or(1, char::len_utf8);
        }
        let url = &tail[..end];
        if url.len() <= "https://".len() {
            out.push_str(&escape(&tail[..1]));
            rest = &tail[1..];
            continue;
        }
        let url = escape(url);
        out.push_str(&format!("<a href=\"{url}\">{url}</a>"));
        rest = &tail[end..];
    }
    out.push_str(&escape(rest));
}

/// Inline code: monospace on a faint background that works on light and dark themes.
pub fn code_span(text: &str) -> String {
    format!(
        "<span font_family=\"monospace\" bgcolor=\"#808080\" bgalpha=\"22%\">\u{2009}{}\u{2009}</span>",
        escape(text)
    )
}

enum Frame {
    Root(Vec<Block>),
    Quote(Vec<Block>),
    List { start: Option<u64>, items: Vec<Item> },
    Item(Item),
    Table { align: Vec<Align>, rows: Vec<Vec<String>>, row: Vec<String> },
}

impl Frame {
    fn blocks(&mut self) -> Option<&mut Vec<Block>> {
        match self {
            Frame::Root(b) | Frame::Quote(b) => Some(b),
            Frame::Item(item) => Some(&mut item.blocks),
            Frame::List { .. } | Frame::Table { .. } => None,
        }
    }
}

#[derive(PartialEq)]
enum Inline {
    Para,
    Heading(u8),
    Cell,
    /// Text directly inside a tight list item.
    Loose,
}

struct Builder {
    stack: Vec<Frame>,
    inline: Option<(Inline, String)>,
    code: Option<(String, String)>,
    link_depth: usize,
}

impl Builder {
    fn push_block(&mut self, block: Block) {
        if let Some(blocks) = self.stack.iter_mut().rev().find_map(Frame::blocks) {
            blocks.push(block);
        }
    }

    /// Inline content needs an open paragraph, heading or cell.
    fn text(&mut self) -> &mut String {
        if self.inline.is_none() {
            self.inline = Some((Inline::Loose, String::new()));
        }
        match &mut self.inline {
            Some((_, text)) => text,
            None => unreachable!("opened above"),
        }
    }

    fn close_inline(&mut self) {
        let Some((kind, text)) = self.inline.take() else { return };
        let text = text.trim_end_matches('\n').to_owned();
        match kind {
            Inline::Cell => {
                if let Some(Frame::Table { row, .. }) = self.stack.last_mut() {
                    row.push(text);
                }
            }
            Inline::Heading(level) => self.push_block(Block::Heading(level, text)),
            Inline::Para | Inline::Loose => {
                if !text.trim().is_empty() {
                    self.push_block(Block::Para(text));
                }
            }
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.close_inline();
                self.inline = Some((Inline::Para, String::new()));
            }
            Tag::Heading { level, .. } => {
                self.close_inline();
                let level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                self.inline = Some((Inline::Heading(level), String::new()));
            }
            Tag::BlockQuote(_) => {
                self.close_inline();
                self.stack.push(Frame::Quote(Vec::new()));
            }
            Tag::CodeBlock(kind) => {
                self.close_inline();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or("").to_lowercase(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.close_inline();
                self.stack.push(Frame::List { start, items: Vec::new() });
            }
            Tag::Item => {
                self.close_inline();
                self.stack.push(Frame::Item(Item::default()));
            }
            Tag::Table(align) => {
                self.close_inline();
                let align = align
                    .into_iter()
                    .map(|a| match a {
                        Alignment::Center => Align::Center,
                        Alignment::Right => Align::Right,
                        Alignment::Left | Alignment::None => Align::Left,
                    })
                    .collect();
                self.stack.push(Frame::Table { align, rows: Vec::new(), row: Vec::new() });
            }
            Tag::TableCell => self.inline = Some((Inline::Cell, String::new())),
            Tag::Emphasis => self.text().push_str("<i>"),
            Tag::Strong => self.text().push_str("<b>"),
            Tag::Strikethrough => self.text().push_str("<s>"),
            Tag::Superscript => self.text().push_str("<sup>"),
            Tag::Subscript => self.text().push_str("<sub>"),
            Tag::Link { dest_url, .. } => {
                self.link_depth += 1;
                let url = escape(&dest_url);
                self.text().push_str(&format!("<a href=\"{url}\">"));
            }
            Tag::Image { dest_url, .. } => {
                self.link_depth += 1;
                let url = escape(&dest_url);
                self.text().push_str(&format!("<a href=\"{url}\">🖼 "));
            }
            Tag::HtmlBlock => {
                self.close_inline();
                self.inline = Some((Inline::Para, String::new()));
            }
            Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::TableHead
            | Tag::TableRow
            | Tag::MetadataBlock(_) => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell | TagEnd::HtmlBlock => {
                self.close_inline()
            }
            TagEnd::CodeBlock => {
                if let Some((lang, body)) = self.code.take() {
                    let body = body.strip_suffix('\n').unwrap_or(&body).to_owned();
                    self.push_block(Block::Code { lang, body });
                }
            }
            TagEnd::BlockQuote(_) => {
                self.close_inline();
                if let Some(Frame::Quote(blocks)) = self.stack.pop() {
                    self.push_block(Block::Quote(blocks));
                }
            }
            TagEnd::Item => {
                self.close_inline();
                if let Some(Frame::Item(item)) = self.stack.pop()
                    && let Some(Frame::List { items, .. }) = self.stack.last_mut()
                {
                    items.push(item);
                }
            }
            TagEnd::List(_) => {
                self.close_inline();
                if let Some(Frame::List { start, items }) = self.stack.pop() {
                    self.push_block(Block::List { start, items });
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(Frame::Table { rows, row, .. }) = self.stack.last_mut() {
                    rows.push(std::mem::take(row));
                }
            }
            TagEnd::Table => {
                if let Some(Frame::Table { align, rows, .. }) = self.stack.pop() {
                    self.push_block(Block::Table { align, rows });
                }
            }
            TagEnd::Emphasis => self.text().push_str("</i>"),
            TagEnd::Strong => self.text().push_str("</b>"),
            TagEnd::Strikethrough => self.text().push_str("</s>"),
            TagEnd::Superscript => self.text().push_str("</sup>"),
            TagEnd::Subscript => self.text().push_str("</sub>"),
            TagEnd::Link | TagEnd::Image => {
                self.link_depth = self.link_depth.saturating_sub(1);
                self.text().push_str("</a>");
            }
            _ => {}
        }
    }

    fn event(&mut self, event: Event) {
        if let Some((_, body)) = &mut self.code
            && let Event::Text(text) = &event
        {
            body.push_str(text);
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                let linked = self.link_depth > 0;
                let out = self.text();
                if linked {
                    out.push_str(&escape(&text));
                } else {
                    linkify(&text, out);
                }
            }
            Event::Code(text) => {
                let span = code_span(&text);
                self.text().push_str(&span);
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                let span = code_span(&text);
                self.text().push_str(&span);
            }
            // Agents do not get to inject markup: their HTML shows as text.
            Event::Html(text) | Event::InlineHtml(text) => {
                let text = escape(&text);
                self.text().push_str(&text);
            }
            Event::FootnoteReference(name) => {
                let note = format!("<sup>[{}]</sup>", escape(&name));
                self.text().push_str(&note);
            }
            Event::SoftBreak | Event::HardBreak => self.text().push('\n'),
            Event::Rule => {
                self.close_inline();
                self.push_block(Block::Rule);
            }
            Event::TaskListMarker(done) => {
                if let Some(Frame::Item(item)) = self.stack.last_mut() {
                    item.task = Some(done);
                }
            }
        }
    }
}

pub fn parse(text: &str) -> Vec<Block> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut builder = Builder {
        stack: vec![Frame::Root(Vec::new())],
        inline: None,
        code: None,
        link_depth: 0,
    };
    for event in Parser::new_ext(text, options) {
        builder.event(event);
    }
    builder.close_inline();
    if let Some((lang, body)) = builder.code.take() {
        builder.push_block(Block::Code { lang, body });
    }
    // Unclosed containers (a cut transcript) still show what they hold.
    while builder.stack.len() > 1 {
        match builder.stack.pop() {
            Some(Frame::Quote(blocks)) => builder.push_block(Block::Quote(blocks)),
            Some(Frame::Item(item)) => {
                if let Some(Frame::List { items, .. }) = builder.stack.last_mut() {
                    items.push(item);
                }
            }
            Some(Frame::List { start, items }) => builder.push_block(Block::List { start, items }),
            Some(Frame::Table { align, rows, .. }) => builder.push_block(Block::Table { align, rows }),
            _ => {}
        }
    }
    match builder.stack.pop() {
        Some(Frame::Root(blocks)) => blocks,
        _ => Vec::new(),
    }
}

/// Colors for highlighted code, as `#rrggbb`.
#[derive(Debug, Clone)]
pub struct CodePalette {
    pub keyword: String,
    pub string: String,
    pub number: String,
    pub comment: String,
    pub function: String,
    pub kind: String,
    pub added: String,
    pub removed: String,
    pub meta: String,
}

fn rgb(hex: &str) -> Option<[f64; 3]> {
    let h = hex.strip_prefix('#')?;
    let byte = |i: usize| h.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()).map(f64::from);
    (h.len() == 6).then_some([byte(0)?, byte(2)?, byte(4)?])
}

fn luminance(c: [f64; 3]) -> f64 {
    let lin = |v: f64| {
        let v = v / 255.;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG contrast ratio between two `#rrggbb` colors (1 when unreadable).
pub fn contrast(a: &str, b: &str) -> f64 {
    match (rgb(a), rgb(b)) {
        (Some(a), Some(b)) => {
            let (x, y) = (luminance(a), luminance(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        }
        _ => 1.,
    }
}

/// Moves `color` toward `toward` until it reaches `ratio` against `bg`.
fn readable(color: &str, toward: &str, bg: &str, ratio: f64) -> String {
    let (Some(c), Some(t)) = (rgb(color), rgb(toward)) else { return color.to_owned() };
    let ([cr, cg, cb], [tr, tg, tb]) = (c, t);
    for step in 0..=10 {
        let k = f64::from(step) / 10.;
        let mix = |a: f64, b: f64| (a + (b - a) * k).round().clamp(0., 255.) as u8;
        let hex = format!("#{:02x}{:02x}{:02x}", mix(cr, tr), mix(cg, tg), mix(cb, tb));
        if contrast(&hex, bg) >= ratio {
            return hex;
        }
    }
    toward.to_owned()
}

impl CodePalette {
    /// From a theme's 16 ANSI colors: for each role the normal variant when it reads well
    /// on the code background, else the brighter one, nudged toward the text color if faint.
    pub fn from_ansi(ansi: &[String], faint: &str, text: &str, bg: &str) -> CodePalette {
        let pick = |normal: usize, fallback: &str| {
            let a = ansi.get(normal).map_or(fallback, String::as_str);
            let b = ansi.get(normal + 8).map_or(fallback, String::as_str);
            // The normal color keeps its saturation whenever it already reads well.
            let best = if contrast(a, bg) >= 4.5 || contrast(a, bg) >= contrast(b, bg) { a } else { b };
            readable(best, text, bg, 4.0)
        };
        CodePalette {
            keyword: pick(5, "#c678dd"),
            string: pick(2, "#98c379"),
            number: pick(3, "#d19a66"),
            comment: readable(faint, text, bg, 3.0),
            function: pick(4, "#61afef"),
            kind: pick(6, "#56b6c2"),
            added: pick(2, "#98c379"),
            removed: pick(1, "#e06c75"),
            meta: pick(6, "#56b6c2"),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Lang {
    Rust,
    Js,
    Python,
    Shell,
    Json,
    Conf,
    Go,
    CLike,
    Sql,
    Css,
    Markup,
    Diff,
    Plain,
}

fn lang_of(name: &str, body: &str) -> Lang {
    match name {
        "rust" | "rs" => Lang::Rust,
        "js" | "javascript" | "jsx" | "ts" | "typescript" | "tsx" | "mjs" | "cjs" => Lang::Js,
        "py" | "python" | "python3" => Lang::Python,
        "sh" | "bash" | "zsh" | "shell" | "console" | "shellsession" | "fish" => Lang::Shell,
        "json" | "jsonc" | "json5" | "jsonl" => Lang::Json,
        "toml" | "yaml" | "yml" | "ini" | "env" | "dotenv" | "conf" | "nginx" | "dockerfile" | "make" | "makefile" => Lang::Conf,
        "go" | "golang" => Lang::Go,
        "c" | "h" | "cpp" | "c++" | "hpp" | "java" | "kotlin" | "kt" | "swift" | "cs" | "csharp" | "dart" | "php" | "scala" => Lang::CLike,
        "sql" | "psql" | "sqlite" | "postgres" | "postgresql" | "mysql" => Lang::Sql,
        "css" | "scss" | "less" => Lang::Css,
        "html" | "xml" | "svg" | "vue" | "svelte" => Lang::Markup,
        "diff" | "patch" => Lang::Diff,
        "" if looks_like_diff(body) => Lang::Diff,
        _ => Lang::Plain,
    }
}

fn looks_like_diff(body: &str) -> bool {
    let lines: Vec<&str> = body.lines().filter(|l| !l.is_empty()).collect();
    lines.iter().any(|l| l.starts_with("@@") || l.starts_with("+++ ") || l.starts_with("--- "))
        && lines.iter().all(|l| l.starts_with(['+', '-', ' ', '@']) || l.starts_with("diff ") || l.starts_with("index "))
}

fn keywords(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Rust => &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if",
            "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct",
            "super", "trait", "true", "type", "unsafe", "use", "where", "while", "Some", "None", "Ok", "Err",
        ],
        Lang::Js => &[
            "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do", "else", "export",
            "extends", "false", "finally", "for", "from", "function", "if", "import", "in", "instanceof", "let", "new", "null", "of",
            "return", "static", "super", "switch", "this", "throw", "true", "try", "typeof", "undefined", "var", "void", "while",
            "yield", "interface", "type", "enum", "implements", "readonly", "as", "declare", "namespace", "private", "public",
            "protected",
        ],
        Lang::Python => &[
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "False",
            "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass",
            "raise", "return", "True", "try", "while", "with", "yield", "self", "match", "case",
        ],
        Lang::Shell => &[
            "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function", "return",
            "export", "local", "readonly", "set", "unset", "exit", "source", "trap", "shift",
        ],
        Lang::Json => &["true", "false", "null"],
        Lang::Conf => &["true", "false", "null", "yes", "no", "on", "off"],
        Lang::Go => &[
            "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if",
            "import", "interface", "map", "package", "range", "return", "select", "struct", "switch", "type", "var", "nil", "true",
            "false",
        ],
        Lang::CLike => &[
            "auto", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do", "else", "enum", "extends",
            "extern", "false", "final", "for", "fun", "func", "if", "implements", "import", "include", "interface", "let", "namespace",
            "new", "null", "nullptr", "override", "package", "private", "protected", "public", "return", "static", "struct", "switch",
            "template", "this", "throw", "true", "try", "typedef", "using", "val", "var", "virtual", "void", "while", "self",
        ],
        Lang::Sql => &[
            "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "join", "left", "right", "inner", "outer",
            "on", "group", "by", "order", "limit", "offset", "having", "create", "table", "index", "view", "drop", "alter", "add",
            "primary", "key", "foreign", "references", "not", "null", "and", "or", "as", "distinct", "union", "all", "case", "when",
            "then", "else", "end", "in", "is", "like", "exists", "default", "begin", "commit", "with", "returning", "asc", "desc",
        ],
        Lang::Css | Lang::Markup | Lang::Diff | Lang::Plain => &[],
    }
}

fn span(out: &mut String, color: &str, text: &str) {
    if text.is_empty() {
        return;
    }
    out.push_str(&format!("<span foreground=\"{color}\">{}</span>", escape(text)));
}

/// Fenced code as Pango markup. Unknown languages come back escaped and uncolored.
pub fn highlight(lang_name: &str, body: &str, pal: &CodePalette) -> String {
    let lang = lang_of(lang_name, body);
    let mut out = String::with_capacity(body.len() * 2);
    match lang {
        Lang::Plain => return escape(body),
        Lang::Diff => {
            for (i, line) in body.split('\n').enumerate() {
                if i > 0 {
                    out.push('\n');
                }
                if line.starts_with("+++") || line.starts_with("---") || line.starts_with("@@") || line.starts_with("diff ") {
                    span(&mut out, &pal.meta, line);
                } else if line.starts_with('+') {
                    span(&mut out, &pal.added, line);
                } else if line.starts_with('-') {
                    span(&mut out, &pal.removed, line);
                } else {
                    out.push_str(&escape(line));
                }
            }
            return out;
        }
        _ => {}
    }
    let words = keywords(lang);
    let chars: Vec<char> = body.chars().collect();
    let text = |from: usize, to: usize| chars.get(from..to).map(|s| s.iter().collect::<String>()).unwrap_or_default();
    let line_comment: &[&str] = match lang {
        Lang::Python | Lang::Shell | Lang::Conf => &["#"],
        Lang::Sql => &["--"],
        Lang::Json | Lang::Css | Lang::Markup => &[],
        _ => &["//"],
    };
    let block_comment = matches!(lang, Lang::Rust | Lang::Js | Lang::Go | Lang::CLike | Lang::Css | Lang::Sql);
    let starts = |i: usize, s: &str| s.chars().enumerate().all(|(k, c)| chars.get(i + k) == Some(&c));
    let mut line_start = true;
    let mut i = 0;
    let mut plain = String::new();
    let flush = |out: &mut String, plain: &mut String| {
        out.push_str(&escape(plain));
        plain.clear();
    };
    while let Some(&c) = chars.get(i) {
        // Comments.
        if let Some(mark) = line_comment.iter().find(|m| starts(i, m)) {
            let word_start = i == 0 || chars.get(i - 1).is_some_and(|p| p.is_whitespace());
            if *mark != "#" || word_start {
                flush(&mut out, &mut plain);
                let end = chars.get(i..).unwrap_or_default().iter().position(|&c| c == '\n').map_or(chars.len(), |n| i + n);
                span(&mut out, &pal.comment, &text(i, end));
                i = end;
                continue;
            }
        }
        if block_comment && starts(i, "/*") {
            flush(&mut out, &mut plain);
            let end = (i + 2..chars.len()).find(|&k| starts(k, "*/")).map_or(chars.len(), |k| k + 2);
            span(&mut out, &pal.comment, &text(i, end));
            i = end;
            continue;
        }
        if lang == Lang::Markup && starts(i, "<!--") {
            flush(&mut out, &mut plain);
            let end = (i + 4..chars.len()).find(|&k| starts(k, "-->")).map_or(chars.len(), |k| k + 3);
            span(&mut out, &pal.comment, &text(i, end));
            i = end;
            continue;
        }
        // Strings.
        let quote = match c {
            '"' => true,
            '\'' => lang != Lang::Rust || chars.get(i + 2) == Some(&'\'') || chars.get(i + 1) == Some(&'\\'),
            '`' => matches!(lang, Lang::Js | Lang::Shell | Lang::Go),
            _ => false,
        };
        if quote && lang != Lang::Markup {
            flush(&mut out, &mut plain);
            let triple = lang == Lang::Python && starts(i, &c.to_string().repeat(3));
            let mut k = i + if triple { 3 } else { 1 };
            while let Some(&d) = chars.get(k) {
                if d == '\\' {
                    k += 2;
                    continue;
                }
                if triple {
                    if starts(k, &c.to_string().repeat(3)) {
                        k += 3;
                        break;
                    }
                } else if d == c {
                    k += 1;
                    break;
                } else if d == '\n' && c != '`' {
                    break;
                }
                k += 1;
            }
            let end = k.min(chars.len());
            // A JSON key is the string right before a colon.
            let is_key = matches!(lang, Lang::Json)
                && chars.get(end..).unwrap_or_default().iter().find(|c| !c.is_whitespace()) == Some(&':');
            span(&mut out, if is_key { &pal.function } else { &pal.string }, &text(i, end));
            i = end;
            line_start = false;
            continue;
        }
        // Markup tags.
        if lang == Lang::Markup && c == '<' {
            flush(&mut out, &mut plain);
            let end = chars.get(i..).unwrap_or_default().iter().position(|&c| c == '>').map_or(chars.len(), |n| i + n + 1);
            span(&mut out, &pal.keyword, &text(i, end));
            i = end;
            continue;
        }
        // Numbers.
        if c.is_ascii_digit() && !chars.get(i.wrapping_sub(1)).is_some_and(|p| p.is_alphanumeric() || *p == '_') {
            flush(&mut out, &mut plain);
            let end = (i..chars.len())
                .find(|&k| !chars.get(k).is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_')))
                .unwrap_or(chars.len());
            span(&mut out, &pal.number, &text(i, end));
            i = end;
            line_start = false;
            continue;
        }
        // Shell variables.
        if lang == Lang::Shell && c == '$' {
            flush(&mut out, &mut plain);
            let end = if chars.get(i + 1) == Some(&'{') {
                chars.get(i..).unwrap_or_default().iter().position(|&c| c == '}').map_or(chars.len(), |n| i + n + 1)
            } else {
                (i + 1..chars.len())
                    .find(|&k| !chars.get(k).is_some_and(|c| c.is_alphanumeric() || *c == '_'))
                    .unwrap_or(chars.len())
            };
            span(&mut out, &pal.kind, &text(i, end));
            i = end;
            continue;
        }
        // Words.
        if c.is_alphabetic() || c == '_' || (lang == Lang::Css && c == '-') {
            let end = (i..chars.len())
                .find(|&k| {
                    !chars
                        .get(k)
                        .is_some_and(|c| c.is_alphanumeric() || *c == '_' || (matches!(lang, Lang::Css | Lang::Shell) && *c == '-'))
                })
                .unwrap_or(chars.len());
            let word = text(i, end);
            let next = chars.get(end..).unwrap_or_default().iter().find(|c| **c != ' ').copied();
            let is_kw = if lang == Lang::Sql {
                words.contains(&word.to_lowercase().as_str())
            } else {
                words.contains(&word.as_str())
            };
            // Calls, shell commands, CSS properties and config keys.
            let is_name = (lang == Lang::Shell && line_start)
                || ((next == Some('(') || (lang == Lang::Rust && next == Some('!'))) && lang != Lang::Css)
                || (lang == Lang::Css && next == Some(':'))
                || (lang == Lang::Conf && line_start && matches!(next, Some('=' | ':')));
            let color = if is_kw {
                Some(&pal.keyword)
            } else if is_name {
                Some(&pal.function)
            } else if word.chars().next().is_some_and(char::is_uppercase)
                && matches!(lang, Lang::Rust | Lang::Js | Lang::Go | Lang::CLike)
            {
                Some(&pal.kind)
            } else {
                None
            };
            match color {
                Some(color) => {
                    flush(&mut out, &mut plain);
                    span(&mut out, color, &word);
                }
                None => plain.push_str(&word),
            }
            i = end;
            line_start = false;
            continue;
        }
        if c == '\n' {
            line_start = true;
        } else if !c.is_whitespace() && (lang != Lang::Shell || !matches!(c, '|' | ';' | '&' | '(')) {
            line_start = false;
        }
        plain.push(c);
        i += 1;
    }
    flush(&mut out, &mut plain);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_markup_follows_commonmark() {
        let blocks = parse("a **b** `c<d>` *e* ~~f~~ 2 * 3 * 4 snake_case_name __init__");
        let [Block::Para(p)] = blocks.as_slice() else { panic!("{blocks:?}") };
        assert!(p.starts_with("a <b>b</b> <span font_family=\"monospace\""));
        assert!(p.contains("c&lt;d&gt;"));
        assert!(p.contains("<i>e</i> <s>f</s> 2 * 3 * 4 snake_case_name"), "{p}");
        assert!(p.ends_with("<b>init</b>"), "__init__ es negrita en CommonMark: {p}");
    }

    #[test]
    fn links_bare_addresses_and_html_are_safe() {
        let blocks = parse("[doc](http://x/?a=1&b=2) y https://github.com/tmux/tmux/wiki. <b>no</b>");
        let [Block::Para(p)] = blocks.as_slice() else { panic!("{blocks:?}") };
        assert!(p.starts_with("<a href=\"http://x/?a=1&amp;b=2\">doc</a> y "), "{p}");
        assert!(p.contains("<a href=\"https://github.com/tmux/tmux/wiki\">https://github.com/tmux/tmux/wiki</a>."), "{p}");
        assert!(p.ends_with("&lt;b&gt;no&lt;/b&gt;"), "{p}");
    }

    #[test]
    fn nested_lists_tasks_and_quotes_keep_their_structure() {
        let blocks = parse("1. uno\n   - a\n     - b\n2. dos\n\n- [x] hecho\n- [ ] falta\n\n> **Ojo:** cita\n>\n> - lista");
        let Block::List { start: Some(1), items } = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(items.len(), 2);
        let Block::List { start: None, items: inner } = &items[0].blocks[1] else { panic!("{items:?}") };
        assert!(matches!(&inner[0].blocks[1], Block::List { items, .. } if items.len() == 1));
        let Block::List { items: tasks, .. } = &blocks[1] else { panic!() };
        assert_eq!(tasks.iter().map(|i| i.task).collect::<Vec<_>>(), [Some(true), Some(false)]);
        let Block::Quote(quote) = &blocks[2] else { panic!("{blocks:?}") };
        assert!(matches!(&quote[0], Block::Para(p) if p == "<b>Ojo:</b> cita"));
        assert!(matches!(&quote[1], Block::List { .. }));
    }

    #[test]
    fn tables_keep_escaped_pipes_inside_cells_and_alignment() {
        let blocks = parse("| A | B |\n|:--|--:|\n| `a \\| b` | ~~x~~ |");
        let Block::Table { align, rows } = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(align, &[Align::Left, Align::Right]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].len(), 2);
        assert!(rows[1][0].contains("a | b"), "{rows:?}");
        assert_eq!(rows[1][1], "<s>x</s>");
    }

    #[test]
    fn headings_code_and_rules() {
        let blocks = parse("#### Cuatro\n\n```Rust title\nlet x = 1 < 2;\n```\n\n---\n\nfin  \nsigue");
        assert_eq!(blocks[0], Block::Heading(4, "Cuatro".into()));
        assert_eq!(blocks[1], Block::Code { lang: "rust".into(), body: "let x = 1 < 2;".into() });
        assert_eq!(blocks[2], Block::Rule);
        assert_eq!(blocks[3], Block::Para("fin\nsigue".into()));
    }

    #[test]
    fn a_cut_transcript_still_shows_its_open_code_block() {
        let blocks = parse("texto\n```bash\necho hola");
        assert_eq!(blocks[1], Block::Code { lang: "bash".into(), body: "echo hola".into() });
    }

    #[test]
    fn code_colors_stay_readable_on_light_and_dark_backgrounds() {
        let ansi: Vec<String> = ["#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
            "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff"]
            .map(String::from)
            .to_vec();
        for (text, bg) in [("#1f2328", "#f6f8fa"), ("#d4d4d4", "#0b0f17")] {
            let pal = CodePalette::from_ansi(&ansi, "#8b949e", text, bg);
            for color in [&pal.keyword, &pal.string, &pal.number, &pal.function, &pal.kind, &pal.added, &pal.removed] {
                assert!(contrast(color, bg) >= 4.0, "{color} sobre {bg}: {}", contrast(color, bg));
            }
            assert!(contrast(&pal.comment, bg) >= 3.0);
        }
    }

    #[test]
    fn highlighter_colors_tokens_and_escapes_everything() {
        let pal = CodePalette::from_ansi(&(0..16).map(|i| format!("#{:02x}0000", 128 + i * 8)).collect::<Vec<_>>(), "#777777", "#000000", "#ffffff");
        let rust = highlight("rust", "fn main() { let s = \"<a>\"; // nota\n}", &pal);
        assert!(rust.contains(&format!("<span foreground=\"{}\">fn</span>", pal.keyword)), "{rust}");
        assert!(rust.contains(&format!("<span foreground=\"{}\">main</span>", pal.function)), "{rust}");
        assert!(rust.contains(&format!("<span foreground=\"{}\">&quot;&lt;a&gt;&quot;</span>", pal.string)), "{rust}");
        assert!(rust.contains(&format!("<span foreground=\"{}\">// nota</span>", pal.comment)), "{rust}");
        let diff = highlight("", "@@ -1 +1 @@\n-a\n+b", &pal);
        assert_eq!(
            diff,
            format!(
                "<span foreground=\"{m}\">@@ -1 +1 @@</span>\n<span foreground=\"{r}\">-a</span>\n<span foreground=\"{a}\">+b</span>",
                m = pal.meta,
                r = pal.removed,
                a = pal.added
            )
        );
        assert_eq!(highlight("text", "a < b & c", &pal), "a &lt; b &amp; c");
        let sh = highlight("bash", "echo $HOME # x", &pal);
        assert!(sh.contains(&format!("<span foreground=\"{}\">echo</span>", pal.function)), "{sh}");
        assert!(sh.contains(&format!("<span foreground=\"{}\">$HOME</span>", pal.kind)), "{sh}");
    }
}
