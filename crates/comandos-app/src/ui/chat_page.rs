//! The desktop chat's messages and composer as one web page (`chat_page.html`), drawn
//! by WebKit from the HTML the server already renders for each message: markdown,
//! highlighted code, tables, copy and edit in place, an approval card and a composer
//! that takes files and pasted images. The header with the pane chips stays native.
use crate::theme::ThemeTokens;
use javascriptcore::ValueExt;
use serde_json::{Value, json};
use webkit2gtk::{
    NavigationPolicyDecisionExt, PolicyDecisionExt, SettingsExt, URIRequestExt, UserContentManagerExt,
    WebViewExt,
};
use gtk::prelude::*;

pub const DOCUMENT: &str = include_str!("chat_page.html");
/// Base the page loads at: the only address it may show; links open in the browser.
const BASE: &str = "comandos-chat://page/";

/// A message from the page: `{type, …}`.
pub fn parse(raw: &str) -> Option<Value> {
    serde_json::from_str::<Value>(raw).ok().filter(|v| v["type"].is_string())
}

/// The page's colors from a theme (missing tokens keep the page's defaults).
pub fn theme_vars(theme: Option<&ThemeTokens>) -> Value {
    let Some(theme) = theme else { return json!({}) };
    let token = |k: &str| theme.values.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    json!({
        "bg": token("bg"), "panel": token("panel"), "panel2": token("panel2"),
        "line": token("line"), "line2": token("line2"), "text": token("text"),
        "dim": token("dim"), "faint": token("faint"), "brand": token("brand"),
        "on-brand": token("onBrand"), "waiting": token("waiting"), "working": token("working"),
        "ok": token("ok"), "warn": token("warn"), "err": token("err"),
    })
}

/// `CV.<function>(<json>)`, as a statement that returns nothing to WebKit.
pub fn call(function: &str, arg: &Value) -> String {
    format!("window.CV&&CV.{function}({arg});void 0;")
}

/// A view showing the page; `on_message` gets each message the page posts.
pub fn view(on_message: impl Fn(Value) + 'static) -> webkit2gtk::WebView {
    let content = webkit2gtk::UserContentManager::new();
    content.register_script_message_handler("chat");
    content.connect_script_message_received(Some("chat"), move |_, result| {
        if let Some(message) = result.js_value().and_then(|v| parse(v.to_str().as_str())) {
            on_message(message);
        }
    });
    let view = webkit2gtk::WebView::builder().user_content_manager(&content).build();
    view.set_hexpand(true);
    view.set_vexpand(true);
    if let Some(settings) = WebViewExt::settings(&view) {
        settings.set_enable_developer_extras(false);
        settings.set_enable_write_console_messages_to_stdout(true);
        settings.set_javascript_can_access_clipboard(true);
    }
    // The page never navigates: links go to the browser (the page posts them too).
    view.connect_decide_policy(|_, decision, _| {
        if let Some(navigation) = decision.downcast_ref::<webkit2gtk::NavigationPolicyDecision>()
            && let Some(uri) = navigation.navigation_action().and_then(|a| a.request()).and_then(|r| r.uri())
            && !uri.starts_with(BASE)
            && uri != "about:blank"
        {
            decision.ignore();
            return true;
        }
        false
    });
    view.load_html(DOCUMENT, Some(BASE));
    view
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_messages_need_a_type() {
        assert_eq!(parse(r#"{"type":"send","text":"hola"}"#).unwrap()["text"], "hola");
        assert!(parse(r#"{"text":"x"}"#).is_none());
        assert!(parse("nope").is_none());
    }

    #[test]
    fn calls_are_statements_with_json_arguments() {
        assert_eq!(call("files", &json!([{"path":"/a b.png"}])), r#"window.CV&&CV.files([{"path":"/a b.png"}]);void 0;"#);
    }

    #[test]
    fn the_page_has_its_hooks_and_no_remote_code() {
        for hook in ["window.CV", "messageHandlers.chat", "\"attach\"", "\"image\"", "\"key\"", "id=\"ask\""] {
            assert!(DOCUMENT.contains(hook), "{hook}");
        }
        assert!(!DOCUMENT.contains("<script src"), "everything inline");
        assert!(!DOCUMENT.contains("http://") && !DOCUMENT.contains("https://"), "no network");
    }
}
