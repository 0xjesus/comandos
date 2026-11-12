//! Left panel state and geometry retain the Python contract.
use serde_json::{Value, json};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutError(pub String);
impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for LayoutError {}
/// The left panel's persisted visibility and width; other keys pass through.
#[derive(Debug, Clone, PartialEq)]
pub struct SideState {
    pub left_hidden: bool,
    pub left_pos: i32,
    persisted: Value,
}
impl Default for SideState {
    fn default() -> Self {
        Self {
            left_hidden: false,
            left_pos: 620,
            persisted: json!({}),
        }
    }
}
impl SideState {
    pub fn from_json(value: &Value) -> Result<Self, LayoutError> {
        if !value.is_null() && !value.is_object() {
            return Err(LayoutError("layout must be an object".into()));
        }
        let mut state = Self {
            persisted: if value.is_object() {
                value.clone()
            } else {
                json!({})
            },
            ..Self::default()
        };
        state.left_hidden = value
            .get("leftHidden")
            .is_some_and(comandos_core::json::truthy);
        if let Some(pos) = value
            .get("leftPos")
            .filter(|v| comandos_core::json::truthy(v))
        {
            state.left_pos =
                super::app_commands::python_int(pos).map_err(|e| LayoutError(e.to_string()))?;
        }
        Ok(state)
    }
    pub fn to_json(&self) -> Value {
        self.persisted.clone()
    }
    pub fn set_left(&mut self, hidden: bool, position: i32) {
        if hidden && position >= 200 {
            self.left_pos = position;
            if let Some(map) = self.persisted.as_object_mut() {
                map.insert("leftPos".into(), json!(position));
            }
        }
        self.left_hidden = hidden;
        if let Some(map) = self.persisted.as_object_mut() {
            map.insert("leftHidden".into(), json!(hidden));
        }
    }
}

/// The immutable desktop initialization accepts Python int conversions and round ties-even.
pub fn pane_initial_position(width: i32, document: &Value) -> Option<i32> {
    if width < 900 {
        return None;
    }
    let saved = document
        .get("position")
        .filter(|v| comandos_core::json::truthy(v))
        .and_then(|v| super::app_commands::python_int(v).ok())
        .unwrap_or(0);
    let position = if saved != 0 {
        saved
    } else {
        (f64::from(width) * 0.27).round_ties_even() as i32
    };
    Some(position.clamp(300, (width - 720).max(320)))
}

#[derive(Debug, Clone, PartialEq)]
pub struct LeftFrame {
    pub offset: f64,
    pub mask_start: Option<f64>,
    pub alpha: f64,
    pub edge: f64,
    pub shadow_alpha: Option<f64>,
}
pub fn left_frame(hiding: bool, progress: f64, x: f64, width: f64) -> LeftFrame {
    let k = if hiding { progress } else { 1.0 - progress };
    let offset = -width * k;
    let edge = x + width + offset;
    LeftFrame {
        offset,
        mask_start: (!hiding).then(|| x.max(edge)),
        alpha: 1.0 - 0.45 * k,
        edge,
        shadow_alpha: (k > 0.0 && k < 1.0 && edge > x).then_some(0.28 * (1.0 - k)),
    }
}

impl LeftFrame {
    pub fn to_json(&self) -> Value {
        json!({"offset":self.offset,"mask_start":self.mask_start,"alpha":self.alpha,"edge":self.edge,"shadow_alpha":self.shadow_alpha})
    }
}

/// A GTK frame clock starts a transition on its first frame, as the original _tween.
pub struct Tween {
    start: std::cell::Cell<Option<i64>>,
    from: f64,
    to: f64,
    millis: u64,
}
impl Tween {
    pub fn new(from: f64, to: f64, millis: u64) -> Self {
        Self {
            start: std::cell::Cell::new(None),
            from,
            to,
            millis,
        }
    }
    pub fn step(&self, now: i64) -> (f64, bool) {
        let start = self.start.get().unwrap_or(now);
        self.start.set(Some(start));
        let t = ((now.saturating_sub(start)) as f64 / (self.millis.max(1) as f64 * 1000.)).min(1.);
        let ease = 1. - (1. - t).powf(3.);
        (self.from + (self.to - self.from) * ease, t >= 1.)
    }
}
