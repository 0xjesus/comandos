//! Hides Claude Code's gray tool noise («└ Read…», «Referenced file…»,
//! «Skills restored», «(ctrl+o to expand)», the recap) when painting: rows of
//! a pane whose text is all gray are not drawn and the rows above move down to
//! close the gap. Only the picture changes; tmux and the grid stay intact.
use crate::term::engine::{RowRender, RunKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    Gray,
    Bright,
    Glyph,
}

/// What one grid row shows: `(first col, end col, ink, first visible char)`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowInk {
    pub segs: Vec<(u16, u16, Ink, char)>,
}

fn luma([r, g, b]: [u8; 3]) -> f64 {
    0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)
}

/// Gray = dimmed, or an unsaturated colour clearly darker than normal text.
pub fn is_gray(fg: [u8; 3], dim: bool, normal: [u8; 3]) -> bool {
    let spread = fg.iter().max().unwrap_or(&0) - fg.iter().min().unwrap_or(&0);
    dim || (spread <= 34 && luma(fg) < luma(normal) * 0.82)
}

pub fn row_ink(row: &RowRender, normal: [u8; 3]) -> RowInk {
    let mut out = RowInk::default();
    for run in &row.runs {
        if run.style.hidden {
            continue;
        }
        let end = run.col.saturating_add(run.cells);
        if matches!(run.kind, RunKind::Box(_)) {
            let first = run.text.chars().next().unwrap_or('─');
            // A coloured drawing (Claude Code's logo, a bar) is content; a
            // rule or a gray frame is not.
            let drawn = !matches!(first, '─' | '━') && !is_gray(run.style.fg, run.style.dim, normal);
            out.segs.push((run.col, end, if drawn { Ink::Bright } else { Ink::Glyph }, first));
            continue;
        }
        let Some((lead, first)) = run.text.char_indices().find(|(_, c)| !c.is_whitespace()) else {
            continue;
        };
        let lead = u16::try_from(run.text[..lead].chars().count()).unwrap_or(0);
        let start = run.col.saturating_add(lead);
        // Text on a painted background (a tmux selection, a highlight) stays.
        let painted = row
            .bg_runs
            .iter()
            .any(|&(col, cells, _)| col < end && col.saturating_add(cells) > start);
        let ink = if !painted && is_gray(run.style.fg, run.style.dim, normal) {
            Ink::Gray
        } else {
            Ink::Bright
        };
        // «❯ /model»: the command is noted apart from the prompt mark.
        let command = matches!(first, '❯' | '>')
            .then(|| run.text.chars().enumerate().skip(usize::from(lead) + 1).find(|(_, c)| !c.is_whitespace()))
            .flatten()
            .filter(|(_, c)| *c == '/')
            .and_then(|(i, _)| u16::try_from(i).ok())
            .map(|i| run.col.saturating_add(i))
            .filter(|col| *col < end);
        match command {
            Some(col) => {
                out.segs.push((start, start.saturating_add(1), ink, first));
                out.segs.push((col, end, ink, '/'));
            }
            None => out.segs.push((start, end, ink, first)),
        }
    }
    out
}

/// A pane: columns `[left, left + width)`, rows `[top, top + height)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub left: u16,
    pub width: u16,
    pub top: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Keep,
    Prompt,
    /// A prompt that runs a slash command («❯ /model»).
    Command,
    Blank,
    Rule,
    Gray,
    /// Gray that is surely tool output, never the tail of your prompt.
    Tool,
}

