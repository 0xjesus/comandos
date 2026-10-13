#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use comandos_app::term::{
    engine::{Ime, KeyAction, Modes},
    keys::key_action,
    links::{normalize_wrapped_url_text, url_from_wrapped_text},
    schedule::PaintSchedule,
};
#[test]
fn wrapped_links_keep_utf8_and_point_coordinates() {
    assert_eq!(
        normalize_wrapped_url_text("https://example.org/ñandú"),
        "https://example.org/ñandú"
    );
    assert_eq!(
        normalize_wrapped_url_text("https://example.org/\n  ñandú"),
        "https://example.org/ñandú"
    );
    assert_eq!(
        url_from_wrapped_text("one https://example.org/\nñandú two", "", Some(1), Some(1)),
        Some("https://example.org/ñandú".into())
    );
    assert_eq!(url_from_wrapped_text("/model", "", None, None), None);
    assert_eq!(
        url_from_wrapped_text("/home/test/\nfile.rs:22", "", None, None),
        Some("/home/test/file.rs:22".into())
    );
}

#[test]
fn padded_terminal_wrap_returns_the_entire_link_from_either_row() {
    use comandos_app::term::{
        engine::{Palette, TermEngine},
        links::link_at,
    };
    let now = std::time::Instant::now();
    let palette = Palette::xterm_default([255; 3], [0; 3], [255; 3], [0; 3], [255; 3]);
    let mut engine = TermEngine::new(60, 4, 10, palette, false, now);
    engine.feed(
        b"https://example.invalid/very/long/\r\n  resource?id=1\r\nNext step",
        now,
    );
    for (row, col) in [(0, 8), (1, 5)] {
        assert_eq!(
            link_at(&engine, (i32::from(row), col), row, col),
            Some("https://example.invalid/very/long/resource?id=1".into())
        );
    }
}

#[test]
fn terminal_links_do_not_consume_the_following_prose_line() {
    use comandos_app::term::{
        engine::{Palette, TermEngine},
        links::link_at,
    };
    for next in ["Next step", "  Next step", "    Done"] {
        let now = std::time::Instant::now();
        let palette = Palette::xterm_default([255; 3], [0; 3], [255; 3], [0; 3], [255; 3]);
        let mut engine = TermEngine::new(60, 4, 10, palette, false, now);
        engine.feed(
            format!("https://example.invalid/docs\r\n{next}").as_bytes(),
            now,
        );
        assert_eq!(
            link_at(&engine, (0, 8), 0, 8),
            Some("https://example.invalid/docs".into()),
            "{next}"
        );
        assert_eq!(
            link_at(&engine, (1, 5), 1, 5),
            None,
            "prose is not a link: {next}"
        );
    }
}

