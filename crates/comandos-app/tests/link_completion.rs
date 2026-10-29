#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]
//! Links split over two lines are completed from the clicked pane's own text, also
//! when the pane is scrolled back in copy mode (the mouse wheel enters it).
mod support;
use comandos_app::{config::RunMode, term::links, ui::menu::complete_in_pane};

#[test]
fn copy_mode_shows_the_lines_scrolled_back_to() {
    assert!(links::shown_lines("0||40").is_empty());
    assert!(links::shown_lines("1|0|40").is_empty());
    assert_eq!(links::shown_lines("1|22|10\n"), ["-S", "-25", "-E", "-10"]);
    assert_eq!(links::recent_lines(), ["-S", "-2000"]);
}

#[test]
fn a_split_path_read_after_scrolling_back_opens_whole() {
    let Some(fx) = support::tmux::TestTmux::for_mode(RunMode::Live) else {
        return;
    };
    // 29-oct: the agent's path ran to the edge and went on in the next line; read after
    // scrolling back, opening it gave «Ruta local invalida».
    let dir = std::env::temp_dir().join(format!("link-completion-{}", std::process::id()));
    let folder = dir.join("edc28eb5-aa4b-4707-90f6-d33572dbe5be/scratchpad/parrilla");
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("Parrilla_de_contenido_Jumamex_29oct-13nov.docx");
    std::fs::write(&file, b"x").unwrap();
    let full = file.to_str().unwrap().to_owned();
    let cut = full.find("parrilla/").unwrap() + "parril".len();
    let (head, tail) = full.split_at(cut);
    let script = dir.join("print.sh");
    std::fs::write(
        &script,
        format!(
            "printf '  Ya cambie el documento:\\n  {head}\\n  {tail}\\n'\n\
             i=0; while [ $i -lt 40 ]; do echo \"linea $i\"; i=$((i+1)); done\nsleep 60\n"
        ),
    )
    .unwrap();
    let out = fx.tmux.raw(&[
        "new-session", "-d", "-s", "lk", "-x", "120", "-y", "10", "/bin/sh",
        script.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let pane = String::from_utf8(fx.tmux.raw(&["display-message", "-p", "-t", "=lk:", "#{pane_id}"]).stdout)
        .unwrap()
        .trim()
        .to_owned();
    for _ in 0..100 {
        let shown = fx.tmux.raw(&["capture-pane", "-p", "-t", &pane]).stdout;
        if String::from_utf8_lossy(&shown).contains("linea 39") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // At the bottom the path has already scrolled off: only the history has it.
    assert_eq!(complete_in_pane(&fx.ctl, &pane, tail).as_deref(), Some(full.as_str()));
    // Scrolled back to it in copy mode, from either half.
    fx.tmux.raw(&["copy-mode", "-t", &pane]);
    fx.tmux.raw(&["send-keys", "-t", &pane, "-X", "history-top"]);
    for half in [head.trim_start(), tail] {
        assert_eq!(complete_in_pane(&fx.ctl, &pane, half).as_deref(), Some(full.as_str()), "{half}");
    }
    // A fragment that is nowhere in the pane stays as it was.
    assert_eq!(complete_in_pane(&fx.ctl, &pane, "otra/cosa.docx"), None);
    std::fs::remove_dir_all(&dir).unwrap();
}