fn kind(ink: &RowInk, r: &Region) -> Kind {
    let right = r.left.saturating_add(r.width);
    let mut segs = ink.segs.iter().filter(|s| s.0 < right && s.1 > r.left).peekable();
    let Some(first) = segs.peek().copied() else {
        return Kind::Blank;
    };
    let all: Vec<_> = segs.collect();
    if all.iter().all(|s| s.2 == Ink::Glyph) {
        return if all.iter().all(|s| s.3 == '─' || s.3 == '━') { Kind::Rule } else { Kind::Keep };
    }
    if matches!(first.3, '❯' | '>') {
        let command = all.get(1).is_some_and(|s| s.3 == '/' && s.0 <= first.1.saturating_add(1));
        return if command { Kind::Command } else { Kind::Prompt };
    }
    if all.iter().any(|s| s.2 == Ink::Bright) {
        return Kind::Keep;
    }
    if matches!(first.3, '└' | '⎿' | '※' | '✻' | '✳' | '✶' | '*') {
        return Kind::Tool;
    }
    Kind::Gray
}

/// Which rows stay: gray (unless it continues your prompt) and tool rows go,
/// and runs of blank rows shrink to one. What a slash command printed right
/// under it («⎿ Set model to…») is its answer and stays.
fn keep_flags(kinds: &[Kind]) -> Vec<bool> {
    let mut keep = vec![true; kinds.len()];
    let mut in_prompt = false;
    let mut in_command = false;
    let mut last_blank = true;
    for (i, &k) in kinds.iter().enumerate() {
        keep[i] = match k {
            Kind::Gray => in_prompt || in_command,
            Kind::Tool => in_command,
            Kind::Blank => !last_blank,
            _ => true,
        };
        in_command = match k {
            Kind::Command => true,
            Kind::Gray | Kind::Tool => in_command,
            _ => false,
        };
        in_prompt = match k {
            Kind::Prompt | Kind::Command => true,
            Kind::Gray => in_prompt,
            _ => false,
        };
        if keep[i] {
            last_blank = k == Kind::Blank;
        }
    }
    keep
}

/// History rows (top to bottom) worth showing above a filtered pane, so the
/// rows freed by hidden noise fill with earlier text instead of staying
/// empty: `(row, is blank)`. A row is judged by the rows before it only, so
/// the answer for the first rows does not change when more follow.
pub fn kept_rows(rows: &[RowInk], width: u16) -> Vec<(usize, bool)> {
    let r = Region { left: 0, width, top: 0, height: 0 };
    let kinds: Vec<Kind> = rows.iter().map(|ink| kind(ink, &r)).collect();
    keep_flags(&kinds)
        .into_iter()
        .enumerate()
        .filter_map(|(i, k)| k.then_some((i, kinds[i] == Kind::Blank)))
        .collect()
}

/// Empty display rows at the top of a compacted region.
pub fn holes(map: &[Option<u16>], r: &Region) -> u16 {
    map.iter()
        .flatten()
        .min()
        .map_or(0, |first| first.saturating_sub(r.top))
}

/// Rows of a pane above Claude Code's input box (the rule over it), when
/// the pane shows one. Only such panes are scrolled by the app.
pub fn input_top(rows: &[RowInk], r: &Region) -> Option<u16> {
    let start = usize::from(r.top);
    let end = (start + usize::from(r.height)).min(rows.len());
    if end <= start {
        return None;
    }
    let rules: Vec<usize> = rows[start..end]
        .iter()
        .enumerate()
        .filter(|(_, ink)| kind(ink, r) == Kind::Rule)
        .map(|(i, _)| i)
        .collect();
    match rules.as_slice() {
        [.., above, _below] if end - start - above <= 14 && *above > 0 => u16::try_from(*above).ok(),
        _ => None,
    }
}

/// Row map of a pane scrolled back by the app: the rows above the input box
/// show history, the input box and below stay live.
pub fn scrolled_map(r: &Region, input: u16) -> Vec<Option<u16>> {
    (0..r.height)
        .map(|i| (i >= input).then(|| r.top.saturating_add(i)))
        .collect()
}

