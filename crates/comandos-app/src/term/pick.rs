//! Mouse selection inside one tmux pane, done by the app: the highlight is
//! painted at once (no round trip through tmux copy mode). Ends are tmux
//! lines (0 = top of the live screen, negative = history), so a selection
//! keeps its text while tmux scrolls under it.
use super::engine::{RowRender, RunKind};
use super::noise::Region;

/// A point in the text: (tmux line, column).
pub type Spot = (i64, u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pick {
    pub region: Region,
    pub anchor: Spot,
    pub head: Spot,
    /// Pointer cell; the row may lie above or below the pane while dragging.
    pub pointer: (u16, i32),
    /// Moved past the press cell, or a word/line pick: not a plain click.
    pub dragged: bool,
    /// The button is still down.
    pub held: bool,
    /// Modifiers at press, for replaying a plain click to tmux.
    pub mods: (bool, bool, bool),
}

impl Pick {
    pub fn new(region: Region, at: Spot, pointer: (u16, i32), mods: (bool, bool, bool)) -> Pick {
        Pick { region, anchor: at, head: at, pointer, dragged: false, held: true, mods }
    }
    pub fn right(&self) -> u16 {
        self.region.left.saturating_add(self.region.width).saturating_sub(1)
    }
    /// Start and end, in text order.
    pub fn ordered(&self) -> (Spot, Spot) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }
    /// Where the pointer is in the text, given the tmux line of each pane row.
    pub fn spot_at_pointer(&self, lines: &[Option<i64>]) -> Option<Spot> {
        let (col, row) = self.pointer;
        let top = i32::from(self.region.top);
        let rel = row - top;
        if rel < 0 {
            return lines.iter().flatten().next().map(|&l| (l, self.region.left));
        }
        if rel >= lines.len() as i32 {
            return lines.iter().rev().flatten().next().map(|&l| (l, self.right()));
        }
        let col = col.clamp(self.region.left, self.right());
        let rel = rel as usize;
        // A row with no text (a hole) takes the nearest line above it.
        lines.get(..=rel)?.iter().rev().flatten().next().map(|&l| (l, col))
    }
    /// Per pane row, the inclusive column span to highlight.
    pub fn spans(&self, lines: &[Option<i64>]) -> Vec<(u16, u16, u16)> {
        let (a, b) = self.ordered();
        lines
            .iter()
            .enumerate()
            .filter_map(|(i, line)| {
                let line = (*line)?;
                if line < a.0 || line > b.0 {
                    return None;
                }
                let from = if line == a.0 { a.1 } else { self.region.left };
                let to = if line == b.0 { b.1 } else { self.right() };
                let row = self.region.top.saturating_add(u16::try_from(i).ok()?);
                Some((row, from, to))
            })
            .collect()
    }
    /// The pointer is above (-1) or below (1) the pane: scroll tmux that way.
    pub fn edge(&self) -> i32 {
        let (_, row) = self.pointer;
        if row < i32::from(self.region.top) {
            -1
        } else if row >= i32::from(self.region.top) + i32::from(self.region.height) {
            1
        } else {
            0
        }
    }
}

/// Text of each cell in `[left, left + width)`: a space for empty cells and
/// an empty string for the second half of a wide character.
pub fn cells_of(row: &RowRender, left: u16, width: u16) -> Vec<String> {
    let mut out = vec![String::from(" "); usize::from(width)];
    let mut put = |col: u16, text: &str| {
        if col >= left
            && let Some(slot) = out.get_mut(usize::from(col - left))
        {
            *slot = text.to_owned();
        }
    };
    for run in &row.runs {
        if run.style.hidden {
            continue;
        }
        match run.kind {
            RunKind::Wide => {
                put(run.col, &run.text);
                put(run.col.saturating_add(1), "");
            }
            RunKind::Box(_) => {
                let chars: Vec<char> = run.text.chars().collect();
                for i in 0..run.cells {
                    let ch = chars.get(usize::from(i)).or(chars.last()).copied().unwrap_or(' ');
                    put(run.col.saturating_add(i), ch.encode_utf8(&mut [0; 4]));
                }
            }
            RunKind::Text => {
                for (col, text) in run.cell_texts() {
                    put(col, text);
                }
            }
        }
    }
    out
}

