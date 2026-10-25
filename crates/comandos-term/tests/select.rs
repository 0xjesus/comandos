use comandos_term::{
    engine::{Engine, GridSize, Palette},
    select::*,
};

fn eng(cols: u16) -> Engine {
    Engine::new(
        GridSize { cols, rows: 5 },
        100,
        Palette::xterm_default([255; 3], [0; 3], [255; 3], [0; 3], [80; 3]),
    )
}

#[test]
fn wrapped_rows_copy_without_newline_and_written_trailing_spaces_are_kept() {
    let mut e = eng(10);
    e.advance(b"0123456789abc   \r\nfin", 0.0);
    let s = Selection {
        anchor: (0, 0),
        head: (2, 2),
        mode: SelectMode::Simple,
    };
    // Los espacios que la aplicación escribió son contenido (getTrimmedLength
    // de xterm.js): solo se recortan las celdas nunca escritas.
    assert_eq!(selected_text(&e, &s), "0123456789abc   \nfin");
}

#[test]
fn url_split_by_soft_wrap_is_one_link() {
    let mut e = eng(20);
    e.advance(b"ver https://ejemplo.mx/un/camino/largo ok", 0.0);
    let urls = find_urls(&e, 1);
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0].url, "https://ejemplo.mx/un/camino/largo");
    assert_eq!((urls[0].start.0, urls[0].end.0), (0, 1));
}

#[test]
fn ligature_candidates_are_greedy() {
    assert_eq!(ligature_runs("a <=> b != c"), vec![(2, 5), (8, 10)]);
    assert_eq!(ligature_runs("===>"), vec![(0, 4)]);
}

fn sel(a: (i32, u16), h: (i32, u16), mode: SelectMode) -> Selection {
    Selection {
        anchor: a,
        head: h,
        mode,
    }
}

#[test]
fn reversed_anchor_and_head_give_the_same_text() {
    let mut e = eng(20);
    e.advance(b"hola mundo\r\nsegunda", 0.0);
    let fwd = selected_text(&e, &sel((0, 5), (1, 2), SelectMode::Simple));
    let rev = selected_text(&e, &sel((1, 2), (0, 5), SelectMode::Simple));
    assert_eq!(fwd, "mundo\nseg");
    assert_eq!(rev, fwd);
}

#[test]
fn word_mode_uses_xterm_separators_including_comma_and_backtick() {
    let mut e = eng(30);
    e.advance(b"uno,dos `tres` (cuatro)", 0.0);
    // "dos" entre la coma y el espacio.
    assert_eq!(
        selected_text(&e, &sel((0, 5), (0, 5), SelectMode::Word)),
        "dos"
    );
    assert_eq!(
        selected_text(&e, &sel((0, 10), (0, 10), SelectMode::Word)),
        "tres"
    );
    assert_eq!(
        selected_text(&e, &sel((0, 17), (0, 17), SelectMode::Word)),
        "cuatro"
    );
    // Arrastrar de una palabra a otra las incluye enteras.
    assert_eq!(
        selected_text(&e, &sel((0, 1), (0, 5), SelectMode::Word)),
        "uno,dos"
    );
}

#[test]
fn word_at_selects_blank_run_and_follows_wrapped_rows() {
    let mut e = eng(10);
    e.advance(b"ab         cdefghijklmno", 0.0);
    // Como el doble clic de xterm.js (allowWhitespaceOnlySelection): la racha
    // de espacios es la «palabra».
    assert_eq!(word_at(&e, (0, 5)), Some(((0, 2), (0, 9))));
    // "cdefghijklmno" (13) arranca en la columna 11 → fila 1, cols 1..
    let (start, end) = word_at(&e, (1, 3)).expect("palabra");
    assert_eq!((start, end), ((1, 1), (2, 3)));
}

#[test]
fn line_mode_selects_whole_wrapped_logical_lines() {
    let mut e = eng(10);
    e.advance(b"0123456789abcde\r\notra", 0.0);
    assert_eq!(
        selected_text(&e, &sel((1, 2), (1, 2), SelectMode::Line)),
        "0123456789abcde"
    );
    assert_eq!(
        selected_text(&e, &sel((0, 0), (2, 0), SelectMode::Line)),
        "0123456789abcde\notra"
    );
}

#[test]
fn history_rows_are_negative_and_nbsp_copies_as_space() {
    let mut e = eng(10);
    e.advance(b"h0\r\nh1\r\nh2\r\nh3\r\nh4\r\nh5\r\na\xc2\xa0b", 0.0);
    // Cinco filas de pantalla: «h0» y «h1» pasaron a la historia.
    assert_eq!(e.history_len(), 2);
    assert_eq!(
        selected_text(&e, &sel((-2, 0), (-1, 1), SelectMode::Simple)),
        "h0\nh1"
    );
    assert_eq!(
        selected_text(&e, &sel((4, 0), (4, 2), SelectMode::Simple)),
        "a b"
    );
}

