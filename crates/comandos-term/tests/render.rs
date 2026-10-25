use comandos_term::{
    engine::{Engine, GridSize, Palette},
    glyphs::{BoxOp, CellMetrics, DrawOp, box_ops, draw_ops},
    render::*,
};

fn eng() -> (Engine, Palette) {
    let p = Palette::xterm_default(
        [0xEA, 0xF0, 0xFB],
        [0x0A, 0x0D, 0x13],
        [0xFF, 0xAE, 0x1A],
        [0x0A, 0x0D, 0x13],
        [0x2E, 0x38, 0x52],
    );
    (
        Engine::new(GridSize { cols: 20, rows: 4 }, 100, p.clone()),
        p,
    )
}
const O: RenderOpts = RenderOpts {
    bold_is_bright: true,
    min_contrast: 1.0,
};

#[test]
fn same_style_ascii_is_one_run() {
    let (mut e, p) = eng();
    e.advance(b"abc\x1b[31mde", 0.0);
    let r = render_row(&e, 0, &p, &O);
    assert_eq!(
        r.runs.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(),
        ["abc", "de"]
    );
    assert_eq!(r.runs[1].style.fg, p.ansi[1]);
}

#[test]
fn bold_named_color_becomes_bright() {
    let (mut e, p) = eng();
    e.advance(b"\x1b[1;34mX", 0.0);
    assert_eq!(render_row(&e, 0, &p, &O).runs[0].style.fg, p.ansi[12]);
}

/// Ronda 4, m1: el color sin aclarar queda a mano para los glifos de fondo
/// seleccionados; solo existe si la negrita cambió el color.
#[test]
fn bold_keeps_the_plain_palette_color_only_when_it_brightened() {
    let (mut e, p) = eng();
    e.advance(b"\x1b[1;34mX\x1b[22mY\x1b[1;38;5;100mZ", 0.0);
    let plain: Vec<_> = render_row(&e, 0, &p, &O)
        .runs
        .iter()
        .map(|r| r.style.fg_plain)
        .collect();
    assert_eq!(plain, [Some(p.ansi[4]), None, None]);
}

#[test]
fn inverse_swaps_and_default_bg_is_omitted() {
    let (mut e, p) = eng();
    e.advance(b"a\x1b[7mb", 0.0);
    let r = render_row(&e, 0, &p, &O);
    assert_eq!(r.runs[1].style.fg, p.bg);
    assert_eq!(
        r.bg_runs,
        vec![(1, 1, p.fg)],
        "solo se pinta el fondo que no es el de la terminal"
    );
}

#[test]
fn wide_glyphs_are_their_own_two_cell_run() {
    let (mut e, p) = eng();
    e.advance("a漢b".as_bytes(), 0.0);
    let r = render_row(&e, 0, &p, &O);
    let wide = r.runs.iter().find(|x| x.text == "漢").unwrap();
    assert_eq!((wide.col, wide.cells, &wide.kind), (1, 2, &RunKind::Wide));
}

#[test]
fn box_drawing_is_drawn_not_typed() {
    let (mut e, p) = eng();
    e.advance("─│█".as_bytes(), 0.0);
    let r = render_row(&e, 0, &p, &O);
    assert!(r.runs.iter().all(|x| matches!(x.kind, RunKind::Box(_))));
    // Enmienda de la ronda 1: xterm.js traza la caja como camino SVG con peso
    // en píxeles (no una `HLine` en fracción de celda).
    assert_eq!(
        box_ops('─').unwrap(),
        [BoxOp::Stroke {
            weight: 1,
            d: "M0,.5 L1,.5"
        }]
    );
    assert!(matches!(box_ops('█').unwrap(), [BoxOp::Rect { w, h, .. }] if *w == 1.0 && *h == 1.0));
    assert!(box_ops('a').is_none());
}

