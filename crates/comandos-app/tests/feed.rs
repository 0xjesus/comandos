#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! Ficha viva on the desktop: `/session-feed` → K5 tiles, preview text, answer
//! keys, seen decisions, the web hover contract and the fixed tile geometry.
use comandos_app::feed::{self, FeedState, Marker};
use serde_json::{Value, json};

fn sample() -> Value {
    json!({
        "now": 1794250700.0,
        "sessions": [
            {
                "session": "term-r26549", "label": "FerLongoria", "state": "ask",
                "unread": true, "ts": 1794250552.0, "seen": 1794250000.0,
                "pane": "%51", "panes": 1, "agent": "claude", "model": "opus-5",
                "prompt": "pushea   todo",
                "reply": "Tengo dos commits locales. ¿Los subo?",
                "ask": {
                    "question": "Bash(git push origin main) · Do you want to proceed?",
                    "options": [
                        {"key": "1", "label": "Yes"},
                        {"key": "2", "label": "Yes, and don't ask again"},
                        {"key": "3", "label": "No (esc)"},
                        {"key": "0", "label": "not a menu key"},
                        {"key": "12", "label": "not a digit"}
                    ]
                }
            },
            {"session": "term-work", "state": "work", "unread": false, "ts": 1794250690.0,
             "seen": 0, "pane": "%2", "agent": "codex", "reply": "", "prompt": "", "ask": null},
            {"session": "term-unread", "state": "idle", "unread": true, "ts": 1794250000.0,
             "seen": 1794240000.0, "pane": "%3", "reply": "Listo.", "ask": null},
            {"session": "term-read", "state": "idle", "unread": false, "ts": 1794240000.0,
             "seen": 1794250000.0, "pane": "%4", "reply": "", "ask": null},
            {"session": "term-limit", "state": "limit", "unread": true, "ts": 1794250600.0,
             "seen": 0, "pane": "bogus", "reply": "Usage limit reached", "ask": null},
            {"session": "bad session!", "state": "ask"},
            {"state": "idle"}
        ]
    })
}