/// Grid row → display row for one region, or `None` when nothing moves.
/// The input box (from the rule above it) and everything below never move.
pub fn compact(rows: &[RowInk], r: &Region) -> Option<Vec<Option<u16>>> {
    let start = usize::from(r.top);
    let end = (start + usize::from(r.height)).min(rows.len());
    if end <= start {
        return None;
    }
    let kinds: Vec<Kind> = rows[start..end].iter().map(|ink| kind(ink, r)).collect();
    let rules: Vec<usize> = kinds
        .iter()
        .enumerate()
        .filter(|(_, k)| **k == Kind::Rule)
        .map(|(i, _)| i)
        .collect();
    let len = kinds.len();
    let protect = match rules.as_slice() {
        [.., above, _below] if len - above <= 14 => *above,
        // One rule: a dialog open in place of the input box (the model
        // picker, a permission question). All of it is shown.
        [.., only] if len - only <= 30 => *only,
        _ => len.saturating_sub(3),
    };
    let mut keep = keep_flags(&kinds[..protect]);
    keep.resize(len, true);
    if keep[..protect].iter().all(|k| *k) {
        return None;
    }
    let mut map = vec![None; len];
    let mut slot = protect;
    for i in (0..protect).rev() {
        if keep[i] {
            slot -= 1;
            map[i] = Some(slot);
        }
    }
    for (i, item) in map.iter_mut().enumerate().skip(protect) {
        *item = Some(i);
    }
    Some(
        map.into_iter()
            .map(|d| d.and_then(|d| u16::try_from(d + start).ok()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ink(segs: &[(u16, u16, Ink, char)]) -> RowInk {
        RowInk { segs: segs.to_vec() }
    }
    #[test]
    fn gray_rows_go_away_and_the_answer_moves_down() {
        let region = Region { left: 0, width: 80, top: 0, height: 9 };
        let rows = vec![
            ink(&[(0, 20, Ink::Gray, '❯')]),          // 0 your prompt: kept
            ink(&[(2, 40, Ink::Gray, '└')]),          // 1 noise
            ink(&[(0, 50, Ink::Bright, '●')]),        // 2 answer
            ink(&[(2, 40, Ink::Gray, 'R')]),          // 3 noise
            ink(&[]),                                 // 4 blank
            ink(&[]),                                 // 5 blank (collapsed)
            ink(&[(0, 80, Ink::Glyph, '─')]),         // 6 rule above input
            ink(&[(0, 3, Ink::Bright, '❯')]),         // 7 input
            ink(&[(0, 80, Ink::Glyph, '─')]),         // 8 rule below input
        ];
        let map = compact(&rows, &region).unwrap();
        assert_eq!(map[1], None);
        assert_eq!(map[3], None);
        assert_eq!(map[5], None);
        // Kept rows end right above the input box, in order.
        assert_eq!((map[0], map[2], map[4]), (Some(3), Some(4), Some(5)));
        assert_eq!((map[6], map[7], map[8]), (Some(6), Some(7), Some(8)));
    }
    #[test]
    fn a_pane_without_noise_is_left_alone() {
        let region = Region { left: 0, width: 80, top: 0, height: 4 };
        let rows = vec![
            ink(&[(0, 50, Ink::Bright, '●')]),
            ink(&[]),
            ink(&[(0, 50, Ink::Bright, 'x')]),
            ink(&[]),
        ];
        assert_eq!(compact(&rows, &region), None);
    }
    #[test]
    fn noise_in_one_pane_never_hides_the_other() {
        let left = Region { left: 0, width: 40, top: 0, height: 6 };
        let right = Region { left: 41, width: 39, top: 0, height: 6 };
        let rows = vec![
            ink(&[(2, 30, Ink::Gray, '└'), (41, 70, Ink::Bright, '●')]),
            ink(&[(0, 30, Ink::Bright, '●'), (41, 70, Ink::Bright, 'y')]),
            ink(&[]),
            ink(&[]),
            ink(&[]),
            ink(&[]),
        ];
        assert_eq!(compact(&rows, &left).unwrap()[0], None);
        assert_eq!(compact(&rows, &right), None);
    }
    #[test]
    fn history_fills_with_text_not_noise() {
        let rows = vec![
            ink(&[(0, 50, Ink::Bright, '●')]),
            ink(&[(2, 40, Ink::Gray, '└')]),
            ink(&[]),
            ink(&[]),
            ink(&[(0, 50, Ink::Bright, 'x')]),
            ink(&[(2, 40, Ink::Gray, 'R')]),
            ink(&[]),
        ];
        assert_eq!(kept_rows(&rows, 80), vec![(0, false), (2, true), (4, false), (6, true)]);
        let region = Region { left: 0, width: 80, top: 2, height: 5 };
        assert_eq!(holes(&[None, Some(4), Some(5), None, Some(6)], &region), 2);
    }
    #[test]
    fn a_slash_command_keeps_what_it_printed() {
        let region = Region { left: 0, width: 80, top: 0, height: 9 };
        let rule = ink(&[(0, 80, Ink::Glyph, '─')]);
        let rows = vec![
            ink(&[(0, 1, Ink::Bright, '❯'), (2, 8, Ink::Bright, '/')]), // 0 «❯ /model»
            ink(&[(2, 40, Ink::Gray, '⎿')]),                             // 1 its answer: kept
            ink(&[]),                                                    // 2
            ink(&[(0, 50, Ink::Bright, '●')]),                           // 3
            ink(&[(2, 40, Ink::Gray, '⎿')]),                             // 4 tool noise
            ink(&[]),                                                    // 5
            rule.clone(),
            ink(&[(0, 3, Ink::Bright, '❯')]),
            rule,
        ];
        let map = compact(&rows, &region).unwrap();
        assert_eq!((map[0], map[1], map[4]), (Some(1), Some(2), None));
    }
    #[test]
    fn an_open_dialog_is_shown_whole() {
        // The model picker: one rule, then its gray description and options.
        let region = Region { left: 0, width: 80, top: 0, height: 16 };
        let mut rows = vec![
            ink(&[(0, 50, Ink::Bright, '●')]),
            ink(&[(2, 40, Ink::Gray, '⎿')]),
            ink(&[(0, 80, Ink::Glyph, '─')]),
            ink(&[(2, 14, Ink::Bright, 'S')]),
            ink(&[(2, 70, Ink::Gray, 'S')]),
        ];
        rows.resize(16, ink(&[(4, 60, Ink::Gray, '2')]));
        let map = compact(&rows, &region).unwrap();
        assert_eq!(map[1], None);
        assert!(map[2..].iter().enumerate().all(|(i, shown)| *shown == Some(i as u16 + 2)));
    }
    #[test]
    fn claude_input_box_and_scrolled_rows() {
        let region = Region { left: 0, width: 80, top: 2, height: 6 };
        let rule = ink(&[(0, 80, Ink::Glyph, '─')]);
        let mut rows = vec![ink(&[]); 2];
        rows.extend([
            ink(&[(0, 50, Ink::Bright, '●')]),
            ink(&[(2, 40, Ink::Gray, '└')]),
            ink(&[]),
            rule.clone(),
            ink(&[(0, 3, Ink::Bright, '❯')]),
            rule,
        ]);
        assert_eq!(input_top(&rows, &region), Some(3));
        let map = scrolled_map(&region, 3);
        assert_eq!(map, vec![None, None, None, Some(5), Some(6), Some(7)]);
        assert_eq!(holes(&map, &region), 3);
        let plain = Region { left: 0, width: 80, top: 0, height: 3 };
        assert_eq!(input_top(&[ink(&[]), ink(&[]), ink(&[])], &plain), None);
    }
    #[test]
    fn gray_is_dim_or_dark_neutral() {
        let normal = [211, 218, 228];
        assert!(is_gray([153, 153, 153], false, normal));
        assert!(is_gray([255, 255, 255], true, normal));
        assert!(!is_gray([244, 247, 251], false, normal));
        assert!(!is_gray([243, 129, 114], false, normal));
    }
}
