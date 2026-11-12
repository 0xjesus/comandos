//! Extrae los cuerpos de producción y sustituye únicamente colaboradores de GUI/HTTP.
#![allow(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]
use comandos_app::proc::{ProcSpec, run};
use std::path::{Path, PathBuf};
fn body(source: &str, needle: &str) -> String {
    let start = source.find(needle).expect(needle);
    let brace = start + source[start..].find('{').unwrap();
    let mut level = 0;
    for (i, b) in source.as_bytes().iter().enumerate().skip(brace) {
        if *b == b'{' {
            level += 1;
        } else if *b == b'}' {
            level -= 1;
            if level == 0 {
                return source[start..=i]
                    .replace("pub(super) ", "")
                    .replace("pub(in crate::ui) ", "");
            }
        }
    }
    panic!("unclosed {needle}")
}
fn library(dir: &Path, name: &str) -> PathBuf {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|f| f.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "rlib")
                && p.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with(&format!("lib{name}-"))
        })
        .max_by_key(|p| p.metadata().unwrap().modified().unwrap())
        .unwrap()
}
fn dependency(deps: &Path, app: &Path, name: &str) -> PathBuf {
    let hash = app
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .strip_prefix("libcomandos_app-")
        .unwrap();
    let fingerprints = deps.parent().unwrap().join(".fingerprint");
    let meta: serde_json::Value = serde_json::from_slice(
        &std::fs::read(fingerprints.join(format!("comandos-app-{hash}/lib-comandos_app.json")))
            .unwrap(),
    )
    .unwrap();
    let wanted = meta["deps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d[1] == name)
        .unwrap()[3]
        .as_u64()
        .unwrap();
    for dir in std::fs::read_dir(&fingerprints).unwrap().flatten() {
        let encoded = dir.path().join(format!("lib-{name}"));
        if let Ok(text) = std::fs::read_to_string(encoded) {
            let bytes: Vec<_> = (0..8)
                .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap())
                .collect();
            if u64::from_le_bytes(bytes.try_into().unwrap()) == wanted {
                let folder = dir.file_name();
                let suffix = folder.to_str().unwrap().rsplit('-').next().unwrap();
                let library = deps.join(format!("lib{name}-{suffix}.rlib"));
                if library.exists() {
                    return library;
                }
            }
        }
    }
    panic!("missing matched dependency {name}")
}
pub fn execute() -> serde_json::Value {
    let dir = std::env::var_os("COMANDOS_T14_PROBE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("comandos-t14-probe-{}", std::process::id()))
        });
    std::fs::create_dir_all(&dir).unwrap();
    let app = include_str!("../../src/ui/app.rs");
    let adapter = include_str!("../../src/ui/app_t14.rs");
    let mut methods = body(app, "fn writable(") + &body(app, "fn toggle_favorite(");
    for name in [
        "install_header",
        "dashboard_script",
        "dashboard_click",
        "fetch_marks",
        "set_work_mark",
        "paint_marks",
        "merge_pending_favorites",
        "queue_favorite",
        "apply_pending_favorites",
        "post_next_favorite",
        "append_mark_menu",
        "append_mark_menu_when",
        "tick_hourglass",
    ] {
        methods.push_str(&body(adapter, &format!("fn {name}(")));
    }
    let icon_source = include_str!("../../src/ui/icons.rs");
    let icons = (body(icon_source, "pub fn image(") + &body(icon_source, "pub fn button("))
        .replace(
            "../../../../dash/icons",
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../dash/icons")
                .canonicalize()
                .unwrap()
                .display()
                .to_string(),
        );
    let header_source = include_str!("../../src/ui/header.rs");
    let header = body(header_source, "pub struct Header {")
        + &body(header_source, "impl Header {");
    let tab_source = include_str!("../../src/ui/tab_label.rs");
    let mut tab_methods = String::new();
    for name in ["set_language", "set_favorite", "paint_favorite"] {
        tab_methods.push_str(&body(tab_source, &format!("fn {name}(")));
    }
    let tab = format!(
        r#"mod tab_model {{use crate::{{gtk,Rc,Cell,RefCell}};mod super_marks{{pub use comandos_app::ui::marks::*;}}struct TabLabel{{english:Rc<Cell<bool>>,favorite:Option<crate::Button>,favorite_on:Rc<Cell<bool>>,favorite_frame:Rc<Cell<usize>>,cache:Rc<RefCell<super_marks::IndicatorCache>>,indicator:crate::Image,last_seconds:Rc<Cell<f64>>}}impl TabLabel{{fn paint(&self,_:f64){{}}{tab_methods}}}pub fn verify(){{let button=crate::Button::default();let tab=TabLabel{{english:Rc::new(Cell::new(false)),favorite:Some(button.clone()),favorite_on:Rc::new(Cell::new(false)),favorite_frame:Rc::new(Cell::new(usize::MAX)),cache:Rc::new(RefCell::new(super_marks::IndicatorCache::default())),indicator:crate::Image::new(),last_seconds:Rc::new(Cell::new(0.))}};tab.set_favorite(true);assert_eq!(button.0.2.tooltip.borrow().as_str(),"Quitar de favoritos");tab.set_language(true);assert_eq!(button.0.2.tooltip.borrow().as_str(),"Remove from favorites");tab.set_favorite(false);assert_eq!(button.0.2.tooltip.borrow().as_str(),"Add to favorites");tab.set_language(false);assert_eq!(button.0.2.tooltip.borrow().as_str(),"Marcar como favorita");}}}}"#
    );
    let extra = format!(
        "{tab}mod icons{{use crate::gtk;use gdk_pixbuf::prelude::*;{icons}}}mod header_model{{use crate::{{gtk,glib,gdk}};{header}{} }}",
        include_str!("t14_header_main.rs.txt")
    );
    let source = format!(
        "{}\nimpl App{{{methods}}}\n{extra}\n{}",
        include_str!("t14_probe_preamble.rs.txt"),
        include_str!("t14_probe_main.rs.txt")
    );
    let path = dir.join("main.rs");
    std::fs::write(&path, &source).unwrap();
    let deps = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let app_lib = library(&deps, "comandos_app");
    let exe = dir.join("probe");
    let flags = run(&ProcSpec {
        program: "pkg-config".into(),
        args: vec!["--libs-only-L".into(), "webkit2gtk-4.1".into()],
        stdin: None,
        env: vec![],
        clear_env: false,
        env_remove: vec![],
        cwd: None,
        timeout: std::time::Duration::from_secs(5),
    })
    .unwrap();
    assert_eq!(flags.code, Some(0), "SDK linker paths");
    let mut arguments = vec![
        "--edition=2024".into(),
        path.into_os_string(),
        "-L".into(),
        format!("dependency={}", deps.display()).into(),
        "--extern".into(),
        format!("comandos_app={}", app_lib.display()).into(),
        "--extern".into(),
        format!(
            "serde_json={}",
            dependency(&deps, &app_lib, "serde_json").display()
        )
        .into(),
        "--extern".into(),
        format!(
            "comandos_core={}",
            dependency(&deps, &app_lib, "comandos_core").display()
        )
        .into(),
        "-o".into(),
        exe.clone().into_os_string(),
    ];
    for name in ["glib", "gdk_pixbuf", "gdk"] {
        arguments.push("--extern".into());
        arguments.push(format!("{name}={}", dependency(&deps, &app_lib, name).display()).into());
    }
    for flag in String::from_utf8(flags.stdout).unwrap().split_whitespace() {
        if let Some(path) = flag.strip_prefix("-L") {
            arguments.push("-L".into());
            arguments.push(format!("native={path}").into());
        }
    }
    let output = run(&ProcSpec {
        program: "rustc".into(),
        args: arguments,
        stdin: None,
        env: vec![],
        clear_env: false,
        env_remove: vec![],
        cwd: None,
        timeout: std::time::Duration::from_secs(60),
    })
    .unwrap();
    std::fs::write(dir.join("compile.log"), &output.stderr).unwrap();
    assert_eq!(
        output.code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = run(&ProcSpec {
        program: exe.display().to_string(),
        args: vec![],
        stdin: None,
        env: vec![],
        clear_env: false,
        env_remove: vec![],
        cwd: None,
        timeout: std::time::Duration::from_secs(10),
    })
    .unwrap();
    std::fs::write(dir.join("run.log"), &output.stdout).unwrap();
    std::fs::write(dir.join("run.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap()
}