#[test]
fn link_clicks_and_drags_are_owned_by_the_menu_even_with_mouse_reporting() {
    use comandos_app::term::links::{PrimaryPress, report_mouse};
    assert!(
        report_mouse(false, false),
        "ordinary mouse events still reach tmux"
    );
    assert!(
        !report_mouse(true, false),
        "Shift retains terminal selection"
    );
    assert!(
        !report_mouse(false, true),
        "a link gesture must reach the native menu"
    );
    let press = PrimaryPress::new((10., 20.), (1, 2), Some("https://example.invalid/a".into()));
    assert!(
        press.clone().release((30., 20.), 1).is_none(),
        "dragging never opens the menu"
    );
    let clicked = press.release((10., 20.), 1).unwrap();
    assert_eq!(clicked.link.as_deref(), Some("https://example.invalid/a"));
    let rows = comandos_app::ui::menu::link_rows(false, false);
    assert_eq!(
        rows.iter()
            .map(|(_, _, action)| action.is_some())
            .collect::<Vec<_>>(),
        [true, false]
    );
}
#[test]
fn damage_coalesces_and_blink_stops_without_focus() {
    let mut schedule = PaintSchedule::default();
    schedule.damage(0, &[1, 2]);
    schedule.damage(1, &[2, 3]);
    assert_eq!(schedule.pending_lines(), vec![1, 2, 3]);
    assert_eq!(schedule.next_delay_ms(2), Some(0));
    schedule.painted(2);
    assert!(schedule.pending_lines().is_empty());
    schedule.set_focused(false, 3);
    assert!(!schedule.blink_due(1000));
    schedule.set_focused(true, 1000);
    assert!(schedule.blink_due(1600));
}
#[test]
fn gdk_keys_and_ime_do_not_duplicate_utf8() {
    assert_eq!(
        key_action(
            gdk::keys::constants::Up,
            gdk::ModifierType::empty(),
            &Modes::default()
        ),
        KeyAction::Send(b"\x1b[A".to_vec())
    );
    let mut ime = Ime::default();
    ime.start();
    assert!(ime.keydown_ignored(0));
    assert_eq!(ime.input("insertText", Some("漢"), true), None);
    assert_eq!(ime.end("漢字"), Some("漢字".as_bytes().to_vec()));
    assert_eq!(ime.input("insertText", Some("漢字"), false), None);
}
#[test]
fn frame_cache_preserves_pixels_during_sync_and_flushes_at_deadline() {
    use comandos_app::term::{
        engine::{Palette, TermEngine},
        paint::{CellGeom, FrameCache},
    };
    use std::time::{Duration, Instant};
    let epoch = Instant::now();
    let p = Palette::xterm_default([255; 3], [0; 3], [255; 3], [0; 3], [255; 3]);
    let mut t = TermEngine::new(20, 2, 10, p, false, epoch);
    let geom = CellGeom {
        cell_w: 8.,
        cell_h: 19.,
        origin_x: 0.,
        origin_y: 0.,
        dpr: 1.,
        font_size: 13.,
    };
    let mut frame = FrameCache::default();
    t.feed(b"before", epoch);
    assert!(frame.refresh(&t, &geom));
    let old = frame.ops().to_vec();
    t.feed(b"\x1b[?2026h\rAFTER", epoch + Duration::from_millis(1));
    assert!(!frame.refresh(&t, &geom));
    assert_eq!(frame.ops(), old);
    t.tick(epoch + Duration::from_secs(3));
    assert!(frame.refresh(&t, &geom));
    assert_ne!(frame.ops(), old);
}
#[test]
fn punctuation_digits_and_tab_map_to_terminal_codes() {
    use gdk::{ModifierType as Mod, keys::constants as key};
    let modes = Modes::default();
    assert_eq!(
        key_action(key::bracketleft, Mod::CONTROL_MASK, &modes),
        KeyAction::Send(vec![27])
    );
    assert_eq!(
        key_action(key::space, Mod::CONTROL_MASK, &modes),
        KeyAction::Send(vec![0])
    );
    assert_eq!(
        key_action(key::_1, Mod::MOD1_MASK, &modes),
        KeyAction::Send(b"\x1b1".to_vec())
    );
    assert_eq!(
        key_action(key::ISO_Left_Tab, Mod::empty(), &modes),
        KeyAction::Send(b"\x1b[Z".to_vec())
    );
}
#[test]
fn three_mouse_buttons_keep_identity_in_sgr_press_release_and_drag() {
    use comandos_app::term::{
        engine::{Button, MouseKind, MouseMode, encode_mouse},
        keys::{drag_button, mouse_button},
    };
    let modes = Modes {
        mouse: MouseMode::Drag,
        sgr_mouse: true,
        ..Modes::default()
    };
    for (number, button, state, code) in [
        (1, Button::Left, gdk::ModifierType::BUTTON1_MASK, 0),
        (2, Button::Middle, gdk::ModifierType::BUTTON2_MASK, 1),
        (3, Button::Right, gdk::ModifierType::BUTTON3_MASK, 2),
    ] {
        assert_eq!(mouse_button(number), button);
        assert_eq!(drag_button(state), button);
        assert_eq!(
            encode_mouse(
                mouse_button(number),
                MouseKind::Press,
                4,
                5,
                (false, false, false),
                &modes
            ),
            Some(format!("\x1b[<{code};5;6M").into_bytes())
        );
        assert_eq!(
            encode_mouse(
                mouse_button(number),
                MouseKind::Release,
                4,
                5,
                (false, false, false),
                &modes
            ),
            Some(format!("\x1b[<{code};5;6m").into_bytes())
        );
        assert_eq!(
            encode_mouse(
                drag_button(state),
                MouseKind::Move,
                4,
                5,
                (false, false, false),
                &modes
            ),
            Some(format!("\x1b[<{};5;6M", code + 32).into_bytes())
        );
    }
}
#[test]
fn synchronized_update_freezes_blink_until_end() {
    let mut schedule = PaintSchedule::default();
    schedule.set_focused(true, 0);
    schedule.set_synchronized(true);
    assert!(!schedule.blink_due(800));
    schedule.set_synchronized(false);
    assert!(schedule.blink_due(800));
}

#[test]
fn project_relative_file_links_keep_the_first_component() {
    for path in [
        "outputs/screenwright-etapa3-20261009-v001/index.html",
        "./outputs/index.html",
        "../outputs/index.html",
        "/home/test/outputs/index.html:42:7",
    ] {
        let text = format!("  {path}");
        for col in [3, text.len() - 2] {
            assert_eq!(
                url_from_wrapped_text(&text, "", Some(0), Some(col)),
                Some(path.into()),
                "{path} at {col}"
            );
        }
    }
}