#[test]
fn osc8_links_are_attached_to_runs() {
    let (mut e, p) = eng();
    e.advance(b"\x1b]8;;https://ej.mx\x1b\\ver\x1b]8;;\x1b\\ fin", 0.0);
    let r = render_row(&e, 0, &p, &O);
    let link = r
        .runs
        .iter()
        .find(|x| x.text == "ver")
        .unwrap()
        .link
        .unwrap();
    assert_eq!(r.links[link as usize], "https://ej.mx");
}

#[test]
fn cursor_reports_shape_and_position() {
    let (mut e, _) = eng();
    e.advance(b"ab\x1b[5 q", 0.0);
    let c = cursor(&e);
    assert_eq!((c.line, c.col, c.visible), (0, 2, true));
    assert_eq!(c.shape, CursorShape::Beam);
}

// --- Pruebas propias de A5 -------------------------------------------------

/// Mezcla de xterm.js para el atenuado: el texto se pinta con opacidad
/// 128/255 sobre el fondo de la celda.
fn dim_of(fg: [u8; 3], bg: [u8; 3]) -> [u8; 3] {
    let mix = |f: u8, b: u8| ((u32::from(f) * 128 + u32::from(b) * 127 + 127) / 255) as u8;
    [mix(fg[0], bg[0]), mix(fg[1], bg[1]), mix(fg[2], bg[2])]
}

#[test]
fn spaces_inside_a_run_are_kept_and_trailing_blanks_are_not() {
    let (mut e, p) = eng();
    e.advance(b"a b  c\x1b[31m  \x1b[0m", 0.0);
    let r = render_row(&e, 0, &p, &O);
    assert_eq!(
        r.runs
            .iter()
            .map(|x| (x.col, x.cells, x.text.as_str()))
            .collect::<Vec<_>>(),
        [(0, 6, "a b  c")]
    );
    assert!(r.bg_runs.is_empty());
}

#[test]
fn dim_blends_half_way_toward_the_cell_background() {
    let (mut e, p) = eng();
    e.advance(b"\x1b[2mx\x1b[0;2;41my", 0.0);
    let r = render_row(&e, 0, &p, &O);
    assert_eq!(r.runs[0].style.fg, dim_of(p.fg, p.bg));
    assert!(r.runs[0].style.dim);
    assert_eq!(r.runs[1].style.fg, dim_of(p.fg, p.ansi[1]));
    assert_eq!(r.runs[1].style.bg, p.ansi[1]);
    // El color antes de mezclar: quien pinta lo usa con opacidad 0,5, como
    // addon-canvas (la mezcla redondeada no basta para los bordes alisados).
    assert_eq!(r.runs[0].style.dim_fg, Some(p.fg));
    assert_eq!(r.runs[1].style.dim_fg, Some(p.fg));
}

#[test]
fn dim_source_is_absent_without_dim_or_when_contrast_wins() {
    let (mut e, p) = eng();
    e.advance(b"x\x1b[2;8my", 0.0);
    let r = render_row(&e, 0, &p, &O);
    assert_eq!(r.runs[0].style.dim_fg, None);
    // Oculto: no se dibuja nada, tampoco atenuado.
    assert_eq!(r.runs.get(1).map(|run| run.style.dim_fg), Some(None));
    // Con el contraste mínimo ajustado xterm.js no atenúa.
    let mut e2 = eng().0;
    e2.advance(b"\x1b[2;30mz", 0.0);
    let opts = RenderOpts {
        bold_is_bright: true,
        min_contrast: 7.0,
    };
    let r2 = render_row(&e2, 0, &p, &opts);
    assert!(r2.runs[0].style.dim);
    assert_eq!(r2.runs[0].style.dim_fg, None);
}

#[test]
fn bold_brightens_only_the_first_eight_palette_entries() {
    let (mut e, p) = eng();
    e.advance(
        b"\x1b[1;38;5;2ma\x1b[38;5;9mb\x1b[38;5;100mc\x1b[38;2;1;2;3md\x1b[39me",
        0.0,
    );
    let fgs: Vec<_> = render_row(&e, 0, &p, &O)
        .runs
        .iter()
        .map(|x| x.style.fg)
        .collect();
    assert_eq!(fgs, [p.ansi[10], p.ansi[9], p.ansi[100], [1, 2, 3], p.fg]);

    let plain = RenderOpts {
        bold_is_bright: false,
        min_contrast: 1.0,
    };
    let (mut e, p) = eng();
    e.advance(b"\x1b[1;31mX", 0.0);
    assert_eq!(render_row(&e, 0, &p, &plain).runs[0].style.fg, p.ansi[1]);
}

