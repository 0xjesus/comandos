//! Opens one pane-native terminal (tmux control mode) on a private tmux
//! server and saves pictures of it: live, after typing and scrolled back.
//! `control_shot <socket> <session> <out-prefix>`; needs a display.
use comandos_app::{
    config::RunMode,
    term::{
        engine::Palette,
        view::{TermOptions, TermView},
    },
};
use gtk::prelude::*;
use std::{rc::Rc, time::Duration};

fn shot(window: &gtk::Window, path: &str) {
    let Some(w) = window.window() else { return };
    let (width, height) = (w.width(), w.height());
    if let Some(pixbuf) = w.pixbuf(0, 0, width, height) {
        let _ = pixbuf.savev(path, "png", &[]);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (socket, session, out) = (args[1].clone(), args[2].clone(), args[3].clone());
    gtk::init().expect("display");
    let attach = |flags: &str| {
        vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            format!("tmux -S '{socket}' {flags}attach -t '={session}'; echo '[fin]'; exec cat"),
        ]
    };
    // `CONTROL_SHOT_CURSOR=r,g,b`: the cursor in a theme's colour.
    let cursor = std::env::var("CONTROL_SHOT_CURSOR")
        .ok()
        .and_then(|v| {
            let mut n = v.split(',').filter_map(|p| p.trim().parse::<u8>().ok());
            Some([n.next()?, n.next()?, n.next()?])
        })
        .unwrap_or([211, 218, 228]);
    let palette = Palette::xterm_default([211, 218, 228], [10, 13, 19], cursor, [10, 13, 19], [60, 80, 120]);
    let term = TermView::new(TermOptions {
        argv: attach(""),
        control_argv: Some(attach("-CC ")),
        cwd: std::env::temp_dir(),
        session: Some(session.clone()),
        palette,
        scrollback: 4000,
        preferences: serde_json::json!({}),
        mode: RunMode::Live,
        environment: vec![],
        clear_env: false,
        tmux_size: None,
        shadow_tmux: None,
        before_spawn: None,
        on_title: None,
        on_exit: None,
        on_bell: None,
        on_link: None,
        on_ssh_scroll: None,
    })
    .expect("terminal");
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_default_size(900, 420);
    window.add(term.widget());
    window.show_all();
    let term = Rc::new(term);
    let step = |ms: u64, f: Box<dyn Fn()>| {
        glib::timeout_add_local_once(Duration::from_millis(ms), move || f());
    };
    // `CONTROL_SHOT_MOUSE=<ms,ms,…>`: no scripted typing; something outside
    // (xdotool) moves the mouse and pictures are taken at those times.
    if let Ok(times) = std::env::var("CONTROL_SHOT_MOUSE") {
        let times: Vec<u64> = times.split(',').filter_map(|t| t.parse().ok()).collect();
        let last = times.iter().copied().max().unwrap_or(1000);
        for (i, ms) in times.into_iter().enumerate() {
            let (t, w, o) = (term.clone(), window.clone(), out.clone());
            step(ms, Box::new(move || {
                shot(&w, &format!("{o}-mouse-{i}.png"));
                println!("shot {i}: selection={:?}", t.selection_text());
                if std::env::var_os("CONTROL_SHOT_CURSOR").is_some() {
                    let state = t.diagnostic_snapshot().ok();
                    println!("shot {i}: cursor={} focused={:?}", state.as_ref().map_or("?".into(), |s| s["cursor"].to_string()), state.as_ref().map(|s| s["focused"].clone()));
                }
            }));
        }
        step(last + 300, Box::new(gtk::main_quit));
        gtk::main();
        return;
    }
    let (t, w, o) = (term.clone(), window.clone(), out.clone());
    step(2500, Box::new(move || {
        println!("native={} grid={:?}", t.pane_native(), t.grid_size());
        shot(&w, &format!("{o}-1-live.png"));
        let _ = t.feed("echo hola-\u{f1} y seguimos\r".as_bytes());
    }));
    let (t, w, o) = (term.clone(), window.clone(), out.clone());
    step(3500, Box::new(move || {
        shot(&w, &format!("{o}-2-typed.png"));
        t.scroll_lines(40);
    }));
    for (i, ms) in [3560u64, 3620, 3700].into_iter().enumerate() {
        let (w, o) = (window.clone(), out.clone());
        step(ms, Box::new(move || shot(&w, &format!("{o}-glide-{i}.png"))));
    }
    let (t, w, o) = (term.clone(), window.clone(), out.clone());
    step(4200, Box::new(move || {
        shot(&w, &format!("{o}-3-scrolled.png"));
        // Output keeps arriving while the view stays on the old lines.
        let _ = t.feed(b"seq 900 905\r");
    }));
    let (t, w, o) = (term.clone(), window.clone(), out.clone());
    step(5000, Box::new(move || {
        shot(&w, &format!("{o}-4-scrolled-after-output.png"));
        println!("native={}", t.pane_native());
        gtk::main_quit();
    }));
    gtk::main();
}
