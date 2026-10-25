//! One pane of a desktop split, shown alone on a phone. The phone's tmux client
//! attaches with `ignore-size` at the window's own size, so tmux draws the whole
//! window exactly as the desktop sees it; the page then shows only the chosen
//! pane's cells, scaled to the phone width. Nothing here talks to tmux: the pane
//! geometry is the `/terminal-panes` list the pane chrome already polls
//! (`comandos-term-web` applies it in the browser).
use serde_json::Value;

/// Smallest font a cropped pane is drawn with (CSS px): wider panes overflow
/// the phone instead of becoming unreadable.
pub const MIN_FONT: f64 = 6.0;
/// Phones: the parent page's own breakpoint for the pane chips.
pub const PHONE_WIDTH: f64 = 747.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub pane: String,
    /// Client grid: the whole window plus the status line under it.
    pub cols: u16,
    pub rows: u16,
    pub left: u16,
    pub top: u16,
    pub width: u16,
    pub height: u16,
}

fn field(p: &Value, key: &str) -> Option<u16> {
    p.get(key)?.as_u64().and_then(|n| u16::try_from(n).ok())
}

fn rect(p: &Value) -> Option<(u16, u16, u16, u16)> {
    Some((field(p, "left")?, field(p, "top")?, field(p, "width")?, field(p, "height")?))
}

/// The window the panes tile: its right and bottom edges.
pub fn window_size(panes: &[Value]) -> Option<(u16, u16)> {
    let mut size = (0u16, 0u16);
    for p in panes {
        let (left, top, width, height) = rect(p)?;
        size.0 = size.0.max(left.saturating_add(width));
        size.1 = size.1.max(top.saturating_add(height));
    }
    (size.0 > 0 && size.1 > 0).then_some(size)
}

/// The pane to show: the one the phone asked for, else tmux's active one, else the first.
pub fn chosen<'a>(panes: &'a [Value], wanted: Option<&str>) -> Option<&'a Value> {
    fn id(p: &Value) -> Option<&str> {
        p.get("id").and_then(Value::as_str)
    }
    panes
        .iter()
        .find(|p| wanted.is_some() && id(p) == wanted)
        .or_else(|| panes.iter().find(|p| p.get("active") == Some(&Value::Bool(true))))
        .or_else(|| panes.first())
}

/// What a phone shows: `None` keeps today's whole-terminal view (one pane, a wide
/// screen, or geometry missing from the listing).
pub fn plan(panes: &[Value], wanted: Option<&str>, phone: bool) -> Option<Plan> {
    if !phone || panes.len() < 2 {
        return None;
    }
    let (cols, rows) = window_size(panes)?;
    let pane = chosen(panes, wanted)?;
    let (left, top, width, height) = rect(pane)?;
    Some(Plan {
        pane: pane.get("id")?.as_str()?.to_owned(),
        cols,
        // The status line sits below the window (`status on`, `status-position bottom`).
        rows: rows.checked_add(1)?,
        left,
        top,
        width: width.max(1),
        height: height.max(1),
    })
}

/// Font size (CSS px) that fits the pane in `host` (CSS px), given the cell size one
/// font pixel buys. Never larger than the person's own font, never below
/// [`MIN_FONT`]; quarter pixels so tiny host changes do not re-measure.
pub fn font_size(base: f64, per_px: (f64, f64), host: (f64, f64), pane: (u16, u16)) -> f64 {
    let (pw, ph) = (f64::from(pane.0.max(1)), f64::from(pane.1.max(1)));
    if !(per_px.0 > 0. && per_px.1 > 0. && host.0 > 0. && host.1 > 0.) {
        return base;
    }
    let fit = (host.0 / (pw * per_px.0)).min(host.1 / (ph * per_px.1));
    ((fit.min(base) * 4.).floor() / 4.).max(MIN_FONT)
}

/// CSS that moves the window's cells so the pane sits at the host's top-left corner,
/// and hides every cell outside it: `(transform, clip-path)` for `.xterm-screen`.
pub fn view_css(plan: &Plan, cell: (f64, f64)) -> (String, String) {
    let (cw, ch) = cell;
    // `{}` prints 390.0 as `390`: plain CSS numbers.
    let px = |n: f64| format!("{}px", n + 0.0);
    let x = f64::from(plan.left) * cw;
    let y = f64::from(plan.top) * ch;
    let right = f64::from(plan.cols.saturating_sub(plan.left.saturating_add(plan.width))) * cw;
    let bottom = f64::from(plan.rows.saturating_sub(plan.top.saturating_add(plan.height))) * ch;
    (
        format!("translate({}, {})", px(-x), px(-y)),
        format!("inset({} {} {} {})", px(y), px(right), px(bottom), px(x)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn split() -> Vec<Value> {
        vec![
            json!({"id":"%1","active":true,"left":0,"top":1,"width":77,"height":39}),
            json!({"id":"%4","active":false,"left":78,"top":1,"width":78,"height":39}),
        ]
    }

    #[test]
    fn a_phone_with_a_split_shows_the_chosen_pane_over_the_whole_window() {
        let p = plan(&split(), Some("%4"), true).unwrap();
        assert_eq!(
            p,
            Plan { pane: "%4".into(), cols: 156, rows: 41, left: 78, top: 1, width: 78, height: 39 }
        );
        assert_eq!(plan(&split(), None, true).unwrap().pane, "%1", "tmux's active pane by default");
        assert_eq!(plan(&split(), Some("%9"), true).unwrap().pane, "%1", "a closed pane falls back");
    }

    #[test]
    fn single_panes_wide_screens_and_missing_geometry_keep_the_whole_terminal() {
        assert_eq!(plan(&split()[..1], None, true), None);
        assert_eq!(plan(&split(), Some("%4"), false), None);
        let bare = vec![json!({"id":"%1"}), json!({"id":"%2"})];
        assert_eq!(plan(&bare, None, true), None);
    }

    #[test]
    fn stacked_panes_take_the_full_window_width() {
        let panes = vec![
            json!({"id":"%1","left":0,"top":1,"width":156,"height":19}),
            json!({"id":"%2","left":0,"top":21,"width":156,"height":19}),
        ];
        let p = plan(&panes, Some("%2"), true).unwrap();
        assert_eq!((p.cols, p.rows, p.top, p.width), (156, 41, 21, 156));
    }

    #[test]
    fn the_font_fits_the_pane_width_and_height_within_limits() {
        // 0.6 px of cell width and 1.2 px of row per font pixel.
        let per = (0.6, 1.2);
        // 78 columns in 390 px: 5 px cells, a little over 8 px of font.
        assert_eq!(font_size(14., per, (390., 700.), (78, 39)), 8.25);
        // A short host limits by height.
        assert_eq!(font_size(14., per, (390., 300.), (78, 39)), 6.25);
        // Narrow panes never grow past the person's font.
        assert_eq!(font_size(14., per, (390., 700.), (20, 10)), 14.);
        // A full-width stacked pane stops at the minimum and overflows.
        assert_eq!(font_size(14., per, (390., 700.), (238, 19)), MIN_FONT);
        assert_eq!(font_size(14., (0., 0.), (390., 700.), (78, 39)), 14.);
    }

    #[test]
    fn the_view_shifts_the_pane_to_the_corner_and_clips_the_rest() {
        let p = plan(&split(), Some("%4"), true).unwrap();
        let (transform, clip) = view_css(&p, (5., 10.));
        assert_eq!(transform, "translate(-390px, -10px)");
        assert_eq!(clip, "inset(10px 0px 10px 390px)");
    }
}