#[test]
fn inverse_brightens_the_new_foreground_but_not_the_painted_background() {
    // xterm.js: con inverso, el color de texto es el antiguo fondo (y el
    // negrita lo aclara); el fondo pintado es el antiguo texto, sin aclarar.
    let (mut e, p) = eng();
    e.advance(b"\x1b[1;7;31;44mX", 0.0);
    let r = render_row(&e, 0, &p, &O);
    assert_eq!(r.runs[0].style.fg, p.ansi[12]);
    assert_eq!(r.runs[0].style.bg, p.ansi[1]);
    assert_eq!(r.bg_runs, vec![(0, 1, p.ansi[1])]);
}

#[test]
fn hidden_text_takes_the_background_color() {
    let (mut e, p) = eng();
    e.advance(b"\x1b[8;31;42mX", 0.0);
    let s = render_row(&e, 0, &p, &O).runs[0].style;
    assert!(s.hidden);
    assert_eq!((s.fg, s.bg), (p.ansi[2], p.ansi[2]));
    // xterm.js no dibuja nada de una celda oculta: tampoco subrayado ni tachado.
    let (mut e, p) = eng();
    e.advance(b"\x1b[8;4;9;58;5;1mX", 0.0);
    let s = render_row(&e, 0, &p, &O).runs[0].style;
    assert_eq!(
        (s.underline, s.strike, s.underline_color),
        (Underline::None, false, None)
    );
}

#[test]
fn underline_color_follows_sgr_58_like_xterm_js() {
    let (mut e, p) = eng();
    e.advance(
        b"\x1b[4;58;5;1mA\x1b[1;58;5;2mB\x1b[58;2;1;2;3mC\x1b[59mD\x1b[0;4;7;58;5;3mE",
        0.0,
    );
    let colors: Vec<_> = render_row(&e, 0, &p, &O)
        .runs
        .iter()
        .map(|x| x.style.underline_color)
        .collect();
    // Negrita aclara el índice 0–7; el inverso no toca el subrayado.
    assert_eq!(
        colors,
        [
            Some(p.ansi[1]),
            Some(p.ansi[10]),
            Some([1, 2, 3]),
            None,
            Some(p.ansi[3])
        ]
    );
}

#[test]
fn drawing_glyph_with_combining_mark_is_left_to_the_font() {
    // xterm.js busca la cadena entera de la celda en sus tablas.
    let (mut e, p) = eng();
    e.advance("─\u{301}│".as_bytes(), 0.0);
    let r = render_row(&e, 0, &p, &O);
    assert_eq!(r.runs[0].kind, RunKind::Text);
    assert_eq!(r.runs[0].text, "─\u{301}");
    assert_eq!(r.runs[1].kind, RunKind::Box('│'));
}

#[test]
fn underline_variants_and_attributes() {
    let (mut e, p) = eng();
    e.advance(
        b"\x1b[4ma\x1b[4:2mb\x1b[4:3mc\x1b[4:4md\x1b[4:5me\x1b[0;3;9mf",
        0.0,
    );
    let r = render_row(&e, 0, &p, &O);
    let lines: Vec<_> = r.runs.iter().map(|x| x.style.underline).collect();
    assert_eq!(
        lines,
        [
            Underline::Single,
            Underline::Double,
            Underline::Curly,
            Underline::Dotted,
            Underline::Dashed,
            Underline::None
        ]
    );
    let last = r.runs[5].style;
    assert!(last.italic && last.strike && !last.bold);
}