#[test]
fn visible_relative_path_is_clickable_from_its_first_and_last_words() {
    use comandos_app::term::{
        engine::{Palette, TermEngine},
        links::link_at,
    };
    let now = std::time::Instant::now();
    let palette = Palette::xterm_default([255; 3], [0; 3], [255; 3], [0; 3], [255; 3]);
    let mut engine = TermEngine::new(100, 4, 10, palette, false, now);
    let path = "outputs/screenwright-etapa3-20261009-v001/index.html";
    engine.feed(
        format!("  {path}\r\nNo están demostrados todos los criterios").as_bytes(),
        now,
    );
    for col in [3, 15, 48] {
        assert_eq!(link_at(&engine, (0, col), 0, col), Some(path.into()));
    }
}

#[test]
fn link_split_across_two_lines_is_completed_from_the_clicked_pane() {
    use comandos_app::term::links::complete_from_pane;
    let file = "/home/test/MVPS/CATALOGOS/demos/signara__2026-10-10__v009/exports/signara-es.mp4";
    let exists = |p: &str| p == file;
    // `capture-pane -J` joins tmux soft wraps: the full path is on one line.
    let joined = format!("• Español: {file}\n");
    // Clicking either half yields only that half from the window grid.
    for half in [&file[..40], &file[40..]] {
        assert_eq!(complete_from_pane(&joined, half, exists), Some(file.into()), "{half}");
    }
    // The agent hard-wrapped the path itself (indented continuation).
    let hard = format!("  - {}\n    {} (69 MB)\n", &file[..50], &file[50..]);
    assert_eq!(complete_from_pane(&hard, &file[50..], exists), Some(file.into()));
    // URLs split across two lines.
    let url = "https://example.org/a/very/long/path?with=query&and=more";
    let text = format!("ver {}\n  {} listo\n", &url[..30], &url[30..]);
    assert_eq!(complete_from_pane(&text, &url[..30], |_| false), Some(url.into()));
    // A local path is never extended into something that does not exist.
    let wrong = "  /home/test/notes.md\n  src/other.rs\n";
    assert_eq!(complete_from_pane(wrong, "/home/test/notes.md", |_| false), None);
    // Line suffixes survive the existence check.
    let with_line = format!("{file}:12\n");
    assert_eq!(complete_from_pane(&with_line, &file[..30], exists), Some(format!("{file}:12")));
}

#[test]
fn sentence_punctuation_after_a_link_is_not_part_of_it() {
    use comandos_app::term::{
        engine::{Palette, TermEngine},
        links::{complete_from_pane, link_at},
    };
    // 13-oct: «…subtitulado.mp4.» at the end of a sentence gave «Ruta local invalida».
    for (text, link) in [
        ("El video: /home/test/v-subtitulado.mp4. Junto", "/home/test/v-subtitulado.mp4"),
        ("ver /home/test/a.rs, /home/test/b.rs", "/home/test/a.rs"),
        ("en ~/demos/gaas/.", "~/demos/gaas/"),
        ("abre https://example.org/docs.", "https://example.org/docs"),
    ] {
        assert_eq!(
            url_from_wrapped_text(text, "", Some(0), Some(text.find(['/', '~', 'h']).unwrap() + 3)),
            Some(link.into()),
            "{text}"
        );
    }
    let file = "/home/test/Videos/gaas/gaas-flujo-completo-v2-subtitulado.mp4";
    let exists = |p: &str| p == file;
    assert_eq!(
        complete_from_pane(&format!("ver {file}.\n"), &format!("{file}."), exists),
        Some(file.into())
    );
    // An agent hard wrap in the middle of the path, ending a sentence.
    let now = std::time::Instant::now();
    let palette = Palette::xterm_default([255; 3], [0; 3], [255; 3], [0; 3], [255; 3]);
    let mut engine = TermEngine::new(60, 4, 10, palette, false, now);
    engine.feed(
        format!("● Video: {}\r\n  {}. Junto a él.", &file[..48], &file[48..]).as_bytes(),
        now,
    );
    for (row, col) in [(0u16, 20u16), (1, 4)] {
        assert_eq!(
            link_at(&engine, (i32::from(row), col), row, col),
            Some(file.into()),
            "row {row}"
        );
    }
}

#[test]
fn a_path_glued_to_the_next_line_falls_back_to_the_part_that_exists() {
    use comandos_app::term::links::complete_from_pane;
    let file = "/home/test/notes.md";
    let text = format!("  {file}\n  - otro punto\n");
    assert_eq!(
        complete_from_pane(&text, &format!("{file}-"), |p| p == file),
        Some(file.into())
    );
}

#[test]
fn opening_a_path_ignores_a_trailing_period() {
    let dir = std::env::temp_dir().join(format!("links-period-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("video.mp4");
    std::fs::write(&file, b"x").unwrap();
    let raw = format!("{}.", file.display());
    assert_eq!(comandos_app::ui::menu::clean_local_path(&raw), Some(file.clone()));
    let plan =
        comandos_app::ui::menu::open_plan(&raw, comandos_app::ui::menu::OpenIntent::Reveal)
            .unwrap();
    assert_eq!(plan.reveal, Some(file));
    std::fs::remove_dir_all(&dir).unwrap();
}
