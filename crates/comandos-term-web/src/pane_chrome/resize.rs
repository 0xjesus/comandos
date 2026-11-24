//! The pane resize queue only compares its three protocol fields.
use serde_json::{Number, Value, json};
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ResizeRequest {
    pane: String,
    axis: &'static str,
    // The size as JSON.stringify writes it; nonfinite geometry becomes null.
    size: Option<Number>,
}
impl ResizeRequest {
    pub(crate) fn new(pane: String, vertical: bool, size: f64) -> Self {
        Self {
            pane,
            axis: if vertical { "x" } else { "y" },
            size: cells(size),
        }
    }
    pub(crate) fn body(&self, session: &str) -> Value {
        let mut body =
            json!({"session":session,"action":"resize","pane":self.pane,"axis":self.axis});
        // Insert the existing number directly: re-serializing a Number through
        // ValueSerializer would retain its generic RawValue deserializer.
        body["size"] = self.size.clone().map(Value::Number).unwrap_or(Value::Null);
        body
    }
}
/// A whole number of cells goes out as `40`, never `40.0`: `/terminal-panes`
/// takes only integers, so a drag that wrote `40.0` changed nothing. Same text as
/// the original page's `JSON.stringify` (-0 is `0`).
fn cells(size: f64) -> Option<Number> {
    const SAFE: f64 = 9_007_199_254_740_992.0;
    if size.fract() == 0.0 && size.abs() < SAFE {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return Some(if size < 0.0 { Number::from(size as i64) } else { Number::from(size as u64) });
    }
    Number::from_f64(size)
}