#[test]
fn wide_chars_are_copied_once() {
    let mut e = eng(10);
    e.advance("日本x".as_bytes(), 0.0);
    assert_eq!(
        selected_text(&e, &sel((0, 0), (0, 5), SelectMode::Simple)),
        "日本x"
    );
}

#[test]
fn single_row_url_trailing_punctuation_is_left_out() {
    let mut e = eng(60);
    e.advance(b"mira (https://a.mx/p?q=1). y http://b.mx", 0.0);
    let urls = find_urls(&e, 0);
    let list: Vec<&str> = urls.iter().map(|u| u.url.as_str()).collect();
    assert_eq!(list, vec!["https://a.mx/p?q=1", "http://b.mx"]);
    assert_eq!(urls[0].start, (0, 6));
    assert_eq!(urls[0].end, (0, 23));
}

#[test]
fn osc8_hyperlinks_are_found_per_row() {
    let mut e = eng(40);
    e.advance(
        b"ir a \x1b]8;;https://osc.mx/x\x1b\\este sitio\x1b]8;;\x1b\\ ya",
        0.0,
    );
    let urls = find_urls(&e, 0);
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0].url, "https://osc.mx/x");
    assert_eq!((urls[0].start, urls[0].end), ((0, 5), (0, 14)));
    // Solo http/https.
    let mut e = eng(40);
    e.advance(b"\x1b]8;;file:///etc/passwd\x1b\\x\x1b]8;;\x1b\\", 0.0);
    assert!(find_urls(&e, 0).is_empty());
}

#[test]
fn soft_wrapped_url_is_found_from_either_row_and_not_from_others() {
    let mut e = eng(20);
    e.advance(b"ver https://ejemplo.mx/un/camino/largo ok\r\nfin", 0.0);
    assert_eq!(find_urls(&e, 0).len(), 1);
    assert_eq!(find_urls(&e, 1).len(), 1);
    assert!(find_urls(&e, 3).is_empty());
}

#[test]
fn ligature_runs_misc() {
    assert!(ligature_runs("").is_empty());
    assert!(ligature_runs("abc def").is_empty());
    assert_eq!(ligature_runs("<===>"), vec![(0, 5)]);
    assert_eq!(ligature_runs("é->é"), vec![(1, 3)]);
    assert_eq!(ligature_runs("->->"), vec![(0, 2), (2, 4)]);
    assert_eq!(ligature_runs("a\\/b"), vec![(1, 3)]);
}

// --- ronda 1 de revisión ---

#[test]
fn space_at_the_edge_of_a_soft_wrapped_row_is_kept() {
    let mut e = eng(10);
    e.advance(b"012345678 abc", 0.0);
    let s = sel((0, 0), (1, 9), SelectMode::Simple);
    assert_eq!(selected_text(&e, &s), "012345678 abc");
}

#[test]
fn erased_cells_are_trimmed_but_written_spaces_are_not() {
    let mut e = eng(10);
    // «ab» + 3 espacios escritos; la fila 1 se borra con EL tras escribirla.
    e.advance(b"ab   \r\nxyz\x1b[2K\x1b[1;9H", 0.0);
    let s = sel((0, 0), (1, 9), SelectMode::Simple);
    assert_eq!(selected_text(&e, &s), "ab   \n");
    // ECH borra y deja la celda sin escribir.
    let mut e = eng(10);
    e.advance(b"ab   \x1b[1;3H\x1b[3X", 0.0);
    let s = sel((0, 0), (0, 9), SelectMode::Simple);
    assert_eq!(selected_text(&e, &s), "ab");
}

#[test]
fn tabs_copy_as_spaces_like_xterm() {
    let mut e = eng(20);
    e.advance(b"a\tb\r\nc\t", 0.0);
    assert_eq!(
        selected_text(&e, &sel((0, 0), (1, 19), SelectMode::Simple)),
        "a       b\nc"
    );
}

#[test]
fn soft_wrap_strips_trailing_dot_like_weblinks() {
    let mut e = eng(20);
    e.advance(b"ver https://ejemplo.mx/abcdefgh. fin", 0.0);
    for line in [0, 1] {
        let urls = find_urls(&e, line);
        assert_eq!(urls.len(), 1, "fila {line}");
        assert_eq!(urls[0].url, "https://ejemplo.mx/abcdefgh");
        assert_eq!((urls[0].start, urls[0].end), ((0, 4), (1, 10)));
    }
}

