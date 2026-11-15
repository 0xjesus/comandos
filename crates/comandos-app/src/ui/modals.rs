//! Modal policy shared by native panels and mode-profiled web views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    Url,
    Center,
    Help,
    Snippets,
}
/// Name of the window that shows a dashboard panel in the middle of the app.
pub const CENTER: &str = "chains";
pub fn modal_size(kind: ModalKind, width: i32, height: i32) -> (i32, i32) {
    match kind {
        ModalKind::Center => (
            (width - 120).clamp(640, 1040),
            (height - 90).clamp(420, 700),
        ),
        _ => ((width - 120).max(640), (height - 90).max(420)),
    }
}
pub fn close_allowed(dismissable: bool, escape: bool, backdrop: bool) -> bool {
    dismissable && (escape || backdrop)
}
pub fn show_modal_panel(
    app: &std::rc::Rc<super::app::App>,
    panel: &gtk::Widget,
    dismissable: bool,
    on_close: std::rc::Rc<dyn Fn()>,
) {
    app.mount_custom_panel(panel, dismissable, on_close);
}
