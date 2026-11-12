//! "Losas": each tmux pane is painted as a tile behind its text. tmux draws
//! its borders in colors no program uses (set in config/tmux.conf), so the
//! panes are found on the very screen tmux sends: tiles never lag behind
//! the text and need no tmux query while the window resizes.

/// `pane-border-style`.
pub const LINE: [u8; 3] = [0x2B, 0x35, 0x48];
/// `pane-active-border-style`: normal, copy mode, synchronized panes.
pub const ACTIVE: [[u8; 3]; 3] = [[0x8B, 0x7C, 0xF7], [0xF5, 0xB5, 0x45], [0xF2, 0x5F, 0x5D]];

/// A tmux border color: never painted as text.
pub fn is_border(rgb: [u8; 3]) -> bool {
    rgb == LINE || ACTIVE.contains(&rgb)
}

/// What one cell holds, for finding panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cell {
    #[default]
    Other,
    /// Border `─` (or a blank inside a border row).
    Rule { active: bool },
    /// Border `│` or a junction.
    Post,
}

pub fn classify(ch: char, rgb: [u8; 3]) -> Cell {
    if !is_border(rgb) {
        return Cell::Other;
    }
    match ch {
        '│' | '┃' | '├' | '┤' | '┼' | '┬' | '┴' | '┌' | '┐' | '└' | '┘' => Cell::Post,
        _ => Cell::Rule { active: ACTIVE.contains(&rgb) },
    }
}

/// A pane in cells: its title row is `top`, its text runs to `top + height`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    pub left: u16,
    pub top: u16,
    pub width: u16,
    pub height: u16,
    pub active: bool,
}

/// Panes from a screen of classified cells (`rows × cols`, row-major). Each
/// pane opens with a title row of border cells (`pane-border-status top`);
/// it ends at the next title row below it or at `bottom` (above tmux's
/// status line).
pub fn detect(cells: &[Cell], cols: u16, bottom: u16) -> Vec<Tile> {
    let at = |row: u16, col: u16| {
        cells
            .get(usize::from(row) * usize::from(cols) + usize::from(col))
            .copied()
            .unwrap_or_default()
    };
    let rule = |row: u16, col: u16| matches!(at(row, col), Cell::Rule { .. });
    let mut tiles = Vec::new();
    for row in 0..bottom {
        let mut col = 0;
        while col < cols {
            if !rule(row, col) {
                col += 1;
                continue;
            }
            let start = col;
            let mut active = false;
            let mut end = col;
            // A title row: border cells, with short blanks for the title text.
            while col < cols {
                match at(row, col) {
                    Cell::Rule { active: a } => {
                        active |= a;
                        end = col + 1;
                    }
                    Cell::Post => break,
                    Cell::Other if col - end < 3 => {}
                    Cell::Other => break,
                }
                col += 1;
            }
            let width = end - start;
            if width < 4 {
                continue;
            }
            let mut last = row + 1;
            while last < bottom && !rule(last, start) && at(last, start) != Cell::Post {
                last += 1;
            }
            if last > row + 1 {
                tiles.push(Tile { left: start, top: row, width, height: last - row, active });
            }
        }
    }
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;
    /// `-` rule, `=` active rule, `|` post, anything else plain text.
    fn screen(lines: &[&str]) -> (Vec<Cell>, u16) {
        let cols = lines[0].chars().count() as u16;
        let cells = lines
            .iter()
            .flat_map(|l| {
                l.chars().map(|c| match c {
                    '-' => Cell::Rule { active: false },
                    '=' => Cell::Rule { active: true },
                    '|' => Cell::Post,
                    _ => Cell::Other,
                })
            })
            .collect();
        (cells, cols)
    }
    #[test]
    fn finds_a_left_pane_and_two_stacked_on_the_right() {
        let (cells, cols) = screen(&[
            "== ======|--- -----",
            "hola     |text     ",
            "─────    |more     ",
            "abc      |-- ------",
            "         |x        ",
            "status line here   ",
        ]);
        let tiles = detect(&cells, cols, 5);
        assert_eq!(
            tiles,
            vec![
                Tile { left: 0, top: 0, width: 9, height: 5, active: true },
                Tile { left: 10, top: 0, width: 9, height: 3, active: false },
                Tile { left: 10, top: 3, width: 9, height: 2, active: false },
            ]
        );
    }
    #[test]
    fn program_rules_are_not_borders() {
        assert_eq!(classify('─', [0x88, 0x88, 0x88]), Cell::Other);
        assert_eq!(classify('─', LINE), Cell::Rule { active: false });
        assert_eq!(classify('┬', ACTIVE[0]), Cell::Post);
    }
}