#[test]
fn feed_parses_entries_and_drops_invalid_sessions_and_option_keys() {
    let feed = feed::parse_feed(&sample()).unwrap();
    assert_eq!(feed.now, 1794250700.0);
    assert_eq!(
        feed.sessions.keys().collect::<Vec<_>>(),
        [
            "term-limit",
            "term-r26549",
            "term-read",
            "term-unread",
            "term-work"
        ]
    );
    let ask = &feed.sessions["term-r26549"];
    assert_eq!(ask.state, FeedState::Ask);
    let keys = ask
        .ask
        .as_ref()
        .unwrap()
        .options
        .iter()
        .map(|o| o.key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(keys, ["1", "2", "3"]);
    assert_eq!(feed.sessions["term-work"].ask, None);
    // Not a feed: 404 bodies, null and old servers leave the tiles as today.
    assert!(feed::parse_feed(&Value::Null).is_none());
    assert!(feed::parse_feed(&json!({"error": "not found"})).is_none());
    assert!(feed::parse_feed(&json!({"sessions": {}})).is_none());
}

#[test]
fn k5_classes_follow_the_one_violet_rule() {
    let feed = feed::parse_feed(&sample()).unwrap();
    let look = |s: &str, seen: bool| feed::tile_look(Some(&feed), s, seen);
    assert_eq!(look("term-r26549", false).class, Some("k5-ask"));
    assert_eq!(look("term-r26549", false).marker, Marker::Ask);
    assert_eq!(
        look("term-limit", false).class,
        Some("k5-ask"),
        "limit also waits"
    );
    assert_eq!(look("term-work", false).class, Some("k5-work"));
    assert_eq!(look("term-work", false).marker, Marker::Ring);
    assert_eq!(look("term-unread", false).class, Some("k5-unread"));
    assert_eq!(look("term-unread", false).marker, Marker::Dot);
    assert_eq!(look("term-read", false).class, Some("k5-quiet"));
    assert_eq!(look("term-read", false).marker, Marker::None);
    // A tile without an agent is quiet while the feed answers.
    assert_eq!(look("local", false).class, Some("k5-quiet"));
    // The open tab of a focused window is being read: no unread dot there…
    assert_eq!(look("term-unread", true).class, Some("k5-quiet"));
    // …but «te espera» always shows.
    assert_eq!(look("term-r26549", true).class, Some("k5-ask"));
    // Without a feed (404, errors) the tile looks exactly like today.
    assert_eq!(
        feed::tile_look(None, "term-r26549", false),
        feed::TileLook::TODAY
    );
    for class in feed::K5_CLASSES {
        assert!(class.starts_with("k5-"));
    }
}

#[test]
fn preview_verbs_time_and_text() {
    let feed = feed::parse_feed(&sample()).unwrap();
    let s = |k: &str| &feed.sessions[k];
    assert_eq!(feed::verb(s("term-r26549"), false), "pide permiso");
    assert_eq!(feed::verb(s("term-limit"), false), "llegó al límite de uso");
    assert_eq!(feed::verb(s("term-work"), false), "está trabajando");
    assert_eq!(feed::verb(s("term-unread"), false), "respondió");
    assert_eq!(feed::verb(s("term-read"), false), "sin actividad");
    assert_eq!(feed::verb(s("term-unread"), true), "replied");

    assert_eq!(feed::time_ago(1000., 990., false), "ahora");
    assert_eq!(feed::time_ago(1000., 990., true), "now");
    assert_eq!(
        feed::time_ago(1000. + 14. * 60. + 5., 1000., false),
        "14 min"
    );
    assert_eq!(feed::time_ago(1000. + 3. * 3600., 1000., false), "3 h");
    assert_eq!(feed::time_ago(1000. + 2. * 86400., 1000., false), "2 d");
    assert_eq!(feed::time_ago(1000., 0., false), "", "unknown time");
    assert_eq!(feed::time_ago(1000., 2000., false), "ahora", "clock skew");

    assert_eq!(
        feed::quote(s("term-r26549")),
        "Bash(git push origin main) · Do you want to proceed?"
    );
    assert_eq!(feed::quote(s("term-unread")), "Listo.");
    assert_eq!(feed::menu(s("term-unread")).len(), 0);
    assert_eq!(feed::answer_keys(s("term-r26549")), ["1", "2", "3"]);
    assert!(
        feed::answer_keys(s("term-limit")).is_empty(),
        "limit has no menu"
    );
    assert_eq!(feed::one_line(&s("term-r26549").prompt), "pushea todo");
    assert_eq!(feed::initials("Lola - Prospectos"), "LP");
    assert_eq!(feed::initials("FerLongoria"), "FE");
    assert_eq!(feed::initials(""), "");
    assert_eq!(feed::agent_glyph("claude"), "✻");
    assert_eq!(feed::agent_glyph("codex"), "◎");
    assert_eq!(feed::agent_glyph("grok"), "");
}

#[test]
fn only_digits_of_the_visible_menu_are_answered() {
    let feed = feed::parse_feed(&sample()).unwrap();
    let ask = feed.sessions.get("term-r26549");
    assert_eq!(feed::answer_digit(ask, '1').as_deref(), Some("1"));
    assert_eq!(feed::answer_digit(ask, '3').as_deref(), Some("3"));
    assert_eq!(feed::answer_digit(ask, '4'), None, "not in the menu");
    assert_eq!(feed::answer_digit(ask, '0'), None);
    assert_eq!(feed::answer_digit(ask, 'y'), None);
    assert_eq!(
        feed::answer_digit(feed.sessions.get("term-work"), '1'),
        None
    );
    assert_eq!(
        feed::answer_digit(feed.sessions.get("term-limit"), '1'),
        None
    );
    assert_eq!(feed::answer_digit(None, '1'), None);
    // A stale menu on a session that stopped asking is not answered.
    let mut stale = feed.sessions["term-r26549"].clone();
    stale.state = FeedState::Idle;
    assert_eq!(feed::answer_digit(Some(&stale), '1'), None);
}

#[test]
fn answer_mirrors_the_notification_choose() {
    let feed = feed::parse_feed(&sample()).unwrap();
    let [digit, enter] = feed::answer_payloads(&feed.sessions["term-r26549"], "2");
    assert_eq!(
        digit,
        json!({"session": "term-r26549", "key": "2", "pane": "%51"})
    );
    assert_eq!(
        enter,
        json!({"session": "term-r26549", "key": "Enter", "pane": "%51"})
    );
    assert_eq!(
        feed::CHOOSE_ENTER_MS as u128,
        comandos_notifyd::actions::CHOOSE_DELAY.as_millis()
    );
    // An invalid pane is never forwarded; the server picks the session's pane.
    let [digit, _] = feed::answer_payloads(&feed.sessions["term-limit"], "1");
    assert_eq!(digit, json!({"session": "term-limit", "key": "1"}));
    assert!(feed::valid_pane("%0"));
    assert!(!feed::valid_pane("%"));
    assert!(!feed::valid_pane("51"));
    assert!(!feed::valid_pane("%5a"));
}

#[test]
fn seen_is_marked_for_the_open_tab_of_a_focused_window() {
    let feed = feed::parse_feed(&sample()).unwrap();
    assert_eq!(
        feed::seen_on_poll(&feed, Some("term-unread"), true).as_deref(),
        Some("term-unread")
    );
    assert_eq!(
        feed::seen_on_poll(&feed, Some("term-unread"), false),
        None,
        "unfocused"
    );
    assert_eq!(feed::seen_on_poll(&feed, None, true), None);
    assert_eq!(
        feed::seen_on_poll(&feed, Some("local"), true),
        None,
        "not in feed"
    );
    assert_eq!(
        feed::seen_on_poll(&feed, Some("term-read"), true),
        None,
        "nothing new"
    );
    // Working sessions still advance the mark when their ts is newer.
    assert_eq!(
        feed::seen_on_poll(&feed, Some("term-work"), true).as_deref(),
        Some("term-work")
    );

    assert_eq!(
        feed::seen_on_switch(Some("a"), Some("term-x"), true).as_deref(),
        Some("term-x")
    );
    assert_eq!(
        feed::seen_on_switch(Some("term-x"), Some("term-x"), true),
        None,
        "no switch"
    );
    assert_eq!(
        feed::seen_on_switch(Some("a"), Some("term-x"), false),
        None,
        "unfocused"
    );
    assert_eq!(feed::seen_on_switch(Some("a"), None, true), None);
    assert_eq!(feed::seen_on_switch(None, Some("bad key!"), true), None);
}

#[test]
fn preview_opens_under_the_tile_and_never_covers_it() {
    let (w, gap) = (feed::PREVIEW_WIDTH, feed::PREVIEW_GAP);
    // Left-aligned under the tile.
    assert_eq!(
        feed::preview_position((100, 50, 240, 28), (0, 1800), w, gap),
        (100, 84)
    );
    // Right-aligned when it would overflow the window.
    assert_eq!(
        feed::preview_position((1600, 50, 240, 28), (0, 1800), w, gap),
        (1600 + 240 - w, 84)
    );
    // Clamped to the window's left edge.
    assert_eq!(
        feed::preview_position((10, 0, 100, 28), (0, 200), w, gap),
        (0, 34)
    );
    for tile in [(100, 50, 240, 28), (1600, 50, 240, 28), (0, 0, 28, 28)] {
        let (_, y) = feed::preview_position(tile, (0, 1800), w, gap);
        assert!(y >= tile.1 + tile.3, "card starts below the tile");
    }
    assert_eq!(feed::PREVIEW_DELAY_MS, 350);
    assert_eq!(feed::PREVIEW_GRACE_MS, 150);
    assert_eq!(w, 320);
}

#[test]
fn web_hover_contract() {
    assert_eq!(
        feed::feed_hover_message(r#"{"feedHover":"term-r26549"}"#).as_deref(),
        Some("term-r26549")
    );
    assert_eq!(
        feed::feed_hover_message(r#"{"feedHover":""}"#).as_deref(),
        Some("")
    );
    assert_eq!(
        feed::feed_hover_message(r#"{"feedHover":null}"#).as_deref(),
        Some("")
    );
    assert_eq!(
        feed::feed_hover_message(r#"{"feedHover":"x';alert(1)"}"#),
        None
    );
    assert_eq!(feed::feed_hover_message(r#"{"session":"term-a"}"#), None);
    assert_eq!(feed::feed_hover_message("not json"), None);
    let script = feed::hover_script("term-r26549");
    assert!(
        script.contains(r#"window.ComandosFeed.hover("term-r26549")"#),
        "{script}"
    );
    assert!(script.starts_with("try{window.ComandosFeed&&"));
    // Arguments are serialized, never interpolated raw.
    assert!(feed::hover_script(r#"a"b"#).contains(r#"hover("a\"b")"#));
    assert!(feed::hover_script("").contains(r#"hover("")"#));
}

#[test]
fn working_ring_turns_once_per_1_1_seconds_and_stays_still_without_animations() {
    assert_eq!(feed::ring_angle(0., true), 0.);
    let quarter = feed::ring_angle(feed::RING_TURN_SECS / 4., true);
    assert!((quarter - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    assert!(feed::ring_angle(feed::RING_TURN_SECS * 3.5, true) < std::f64::consts::TAU);
    assert_eq!(feed::ring_angle(12.3, false), 0.);
}

/// GTK sizes the strip tiles from code (`tabstrip.rs` margins and size
/// requests); an EventBox never measures its CSS border, padding or min-size.
/// The K5 rules may only paint, so a state change never moves or resizes a tile.
#[test]
fn k5_states_never_change_tile_geometry() {
    let css = include_str!("../src/ui/app-original.css");
    let start = css.find("/* Ficha viva K5").expect("K5 block");
    let block = &css[start..];
    let preview = block.find("/* Vista previa").expect("preview block");
    let tiles = &block[..preview];
    for property in [
        "padding",
        "margin",
        "min-height",
        "min-width",
        "font-size",
        "border-width",
        "letter-spacing",
    ] {
        assert!(
            !tiles.contains(&format!("{property}:")),
            "K5 tile rule sets {property}"
        );
    }
    // Weight only changes inside the fixed-width rows layout.
    for rule in tiles.split('}') {
        if rule.contains("font-weight: 500") {
            assert!(rule.contains(".tabstrip-rows"), "{rule}");
        }
    }
    // No violet on the open tab: the old brand underline is gone.
    assert!(!css.contains("inset 0 -2px 0 0 @BRAND@"));
    assert!(css.contains(".tabstrip-rows .strip-tab.cur { background-color: mix(@BAR@, @LINE2@, 0.35); border-color: alpha(@TEXT@, 0.55)"));
    // The marker slot is always reserved at 9×9 and never hidden.
    let label = include_str!("../src/ui/tab_label.rs");
    assert!(
        label.contains(
            "marker.set_size_request(crate::feed::MARKER_SIZE, crate::feed::MARKER_SIZE)"
        )
    );
    for hidden in [
        "marker.hide()",
        "marker.set_no_show_all",
        "marker.set_visible(",
    ] {
        assert!(!label.contains(hidden), "{hidden}");
    }
    assert_eq!(feed::MARKER_SIZE, 9);
    // Status lights stay retired (11-oct).
    assert!(label.contains("dot.set_no_show_all(true)"));
    assert!(!label.contains("dot.show()"));
}

#[test]
fn k5_css_uses_theme_tokens_for_every_theme() {
    let tokens: Value =
        serde_json::from_str(include_str!("../src/ui/desktop-themes.json")).unwrap();
    for (name, _) in tokens.as_object().unwrap() {
        let theme = comandos_app::theme::desktop_theme(name, &Value::Null).unwrap();
        let css = comandos_app::theme::base_css(&theme);
        for token in [
            "@BRAND@", "@TEXT@", "@DIM@", "@BAR@", "@LINE@", "@LINE2@", "@FAINT@",
        ] {
            assert!(!css.contains(token), "{name}: unresolved {token}");
        }
        let brand = theme.values["brand"].as_str().unwrap();
        assert!(
            css.contains(&format!(".strip-tab .k5-mark {{ color: {brand}; }}")),
            "{name}"
        );
    }
}