#[test]
fn background_runs_group_equal_colors_and_cover_wide_spacers() {
    let (mut e, p) = eng();
    e.advance("\x1b[41m  \x1b[42m 漢\x1b[0m z".as_bytes(), 0.0);
    let r = render_row(&e, 0, &p, &O);
    assert_eq!(r.bg_runs, vec![(0, 2, p.ansi[1]), (2, 3, p.ansi[2])]);
}

#[test]
fn combining_marks_get_their_own_run() {
    let (mut e, p) = eng();
    e.advance("ae\u{301}b".as_bytes(), 0.0);
    let r = render_row(&e, 0, &p, &O);
    assert_eq!(
        r.runs
            .iter()
            .map(|x| (x.col, x.cells, x.text.as_str(), x.kind))
            .collect::<Vec<_>>(),
        [
            (0, 1, "a", RunKind::Text),
            (1, 1, "e\u{301}", RunKind::Text),
            (2, 1, "b", RunKind::Text)
        ]
    );
}

#[test]
fn non_latin_narrow_glyphs_are_drawn_one_cell_at_a_time() {
    let (mut e, p) = eng();
    e.advance("ñá αβ".as_bytes(), 0.0);
    let r = render_row(&e, 0, &p, &O);
    let texts: Vec<_> = r.runs.iter().map(|x| x.text.as_str()).collect();
    assert_eq!(texts, ["ñá", "α", "β"]);
}

#[test]
fn links_are_deduplicated_per_row() {
    let (mut e, p) = eng();
    e.advance(
        b"\x1b]8;;https://a.mx\x1b\\uno\x1b]8;;\x1b\\ \x1b]8;;https://b.mx\x1b\\dos\x1b]8;;\x1b\\ \x1b]8;;https://a.mx\x1b\\tres\x1b]8;;\x1b\\",
        0.0,
    );
    let r = render_row(&e, 0, &p, &O);
    assert_eq!(r.links, ["https://a.mx", "https://b.mx"]);
    let ids: Vec<_> = r.runs.iter().map(|x| (x.text.as_str(), x.link)).collect();
    assert_eq!(ids, [("uno", Some(0)), ("dos", Some(1)), ("tres", Some(0))]);
}

#[test]
fn osc4_palette_changes_are_rendered() {
    let (mut e, p) = eng();
    e.advance(b"\x1b]4;1;rgb:12/34/56\x1b\\\x1b[31mX", 0.0);
    assert_eq!(
        render_row(&e, 0, &p, &O).runs[0].style.fg,
        [0x12, 0x34, 0x56]
    );
}

#[test]
fn rows_outside_the_screen_are_empty_and_history_is_reachable() {
    let (mut e, p) = eng();
    e.advance(b"uno\r\ndos\r\ntres\r\ncuatro\r\ncinco", 0.0);
    assert_eq!(render_row(&e, 0, &p, &O).runs[0].text, "dos");
    let off = render_row(&e, 4, &p, &O);
    assert_eq!((off.line, off.runs.len(), off.bg_runs.len()), (4, 0, 0));
    e.scroll_display(1);
    assert_eq!(e.display_offset(), 1);
    assert_eq!(render_row(&e, 0, &p, &O).runs[0].text, "uno");
    assert_eq!(render_row(&e, 3, &p, &O).runs[0].text, "cuatro");
}

#[test]
fn render_into_reuses_buffers_and_matches_render_row() {
    let (mut e, p) = eng();
    e.advance(b"\x1b[41mab\x1b[0m cd \x1b[32mef", 0.0);
    let mut out = RowRender::default();
    render_row_into(&e, 0, &p, &O, &mut out);
    let cap = (out.runs.capacity(), out.bg_runs.capacity());
    let first = out.clone();
    render_row_into(&e, 0, &p, &O, &mut out);
    assert_eq!(out, first);
    assert_eq!(out, render_row(&e, 0, &p, &O));
    assert_eq!((out.runs.capacity(), out.bg_runs.capacity()), cap);

    // Una fila con menos tiras y luego con más vuelve a la misma salida.
    let (mut short, _) = eng();
    short.advance(b"x", 0.0);
    render_row_into(&short, 0, &p, &O, &mut out);
    assert_eq!(out, render_row(&short, 0, &p, &O));
    render_row_into(&e, 0, &p, &O, &mut out);
    assert_eq!(out, first);
}

