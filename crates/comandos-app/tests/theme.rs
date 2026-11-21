#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use comandos_app::theme::{
    base_css, button_style_css, desktop_theme, header_css, is_dark, prefs_theme, theme_css,
    themes_from_file,
};

#[test]
fn config_themes_produce_desktop_tokens_for_all_nine_names() {
    let raw = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/themes.json"
    ));
    let themes = themes_from_file(Some(raw));
    for name in [
        "noche",
        "dia",
        "calido",
        "termius",
        "bruno",
        "superglass",
        "neon",
        "contraste",
        "ubuntu",
    ] {
        let tokens = desktop_theme(name, &themes).expect(name);
        assert_eq!(tokens.ansi.len(), 16);
        assert!(tokens.ansi.iter().all(|color| color.starts_with('#')));
        assert!(tokens.values.contains_key("name"));
        assert!(!theme_css(&tokens).contains("@CC_THEME_"));
        assert!(theme_css(&tokens).contains(".tabstrip"));
    }
}

#[test]
fn five_button_styles_and_fallback_emit_header_button_css() {
    let raw = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/themes.json"
    ));
    let themes = themes_from_file(Some(raw));
    let tokens = desktop_theme("noche", &themes).unwrap();
    let sutil = button_style_css("sutil", &tokens);
    assert_eq!(button_style_css("nope", &tokens), sutil);
    for style in ["sutil", "arcade", "tecla", "pixel", "consola"] {
        let css = button_style_css(style, &tokens);
        assert!(css.contains("button.cc-key"));
        assert!(css.contains("scrollbar button"));
    }
}

#[test]
fn three_css_layers_match_full_original_ast_for_all_themes() {
    use comandos_app::proc::{ProcSpec, run};
    use serde_json::json;
    let themes = themes_from_file(Some(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/themes.json"
    ))));
    let names = [
        "noche",
        "dia",
        "calido",
        "termius",
        "bruno",
        "superglass",
        "neon",
        "contraste",
        "ubuntu",
    ];
    let tokens: Vec<_> = names
        .iter()
        .map(|name| desktop_theme(name, &themes).unwrap())
        .collect();
    let script = r#"import ast,json,sys
nodes=ast.parse(open(sys.argv[1]).read()).body
exec(compile(ast.Module(body=[n for n in nodes if isinstance(n,ast.FunctionDef) and n.name in {'_build_hb_css','button_style_css','theme_css'}],type_ignores=[]),sys.argv[1],'exec'))
APP_CSS=ast.literal_eval(next(n.value for n in nodes if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='APP_CSS' for t in n.targets)))
_ICONS_DIR=sys.argv[2]
out=[]
for THEME in json.load(sys.stdin):
 out.append({'app':theme_css(THEME),'header':_build_hb_css().decode(),'buttons':[button_style_css(style,THEME).decode() for style in ['sutil','arcade','tecla','pixel','consola']]})
print(json.dumps(out))
"#;
    let out = run(&ProcSpec {
        program: "python3".into(),
        args: vec![
            "-c".into(),
            script.into(),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../bin/cc-app").into(),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../dash/icons").into(),
        ],
        stdin: Some(
            serde_json::to_vec(&tokens.iter().map(|t| &t.values).collect::<Vec<_>>()).unwrap(),
        ),
        env: vec![
            (
                "HOME".into(),
                "/tmp/comandos-theme-oracle-private-home".into(),
            ),
            ("PATH".into(), "/usr/bin:/bin".into()),
        ],
        clear_env: true,
        env_remove: vec![],
        cwd: None,
        timeout: std::time::Duration::from_secs(5),
    })
    .unwrap();
    assert_eq!(
        out.code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let oracle: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let actual: Vec<_> = tokens
        .iter()
        .map(|t| {
            let buttons: Vec<_> = ["sutil", "arcade", "tecla", "pixel", "consola"]
                .iter()
                .map(|style| button_style_css(style, t))
                .collect();
            json!({"app":base_css(t),"header":header_css(t),"buttons":buttons})
        })
        .collect();
    assert_eq!(json!(actual), oracle);
}

#[test]
fn cold_start_theme_matches_original_when_preferences_are_unavailable() {
    use comandos_app::proc::{ProcSpec, run};
    let script = r#"import ast,sys
nodes=ast.parse(open(sys.argv[1]).read()).body
node=next(n for n in nodes if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='THEME' for t in n.targets))
scope={'PREFS':{},'THEMES':{'noche':'noche','bruno':'bruno'}}
exec(compile(ast.Module(body=[node],type_ignores=[]),sys.argv[1],'exec'),scope)
print(scope['THEME'])
"#;
    let env = [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "XDG_RUNTIME_DIR",
        "TMPDIR",
        "TMP",
        "TEMP",
    ]
    .into_iter()
    .map(|key| {
        (
            key.into(),
            std::env::var_os(key).expect("private oracle environment"),
        )
    })
    .collect();
    let oracle = run(&ProcSpec {
        program: "/usr/bin/python3".into(),
        args: vec![
            "-c".into(),
            script.into(),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../bin/cc-app").into(),
        ],
        stdin: None,
        env,
        clear_env: true,
        env_remove: vec![],
        cwd: None,
        timeout: std::time::Duration::from_secs(5),
    })
    .unwrap();
    assert_eq!(oracle.code, Some(0));
    assert_eq!(
        comandos_app::theme::DEFAULT_THEME,
        std::str::from_utf8(&oracle.stdout).unwrap().trim()
    );
}

#[test]
fn user_theme_paints_its_colours_over_its_base() {
    let themes = themes_from_file(Some(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/themes.json"
    ))));
    let base = desktop_theme("dia", &themes).unwrap();
    let mut pal = base.ansi.clone();
    pal[1] = "#123456".into();
    let prefs = serde_json::json!({
        "theme": "u-mio",
        "user_themes": [{"id": "u-mio", "base": "dia", "colors": {
            "brand": "#ff00aa", "bg": "#101010", "name": "#000000", "pal": pal,
            "nope": "#ffffff", "text": "blue"
        }}]
    });
    let t = prefs_theme(&prefs, &themes).unwrap();
    assert_eq!(t.values["brand"], "#FF00AA");
    assert_eq!(t.values["bg"], "#101010");
    assert_eq!(
        t.values["text"], base.values["text"],
        "invalid colours are ignored"
    );
    assert_eq!(t.values["name"], "dia", "state roles come from the base");
    assert!(!t.values.contains_key("nope"));
    assert_eq!(t.ansi[1], "#123456");
    assert!(is_dark(&t));
    assert!(!is_dark(&base));
    // An unknown user theme falls back like any unknown name.
    let missing = serde_json::json!({"theme": "u-gone"});
    assert_eq!(
        prefs_theme(&missing, &themes).unwrap().values["bg"],
        desktop_theme("noche", &themes).unwrap().values["bg"]
    );
}
