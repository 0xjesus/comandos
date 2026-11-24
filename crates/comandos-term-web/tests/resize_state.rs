//! The resize request writes its size as the original page's `JSON.stringify` did.
#[path = "../src/pane_chrome/resize.rs"]
mod resize;
use resize::ResizeRequest;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test;
fn size_text(size: f64) -> String {
    let body = ResizeRequest::new("%1".into(), true, size).body("private-session");
    body["size"].to_string()
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_dragged_size_goes_out_as_a_whole_number_of_cells() {
    // `/terminal-panes` refuses `40.0`: the drag then changed nothing.
    for (size, text) in [
        (40., "40"),
        (2., "2"),
        (0., "0"),
        (-0., "0"),
        (-2., "-2"),
        (2.5, "2.5"),
        (f64::INFINITY, "null"),
        (f64::NEG_INFINITY, "null"),
        (f64::NAN, "null"),
    ] {
        assert_eq!(size_text(size), text, "{size}");
    }
    let body = ResizeRequest::new("%7".into(), false, 12.).body("s");
    assert_eq!(
        body.to_string(),
        r#"{"session":"s","action":"resize","pane":"%7","axis":"y","size":12}"#
    );
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn the_queue_skips_a_request_equal_to_the_one_sent() {
    let mut cases = Vec::new();
    for pane in ["%1", "%2"] {
        for vertical in [false, true] {
            for size in [0., -0., 2., 2.5, 40., f64::NAN, f64::INFINITY] {
                let request = ResizeRequest::new(pane.into(), vertical, size);
                assert!(request == request.clone());
                let payload = request.body("s").to_string();
                cases.push((request, payload));
            }
        }
    }
    for (a, payload_a) in &cases {
        for (b, payload_b) in &cases {
            assert_eq!(a == b, payload_a == payload_b);
        }
    }
}
