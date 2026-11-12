use crate::config::AppConfig;
use gtk::prelude::*;

pub fn create(application: &gtk::Application, cfg: &AppConfig) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::new(application);
    window.set_title(cfg.title());
    window.set_default_size(1600, 880);
    gdk::set_program_class(cfg.wm_class());
    window.set_icon_name(Some("centro-claude"));
    window.set_decorated(false);
    window.set_position(gtk::WindowPosition::Center);
    window.set_widget_name(cfg.wm_class());
    window.set_app_paintable(true);
    window.connect_realize(|window| disable_frame_sync(window));
    if let Some(screen) = gdk::Screen::default()
        && let Some(visual) = screen.rgba_visual()
    {
        window.set_visual(Some(&visual));
    }
    window
}

/// On X11 GTK freezes the window's frame clock after every painted frame until
/// mutter answers `_NET_WM_FRAME_DRAWN`. Mutter answers on its next composite,
/// and only after 6 refresh intervals (100 ms at 60 Hz) when it thinks the
/// window is covered; with the tab and feed animations repainting the window
/// all the time, terminal output waited 40-100 ms for its frame
/// (`~/.cache/comandos/lag.log`: `frame … tick≈wait clock_gap≈100`). Without
/// frame sync the clock paces itself at 60 Hz and mutter composites the latest
/// picture, like any X11 app that does not speak the protocol.
fn disable_frame_sync(window: &gtk::ApplicationWindow) {
    if let Some(x11) = window
        .window()
        .and_then(|w| w.downcast::<gdkx11::X11Window>().ok())
    {
        x11.set_frame_sync_enabled(false);
    }
}

pub fn header(window: &gtk::ApplicationWindow) -> gtk::HeaderBar {
    let header = gtk::HeaderBar::new();
    header.set_show_close_button(false);
    header.style_context().add_class("cc-headerbar");
    let brand = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    brand.pack_start(&gtk::Label::new(Some("●")), false, false, 0);
    let name = gtk::Label::new(Some("COMANDOS"));
    name.style_context().add_class("cc-brand");
    brand.pack_start(&name, false, false, 0);
    header.pack_start(&brand);
    for (icon, action) in [
        ("window-close-symbolic", 0),
        ("window-maximize-symbolic", 1),
        ("window-minimize-symbolic", 2),
    ] {
        let button = gtk::Button::from_icon_name(Some(icon), gtk::IconSize::Button);
        let weak = window.downgrade();
        button.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                match action {
                    0 => window.close(),
                    1 => {
                        if window.is_maximized() {
                            window.unmaximize();
                        } else {
                            window.maximize();
                        }
                    }
                    _ => window.iconify(),
                }
            }
        });
        header.pack_end(&button);
    }
    header.add_events(gdk::EventMask::BUTTON_PRESS_MASK);
    let weak = window.downgrade();
    header.connect_button_press_event(move |_, event| {
        if event.button() == 1
            && let Some(window) = weak.upgrade()
        {
            let (x, y) = event.root();
            window.begin_move_drag(1, x as i32, y as i32, event.time());
        }
        glib::Propagation::Proceed
    });
    header
}

#[derive(Debug)]
pub enum InstanceError {
    Guard(crate::guard::GuardError),
    Contended,
    Io(String),
}
pub fn instance_lock(
    cfg: &AppConfig,
    guard: &crate::guard::WriteGuard,
    display: &str,
) -> Result<nix::fcntl::Flock<std::fs::File>, InstanceError> {
    use nix::fcntl::{Flock, FlockArg};
    let base = cfg.sandbox_root().unwrap_or(cfg.runtime_dir());
    let file = guard
        .open_lock(&base.join(cfg.lock_file_name(display)))
        .map_err(InstanceError::Guard)?;
    Flock::lock(file, FlockArg::LockExclusiveNonblock).map_err(|(_, error)| {
        if error == nix::errno::Errno::EWOULDBLOCK {
            InstanceError::Contended
        } else {
            InstanceError::Io(error.to_string())
        }
    })
}
pub fn activate_existing(
    cfg: &AppConfig,
    runner: &dyn Fn(
        &crate::proc::ProcSpec,
    ) -> Result<crate::proc::ProcOutput, crate::proc::ProcError>,
) -> bool {
    runner(&crate::proc::ProcSpec {
        program: "wmctrl".into(),
        args: vec![
            "-x".into(),
            "-a".into(),
            format!("{}.{}", cfg.wm_class(), cfg.wm_class()).into(),
        ],
        stdin: None,
        env: vec![],
        clear_env: false,
        env_remove: vec![],
        cwd: None,
        timeout: std::time::Duration::from_secs(5),
    })
    .is_ok_and(|output| output.code == Some(0))
}