#[test]
fn tmux_hard_wrap_beats_truncated_weblinks_match() {
    // La fila 0 está llena y termina en «.»: WebLinks la corta en «ejemplo»;
    // el proveedor de tmux la sigue por la fila 1.
    let mut e = eng(20);
    e.advance(b"ver https://ejemplo.\r\nmx/un/camino/lar ok\r\nfin", 0.0);
    for line in [0, 1] {
        let urls = find_urls(&e, line);
        assert_eq!(urls.len(), 1, "fila {line}");
        assert_eq!(urls[0].url, "https://ejemplo.mx/un/camino/lar");
        assert_eq!((urls[0].start, urls[0].end), ((0, 4), (1, 15)));
    }
    assert!(find_urls(&e, 2).is_empty());
}

#[test]
fn url_scan_is_bounded_on_a_huge_full_row_block() {
    // 10 000 filas llenas: el trabajo por llamada no crece con el bloque.
    let mut e = Engine::new(
        GridSize { cols: 20, rows: 5 },
        20_000,
        Palette::xterm_default([255; 3], [0; 3], [255; 3], [0; 3], [80; 3]),
    );
    let block = "a".repeat(20 * 10_000);
    e.advance(block.as_bytes(), 0.0);
    let started = std::time::Instant::now();
    assert!(find_urls(&e, 4).is_empty());
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
}

#[test]
fn hard_wrapped_url_longer_than_the_row_cap_is_not_joined_from_its_tail() {
    let mut e = Engine::new(
        GridSize { cols: 20, rows: 5 },
        2000,
        Palette::xterm_default([255; 3], [0; 3], [255; 3], [0; 3], [80; 3]),
    );
    // «http://» + 2000 caracteres en 100 filas con wrap duro (CRLF).
    let mut text = String::from("http://");
    text.push_str(&"b".repeat(13));
    for _ in 0..99 {
        text.push_str("\r\n");
        text.push_str(&"b".repeat(20));
    }
    text.push_str("\r\nfin");
    e.advance(text.as_bytes(), 0.0);
    // La fila 3 es la última con «b»; el esquema queda 99 filas más arriba.
    assert!(find_urls(&e, 3).is_empty());
    // Desde la primera fila sí es un enlace (WebLinks ve «http://bbbb…»).
    assert_eq!(find_urls(&e, -96).len(), 1);
}

#[test]
fn idn_hosts_are_not_links_like_xterm() {
    let mut e = eng(40);
    e.advance("https://ñandú.mx/x y https://ok.mx".as_bytes(), 0.0);
    let urls = find_urls(&e, 0);
    let list: Vec<&str> = urls.iter().map(|u| u.url.as_str()).collect();
    assert_eq!(list, vec!["https://ok.mx"]);
}

// --- ronda 2 de revisión ---

#[test]
fn tmux_exception_replaces_every_overlapped_weblinks_span() {
    let mut e = eng(24);
    e.advance(b"https://a.b(https://c.dd\r\nef/g ok", 0.0);
    for line in [0, 1] {
        let urls = find_urls(&e, line);
        for (i, a) in urls.iter().enumerate() {
            for b in urls.iter().skip(i + 1) {
                assert!(a.end < b.start || b.end < a.start, "solape: {urls:?}");
            }
        }
        assert_eq!(urls.len(), 1, "fila {line}: {urls:?}");
        assert_eq!(urls[0].url, "https://a.b(https://c.ddef/g");
    }
}

#[test]
fn written_trailing_spaces_survive_a_column_shrink() {
    let mut e = eng(10);
    e.advance(b"abc      ", 0.0);
    e.resize(GridSize { cols: 5, rows: 5 }, (8, 16));
    // La línea lógica sigue siendo «abc» + 6 espacios escritos.
    let text = selected_text(&e, &sel((0, 0), (0, 0), SelectMode::Line));
    assert_eq!(text, "abc      ");
}

#[test]
fn erase_and_shift_operations_leave_cells_that_are_not_written() {
    use alacritty_terminal::{
        index::{Column, Line},
        term::cell::Flags,
    };
    let written = |e: &Engine, col: usize| {
        e.term().grid()[Line(0)][Column(col)]
            .flags
            .contains(Flags::WRITTEN)
    };
    // Diez espacios escritos; cada operación borra o desplaza.
    let spaces = "          ";
    let mut e = eng(10);
    e.advance(spaces.as_bytes(), 0.0);
    assert!((0..10).all(|c| written(&e, c)));
    // ECH: columnas 2..4 borradas.
    e.advance(b"\x1b[1;3H\x1b[2X", 0.0);
    assert!(!written(&e, 2) && !written(&e, 3) && written(&e, 4));
    // EL (borrar hasta el final desde la columna 8).
    e.advance(b"\x1b[1;9H\x1b[K", 0.0);
    assert!(written(&e, 7) && !written(&e, 8) && !written(&e, 9));
    // DCH: borra 3 desde la columna 0; las vacías de la derecha no están escritas.
    let mut e = eng(10);
    e.advance(spaces.as_bytes(), 0.0);
    e.advance(b"\x1b[1;1H\x1b[3P", 0.0);
    assert!(written(&e, 6) && !written(&e, 7) && !written(&e, 9));
    // ICH: las 3 celdas insertadas no están escritas.
    let mut e = eng(10);
    e.advance(spaces.as_bytes(), 0.0);
    e.advance(b"\x1b[1;1H\x1b[3@", 0.0);
    assert!(!written(&e, 0) && !written(&e, 2) && written(&e, 3));
}

