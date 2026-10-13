//! Python desktop wrapped URL/local-path rules; native OSC8 stays in engine/select.
use regex::Regex;
use std::sync::OnceLock;
const WRAP: &str = r"[ \t]*\r?\n[ \t]*";
const URL: &str = r#"https?://(?:[^\s'"<>\]\)]+|[ \t]*\r?\n[ \t]*[^\s'"<>\]\)]+)+"#;
fn wrap() -> Option<&'static Regex> {
    static VALUE: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(WRAP)).as_ref().ok()
}
fn patterns() -> Option<&'static [Regex; 2]> {
    static VALUE: OnceLock<Result<[Regex; 2], regex::Error>> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            Ok([
                Regex::new(URL)?,
                Regex::new(r"(?:~/|/|[\w.+@%,=~-]+/)(?:[\w.+@%,=~/-]+|[ \t]*\r?\n[ \t]*[\w.+@%,=~/-]+)+(?::\d+){0,2}")?,
            ])
        })
        .as_ref()
        .ok()
}
/// Sentence punctuation right after a link is prose, not part of it
/// («…subtitulado.mp4.» gave «Ruta local invalida», 13-oct).
pub fn trim_trailing_punctuation(link: &str) -> &str {
    link.trim_end_matches(['.', ',', ';', ':', '!', '?'])
}
pub fn normalize_wrapped_url_text(text: &str) -> String {
    let (Some(patterns), Some(wrap)) = (patterns(), wrap()) else {
        return text.to_owned();
    };
    let Some(url) = patterns.first() else {
        return text.to_owned();
    };
    url.replace_all(text, |caps: &regex::Captures<'_>| {
        wrap.replace_all(caps.get(0).map_or("", |m| m.as_str()), "")
            .into_owned()
    })
    .into_owned()
}
pub fn url_from_wrapped_text(
    text: &str,
    fragment: &str,
    row: Option<usize>,
    col: Option<usize>,
) -> Option<String> {
    let (Some(patterns), Some(wrap)) = (patterns(), wrap()) else {
        return None;
    };
    let point = row.zip(col).and_then(|(row, col)| {
        let mut offset = 0;
        for (index, line) in text.split_inclusive('\n').enumerate() {
            if index == row {
                return Some(offset + line.char_indices().nth(col).map_or(line.len(), |(i, _)| i));
            }
            offset += line.len();
        }
        None
    });
    let frag = normalize_wrapped_url_text(fragment).trim().to_owned();
    let mut candidates = Vec::new();
    for pattern in patterns {
        for m in pattern.find_iter(text) {
            let joined =
                trim_trailing_punctuation(&wrap.replace_all(m.as_str(), "")).to_owned();
            let body = joined
                .rsplit_once(':')
                .filter(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                .map_or(joined.as_str(), |(body, _)| body);
            if !joined.starts_with("http") && body.starts_with('/') && body.matches('/').count() < 2
            {
                continue;
            }
            candidates.push((m.start(), m.end(), joined));
        }
    }
    let candidates: Vec<_> = candidates
        .iter()
        .enumerate()
        .filter(|(index, m)| {
            !candidates
                .iter()
                .enumerate()
                .any(|(other, o)| other != *index && o.0 <= m.0 && m.1 <= o.1)
        })
        .map(|(_, m)| m)
        .collect();
    for candidate in &candidates {
        if (!frag.is_empty() && (candidate.2.contains(&frag) || frag.contains(&candidate.2)))
            || point.is_some_and(|p| candidate.0 <= p && p < candidate.1)
        {
            return Some(candidate.2.clone());
        }
    }
    if frag.is_empty() && point.is_none() && candidates.len() == 1 {
        return candidates.first().map(|c| c.2.clone());
    }
    (!frag.is_empty()).then_some(frag)
}

/// Completes a clicked link fragment from its own pane's text (`capture-pane -J`, which
/// joins tmux soft wraps and excludes neighbouring panes). Agent hard wraps are joined
/// too, but a local path is only extended when `exists` confirms the longer path, so an
/// unrelated next line never gets glued on. `None` keeps the caller's fragment.
pub fn complete_from_pane(
    text: &str,
    fragment: &str,
    exists: impl Fn(&str) -> bool,
) -> Option<String> {
    let (Some(patterns), Some(wrap)) = (patterns(), wrap()) else {
        return None;
    };
    let frag = normalize_wrapped_url_text(fragment).trim().to_owned();
    if frag.is_empty() {
        return None;
    }
    let frag = trim_trailing_punctuation(&frag).to_owned();
    let mut best: Option<String> = None;
    let mut keep = |candidate: String| {
        if best.as_ref().is_none_or(|b| candidate.len() > b.len()) {
            best = Some(candidate);
        }
    };
    let found = |joined: &str| {
        let body = joined
            .split_once(':')
            .filter(|(_, n)| n.split(':').all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
            .map_or(joined, |(body, _)| body);
        exists(body)
    };
    for (index, pattern) in patterns.iter().enumerate() {
        for m in pattern.find_iter(text) {
            let joined =
                trim_trailing_punctuation(&wrap.replace_all(m.as_str(), "")).to_owned();
            if !joined.contains(&frag) {
                continue;
            }
            if index == 0 {
                keep(joined);
                continue;
            }
            // A hard wrap may glue the next line onto the path («notes.md» + «- item»):
            // try the path up to each line break, longest first, and keep one that exists.
            let heads = wrap.find_iter(m.as_str()).map(|brk| {
                trim_trailing_punctuation(&wrap.replace_all(&m.as_str()[..brk.start()], ""))
                    .to_owned()
            });
            let mut prefixes: Vec<String> = heads.collect();
            prefixes.push(joined);
            for prefix in prefixes.into_iter().rev() {
                if found(&prefix) {
                    keep(prefix);
                    break;
                }
            }
        }
    }
    best
}

/// One primary press owns its URL and cell until the matching release.
#[derive(Debug, Clone, PartialEq)]
pub struct PrimaryPress {
    position: (f64, f64),
    pub point: (u16, u16),
    pub link: Option<String>,
}
impl PrimaryPress {
    pub fn new(position: (f64, f64), point: (u16, u16), link: Option<String>) -> Self {
        Self {
            position,
            point,
            link,
        }
    }
    pub fn release(self, position: (f64, f64), button: u32) -> Option<Self> {
        (button == 1
            && (position.0 - self.position.0).abs() < 5.
            && (position.1 - self.position.1).abs() < 5.)
            .then_some(self)
    }
}

pub fn link_at(
    engine: &super::engine::TermEngine,
    point: super::engine::Point,
    row: u16,
    col: u16,
) -> Option<String> {
    use super::engine::{RowRender, find_urls};
    if let Some(url) = engine.hyperlink_at(row, col) {
        return Some(url);
    }
    let native = find_urls(engine.engine(), point.0)
        .into_iter()
        .find(|url| url.start <= point && point <= url.end)
        .map(|url| url.url);
    let mut text = String::new();
    let mut rendered = RowRender::default();
    let (cols, rows) = engine.size();
    let first = usize::from(row.saturating_sub(3));
    for line in first..usize::from(row.saturating_add(5).min(rows)) {
        engine.render_line(line, &mut rendered);
        let mut column = 0;
        for run in &rendered.runs {
            while column < run.col {
                text.push(' ');
                column += 1;
            }
            text.push_str(&run.text);
            column = run.col.saturating_add(run.cells);
        }
        while column < cols {
            text.push(' ');
            column += 1;
        }
        text.push('\n');
    }
    let lines: Vec<_> = text.split_inclusive('\n').collect();
    let continues = |index: usize| {
        if engine.row_wraps(first + index) {
            return true;
        }
        let Some((previous, next)) = lines.get(index).zip(lines.get(index + 1)) else {
            return false;
        };
        let mut words = next.split_whitespace();
        let fragment = words.next().unwrap_or_default();
        let has_component = fragment
            .trim_end_matches(['.', ',', ';', '!'])
            .contains(['/', '?', '&', '=', '#', '%', '_', '-', '.', ':']);
        // A TUI may wrap inside a padded column. Require an indented URL
        // component, rather than treating every hard newline as a soft wrap.
        next.starts_with([' ', '\t'])
            && !fragment.starts_with("http://")
            && !fragment.starts_with("https://")
            && (has_component
                || (words.next().is_none()
                    && previous
                        .trim_end()
                        .ends_with(['/', '?', '&', '=', '#', '%', '_', '-', '.'])))
    };
    let clicked = usize::from(row) - first;
    let mut top = clicked;
    let mut bottom = clicked;
    while top > 0 && continues(top - 1) {
        top -= 1;
    }
    while bottom + 1 < lines.len() && continues(bottom) {
        bottom += 1;
    }
    let joined = lines.get(top..=bottom).unwrap_or_default().concat();
    let wrapped = url_from_wrapped_text(&joined, "", Some(clicked - top), Some(usize::from(col)));
    match (native, wrapped) {
        (Some(native), Some(wrapped))
            if wrapped.starts_with(&native) && !text.contains(&wrapped) =>
        {
            Some(wrapped)
        }
        (Some(native), _) => Some(native),
        (None, wrapped) => wrapped,
    }
}

/// Whether a gesture should be forwarded to the terminal application.
pub fn report_mouse(shift: bool, link_gesture: bool) -> bool {
    !shift && !link_gesture
}
