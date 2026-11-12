//! Modal policy shared by native panels and mode-profiled web views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    Url,
    Help,
    Snippets,
}
pub fn modal_size(_kind: ModalKind, width: i32, height: i32) -> (i32, i32) {
    ((width - 120).max(640), (height - 90).max(420))
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