/// Contraste WCAG como lo calcula xterm.js (`rgb.relativeLuminance2`).
fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let lum = |c: [u8; 3]| {
        let ch = |v: u8| {
            let s = f64::from(v) / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])
    };
    let (x, y) = (lum(a), lum(b));
    if x < y {
        (y + 0.05) / (x + 0.05)
    } else {
        (x + 0.05) / (y + 0.05)
    }
}

#[test]
fn minimum_contrast_lifts_text_but_not_drawing_glyphs() {
    let (mut e, p) = eng();
    e.advance("\x1b[38;2;20;20;30mA─".as_bytes(), 0.0);
    let opts = RenderOpts {
        bold_is_bright: true,
        min_contrast: 4.5,
    };
    let r = render_row(&e, 0, &p, &opts);
    assert!(contrast(r.runs[0].style.fg, p.bg) >= 4.5);
    assert_eq!(
        r.runs[1].style.fg,
        [20, 20, 30],
        "los glifos de dibujo no se ajustan"
    );
    // Con 1.0 (xterm.js por omisión) no se toca nada.
    assert_eq!(render_row(&e, 0, &p, &O).runs[0].style.fg, [20, 20, 30]);
}

#[test]
fn cursor_tracks_wide_cells_visibility_and_scrollback() {
    let (mut e, _) = eng();
    e.advance("漢\x1b[D".as_bytes(), 0.0);
    let c = cursor(&e);
    assert_eq!((c.col, c.wide, c.shape), (0, true, CursorShape::Block));

    e.advance(b"\x1b[?25l", 0.0);
    assert!(!cursor(&e).visible);
    e.advance(b"\x1b[?25h\x1b[2 q", 0.0);
    assert!(cursor(&e).visible);

    e.advance(b"\r\n1\r\n2\r\n3\r\n4\r\n5", 0.0);
    e.scroll_display(1);
    let c = cursor(&e);
    assert_eq!(
        (c.line, c.visible),
        (4, false),
        "la vista desplazada deja el cursor fuera"
    );
}

fn metrics(w: f64, h: f64, dpr: f64, font_size: f64) -> CellMetrics {
    CellMetrics {
        cell_w: w,
        cell_h: h,
        dpr,
        font_size,
    }
}

fn ops(c: char, m: &CellMetrics) -> Vec<DrawOp> {
    let mut out = Vec::new();
    assert!(draw_ops(c, m, &mut out), "{c:?}");
    out
}

#[test]
fn every_xterm_custom_glyph_has_ops() {
    // Lo que `tryDrawCustomChar` de addon-canvas 0.7.0 dibuja a mano.
    let drawn = ('\u{2500}'..='\u{259F}')
        .chain('\u{1FB70}'..='\u{1FB8B}')
        .chain('\u{1FB95}'..='\u{1FB97}')
        .chain('\u{E0B0}'..='\u{E0BF}');
    for c in drawn {
        assert!(box_ops(c).is_some_and(|o| !o.is_empty()), "falta {c:?}");
    }
    for c in [
        '\u{24FF}',
        '\u{25A0}',
        '\u{1FB6F}',
        '\u{1FB8C}',
        '\u{1FB94}',
        '\u{1FB98}',
        '\u{E0AF}',
        '\u{E0C0}',
        'a',
    ] {
        assert!(box_ops(c).is_none(), "{c:?}");
        assert!(!draw_ops(
            c,
            &metrics(9.0, 20.0, 1.0, 14.0),
            &mut Vec::new()
        ));
    }
    // Alias de `powerlineDefinitions`.
    assert_eq!(box_ops('\u{E0BB}'), box_ops('\u{E0BD}'));
    assert_eq!(box_ops('\u{E0BF}'), box_ops('\u{E0B9}'));
}