#[test]
fn block_mode_copies_each_row_cut_to_the_columns() {
    let mut e = eng(10);
    e.advance(b"abcdef\r\n123456\r\nxy\r\n0123456789wrap", 0.0);
    let block = sel((0, 1), (2, 3), SelectMode::Block);
    assert_eq!(selection_bounds(&e, &block), Some(((0, 1), (2, 3))));
    assert_eq!(selected_text(&e, &block), "bcd\n234\ny");
    // Esquinas cruzadas: el mismo rectángulo.
    assert_eq!(
        selected_text(&e, &sel((2, 1), (0, 3), SelectMode::Block)),
        "bcd\n234\ny"
    );
    // Las filas envueltas no se unen en columna.
    assert_eq!(
        selected_text(&e, &sel((3, 8), (4, 9), SelectMode::Block)),
        "89\n"
    );
}

#[test]
fn wide_char_tails_are_the_second_half() {
    let mut e = eng(10);
    e.advance("日本x".as_bytes(), 0.0);
    assert!(!is_wide_char_tail(&e, (0, 0)));
    assert!(is_wide_char_tail(&e, (0, 1)));
    assert!(is_wide_char_tail(&e, (0, 3)));
    assert!(!is_wide_char_tail(&e, (0, 4)));
    assert!(!is_wide_char_tail(&e, (9, 0)), "fuera de la rejilla");
}

/// `moveToCellSequence` con historia: celdas en línea recta.
#[test]
fn alt_click_counts_cells_in_the_normal_screen() {
    let mut e = eng(10);
    e.advance(b"\r\n\r\nabcde", 0.0);
    let arrows = |n: usize, d: &str| d.repeat(n).into_bytes();
    assert_eq!(move_to_cell(&e, (2, 2), false, true), arrows(3, "\x1b[D"));
    assert_eq!(move_to_cell(&e, (5, 2), false, true), Vec::<u8>::new());
    assert_eq!(move_to_cell(&e, (1, 4), false, true), arrows(16, "\x1b[C"));
    assert_eq!(move_to_cell(&e, (8, 0), true, true), arrows(17, "\x1bOD"));
}

/// Sin historia (pantalla alternativa) recorre la rejilla como xterm.js,
/// incluido que hacia la izquierda en la misma fila no se mueve.
#[test]
fn alt_click_walks_the_grid_in_the_alternate_screen() {
    let mut e = eng(10);
    e.advance(b"\x1b[?1049h\x1b[3;4H", 0.0);
    let arrows = |n: usize, d: &str| d.repeat(n).into_bytes();
    assert_eq!(move_to_cell(&e, (6, 2), false, false), arrows(3, "\x1b[C"));
    assert_eq!(move_to_cell(&e, (1, 2), false, false), Vec::<u8>::new());
    assert_eq!(move_to_cell(&e, (3, 4), false, false), arrows(2, "\x1b[B"));
    assert_eq!(move_to_cell(&e, (3, 0), true, false), arrows(2, "\x1bOA"));
}

#[test]
fn wrapped_url_does_not_take_the_sentence_period() {
    let mut e = eng(20);
    e.advance(b"ver https://ejemplo.mx/un/camino/largo. Listo\r\nfin", 0.0);
    let urls = find_urls(&e, 1);
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0].url, "https://ejemplo.mx/un/camino/largo");
}

#[test]
fn a_cropped_pane_copies_only_its_own_columns() {
    let mut e = eng(20);
    // Two side-by-side panes: columns 0..10 and 10..20 (no border, for brevity).
    e.advance(b"left one  RIGHT ONE\r\nleft two  RIGHT TWO\r\nleft 3    RIGHT 3", 0.0);
    let s = Selection {
        anchor: (0, 5),
        head: (2, 19),
        mode: SelectMode::Simple,
    };
    assert_eq!(selected_text_within(&e, &s, (0, 10)), "one\nleft two\nleft 3");
    assert_eq!(selected_text_within(&e, &s, (10, 20)), "RIGHT ONE\nRIGHT TWO\nRIGHT 3");
    let other = Selection {
        anchor: (1, 12),
        head: (1, 14),
        mode: SelectMode::Simple,
    };
    assert_eq!(selected_text_within(&e, &other, (0, 10)), "");
}
