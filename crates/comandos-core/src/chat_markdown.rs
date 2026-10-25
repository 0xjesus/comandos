//! Agent markdown for the chat views, toolkit-neutral. CommonMark plus the GitHub parts
//! agents use (tables, task lists, strikethrough, footnotes, `> [!NOTE]` alerts), parsed by
//! `pulldown-cmark`.
//!
//! - [`to_html`]: safe HTML for the phone, rendered once by the server and inserted as is.
//!   Every text is escaped, agent HTML shows as text, links are only http(s)/mailto.
//! - [`to_plain`]: clean text for a clipboard (no `**`, bullets as «•»).
//! - [`tokens`]: a small syntax highlighter for fenced code; each view colors the kinds.
use pulldown_cmark::{Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd, TextMergeStream};

/// Escapes text for HTML content and attribute values.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
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

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM
}

/// The language of a fence info string: «rust» from «Rust title="x"», «{.python}» or
/// «rust,no_run». At most 24 characters; anything else is the view's business.
pub fn code_lang(info: &str) -> String {
    let first = info.split_whitespace().next().unwrap_or("");
    let first = first.split(',').next().unwrap_or("");
    first
        .trim_matches(|c: char| matches!(c, '{' | '}' | '.'))
        .chars()
        .take(24)
        .collect::<String>()
        .to_lowercase()
}

/// Only addresses that open a page or a mail client; anything else stays as text.
fn safe_href(url: &str, kind: LinkType) -> Option<String> {
    let url = url.trim();
    if kind == LinkType::Email {
        return (!url.is_empty() && !url.contains(':')).then(|| format!("mailto:{url}"));
    }
    let lower = url.to_ascii_lowercase();
    let scheme = ["http://", "https://"].iter().any(|s| lower.starts_with(s) && lower.len() > s.len()) || lower.starts_with("mailto:");
    (scheme && !url.chars().any(char::is_control)).then(|| url.to_owned())
}