/// A number for the text a row shows in `[left, left + width)`: 0 for an
/// empty row, equal for equal text whatever its colours. It tells a screen
/// row from the same line read back from tmux without building strings.
pub fn row_mark(row: &RowRender, left: u16, width: u16) -> u64 {
    let right = left.saturating_add(width);
    let mut sum = 0u64;
    let mut add = |col: u16, text: &str| {
        if col < left || col >= right || text.trim().is_empty() {
            return;
        }
        let mut h = (0xcbf2_9ce4_8422_2325 ^ u64::from(col - left)).wrapping_mul(0x0100_0000_01b3);
        for b in text.bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
        sum = sum.wrapping_add(h | 1);
    };
    for run in &row.runs {
        if run.style.hidden {
            continue;
        }
        match run.kind {
            RunKind::Wide => add(run.col, &run.text),
            RunKind::Box(_) => {
                let mut last = ' ';
                let mut chars = run.text.chars();
                for i in 0..run.cells {
                    last = chars.next().unwrap_or(last);
                    add(run.col.saturating_add(i), last.encode_utf8(&mut [0; 4]));
                }
            }
            RunKind::Text => {
                for (col, text) in run.cell_texts() {
                    add(col, text);
                }
            }
        }
    }
    sum
}

fn word_char(cell: &str) -> bool {
    !cell.is_empty()
        && !cell.chars().all(char::is_whitespace)
        && !matches!(cell, "│" | "|" | "(" | ")" | "[" | "]" | "{" | "}" | "<" | ">" | "\"" | "'" | "`" | "," | ";")
}

/// Columns (relative to the cells) of the word around `at`.
pub fn word_bounds(cells: &[String], at: usize) -> (usize, usize) {
    let is_word = |i: usize| cells.get(i).is_some_and(|c| word_char(c) || c.is_empty());
    if !is_word(at) {
        return (at, at);
    }
    let mut from = at;
    while from > 0 && is_word(from - 1) {
        from -= 1;
    }
    let mut to = at;
    while is_word(to + 1) {
        to += 1;
    }
    (from, to)
}

/// One row's selected text, without trailing blanks.
pub fn span_text(cells: &[String], from: usize, to: usize) -> String {
    let mut text: String = cells
        .iter()
        .skip(from)
        .take(to.saturating_sub(from) + 1)
        .map(String::as_str)
        .collect();
    text.truncate(text.trim_end().len());
    text
}

/// tmux's copy-mode position `[12/3400]` at the right end of a pane's top
/// row: how many lines the pane is scrolled back.
pub fn copy_position(top_row: &[String]) -> Option<u32> {
    let text: String = top_row.concat();
    let text = text.trim_end();
    let inner = text.strip_suffix(']')?;
    let open = inner.rfind('[')?;
    let (pos, size) = inner.get(open + 1..)?.split_once('/')?;
    size.parse::<u64>().ok()?;
    pos.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn region() -> Region {
        Region { left: 10, width: 20, top: 2, height: 4 }
    }
    #[test]
    fn spans_follow_text_lines_not_rows() {
        // Rows show lines 5, (hole), 7, 8.
        let lines = [Some(5), None, Some(7), Some(8)];
        let mut pick = Pick::new(region(), (5, 15), (15, 2), (false, false, false));
        pick.head = (8, 12);
        assert_eq!(pick.spans(&lines), vec![(2, 15, 29), (4, 10, 29), (5, 10, 12)]);
        // After tmux scrolls by one the same text sits one row lower.
        let lines = [Some(4), Some(5), Some(6), Some(7)];
        assert_eq!(pick.spans(&lines)[0], (3, 15, 29));
    }
    #[test]
    fn pointer_outside_the_pane_reaches_the_edges() {
        let lines = [Some(-3), Some(-2), Some(-1), Some(0)];
        let mut pick = Pick::new(region(), (0, 12), (12, 5), (false, false, false));
        pick.pointer = (40, -4);
        assert_eq!(pick.spot_at_pointer(&lines), Some((-3, 10)));
        assert_eq!(pick.edge(), -1);
        pick.pointer = (40, 9);
        assert_eq!(pick.spot_at_pointer(&lines), Some((0, 29)));
        assert_eq!(pick.edge(), 1);
    }
    #[test]
    fn words_text_and_copy_position() {
        let cells: Vec<String> = "ls  /tmp/a.txt  │x".chars().map(String::from).collect();
        assert_eq!(word_bounds(&cells, 6), (4, 13));
        assert_eq!(span_text(&cells, 4, 15), "/tmp/a.txt");
        let top: Vec<String> = "● hola          [25/2096]  ".chars().map(String::from).collect();
        assert_eq!(copy_position(&top), Some(25));
        let plain: Vec<String> = "● hola".chars().map(String::from).collect();
        assert_eq!(copy_position(&plain), None);
    }
}
