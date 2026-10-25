use comandos_term::proto::*;

#[test]
fn tty_init_is_raw_json_without_prefix() {
    let init = parse_init(Dialect::Tty, br#"{"AuthToken":"","columns":96,"rows":30}"#).unwrap();
    assert_eq!((init.cols, init.rows, init.session), (96, 30, None));
}

#[test]
fn v1_init_carries_session() {
    let init = parse_init(
        Dialect::V1,
        br#"{"v":1,"cols":80,"rows":24,"session":"term-1-2"}"#,
    )
    .unwrap();
    assert_eq!(init.session.as_deref(), Some("term-1-2"));
    assert!(!init.ignore_size);
    assert!(parse_init(Dialect::V1, br#"{"v":2,"cols":80,"rows":24}"#).is_err());
}

#[test]
fn only_an_explicit_v1_flag_keeps_a_client_from_sizing_the_window() {
    let init = |raw: &[u8]| parse_init(Dialect::V1, raw).unwrap().ignore_size;
    assert!(init(br#"{"v":1,"cols":156,"rows":41,"session":"s","ignoreSize":true}"#));
    assert!(!init(br#"{"v":1,"cols":156,"rows":41,"ignoreSize":"yes"}"#));
    assert!(!init(br#"{"v":1,"cols":156,"rows":41,"ignoreSize":false}"#));
    let tty = parse_init(Dialect::Tty, br#"{"columns":96,"rows":30,"ignoreSize":true}"#).unwrap();
    assert!(!tty.ignore_size, "the ttyd dialect has no such flag");
}

#[test]
fn sizes_out_of_range_are_rejected() {
    assert!(matches!(
        parse_init(Dialect::Tty, br#"{"columns":0,"rows":30}"#),
        Err(ProtoError::BadSize)
    ));
    assert!(matches!(
        parse_init(Dialect::Tty, br#"{"columns":80,"rows":9999}"#),
        Err(ProtoError::BadSize)
    ));
}

#[test]
fn client_messages_parse_from_text_or_binary_frames() {
    assert_eq!(
        parse_client(b"0ls\r").unwrap(),
        ClientMsg::Input(b"ls\r".to_vec())
    );
    assert_eq!(
        parse_client(br#"1{"columns":120,"rows":40}"#).unwrap(),
        ClientMsg::Resize {
            cols: 120,
            rows: 40
        }
    );
    assert_eq!(parse_client(b"2").unwrap(), ClientMsg::Pause);
    assert_eq!(parse_client(b"3").unwrap(), ClientMsg::Resume);
    assert!(parse_client(b"9").is_err());
    // 70 000 bytes (antes por encima del tope de 64 KiB) ya pasan; el tope es 1 MiB.
    assert!(parse_client(&[b'0'; 70_000]).is_ok());
    assert!(parse_client(&vec![b'0'; MAX_CLIENT_FRAME + 1]).is_err());
}

#[test]
fn tty_args_keep_order_and_decode() {
    assert_eq!(
        tty_args("arg=t%20k&x=1&arg=term-1"),
        vec!["t k".to_string(), "term-1".to_string()]
    );
}

#[test]
fn server_frames_have_ttyd_prefixes() {
    assert_eq!(output(b"hi"), b"0hi".to_vec());
    assert_eq!(
        title("cc-webterm-attach (host)"),
        b"1cc-webterm-attach (host)".to_vec()
    );
    assert_eq!(&prefs("{}")[..1], b"2");
}

// --- Pruebas propias de A2 ---

#[test]
fn dialect_comes_from_the_negotiated_subprotocol() {
    assert_eq!(Dialect::from_protocol(Some("tty")), Some(Dialect::Tty));
    assert_eq!(
        Dialect::from_protocol(Some("comandos.term.v1")),
        Some(Dialect::V1)
    );
    assert_eq!(Dialect::from_protocol(Some("TTY")), None);
    assert_eq!(Dialect::from_protocol(None), None);
    // El servidor prefiere v1 si el cliente ofrece ambos.
    assert_eq!(PROTOCOLS, &["comandos.term.v1", "tty"]);
}

// ttyd 1.6.3 arma las preferencias con json-c: cada `-t clave=valor` pasa por
// `json_tokener_parse` (si no es JSON queda como cadena), en orden de inserción,
// y `json_object_to_json_string` las serializa en el formato «espaciado» de
// json-c. El literal se comprobó con la libjson-c 0.15 que enlaza /usr/bin/ttyd;
// A3 lo vigila contra un ttyd real (`term_ttyd_frames.rs`).
const GOLDEN_PREFS: &str = concat!(
    r##"{ "theme": { "background": "#0A0D13", "foreground": "#EAF0FB", "cursor": "#FFAE1A", "##,
    r##""selectionBackground": "#2E3852" }, "fontSize": 11, "##,
    r##""fontFamily": "Ubuntu Sans Mono, JetBrainsMono Nerd Font Mono, JetBrainsMono Nerd Font, "##,
    r##"JetBrains Mono, DejaVu Sans Mono, monospace", "rendererType": "canvas", "##,
    r##""scrollback": 10000, "cursorBlink": true, "disableLeaveAlert": true, "##,
    r##""disableResizeOverlay": true }"##,
);

#[test]
fn tty_prefs_match_json_c_spaced_output_of_cc_webterm_options() {
    assert_eq!(TTY_PREFS, GOLDEN_PREFS);
    let frame = prefs(TTY_PREFS);
    assert_eq!(frame.first(), Some(&b'2'));
    assert_eq!(&frame[1..], GOLDEN_PREFS.as_bytes());
}

#[test]
fn tty_prefs_carry_what_term_html_reads() {
    // term.html aplica `prefs.fontFamily` y `+prefs.fontSize` del mensaje '2'.
    let v: serde_json::Value = serde_json::from_str(TTY_PREFS).unwrap();
    assert_eq!(v["fontSize"].as_u64(), Some(11));
    assert_eq!(
        v["fontFamily"].as_str(),
        Some(
            "Ubuntu Sans Mono, JetBrainsMono Nerd Font Mono, JetBrainsMono Nerd Font, JetBrains Mono, DejaVu Sans Mono, monospace"
        )
    );
    assert_eq!(v["theme"]["cursor"].as_str(), Some("#FFAE1A"));
    assert_eq!(v["scrollback"].as_u64(), Some(10_000));
    assert_eq!(v["cursorBlink"].as_bool(), Some(true));
}

#[test]
fn tty_title_uses_ttyd_format() {
    // ttyd 1.6.3: sprintf("%c%s (%s)", '1', server->command, hostname).
    assert_eq!(
        tty_title("/home/u/ComandOS/bin/cc-webterm-attach", "box"),
        "/home/u/ComandOS/bin/cc-webterm-attach (box)"
    );
    assert_eq!(
        title(&tty_title("cc-webterm-attach", "host")),
        b"1cc-webterm-attach (host)".to_vec()
    );
}

#[test]
fn ttyd_ui_init_with_token_and_extra_keys_is_accepted() {
    // La UI embebida de ttyd manda el AuthToken real; sin `-c` se ignora.
    let init = parse_init(
        Dialect::Tty,
        br#"{"AuthToken":"dXNlcjpwYXNz","columns":2,"rows":1,"extra":[1,2]}"#,
    )
    .unwrap();
    assert_eq!((init.cols, init.rows), (2, 1));
    let init = parse_init(Dialect::Tty, br#"{"columns":1000,"rows":500}"#).unwrap();
    assert_eq!((init.cols, init.rows), (1000, 500));
}

#[test]
fn init_rejects_malformed_or_hostile_frames() {
    assert_eq!(parse_init(Dialect::Tty, b""), Err(ProtoError::BadJson));
    assert_eq!(parse_init(Dialect::Tty, b"0ls"), Err(ProtoError::BadJson));
    assert_eq!(
        parse_init(Dialect::Tty, br#"[96,30]"#),
        Err(ProtoError::BadSize)
    );
    assert_eq!(
        parse_init(Dialect::Tty, br#"{"columns":1001,"rows":30}"#),
        Err(ProtoError::BadSize)
    );
    assert_eq!(
        parse_init(Dialect::Tty, br#"{"columns":80,"rows":0}"#),
        Err(ProtoError::BadSize)
    );
    assert_eq!(
        parse_init(Dialect::Tty, br#"{"columns":-80,"rows":30}"#),
        Err(ProtoError::BadSize)
    );
    assert_eq!(
        parse_init(Dialect::Tty, br#"{"columns":80.5,"rows":30}"#),
        Err(ProtoError::BadSize)
    );
    assert_eq!(
        parse_init(Dialect::Tty, br#"{"columns":"80","rows":30}"#),
        Err(ProtoError::BadSize)
    );
    assert_eq!(
        parse_init(Dialect::Tty, br#"{"columns":65616,"rows":30}"#),
        Err(ProtoError::BadSize)
    );
    // El dialecto tty no acepta las claves de v1 ni al revés.
    assert_eq!(
        parse_init(Dialect::Tty, br#"{"cols":80,"rows":24}"#),
        Err(ProtoError::BadSize)
    );
    assert_eq!(
        parse_init(Dialect::V1, br#"{"v":1,"columns":80,"rows":24}"#),
        Err(ProtoError::BadSize)
    );
    assert_eq!(
        parse_init(Dialect::V1, br#"{"cols":80,"rows":24}"#),
        Err(ProtoError::BadVersion)
    );
    // Un init enorme se corta antes de tocar el parser.
    let mut big = br#"{"AuthToken":""#.to_vec();
    big.extend(std::iter::repeat_n(b'a', 70_000));
    big.extend_from_slice(br#"","columns":80,"rows":24}"#);
    assert_eq!(parse_init(Dialect::Tty, &big), Err(ProtoError::TooLarge));
    // Anidamiento profundo: error limpio, sin desbordar la pila.
    let deep = [b"[".repeat(4000), b"]".repeat(4000)].concat();
    assert!(parse_init(Dialect::Tty, &deep).is_err());
}

#[test]
fn v1_session_is_optional_and_only_a_string() {
    let init = parse_init(Dialect::V1, br#"{"v":1,"cols":80,"rows":24}"#).unwrap();
    assert_eq!(init.session, None);
    let init = parse_init(Dialect::V1, br#"{"v":1,"cols":80,"rows":24,"session":7}"#).unwrap();
    assert_eq!(init.session, None);
}

#[test]
fn client_frames_are_bounded_and_strict() {
    assert_eq!(parse_client(b""), Err(ProtoError::Unknown));
    assert_eq!(parse_client(b"{"), Err(ProtoError::Unknown));
    // Entrada binaria arbitraria (no UTF-8) pasa intacta.
    assert_eq!(
        parse_client(b"0\xff\x00\x1b[A").unwrap(),
        ClientMsg::Input(b"\xff\x00\x1b[A".to_vec())
    );
    assert_eq!(parse_client(b"0").unwrap(), ClientMsg::Input(Vec::new()));
    // Justo en el límite: 1 MiB de carga + el prefijo (un pegado grande en una
    // sola trama, como lo manda term.html); un byte más es `TooLarge`.
    assert_eq!(MAX_INPUT, 1024 * 1024);
    assert_eq!(MAX_CLIENT_FRAME, MAX_INPUT + 1);
    let mut edge = vec![b'0'];
    edge.extend(std::iter::repeat_n(b'x', MAX_INPUT));
    assert_eq!(edge.len(), MAX_CLIENT_FRAME);
    assert!(matches!(parse_client(&edge), Ok(ClientMsg::Input(v)) if v.len() == MAX_INPUT));
    edge.push(b'x');
    assert_eq!(parse_client(&edge), Err(ProtoError::TooLarge));
    assert_eq!(parse_client(b"1nope"), Err(ProtoError::BadJson));
    assert_eq!(
        parse_client(br#"1{"columns":1,"rows":40}"#),
        Err(ProtoError::BadSize)
    );
    // Un resize enorme no llega al parser JSON.
    let mut big = b"1".to_vec();
    big.extend(std::iter::repeat_n(b' ', 10_000));
    big.extend_from_slice(br#"{"columns":80,"rows":24}"#);
    assert_eq!(parse_client(&big), Err(ProtoError::TooLarge));
}

#[test]
fn tty_args_follow_ttyd_url_arg() {
    // Solo los campos `arg=…` (ttyd compara con "arg="); `+` es espacio.
    assert_eq!(
        tty_args("arg=a+b&arg&arg=&argx=1&token=t&arg=%7E%2F"),
        vec!["a b".to_string(), String::new(), "~/".to_string()]
    );
    assert!(tty_args("").is_empty());
    assert!(tty_args("auth=x&theme=dark").is_empty());
}

#[test]
fn errors_render_in_spanish() {
    assert_eq!(
        ProtoError::BadSize.to_string(),
        "tamaño de terminal fuera de rango"
    );
    let err: &dyn std::error::Error = &ProtoError::TooLarge;
    assert!(!err.to_string().is_empty());
}

#[test]
fn a_one_mib_paste_passes_whole_and_one_byte_more_is_too_large() {
    // Pegado de 1 MiB con saltos de línea y UTF-8, como lo manda `term.html`.
    let paste: Vec<u8> = "línea de pegado\n"
        .bytes()
        .cycle()
        .take(MAX_INPUT)
        .collect();
    let mut frame = vec![b'0'];
    frame.extend_from_slice(&paste);
    assert_eq!(parse_client(&frame), Ok(ClientMsg::Input(paste)));
    frame.push(b'x');
    assert_eq!(parse_client(&frame), Err(ProtoError::TooLarge));
}
