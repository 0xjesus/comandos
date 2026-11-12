use comandos_web::components::work_marks as m;
use comandos_web_view::work_marks as view;
use serde_json::json;
#[test]
fn human_mark_wins_and_finished_turns_stay_neutral() {
    assert_eq!(
        m::display("frozen", "working", false),
        json!({"icon":"frozen","label":"Congelado","animated":true,"mark":"frozen"})
    );
    for state in ["completed", "cancelled", "failed", ""] {
        assert_eq!(
            m::display("none", state, false).get("icon"),
            Some(&json!("none"))
        );
    }
    assert_eq!(
        m::channels("frozen", "working"),
        json!({"ai":"work","sticker":"Aparcado","mark":"frozen","suggest":false})
    );
    assert_eq!(
        m::channels("none", "completed").get("suggest"),
        Some(&json!(true))
    );
}
#[test]
fn ambiguous_pane_bindings_never_guess() {
    let panes = json!([{"session":"s","paneId":"%1","paneKey":"a"},{"session":"s","paneId":"%1","paneKey":"b"}]);
    assert_eq!(
        m::target_for_row("s|%1", &panes).get("scope"),
        Some(&json!("session"))
    );
    assert!(m::target_for_row("", &panes).is_null());
}
#[test]
fn session_activity_uses_most_urgent_pane() {
    assert_eq!(
        m::activity_for(
            &json!({"scope":"session","key":"s"}),
            &json!({"a":{"session":"s","state":"completed"},"b":{"session":"s","state":"awaiting_input"}})
        ),
        json!("awaiting_input")
    );
}
#[test]
fn menu_scope_favorite_is_independent_and_keyboard_wraps() {
    let items = view::menu_items(
        "session",
        &json!({"mark":"frozen","favorite":true}),
        false,
        false,
    );
    assert_eq!(
        items.get(4).and_then(|v| v.get("value")),
        Some(&json!(true))
    );
    assert_eq!(m::next_index("ArrowDown", 4, 5), 0);
    assert_eq!(m::next_index("ArrowUp", 0, 5), 4);
    let dom = view::menu(&items);
    assert_eq!(dom.matches("menuitemradio").count(), 4);
    assert!(dom.contains("menuitemcheckbox"));
    assert!(dom.contains("Congelado"));
}