#[test]
fn box_lines_are_pixel_weights_snapped_to_pixel_centers() {
    use DrawOp::*;
    let m = metrics(9.0, 20.0, 1.0, 14.0);
    // `lineWidth = dpr · peso`; `round(c + .5) − .5` y el máximo acotado.
    assert_eq!(
        ops('─', &m),
        [
            BeginPath,
            MoveTo { x: 0.0, y: 10.5 },
            LineTo { x: 9.0, y: 10.5 },
            Stroke { line_width: 1.0 }
        ]
    );
    assert_eq!(
        ops('│', &m),
        [
            BeginPath,
            MoveTo { x: 4.5, y: 0.0 },
            LineTo { x: 4.5, y: 20.0 },
            Stroke { line_width: 1.0 }
        ]
    );
    assert_eq!(ops('━', &m).last(), Some(&Stroke { line_width: 3.0 }));
    let hidpi = metrics(18.0, 40.0, 2.0, 14.0);
    assert_eq!(
        ops('─', &hidpi),
        [
            BeginPath,
            MoveTo { x: 0.0, y: 20.5 },
            LineTo { x: 18.0, y: 20.5 },
            Stroke { line_width: 2.0 }
        ]
    );
    assert_eq!(ops('┃', &hidpi).last(), Some(&Stroke { line_width: 6.0 }));
    // Mitad fina, mitad gruesa: dos trazos en el orden de las claves (1, 3).
    let mixed: Vec<_> = ops('╼', &m)
        .into_iter()
        .filter(|op| matches!(op, Stroke { .. }))
        .collect();
    assert_eq!(
        mixed,
        [Stroke { line_width: 1.0 }, Stroke { line_width: 3.0 }]
    );
}

#[test]
fn double_lines_and_round_corners_use_the_real_cell_aspect() {
    use DrawOp::*;
    // t = .15 / 20 · 9 = .0675: filas .4325 y .5675 → 8,65 y 11,35 → 8,5 y 11,5.
    let m = metrics(9.0, 20.0, 1.0, 14.0);
    assert_eq!(
        ops('═', &m),
        [
            BeginPath,
            MoveTo { x: 0.0, y: 8.5 },
            LineTo { x: 9.0, y: 8.5 },
            MoveTo { x: 0.0, y: 11.5 },
            LineTo { x: 9.0, y: 11.5 },
            Stroke { line_width: 1.0 }
        ]
    );
    // e = .15 en horizontal: .35 · 9 = 3,15 → 3,5; .65 · 9 = 5,85 → 5,5.
    assert_eq!(
        ops('║', &m),
        [
            BeginPath,
            MoveTo { x: 3.5, y: 0.0 },
            LineTo { x: 3.5, y: 20.0 },
            MoveTo { x: 5.5, y: 0.0 },
            LineTo { x: 5.5, y: 20.0 },
            Stroke { line_width: 1.0 }
        ]
    );
    // Esquina: recta hasta .5 + t/.15·.5 = .725 (14,5) y Bézier nativa.
    assert_eq!(
        ops('╭', &m),
        [
            BeginPath,
            MoveTo { x: 4.5, y: 20.0 },
            LineTo { x: 4.5, y: 14.5 },
            CurveTo {
                x1: 4.5,
                y1: 14.5,
                x2: 4.5,
                y2: 10.5,
                x: 9.0,
                y: 10.5
            },
            Stroke { line_width: 1.0 }
        ]
    );
    // Otra proporción mueve la doble línea: t = .15 / 16 · 8 = .075 → 6,8 y 9,2.
    let narrow = metrics(8.0, 16.0, 1.0, 12.0);
    let ys: Vec<f64> = ops('═', &narrow)
        .iter()
        .filter_map(|op| match *op {
            MoveTo { y, .. } => Some(y),
            _ => None,
        })
        .collect();
    assert_eq!(ys, [6.5, 9.5]);
}

