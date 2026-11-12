#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
#[path = "support/t18_native.rs"]
mod native;
#[test]
fn actual_action_lookup_preserves_root_selection_and_rejects_cancelled_replaced_instances() {
    let app = include_str!("../src/ui/app_t18.rs");
    let methods = native::body(app, "fn action_session(")
        + &native::body(app, "fn term_for_action(")
        + &native::body(app, "fn owns_term_instance(")
        + &native::body(include_str!("../src/ui/app_t16.rs"), "fn instance(")
        + &native::body(
            include_str!("../src/ui/app_t13.rs"),
            "fn with_current_term(",
        );
    let source = include_str!("support/t18_instance.rs.txt").replace("// ACTUAL_METHODS", &methods);
    assert_eq!(
        native::execute_source(&source, &serde_json::json!({})),
        serde_json::json!({"cases":8})
    );
}
#[test]
fn actual_mosaic_fill_close_cancels_stale_replay_and_twenty_cycles_release_clients() {
    let app = include_str!("../src/ui/app_t18_views.rs");
    let methods = native::body(app, "fn mosaic_fill(") + &native::body(app, "fn mosaic_close(");
    let source = include_str!("support/t18_fill.rs.txt").replace("// ACTUAL_METHODS", &methods);
    assert_eq!(
        native::execute_source(&source, &serde_json::json!({})),
        serde_json::json!({"cycles":20,"clients_after":0,"sources_after":0,"replay_calls":0})
    );
}
#[test]
fn actual_tabs_layout_keeps_arrows_paints_accent_and_persist_false_does_not_post() {
    let methods = native::body(
        include_str!("../src/ui/app_foundation.rs"),
        "fn apply_tabs_layout(",
    );
    let source = include_str!("support/t18_rows.rs.txt").replace("// ACTUAL_METHODS", &methods);
    assert_eq!(
        native::execute_source(&source, &serde_json::json!({})),
        serde_json::json!({"posts":1,"cases":5})
    );
}
#[test]
fn actual_reader_bridge_rebinds_edition_and_drops_callbacks_and_pages_on_close() {
    let source = include_str!("../src/ui/app_t18_views.rs");
    let methods = native::body(source, "fn reader_action(")
        + &native::body(source, "fn reader_close(")
        + &native::body(source, "fn reader_apply(")
        + &native::body(source, "fn bind_aux_bridge(");
    let fixture = include_str!("support/t18_reader.rs.txt").replace("// ACTUAL_METHODS", &methods);
    assert_eq!(
        native::execute_source(&fixture, &serde_json::json!({})),
        serde_json::json!({"cycles":20,"pages_after":0,"handlers_after":0,"old_replay_actions":0})
    );
}
#[test]
fn actual_window_drop_cancels_requests_disconnects_handlers_and_destroys_pages() {
    let app = include_str!("../src/ui/app_t18.rs");
    let methods = native::body(app, "pub(super) struct WindowView {")
        + &native::body(app, "impl Drop for WindowView {");
    let source = include_str!("support/t18_window.rs.txt").replace("// ACTUAL_METHODS", &methods);
    assert_eq!(
        native::execute_source(&source, &serde_json::json!({})),
        serde_json::json!({"cycles":20,"pages_after":0,"handlers_after":0,"close_calls":20})
    );
}
