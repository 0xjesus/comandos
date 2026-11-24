//! Pane frames, resize gutters and account controls from the terminal tail.
//! Geometry is independent of the renderer; the DOM adapter uses live WebTerm cells.
#[cfg(target_arch = "wasm32")]
mod resize;
use serde_json::Value;

pub fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .filter(|v| !v.is_null())
        .map(comandos_web_dom::api::js_string_of)
        .unwrap_or_default()
}
pub fn number(v: &Value, key: &str) -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    let number = v.get(key).and_then(Value::as_f64);
    #[cfg(target_arch = "wasm32")]
    let number = v
        .get(key)
        .and_then(Value::as_number)
        .and_then(|value| crate::number_text::parse(&value.to_string()))
        .filter(|value| value.is_finite());
    number.unwrap_or(0.0)
}
pub fn escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&#39;".into(),
            _ => c.to_string(),
        })
        .collect()
}
#[derive(Clone, Debug, PartialEq)]
pub struct Gutter {
    pub vertical: bool,
    pub pane: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
#[derive(Clone, Copy)]
pub struct Geometry {
    pub ox: f64,
    pub oy: f64,
    pub cw: f64,
    pub ch: f64,
    pub cols: f64,
    pub rows: f64,
}
pub fn neighbor<'a>(panes: &'a [Value], g: &Gutter) -> Option<&'a Value> {
    let p = panes.iter().find(|p| text(p, "id") == g.pane)?;
    let mut best = None;
    let mut most = 0.0;
    for o in panes {
        if text(o, "id") == g.pane {
            continue;
        }
        let lap = if g.vertical {
            if number(o, "left") == number(p, "left") + number(p, "width") + 1.0 {
                (number(p, "top") + number(p, "height")).min(number(o, "top") + number(o, "height"))
                    - number(p, "top").max(number(o, "top"))
            } else {
                0.0
            }
        } else if number(o, "top") == number(p, "top") + number(p, "height") + 1.0 {
            (number(p, "left") + number(p, "width")).min(number(o, "left") + number(o, "width"))
                - number(p, "left").max(number(o, "left"))
        } else {
            0.0
        };
        if lap > most {
            most = lap;
            best = Some(o)
        }
    }
    best
}
pub fn hit(g: &Gutter, x: f64, y: f64, slack: f64) -> bool {
    x >= g.x - slack && x <= g.x + g.w + slack && y >= g.y - slack && y <= g.y + g.h + slack
}
pub fn frame(p: &Value, a: Geometry) -> Option<([f64; 6], Vec<Gutter>)> {
    let (left, top, w, h) = (
        number(p, "left"),
        number(p, "top"),
        number(p, "width"),
        number(p, "height"),
    );
    if w < 8.0 || h <= 0.0 || top >= a.rows || left >= a.cols {
        return None;
    }
    let rail = a.oy + if top >= 1.0 { (top - 1.0) * a.ch } else { 0.0 };
    let pad = (a.cw / 2.0 - 2.0).clamp(1.0, 3.0);
    let x0 = a.ox + left * a.cw - if left == 0.0 { 5.0 } else { pad };
    let x1 = (a.ox + a.cols * a.cw + 3.0).min(a.ox + (left + w) * a.cw + pad);
    let y0 = if top <= 1.0 { rail - 17.0 } else { rail + 3.0 };
    let bottom = a.oy + (top + h) * a.ch;
    let at_bottom = top + h >= a.rows;
    let y1 = (a.oy + a.rows * a.ch + 3.0).min(if at_bottom {
        bottom + 3.0
    } else {
        bottom - 2.0
    });
    let mut gs = Vec::new();
    if left + w < a.cols {
        gs.push(Gutter {
            vertical: true,
            pane: text(p, "id"),
            x: a.ox + (left + w) * a.cw,
            y: y0 + 6.0,
            w: a.cw,
            h: (y1 - y0 - 12.0).max(0.0),
        });
    }
    if !at_bottom {
        gs.push(Gutter {
            vertical: false,
            pane: text(p, "id"),
            x: x0 + 10.0,
            y: bottom - 2.0,
            w: (x1 - x0 - 20.0).max(0.0),
            h: 5.0,
        });
    }
    Some(([x0, y0, x1, y1, rail + a.ch, rail], gs))
}
pub fn active(panes: &[Value], local: &Option<String>, remote: &Option<String>) -> Option<String> {
    local
        .iter()
        .chain(remote.iter())
        .find(|id| panes.iter().any(|p| text(p, "id") == **id))
        .cloned()
        .or_else(|| {
            panes
                .iter()
                .find(|p| comandos_web_dom::api::js_truthy(&p["active"]))
                .map(|p| text(p, "id"))
        })
}
pub fn stage(s: &Value, alias: &str) -> String {
    if text(s, "state") == "awaiting_confirmation" {
        return awaiting(alias);
    }
    let code = text(s, "stageCode");
    let state = text(s, "state");
    match if code.is_empty() {
        state.as_str()
    } else {
        code.as_str()
    } {
        "validating" => "Validando…",
        "waiting" => "Interrumpiendo lo que está haciendo…",
        "snapshot" => "Guardando la conversación…",
        "applying" => "Abriendo la conversación en la otra cuenta…",
        "verifying" => "Confirmando que siguió igual…",
        "recovering" => "Recuperando la conversación original…",
        _ => return text(s, "stage"),
    }
    .into()
}
pub fn awaiting(alias: &str) -> String {
    format!(
        "La conversación ya corre en {alias}, pero el pane pide algo (confianza de carpeta, login o esfuerzo): respóndelo en su terminal y el cambio se confirma solo."
    )
}
pub fn terminal_state(s: &Value) -> bool {
    matches!(
        text(s, "state").as_str(),
        "confirmed" | "failed" | "rolled_back" | "recovery_required"
    )
}

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
pub(crate) use browser::fetch as fetch_api;
#[cfg(target_arch = "wasm32")]
pub use browser::{attach, chosen, dispose, unchanged};
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn geometry_matches_stacked_and_vertical_gutters() {
        let p = json!({"id":"%1","left":0,"top":1,"width":20,"height":10});
        let (r, g) = frame(
            &p,
            Geometry {
                ox: 10.,
                oy: 20.,
                cw: 8.,
                ch: 16.,
                cols: 41.,
                rows: 22.,
            },
        )
        .unwrap();
        assert_eq!(r, [5., 3., 172., 194., 36., 20.]);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].x, 170.);
        assert!(hit(&g[0], 164., 20., 8.));
        assert!(hit(&g[0], 157., 20., 14.));
        assert!(!hit(&g[0], 155., 20., 14.));
    }
    #[test]
    fn neighboring_pane_uses_largest_shared_edge_and_focus_is_local_first() {
        let p = vec![
            json!({"id":"%1","left":0,"top":1,"width":20,"height":10,"active":true}),
            json!({"id":"%2","left":21,"top":1,"width":20,"height":3}),
            json!({"id":"%3","left":21,"top":5,"width":20,"height":6}),
        ];
        let g = Gutter {
            vertical: true,
            pane: "%1".into(),
            x: 0.,
            y: 0.,
            w: 0.,
            h: 0.,
        };
        assert_eq!(text(neighbor(&p, &g).unwrap(), "id"), "%3");
        assert_eq!(
            active(&p, &Some("%2".into()), &Some("%3".into())),
            Some("%2".into())
        );
        assert_eq!(
            active(&p, &Some("gone".into()), &Some("%3".into())),
            Some("%3".into())
        );
    }
    #[test]
    fn account_progress_is_not_confirmed_by_network_failure_or_waiting() {
        assert!(!terminal_state(&json!({"state":"awaiting_confirmation"})));
        for s in ["confirmed", "failed", "rolled_back", "recovery_required"] {
            assert!(terminal_state(&json!({"state":s})));
        }
        assert!(stage(&json!({"state":"awaiting_confirmation"}), "work").contains("work"));
        assert_eq!(escape("<&\"'"), "&lt;&amp;&quot;&#39;");
    }
}