#[test]
fn blocks_are_eighths_and_shades_are_pixel_patterns() {
    use DrawOp::*;
    let m = metrics(9.0, 20.0, 1.0, 14.0);
    assert_eq!(
        ops('█', &m),
        [FillRect {
            x: 0.0,
            y: 0.0,
            w: 9.0,
            h: 20.0
        }]
    );
    assert_eq!(
        ops('▄', &m),
        [FillRect {
            x: 0.0,
            y: 10.0,
            w: 9.0,
            h: 10.0
        }]
    );
    assert_eq!(
        ops('▕', &m),
        [FillRect {
            x: 7.875,
            y: 0.0,
            w: 1.125,
            h: 20.0
        }]
    );
    assert_eq!(
        ops('\u{1FB97}', &m),
        [
            FillRect {
                x: 0.0,
                y: 5.0,
                w: 9.0,
                h: 5.0
            },
            FillRect {
                x: 0.0,
                y: 15.0,
                w: 9.0,
                h: 5.0
            }
        ]
    );
    // Cobertura de las tramas de xterm.js: 12,5 %, 25 %, 75 %.
    for (c, ones, cells) in [('░', 2, 16), ('▒', 2, 8), ('▓', 6, 8)] {
        let [FillPattern { mask }] = ops(c, &m)[..] else {
            panic!("{c:?}");
        };
        let total: usize = mask.iter().map(|row| row.len()).sum();
        let set: usize = mask
            .iter()
            .flat_map(|row| row.iter())
            .filter(|&&v| v == 1)
            .count();
        assert_eq!((set, total), (ones, cells), "{c:?}");
    }
    assert!(matches!(box_ops('▒').unwrap(), [BoxOp::Pattern(_)]));
}

#[test]
fn powerline_is_clipped_padded_and_sized_by_font() {
    use DrawOp::*;
    // fontSize 12 → d = 1 px; E0B0: rightPadding 2 → margen derecho d.
    let m = metrics(9.0, 20.0, 1.0, 12.0);
    assert_eq!(
        ops('\u{E0B0}', &m),
        [
            ClipCell,
            BeginPath,
            MoveTo { x: 0.0, y: 0.0 },
            LineTo { x: 8.0, y: 10.0 },
            LineTo { x: 0.0, y: 20.0 },
            Fill
        ]
    );
    // E0B1: contorno de d px, márgenes de medio d a cada lado, sin ajuste.
    assert_eq!(
        ops('\u{E0B1}', &m),
        [
            ClipCell,
            BeginPath,
            MoveTo { x: -7.5, y: -10.0 },
            LineTo { x: 8.5, y: 10.0 },
            LineTo { x: -7.5, y: 30.0 },
            Stroke { line_width: 1.0 }
        ]
    );
    // dpr 2: el contorno mide 2 px de dispositivo.
    let hidpi = metrics(18.0, 40.0, 2.0, 12.0);
    assert_eq!(
        ops('\u{E0B5}', &hidpi).last(),
        Some(&Stroke { line_width: 2.0 })
    );
    assert!(
        ops('\u{E0B4}', &m)
            .iter()
            .any(|op| matches!(op, CurveTo { .. }))
    );
}

