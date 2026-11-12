//! Compiled dashboard shell. It includes no coordinator or legacy scripts.
//! The caller supplies the native WASM loader separately after packaging.
use maud::{Markup, PreEscaped, html};

pub const CSS: &str = include_str!("index_page.css");
pub const HEAD: &str = include_str!("index_page_head.html");
pub const LINKS: &str = include_str!("index_page_links.html");
pub const BODY: &str = include_str!("index_page_body.html");

/// Original static DOM, metadata and cascade order, with an explicit document.
/// This shell deliberately does not claim that the coordinator is all ported.
pub fn shell(lang: &str) -> Markup {
    html! {
        (maud::DOCTYPE)
        html lang=(lang) {
            head { (PreEscaped(HEAD)) (PreEscaped("<style>")) (PreEscaped(CSS)) (PreEscaped("</style>")) (PreEscaped(LINKS)) }
            body { (PreEscaped(BODY)) }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    const ORIGINAL: &str = include_str!("../../../dash/index.html");
    #[test]
    fn static_bytes_and_cascade_match_original() {
        let (head, rest) = ORIGINAL.split_once("<style>").unwrap();
        let (css, rest) = rest.split_once("</style>").unwrap();
        let links = rest.split_once("<script").unwrap().0;
        let body = ORIGINAL
            .split_once("<div id=\"panes\">")
            .unwrap()
            .1
            .split_once("<script>")
            .unwrap()
            .0;
        assert_eq!(HEAD, head);
        assert_eq!(CSS, css);
        assert_eq!(LINKS, links);
        assert_eq!(BODY, format!("<div id=\"panes\">{body}"));
        let rendered = shell("es").into_string();
        assert!(rendered.contains(&format!("<style>{CSS}</style>{LINKS}")));
        assert!(rendered.contains(BODY));
    }
    #[test]
    fn standalone_render_needs_no_original_scripts_or_executable_attributes() {
        let rendered = shell("en\" onload=\"bad").into_string();
        assert!(!rendered.contains("<script"));
        assert!(!rendered.contains("onclick="));
        assert!(!rendered.contains("eval("));
        assert!(rendered.contains("lang=\"en&quot; onload=&quot;bad\""));
        for id in [
            "panes",
            "tabbar",
            "settings",
            "remote",
            "servers",
            "command-sidebar",
            "session-overview",
        ] {
            assert!(rendered.contains(&format!("id=\"{id}\"")), "{id}");
        }
    }
    #[test]
    fn retired_modules_are_absent_from_the_shell() {
        let rendered = shell("es").into_string();
        for id in [
            "usage",
            "an-root",
            "btn-usage",
            "btn-notif",
            "notif-badge",
            "notif-panel",
            "notice-settings",
            "btn-pomo",
            "pomo-panel",
        ] {
            assert!(!rendered.contains(&format!("id=\"{id}\"")), "{id}");
        }
        for css in ["#pomo-panel", "#notif-panel", ".nf2", "data-only-panel=\"usage\""] {
            assert!(!CSS.contains(css), "{css}");
        }
        assert!(!LINKS.contains("analytics.css"));
    }
}
