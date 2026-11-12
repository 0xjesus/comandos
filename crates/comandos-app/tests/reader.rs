#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use comandos_app::ui::reader::reader_layout_state;
#[path = "support/t18_oracle.rs"]
mod oracle;
#[test]
fn reader_with_terminal_keeps_both_regions() {
    for (open, terminal, terms) in [
        (false, false, true),
        (false, true, true),
        (true, false, false),
        (true, true, true),
    ] {
        assert_eq!(
            reader_layout_state(open, terminal),
            serde_json::json!({"terminals":terms,"reader":open})
        );
    }
}
#[test]
fn owned_modals_keep_original_sizes_and_must_answer_close_policy() {
    use comandos_app::ui::modals::{ModalKind, close_allowed, modal_size};
    for (width, height) in [
        (300, 300),
        (760, 570),
        (1280, 720),
        (1280, 1200),
        (4000, 3000),
    ] {
        let (w, h) = modal_size(ModalKind::Url, width, height);
        assert_eq!(
            serde_json::json!([w, h]),
            oracle::original(
                serde_json::json!({"op":"modal-size","kind":"url","width":width,"height":height})
            )
        );
    }
    for dismissable in [false, true] {
        for (escape, backdrop) in [(false, true), (true, false), (false, false)] {
            let original = oracle::original(
                serde_json::json!({"op":"modal-close","dismissable":dismissable,"escape":escape,"backdrop":backdrop}),
            );
            assert_eq!(
                original["closed"],
                close_allowed(dismissable, escape, backdrop)
            );
            if backdrop {
                assert_eq!(original["consumed"], true);
            }
        }
    }
}

#[test]
fn reader_bridge_accepts_only_its_exact_owned_local_page() {
    use comandos_app::ui::reader::{message_owned, reader_uri};
    let base = "http://127.0.0.1:4777";
    let news = reader_uri(base, "2026-10-08@10:00", "test").unwrap();
    let chains = format!("{base}/?panel=chains&v=test&session=fixture&pane=%251");
    assert!(message_owned(&news, &news, base, "reader"));
    assert!(!message_owned(&chains, &chains, base, "chains"));
    assert!(!message_owned(&chains, &chains, base, "reader"));
    assert!(!message_owned(
        &format!("{news}#changed"),
        &news,
        base,
        "reader"
    ));
    assert!(!message_owned(
        &news,
        &news,
        "https://example.com",
        "reader"
    ));
    assert!(!message_owned(&news, &news, base, "arbitrary"));
}