/// Bare `http(s)://` addresses, without the punctuation that ends a sentence: (start, end).
fn bare_urls(text: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find("http://").into_iter().chain(text[from..].find("https://")).min().map(|i| from + i) {
        let tail = &text[at..];
        let mut end = tail.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`')).unwrap_or(tail.len());
        while end > 0 && tail[..end].ends_with(['.', ',', ';', ':', '!', '?', ')', ']', '\'', '*', '_']) {
            // Keep a closing parenthesis whose opening one is inside the address.
            if tail[..end].ends_with(')') && tail[..end].matches('(').count() >= tail[..end].matches(')').count() {
                break;
            }
            end -= tail[..end].chars().next_back().map_or(1, char::len_utf8);
        }
        let scheme = if tail.starts_with("https://") { 8 } else { 7 };
        if end > scheme {
            found.push((at, at + end));
            from = at + end;
        } else {
            from = at + scheme;
        }
        if from >= text.len() {
            break;
        }
    }
    found
}

fn anchor(href: &str) -> String {
    format!("<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">", escape(href))
}

fn linkify(text: &str, out: &mut String) {
    let mut last = 0;
    for (start, end) in bare_urls(text) {
        out.push_str(&escape(&text[last..start]));
        let url = &text[start..end];
        out.push_str(&anchor(url));
        out.push_str(&escape(url));
        out.push_str("</a>");
        last = end;
    }
    out.push_str(&escape(&text[last..]));
}

fn heading(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn alert(kind: BlockQuoteKind) -> (&'static str, &'static str) {
    match kind {
        BlockQuoteKind::Note => ("note", "Nota"),
        BlockQuoteKind::Tip => ("tip", "Consejo"),
        BlockQuoteKind::Important => ("important", "Importante"),
        BlockQuoteKind::Warning => ("warning", "Advertencia"),
        BlockQuoteKind::Caution => ("caution", "Precaución"),
    }
}

/// A fenced or indented block: the language and a copy button above, highlighted code
/// that scrolls sideways. The head is not editable when the bubble is.
fn code_block(lang: &str, body: &str, out: &mut String) {
    let label = if lang.is_empty() { "código" } else { lang };
    out.push_str(&format!(
        "<div class=\"cv-code\"><div class=\"cv-code-head\" contenteditable=\"false\" data-lang=\"{}\"><button type=\"button\" class=\"cv-copy\" data-cv-act=\"copy-code\" aria-label=\"Copiar código\"></button></div><pre><code>",
        escape(label)
    ));
    out.push_str(&highlight_html(lang, body));
    out.push_str("</code></pre></div>");
}

struct Table {
    align: Vec<Alignment>,
    head: bool,
    col: usize,
}

/// Markdown as safe HTML for the phone's chat bubbles.
pub fn to_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let mut code: Option<(String, String)> = None;
    let mut tables: Vec<Table> = Vec::new();
    // Each open link or image closes with the tag it opened (`a` or `span`).
    let mut links: Vec<&'static str> = Vec::new();
    for event in TextMergeStream::new(Parser::new_ext(text, options())) {
        if let Some((_, body)) = &mut code {
            match event {
                Event::Text(t) => body.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, body) = code.take().unwrap_or_default();
                    code_block(&lang, body.strip_suffix('\n').unwrap_or(&body), &mut out);
                }
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => out.push_str("<p>"),
                Tag::Heading { level, .. } => out.push_str(&format!("<h{} class=\"cv-h\">", heading(level))),
                Tag::BlockQuote(None) => out.push_str("<blockquote>"),
                Tag::BlockQuote(Some(kind)) => {
                    let (class, title) = alert(kind);
                    out.push_str(&format!("<blockquote class=\"cv-alert cv-alert-{class}\"><p class=\"cv-alert-t\">{title}</p>"));
                }
                Tag::CodeBlock(kind) => {
                    let lang = match kind {
                        CodeBlockKind::Fenced(info) => code_lang(&info),
                        CodeBlockKind::Indented => String::new(),
                    };
                    code = Some((lang, String::new()));
                }
                Tag::HtmlBlock => out.push_str("<div class=\"cv-raw\">"),
                Tag::List(Some(1)) => out.push_str("<ol>"),
                Tag::List(Some(start)) => out.push_str(&format!("<ol start=\"{start}\">")),
                Tag::List(None) => out.push_str("<ul>"),
                Tag::Item => out.push_str("<li>"),
                Tag::FootnoteDefinition(name) => out.push_str(&format!("<div class=\"cv-fn\"><sup>{}</sup> ", escape(&name))),
                Tag::Table(align) => {
                    out.push_str("<div class=\"cv-table\"><table>");
                    tables.push(Table { align, head: false, col: 0 });
                }
                Tag::TableHead => {
                    out.push_str("<thead><tr>");
                    if let Some(t) = tables.last_mut() {
                        t.head = true;
                        t.col = 0;
                    }
                }
                Tag::TableRow => {
                    out.push_str("<tr>");
                    if let Some(t) = tables.last_mut() {
                        t.col = 0;
                    }
                }
                Tag::TableCell => {
                    let (cell, align) = tables.last().map_or(("td", Alignment::None), |t| {
                        (if t.head { "th" } else { "td" }, t.align.get(t.col).copied().unwrap_or(Alignment::None))
                    });
                    let class = match align {
                        Alignment::Center => " class=\"cv-c\"",
                        Alignment::Right => " class=\"cv-r\"",
                        Alignment::Left | Alignment::None => "",
                    };
                    out.push_str(&format!("<{cell}{class}>"));
                }
                Tag::Emphasis => out.push_str("<em>"),
                Tag::Strong => out.push_str("<strong>"),
                Tag::Strikethrough => out.push_str("<del>"),
                Tag::Superscript => out.push_str("<sup>"),
                Tag::Subscript => out.push_str("<sub>"),
                Tag::Link { link_type, dest_url, title, .. } => match safe_href(&dest_url, link_type) {
                    Some(href) => {
                        let title = if title.is_empty() { String::new() } else { format!(" title=\"{}\"", escape(&title)) };
                        out.push_str(&format!("<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\"{title}>", escape(&href)));
                        links.push("a");
                    }
                    None => {
                        out.push_str(&format!("<span class=\"cv-link\" title=\"{}\">", escape(&dest_url)));
                        links.push("span");
                    }
                },
                Tag::Image { link_type, dest_url, .. } => {
                    match safe_href(&dest_url, link_type) {
                        Some(href) => {
                            out.push_str(&format!("<a class=\"cv-img\" href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">🖼 ", escape(&href)));
                            links.push("a");
                        }
                        None => {
                            out.push_str("<span class=\"cv-img\">🖼 ");
                            links.push("span");
                        }
                    }
                }
                Tag::DefinitionList
                | Tag::DefinitionListTitle
                | Tag::DefinitionListDefinition
                | Tag::MetadataBlock(_) => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => out.push_str("</p>"),
                TagEnd::Heading(level) => out.push_str(&format!("</h{}>", heading(level))),
                TagEnd::BlockQuote(_) => out.push_str("</blockquote>"),
                TagEnd::HtmlBlock => out.push_str("</div>"),
                TagEnd::List(ordered) => out.push_str(if ordered { "</ol>" } else { "</ul>" }),
                TagEnd::Item => out.push_str("</li>"),
                TagEnd::FootnoteDefinition => out.push_str("</div>"),
                TagEnd::Table => {
                    out.push_str("</tbody></table></div>");
                    tables.pop();
                }
                TagEnd::TableHead => {
                    out.push_str("</tr></thead><tbody>");
                    if let Some(t) = tables.last_mut() {
                        t.head = false;
                    }
                }
                TagEnd::TableRow => out.push_str("</tr>"),
                TagEnd::TableCell => {
                    let cell = tables.last().map_or("td", |t| if t.head { "th" } else { "td" });
                    out.push_str(&format!("</{cell}>"));
                    if let Some(t) = tables.last_mut() {
                        t.col += 1;
                    }
                }
                TagEnd::Emphasis => out.push_str("</em>"),
                TagEnd::Strong => out.push_str("</strong>"),
                TagEnd::Strikethrough => out.push_str("</del>"),
                TagEnd::Superscript => out.push_str("</sup>"),
                TagEnd::Subscript => out.push_str("</sub>"),
                TagEnd::Link | TagEnd::Image => {
                    let tag = links.pop().unwrap_or("span");
                    out.push_str(&format!("</{tag}>"));
                }
                TagEnd::CodeBlock
                | TagEnd::DefinitionList
                | TagEnd::DefinitionListTitle
                | TagEnd::DefinitionListDefinition
                | TagEnd::MetadataBlock(_) => {}
            },
            Event::Text(t) => {
                if links.is_empty() {
                    linkify(&t, &mut out);
                } else {
                    out.push_str(&escape(&t));
                }
            }
            Event::Code(t) | Event::InlineMath(t) | Event::DisplayMath(t) => {
                out.push_str("<code>");
                out.push_str(&escape(&t));
                out.push_str("</code>");
            }
            // Agents do not get to inject markup: their HTML shows as text. A bare line
            // break (common inside table cells) is the one tag that stays a tag.
            Event::InlineHtml(t) => {
                let tag: String = t.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
                if matches!(tag.as_str(), "<br>" | "<br/>") {
                    out.push_str("<br>");
                } else {
                    out.push_str(&escape(&t));
                }
            }
            Event::Html(t) => out.push_str(&escape(&t)),
            Event::FootnoteReference(name) => out.push_str(&format!("<sup class=\"cv-fnref\">[{}]</sup>", escape(&name))),
            Event::SoftBreak | Event::HardBreak => out.push_str("<br>"),
            Event::Rule => out.push_str("<hr>"),
            Event::TaskListMarker(done) => {
                out.push_str(if done { "<input type=\"checkbox\" disabled checked> " } else { "<input type=\"checkbox\" disabled> " })
            }
        }
    }
    // A cut transcript can stop inside a fence; the parser closes it, but be safe.
    if let Some((lang, body)) = code.take() {
        code_block(&lang, &body, &mut out);
    }
    out
}

/// Clean text for a clipboard: paragraphs apart, «•»/«1.» bullets, «☑»/«☐» tasks, code
/// as is, table cells split by tabs, quotes with «> », links as «text (address)».
pub fn to_plain(text: &str) -> String {
    struct List {
        next: Option<u64>,
    }
    let mut stack: Vec<String> = vec![String::new()];
    let mut lists: Vec<List> = Vec::new();
    let mut bullet: Option<String> = None;
    let mut links: Vec<(String, usize)> = Vec::new();
    let mut in_code = false;
    fn emit(stack: &mut [String], bullet: &mut Option<String>, text: &str) {
        let Some(top) = stack.last_mut() else { return };
        if let Some(b) = bullet.take() {
            top.push_str(&b);
        }
        top.push_str(text);
    }
    fn end_block(stack: &mut [String], gap: &str) {
        if let Some(top) = stack.last_mut() {
            while top.ends_with(' ') {
                top.pop();
            }
            if !top.is_empty() && !top.ends_with("\n\n") {
                if top.ends_with('\n') {
                    top.push_str(&gap[1.min(gap.len())..]);
                } else {
                    top.push_str(gap);
                }
            }
        }
    }
    for event in TextMergeStream::new(Parser::new_ext(text, options())) {
        let tight = !lists.is_empty();
        match event {
            Event::Start(Tag::BlockQuote(kind)) => {
                stack.push(String::new());
                if let Some(kind) = kind {
                    emit(&mut stack, &mut bullet, alert(kind).1);
                    end_block(&mut stack, "\n");
                }
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                let inner = stack.pop().unwrap_or_default();
                let quoted: Vec<String> = inner.trim_end().lines().map(|l| if l.is_empty() { ">".to_owned() } else { format!("> {l}") }).collect();
                emit(&mut stack, &mut bullet, &quoted.join("\n"));
                end_block(&mut stack, "\n\n");
            }
            Event::Start(Tag::List(start)) => {
                if tight {
                    end_block(&mut stack, "\n");
                }
                lists.push(List { next: start });
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                if lists.is_empty() {
                    end_block(&mut stack, "\n\n");
                }
            }
            Event::Start(Tag::Item) => {
                let depth = lists.len().saturating_sub(1);
                let mark = match lists.last_mut().and_then(|l| l.next.as_mut()) {
                    Some(n) => {
                        let mark = format!("{n}. ");
                        *n += 1;
                        mark
                    }
                    None => "• ".to_owned(),
                };
                bullet = Some(format!("{}{mark}", "  ".repeat(depth)));
            }
            Event::End(TagEnd::Item) => {
                bullet = None;
                end_block(&mut stack, "\n");
            }
            Event::TaskListMarker(done) => {
                if let Some(b) = bullet.take() {
                    let indent: String = b.chars().take_while(|c| *c == ' ').collect();
                    bullet = Some(format!("{indent}{}", if done { "☑ " } else { "☐ " }));
                }
            }
            Event::End(TagEnd::Paragraph) => end_block(&mut stack, if tight { "\n" } else { "\n\n" }),
            Event::End(TagEnd::Heading(_)) => end_block(&mut stack, "\n\n"),
            Event::Start(Tag::CodeBlock(_)) => in_code = true,
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                if let Some(top) = stack.last_mut()
                    && top.ends_with('\n')
                    && !top.ends_with("\n\n")
                {
                    top.pop();
                }
                end_block(&mut stack, "\n\n");
            }
            Event::End(TagEnd::HtmlBlock) => end_block(&mut stack, "\n\n"),
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                if let Some(top) = stack.last_mut() {
                    if top.ends_with('\t') {
                        top.pop();
                    }
                    top.push('\n');
                }
            }
            Event::End(TagEnd::TableCell) => emit(&mut stack, &mut bullet, "\t"),
            Event::End(TagEnd::Table) => end_block(&mut stack, "\n"),
            Event::Start(Tag::Link { dest_url, .. }) => {
                let at = stack.last().map_or(0, String::len);
                links.push((dest_url.to_string(), at));
            }
            Event::End(TagEnd::Link) => {
                if let Some((url, at)) = links.pop()
                    && let Some(top) = stack.last_mut()
                {
                    let shown = top.get(at..).unwrap_or("").to_owned();
                    let bare = url.strip_prefix("mailto:").unwrap_or(&url);
                    if !url.is_empty() && shown.trim() != bare {
                        top.push_str(&format!(" ({url})"));
                    }
                }
            }
            Event::Start(Tag::FootnoteDefinition(name)) => emit(&mut stack, &mut bullet, &format!("[{name}] ")),
            Event::End(TagEnd::FootnoteDefinition) => end_block(&mut stack, "\n\n"),
            Event::Text(t) | Event::Code(t) | Event::InlineMath(t) | Event::DisplayMath(t) | Event::Html(t) | Event::InlineHtml(t) => {
                emit(&mut stack, &mut bullet, &t)
            }
            Event::FootnoteReference(name) => emit(&mut stack, &mut bullet, &format!("[{name}]")),
            Event::SoftBreak | Event::HardBreak => emit(&mut stack, &mut bullet, "\n"),
            Event::Rule => {
                emit(&mut stack, &mut bullet, "———");
                end_block(&mut stack, "\n\n");
            }
            _ => {}
        }
        let _ = in_code;
    }
    while stack.len() > 1 {
        let inner = stack.pop().unwrap_or_default();
        emit(&mut stack, &mut bullet, &inner);
    }
    stack.pop().unwrap_or_default().trim_end().to_owned()
}

/// What a piece of highlighted code is; views map it to a color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Plain,
    Keyword,
    String,
    Number,
    Comment,
    Function,
    Kind,
    Added,
    Removed,
    Meta,
}

impl Tok {
    /// CSS class suffix for the HTML view (`tk-kw`, …); empty for plain text.
    pub fn class(self) -> &'static str {
        match self {
            Tok::Plain => "",
            Tok::Keyword => "kw",
            Tok::String => "str",
            Tok::Number => "num",
            Tok::Comment => "com",
            Tok::Function => "fn",
            Tok::Kind => "ty",
            Tok::Added => "add",
            Tok::Removed => "del",
            Tok::Meta => "meta",
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

/// Collects tokens, merging neighbors of the same kind.
struct Tokens(Vec<(Tok, String)>);

impl Tokens {
    fn push(&mut self, kind: Tok, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.0.last_mut() {
            Some((last, body)) if *last == kind => body.push_str(text),
            _ => self.0.push((kind, text.to_owned())),
        }
    }
}

/// Fenced code split into highlighted pieces; concatenated, they are exactly `body`.
/// Unknown languages come back as one plain piece.
pub fn tokens(lang_name: &str, body: &str) -> Vec<(Tok, String)> {
    let lang = lang_of(lang_name, body);
    let mut out = Tokens(Vec::new());
    match lang {
        Lang::Plain => {
            out.push(Tok::Plain, body);
            return out.0;
        }
        Lang::Diff => {
            for (i, line) in body.split('\n').enumerate() {
                if i > 0 {
                    out.push(Tok::Plain, "\n");
                }
                let kind = if line.starts_with("+++") || line.starts_with("---") || line.starts_with("@@") || line.starts_with("diff ") {
                    Tok::Meta
                } else if line.starts_with('+') {
                    Tok::Added
                } else if line.starts_with('-') {
                    Tok::Removed
                } else {
                    Tok::Plain
                };
                out.push(kind, line);
            }
            return out.0;
        }
        _ => {}
    }
    let words = keywords(lang);
    let chars: Vec<char> = body.chars().collect();
    let text = |from: usize, to: usize| chars.get(from..to.min(chars.len())).map(|s| s.iter().collect::<String>()).unwrap_or_default();
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
    while let Some(&c) = chars.get(i) {
        // Comments.
        if let Some(mark) = line_comment.iter().find(|m| starts(i, m)) {
            let word_start = i == 0 || chars.get(i - 1).is_some_and(|p| p.is_whitespace());
            if *mark != "#" || word_start {
                let end = chars.get(i..).unwrap_or_default().iter().position(|&c| c == '\n').map_or(chars.len(), |n| i + n);
                out.push(Tok::Comment, &text(i, end));
                i = end;
                continue;
            }
        }
        if block_comment && starts(i, "/*") {
            let end = (i + 2..chars.len()).find(|&k| starts(k, "*/")).map_or(chars.len(), |k| k + 2);
            out.push(Tok::Comment, &text(i, end));
            i = end;
            continue;
        }
        if lang == Lang::Markup && starts(i, "<!--") {
            let end = (i + 4..chars.len()).find(|&k| starts(k, "-->")).map_or(chars.len(), |k| k + 3);
            out.push(Tok::Comment, &text(i, end));
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
            let is_key = matches!(lang, Lang::Json) && chars.get(end..).unwrap_or_default().iter().find(|c| !c.is_whitespace()) == Some(&':');
            out.push(if is_key { Tok::Function } else { Tok::String }, &text(i, end));
            i = end;
            line_start = false;
            continue;
        }
        // Markup tags.
        if lang == Lang::Markup && c == '<' {
            let end = chars.get(i..).unwrap_or_default().iter().position(|&c| c == '>').map_or(chars.len(), |n| i + n + 1);
            out.push(Tok::Keyword, &text(i, end));
            i = end;
            continue;
        }
        // Numbers.
        if c.is_ascii_digit() && !chars.get(i.wrapping_sub(1)).is_some_and(|p| p.is_alphanumeric() || *p == '_') {
            let end = (i..chars.len())
                .find(|&k| !chars.get(k).is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_')))
                .unwrap_or(chars.len());
            out.push(Tok::Number, &text(i, end));
            i = end;
            line_start = false;
            continue;
        }
        // Shell variables.
        if lang == Lang::Shell && c == '$' {
            let end = if chars.get(i + 1) == Some(&'{') {
                chars.get(i..).unwrap_or_default().iter().position(|&c| c == '}').map_or(chars.len(), |n| i + n + 1)
            } else {
                (i + 1..chars.len()).find(|&k| !chars.get(k).is_some_and(|c| c.is_alphanumeric() || *c == '_')).unwrap_or(chars.len())
            };
            out.push(Tok::Kind, &text(i, end));
            i = end;
            continue;
        }
        // Words.
        if c.is_alphabetic() || c == '_' || (lang == Lang::Css && c == '-') {
            let end = (i..chars.len())
                .find(|&k| !chars.get(k).is_some_and(|c| c.is_alphanumeric() || *c == '_' || (matches!(lang, Lang::Css | Lang::Shell) && *c == '-')))
                .unwrap_or(chars.len());
            let word = text(i, end);
            let next = chars.get(end..).unwrap_or_default().iter().find(|c| **c != ' ').copied();
            let is_kw = if lang == Lang::Sql { words.contains(&word.to_lowercase().as_str()) } else { words.contains(&word.as_str()) };
            // Calls, shell commands, CSS properties and config keys.
            let is_name = (lang == Lang::Shell && line_start)
                || ((next == Some('(') || (lang == Lang::Rust && next == Some('!'))) && lang != Lang::Css)
                || (lang == Lang::Css && next == Some(':'))
                || (lang == Lang::Conf && line_start && matches!(next, Some('=' | ':')));
            let kind = if is_kw {
                Tok::Keyword
            } else if is_name {
                Tok::Function
            } else if word.chars().next().is_some_and(char::is_uppercase) && matches!(lang, Lang::Rust | Lang::Js | Lang::Go | Lang::CLike) {
                Tok::Kind
            } else {
                Tok::Plain
            };
            out.push(kind, &word);
            i = end;
            line_start = false;
            continue;
        }
        if c == '\n' {
            line_start = true;
        } else if !c.is_whitespace() && (lang != Lang::Shell || !matches!(c, '|' | ';' | '&' | '(')) {
            line_start = false;
        }
        let mut buf = [0u8; 4];
        out.push(Tok::Plain, c.encode_utf8(&mut buf));
        i += 1;
    }
    out.0
}

/// Fenced code as escaped HTML with `<span class="tk-…">` pieces.
pub fn highlight_html(lang: &str, body: &str) -> String {
    let mut out = String::with_capacity(body.len() * 2);
    for (kind, text) in tokens(lang, body) {
        match kind {
            Tok::Plain => out.push_str(&escape(&text)),
            kind => {
                out.push_str(&format!("<span class=\"tk-{}\">", kind.class()));
                out.push_str(&escape(&text));
                out.push_str("</span>");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_inline_marks_and_breaks() {
        assert_eq!(
            to_html("<b>x</b> **ok** *em* ~~no~~ `a<b`\nfin  \nsigue"),
            "<p>&lt;b&gt;x&lt;/b&gt; <strong>ok</strong> <em>em</em> <del>no</del> <code>a&lt;b</code><br>fin<br>sigue</p>"
        );
        assert_eq!(to_html("2 * 3 * 4 snake_case_name"), "<p>2 * 3 * 4 snake_case_name</p>");
        assert_eq!(to_html("tarda ~5 min y ~10 MB"), "<p>tarda ~5 min y ~10 MB</p>", "loose tildes are not strikethrough");
    }

    #[test]
    fn all_six_heading_levels() {
        let html = to_html("# a\n## b\n### c\n#### d\n##### e\n###### f");
        for n in 1..=6 {
            assert!(html.contains(&format!("<h{n} class=\"cv-h\">")), "{html}");
        }
    }

    #[test]
    fn nested_ordered_and_task_lists() {
        let html = to_html("3. uno\n   - a\n     - b\n4. dos\n\n- [x] hecho\n- [ ] falta");
        assert!(html.starts_with("<ol start=\"3\"><li>uno<ul><li>a<ul><li>b</li></ul></li></ul></li><li>dos</li></ol>"), "{html}");
        assert!(html.contains("<ul><li><input type=\"checkbox\" disabled checked> hecho</li><li><input type=\"checkbox\" disabled> falta</li></ul>"), "{html}");
        assert_eq!(to_html("1. a\n2. b"), "<ol><li>a</li><li>b</li></ol>");
    }

    #[test]
    fn quotes_hold_several_blocks_and_alerts_get_a_title() {
        let html = to_html("> **Ojo:** cita\n>\n> - lista\n> - dos");
        assert_eq!(html, "<blockquote><p><strong>Ojo:</strong> cita</p><ul><li>lista</li><li>dos</li></ul></blockquote>");
        let alert = to_html("> [!WARNING]\n> Cuidado");
        assert!(alert.starts_with("<blockquote class=\"cv-alert cv-alert-warning\"><p class=\"cv-alert-t\">Advertencia</p>"), "{alert}");
        assert!(alert.contains("Cuidado"), "{alert}");
    }

    #[test]
    fn tables_scroll_sideways_and_keep_alignment() {
        let html = to_html("| A | B | C |\n|:--|:-:|--:|\n| `a \\| b` | ~~x~~ | 3<br>4 |");
        assert_eq!(
            html,
            "<div class=\"cv-table\"><table><thead><tr><th>A</th><th class=\"cv-c\">B</th><th class=\"cv-r\">C</th></tr></thead><tbody><tr><td><code>a | b</code></td><td class=\"cv-c\"><del>x</del></td><td class=\"cv-r\">3<br>4</td></tr></tbody></table></div>"
        );
    }

    #[test]
    fn rules_images_and_footnotes() {
        assert_eq!(to_html("a\n\n---\n\nb"), "<p>a</p><hr><p>b</p>");
        assert_eq!(
            to_html("![logo](https://x.io/a.png)"),
            "<p><a class=\"cv-img\" href=\"https://x.io/a.png\" target=\"_blank\" rel=\"noopener noreferrer\">🖼 logo</a></p>"
        );
        assert_eq!(to_html("![x](data:image/png;base64,AAA)"), "<p><span class=\"cv-img\">🖼 x</span></p>");
        let notes = to_html("Dato[^1].\n\n[^1]: Fuente.");
        assert!(notes.contains("<sup class=\"cv-fnref\">[1]</sup>"), "{notes}");
        assert!(notes.contains("<div class=\"cv-fn\"><sup>1</sup> <p>Fuente.</p></div>"), "{notes}");
    }

    #[test]
    fn every_fence_style_becomes_a_code_block_with_a_copy_button() {
        let head = |lang: &str| format!("<div class=\"cv-code\"><div class=\"cv-code-head\" contenteditable=\"false\" data-lang=\"{lang}\"><button type=\"button\" class=\"cv-copy\" data-cv-act=\"copy-code\" aria-label=\"Copiar código\"></button></div><pre><code>");
        assert_eq!(to_html("```\na < b\n```"), format!("{}a &lt; b</code></pre></div>", head("código")));
        assert_eq!(to_html("~~~text\nhola\n~~~"), format!("{}hola</code></pre></div>", head("text")));
        let four = to_html("````md\n```\ninner\n```\n````");
        assert!(four.starts_with(&head("md")) && four.contains("```\ninner\n```"), "{four}");
        assert!(to_html("```Rust title=\"main.rs\"\nfn main() {}\n```").starts_with(&head("rust")));
        assert!(to_html("```{.python}\nx = 1\n```").starts_with(&head("python")));
        assert!(to_html("```rust,no_run\nlet a = 1;\n```").starts_with(&head("rust")));
        assert_eq!(to_html("texto\n\n    sangría\n"), format!("<p>texto</p>{}sangría</code></pre></div>", head("código")));
    }

    #[test]
    fn a_cut_transcript_still_shows_its_open_code_block() {
        let html = to_html("texto\n```bash\necho hola\nls");
        assert!(html.starts_with("<p>texto</p><div class=\"cv-code\">"), "{html}");
        assert!(html.contains("echo</span> hola\n"), "{html}");
        assert!(html.ends_with("</code></pre></div>"), "{html}");
        let list = to_html("- uno\n- dos con `cod");
        assert!(list.starts_with("<ul><li>uno</li><li>dos con `cod</li></ul>"), "{list}");
    }

    #[test]
    fn agent_html_and_hostile_links_stay_inert() {
        assert_eq!(to_html("<script>alert(1)</script>"), "<div class=\"cv-raw\">&lt;script&gt;alert(1)&lt;/script&gt;</div>");
        assert_eq!(to_html("x <img src=x onerror=alert(1)> y"), "<p>x &lt;img src=x onerror=alert(1)&gt; y</p>");
        assert_eq!(to_html("[x](javascript:alert(1))"), "<p><span class=\"cv-link\" title=\"javascript:alert(1)\">x</span></p>");
        assert_eq!(to_html("[x](JaVaScRiPt:alert(1))"), "<p><span class=\"cv-link\" title=\"JaVaScRiPt:alert(1)\">x</span></p>");
        assert_eq!(to_html("[x](&#106;avascript:alert(1))"), "<p><span class=\"cv-link\" title=\"javascript:alert(1)\">x</span></p>");
        assert_eq!(to_html("[main](src/main.rs)"), "<p><span class=\"cv-link\" title=\"src/main.rs\">main</span></p>");
        assert_eq!(
            to_html("[doc](https://x.io/?a=1&b=\"2\" \"t'\")"),
            "<p><a href=\"https://x.io/?a=1&amp;b=&quot;2&quot;\" target=\"_blank\" rel=\"noopener noreferrer\" title=\"t&#39;\">doc</a></p>"
        );
        let fence = to_html("```\"><img src=x onerror=alert(1)>\nx\n```");
        assert!(fence.contains("data-lang=\"&quot;&gt;&lt;img\""), "{fence}");
        assert!(!fence.contains("<img"), "{fence}");
        assert_eq!(
            to_html("<mailto@x.io> <ana@x.io>"),
            "<p><a href=\"mailto:mailto@x.io\" target=\"_blank\" rel=\"noopener noreferrer\">mailto@x.io</a> <a href=\"mailto:ana@x.io\" target=\"_blank\" rel=\"noopener noreferrer\">ana@x.io</a></p>"
        );
    }

    #[test]
    fn bare_addresses_become_links_without_trailing_punctuation() {
        assert_eq!(
            to_html("ver https://github.com/tmux/tmux/wiki. y (https://x.io/a_(b)) fin"),
            "<p>ver <a href=\"https://github.com/tmux/tmux/wiki\" target=\"_blank\" rel=\"noopener noreferrer\">https://github.com/tmux/tmux/wiki</a>. y (<a href=\"https://x.io/a_(b)\" target=\"_blank\" rel=\"noopener noreferrer\">https://x.io/a_(b)</a>) fin</p>"
        );
        assert_eq!(to_html("solo https:// no"), "<p>solo https:// no</p>");
        assert_eq!(
            to_html("[https://a.io](https://a.io)"),
            "<p><a href=\"https://a.io\" target=\"_blank\" rel=\"noopener noreferrer\">https://a.io</a></p>",
            "no link inside a link"
        );
    }

    #[test]
    fn plain_text_for_the_clipboard() {
        let md = "## Plan\n\nHaz **esto** y [mira](https://x.io).\n\n1. uno\n2. dos\n   - a\n\n- [x] hecho\n- [ ] falta\n\n> cita\n> sigue\n\n```rust\nfn main() {}\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |";
        assert_eq!(
            to_plain(md),
            "Plan\n\nHaz esto y mira (https://x.io).\n\n1. uno\n2. dos\n  • a\n\n☑ hecho\n☐ falta\n\n> cita\n> sigue\n\nfn main() {}\n\nA\tB\n1\t2"
        );
        assert_eq!(to_plain("ver https://a.io y [https://b.io](https://b.io)"), "ver https://a.io y https://b.io");
    }

    #[test]
    fn tokens_cover_the_body_exactly_and_name_each_piece() {
        for (lang, body) in [
            ("rust", "fn main() { let s = \"<a>\"; // nota\n}"),
            ("python", "def f(x):\n    return '''a\nb''' # c"),
            ("bash", "echo $HOME ${X} | grep -v x # y"),
            ("json", "{\"a\": [1, true, null]}"),
            ("", "@@ -1 +1 @@\n-a\n+b"),
            ("weird", "ñandú → 🙂"),
        ] {
            let joined: String = tokens(lang, body).into_iter().map(|(_, t)| t).collect();
            assert_eq!(joined, body, "{lang}");
        }
        let rust = tokens("rust", "fn main() { let s = \"<a>\"; // nota\n}");
        assert!(rust.contains(&(Tok::Keyword, "fn".into())), "{rust:?}");
        assert!(rust.contains(&(Tok::Function, "main".into())), "{rust:?}");
        assert!(rust.contains(&(Tok::String, "\"<a>\"".into())), "{rust:?}");
        assert!(rust.contains(&(Tok::Comment, "// nota".into())), "{rust:?}");
        let sh = tokens("bash", "echo $HOME # x");
        assert!(sh.contains(&(Tok::Function, "echo".into())) && sh.contains(&(Tok::Kind, "$HOME".into())), "{sh:?}");
        let json = tokens("json", "{\"a\": 1}");
        assert!(json.contains(&(Tok::Function, "\"a\"".into())) && json.contains(&(Tok::Number, "1".into())), "{json:?}");
        assert_eq!(
            tokens("", "@@ -1 +1 @@\n-a\n+b"),
            vec![(Tok::Meta, "@@ -1 +1 @@".into()), (Tok::Plain, "\n".into()), (Tok::Removed, "-a".into()), (Tok::Plain, "\n".into()), (Tok::Added, "+b".into())]
        );
    }

    #[test]
    fn highlighted_html_escapes_everything() {
        assert_eq!(highlight_html("text", "a < b & \"c\""), "a &lt; b &amp; &quot;c&quot;");
        assert_eq!(
            highlight_html("js", "const a = '<b>';"),
            "<span class=\"tk-kw\">const</span> a = <span class=\"tk-str\">&#39;&lt;b&gt;&#39;</span>;"
        );
        assert_eq!(code_lang("Rust title=\"x\""), "rust");
        assert_eq!(code_lang(""), "");
    }

}
