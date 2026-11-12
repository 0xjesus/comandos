#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
#[allow(dead_code)]
#[path = "support/t18_native.rs"]
mod native;

#[test]
fn every_web_modal_has_explicit_close_and_survives_focus_changes() {
    let source = include_str!("../src/ui/app_t18_views.rs");
    let modal = native::body(source, "fn open_web_modal(");
    let header = &modal
        [modal.find("let container =").unwrap()..modal.find("container.pack_start(&view").unwrap()];
    let bind = modal
        .lines()
        .find(|line| line.contains("self.bind_window_close("))
        .unwrap();
    let probe = include_str!("support/modal_controls.rs.txt")
        .replace("// HEADER", header)
        .replace("// BIND", bind)
        .replace("// METHOD", &native::body(source, "fn bind_window_close("));
    let receipt = native::execute_source(&probe, &serde_json::json!({}));
    assert_eq!(receipt["buttons"], serde_json::json!([1]));
    assert_eq!(receipt["focus_closes"], 0);
    assert_eq!(receipt["button_closes"], serde_json::json!(["link"]));
    assert_eq!(receipt["escape_closes"], 1);
}