/// Las órdenes de `draw_ops` coinciden número a número con las que emite
/// `tryDrawCustomChar` de `assets/xterm/addon-canvas.js` 0.7.0. El fixture
/// se generó ejecutando esa función del bundle sobre un contexto que anota
/// cada llamada (`fillRect`, `moveTo`, `bezierCurveTo`, `stroke`, `clip`…;
/// una trama se anota como su máscara) para dos celdas: 9×20 a dpr 1 con
/// letra de 14 px y 13×29 a dpr 1,25 con 15 px.
#[test]
fn draw_ops_match_xterm_js_call_for_call() {
    use serde_json::{Value, json};
    let fixture: serde_json::Map<String, Value> =
        serde_json::from_str(include_str!("fixtures/xterm-custom-glyphs.json")).unwrap();
    assert_eq!(fixture.len(), 2 * 207);
    for (key, expected) in &fixture {
        let (cell, code) = key.split_once(':').unwrap();
        let (size, rest) = cell.split_once('@').unwrap();
        let (w, h) = size.split_once('x').unwrap();
        let (dpr, font) = rest.split_once('/').unwrap();
        let num = |s: &str| s.parse::<f64>().unwrap();
        let m = metrics(num(w), num(h), num(dpr), num(font));
        let c = char::from_u32(u32::from_str_radix(code, 16).unwrap()).unwrap();
        let got: Vec<Value> = ops(c, &m)
            .into_iter()
            .map(|op| match op {
                DrawOp::FillRect { x, y, w, h } => json!(["FillRect", x, y, w, h]),
                DrawOp::FillPattern { mask } => json!(["FillPattern", mask]),
                DrawOp::ClipCell => json!(["ClipCell"]),
                DrawOp::BeginPath => json!(["BeginPath"]),
                DrawOp::MoveTo { x, y } => json!(["MoveTo", x, y]),
                DrawOp::LineTo { x, y } => json!(["LineTo", x, y]),
                DrawOp::CurveTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                } => json!(["CurveTo", x1, y1, x2, y2, x, y]),
                DrawOp::Stroke { line_width } => json!(["Stroke", line_width]),
                DrawOp::Fill => json!(["Fill"]),
            })
            .collect();
        // Compara como f64 (el JSON de JavaScript escribe 10 donde Rust 10.0).
        let flat = |v: &Value| -> Vec<String> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|op| {
                    op.as_array()
                        .unwrap()
                        .iter()
                        .map(|x| match x.as_f64() {
                            Some(f) => format!("{f:?}"),
                            None => x.to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .collect()
        };
        assert_eq!(flat(&Value::Array(got)), flat(expected), "{key}");
    }
}

#[test]
fn each_cell_maps_to_its_grapheme_for_per_cell_drawing() {
    // A6 pinta cada celda en `col · cell_w`: un carácter con sus marcas
    // combinantes ocupa una sola celda.
    let (mut e, p) = eng();
    e.advance("ab\u{301}c 漢─x".as_bytes(), 0.0);
    let r = render_row(&e, 0, &p, &O);
    let cells: Vec<(u16, &str)> = r.runs.iter().flat_map(Run::cell_texts).collect();
    assert_eq!(
        cells,
        [
            (0, "a"),
            (1, "b\u{301}"),
            (2, "c"),
            (4, "漢"),
            (6, "─"),
            (7, "x")
        ]
    );
    // Dentro de una tira agrupada, un hueco es su propia celda.
    let (mut e, p) = eng();
    e.advance(b"a  b", 0.0);
    let r = render_row(&e, 0, &p, &O);
    let cells: Vec<(u16, &str)> = r.runs[0].cell_texts().collect();
    assert_eq!(cells, [(0, "a"), (1, " "), (2, " "), (3, "b")]);
}

#[test]
fn underline_color_without_underline_does_not_split_runs() {
    let (mut e, p) = eng();
    e.advance(b"x\x1b[58;5;1my\x1b[4mz", 0.0);
    let r = render_row(&e, 0, &p, &O);
    assert_eq!(
        r.runs
            .iter()
            .map(|x| (x.text.as_str(), x.style.underline_color))
            .collect::<Vec<_>>(),
        [("xy", None), ("z", Some(p.ansi[1]))]
    );
}

#[test]
fn device_size_keeps_real_pixels_and_fixes_css_sized_reports() {
    use comandos_term::render::device_size;
    // A phone at dpr 3 reports real device pixels: kept, rounding included.
    assert_eq!(device_size(1056, 552, (352.0, 184.0), 3.0), (1056, 552));
    assert_eq!(device_size(1057, 551, (352.3, 183.8), 3.0), (1057, 551));
    // Emulation reporting the CSS size at dpr 3: painted at CSS times dpr instead.
    assert_eq!(device_size(352, 184, (352.0, 184.0), 3.0), (1056, 552));
    // dpr 1 and a fractional dpr stay as reported.
    assert_eq!(device_size(800, 600, (800.0, 600.0), 1.0), (800, 600));
    assert_eq!(device_size(1101, 826, (880.6, 660.6), 1.25), (1101, 826));
}
