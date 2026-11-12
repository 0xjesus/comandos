//! Shelf messages are scoped to the WebView instance and its frozen target URI.
use super::bridge::BridgeError;
use serde_json::{Value, json};
pub const SHELF_MIN: i32 = 180;
pub const SHELF_HANDLE: i32 = 6;
pub fn shelf_height(total: f64, saved: Option<f64>) -> f64 {
    if !total.is_finite() {
        return f64::from(SHELF_MIN);
    }
    let wanted = saved
        .filter(|n| n.is_finite() && *n > 0.0)
        .unwrap_or_else(|| (total * 0.48).trunc().clamp(270.0, 520.0));
    wanted.trunc().min(total - 160.0).max(f64::from(SHELF_MIN))
}
pub fn shelf_separator_css(theme: &Value) -> String {
    let token = |key| theme.get(key).and_then(Value::as_str).unwrap_or_default();
    format!(
        "paned.cc-paned paned.cc-shelf-paned > separator{{ min-height:{SHELF_HANDLE}px;background-image:none;border:0;box-shadow:none; background-color:{};}}paned.cc-paned paned.cc-shelf-paned > separator:hover{{ background-color:{};}}",
        token("line"),
        token("brand")
    )
}
/// Shadow cannot execute the web application's mutation-capable JavaScript.
/// Render only public display fields, never raw inventory configuration/credentials.
pub fn shadow_preview(data: &Value, session: &str, pane: &str, english: bool) -> String {
    let esc = |value: &str| {
        glib::markup_escape_text(&value.chars().take(4096).collect::<String>()).to_string()
    };
    let mut html = format!(
        "<!doctype html><html lang=\"{}\"><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none';style-src 'unsafe-inline'\"><style>body{{margin:12px;background:#0A0D13;color:#ECECEC;font:13px sans-serif}}h1{{font-size:15px}}table{{width:100%;border-collapse:collapse}}td,th{{padding:6px;text-align:left;border-bottom:1px solid #223044}}small{{color:#8A8F98}}</style></head><body><h1>MCPs · Skills · {} · {}</h1><p>{}</p>",
        if english { "en" } else { "es" },
        esc(session),
        esc(pane),
        if english {
            "Read only · Shadow"
        } else {
            "Solo lectura · Sombra"
        }
    );
    for (kind, title) in [("mcps", "MCPs"), ("skills", "Skills")] {
        html.push_str(&format!("<h2>{title}</h2><table>"));
        let rows = data
            .get("inventory")
            .and_then(|v| v.get(kind))
            .and_then(Value::as_array);
        for row in rows.into_iter().flatten().take(256) {
            let name = row.get("name").and_then(Value::as_str).unwrap_or_default();
            let id = row.get("id").and_then(Value::as_str).unwrap_or_default();
            let selected = data
                .get("desired")
                .and_then(|v| v.get(kind))
                .and_then(|v| v.get(id))
                .and_then(Value::as_bool);
            let selection = match (english, selected) {
                (true, Some(true)) => "Selected",
                (true, Some(false)) => "Unselected",
                (true, None) => "Unknown",
                (false, Some(true)) => "Seleccionada",
                (false, Some(false)) => "Sin seleccionar",
                (false, None) => "Sin confirmar",
            };
            let tokens = row
                .get("size")
                .and_then(|v| v.get("tokens"))
                .and_then(Value::as_u64)
                .map_or_else(
                    || {
                        if english {
                            "Not measured".into()
                        } else {
                            "Sin medir".into()
                        }
                    },
                    |n| format!("{n} tokens"),
                );
            html.push_str(&format!(
                "<tr><th>{}</th><td>{selection}</td><td>{}</td></tr>",
                esc(name),
                esc(&tokens)
            ));
        }
        html.push_str("</table>");
    }
    html.push_str("</body></html>");
    html
}
/// The read-only HTTP boundary retains the same pane proof across the blocking GET.
pub fn read_inventory_when(
    tmux: &impl super::clipboard::TmuxIo,
    session: &str,
    pane: &str,
    harness: &str,
    allowed: impl Fn() -> bool,
    get: impl FnOnce(&str) -> Result<Value, String>,
) -> Result<Value, String> {
    let socket = tmux.socket().to_path_buf();
    let identity = super::accounts::pane_identity_when(tmux, session, pane, None, &allowed)?;
    let path = format!(
        "/pane-extensions?session={}&pane={}&harness={}",
        super::webview::encode_query(session),
        super::webview::encode_query(pane),
        super::webview::encode_query(harness)
    );
    let data = get(&path)?;
    if !allowed() || tmux.socket() != socket {
        return Err("Extension target cancelled or socket replaced".into());
    }
    super::accounts::pane_identity_when(tmux, session, pane, Some(&identity), allowed)?;
    Ok(data)
}
/// Resolve a keyboard target once, then pin its identity across the cwd lookup.
/// An explicit menu pane never follows a later active-pane change.
pub fn wizard_target_when(
    tmux: &impl super::clipboard::TmuxIo,
    session: &str,
    pane: &str,
    allowed: impl Fn() -> bool,
) -> Result<(String, String, String), String> {
    let socket = tmux.socket().to_path_buf();
    let current = || allowed() && tmux.socket() == socket;
    if !current() || !crate::tab_actions::valid_session(session) {
        return Err("Wizard cancelled or invalid session".into());
    }
    let pane = if pane.is_empty() {
        let target = format!("={session}:");
        let out = tmux
            .read(&[
                "display-message",
                "-p",
                "-t",
                &target,
                "#{session_name}|#{pane_id}",
            ])
            .map_err(|_| "No tmux session".to_string())?;
        if !current() || !out.ok() {
            return Err("Wizard cancelled or socket replaced".into());
        }
        let (actual_session, pane) = out
            .stdout
            .trim()
            .split_once('|')
            .ok_or_else(|| "Wizard pane unavailable".to_string())?;
        if actual_session != session || !super::app_commands::valid_pane(pane) {
            return Err("Wizard pane changed".into());
        }
        pane.to_string()
    } else if super::app_commands::valid_pane(pane) {
        pane.to_string()
    } else {
        return Err("Invalid pane".into());
    };
    let identity = super::accounts::pane_identity_when(tmux, session, &pane, None, current)?;
    if !current() {
        return Err("Wizard cancelled or socket replaced".into());
    }
    let out = tmux
        .read(&[
            "display-message",
            "-p",
            "-t",
            &pane,
            "#{session_name}|#{pane_id}|#{pane_current_path}",
        ])
        .map_err(|_| "No tmux session".to_string())?;
    if !current() || !out.ok() {
        return Err("Wizard cancelled or socket replaced".into());
    }
    // tmux display-message replaces literal newlines in the format with '_'.
    // Session/pane identifiers cannot contain '|'; split only those fields,
    // preserving any separator that belongs to the working directory.
    let mut fields = out.stdout.splitn(3, '|');
    if fields.next() != Some(session) || fields.next() != Some(pane.as_str()) {
        return Err("Wizard pane changed".into());
    }
    let cwd = fields.next().unwrap_or_default().trim().to_string();
    super::accounts::pane_identity_when(tmux, session, &pane, Some(&identity), current)?;
    if !current() {
        return Err("Wizard cancelled or socket replaced".into());
    }
    Ok((session.to_string(), pane, cwd))
}
pub fn extension_message(raw: &str) -> Result<Value, BridgeError> {
    if raw.len() > 8192 {
        return Err(BridgeError::Invalid(
            "extension message exceeds 8 KiB".into(),
        ));
    }
    Ok(if raw == "close" {
        json!({"command":"close"})
    } else {
        Value::Null
    })
}
pub fn extension_uri(
    base: &str,
    session: &str,
    pane: &str,
    harness: &str,
    version: &str,
) -> Result<String, BridgeError> {
    crate::config::loopback_only(base).map_err(BridgeError::Invalid)?;
    if !crate::tab_actions::valid_session(session) || !super::app_commands::valid_pane(pane) {
        return Err(BridgeError::Invalid("invalid extension target".into()));
    }
    let mut uri = format!(
        "{}/extensions.html?session={}&pane={}",
        base.trim_end_matches('/'),
        super::webview::encode_query(session),
        super::webview::encode_query(pane)
    );
    if matches!(harness, "claude" | "codex" | "grok" | "opencode" | "agy") {
        uri.push_str("&harness=");
        uri.push_str(harness);
    }
    uri.push_str("&v=");
    uri.push_str(&super::webview::encode_query(version));
    Ok(uri)
}
pub fn message_owned(
    current_uri: &str,
    owned_uri: &str,
    base: &str,
    message_generation: u64,
    current_generation: u64,
) -> bool {
    message_generation == current_generation
        && current_uri == owned_uri
        && crate::config::loopback_only(base).is_ok()
        && owned_uri.starts_with(&format!("{}/extensions.html?", base.trim_end_matches('/')))
}

use gtk::prelude::*;
use std::cell::{Cell, RefCell};
pub(crate) struct ShelfView {
    pub page: super::webview::OwnedPage,
    pub scope: super::snippets::Scope,
    pub generation: u64,
    pub uri: String,
    pub target: Option<(String, String, String)>,
    pub instance: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub signals: Vec<(glib::Object, glib::SignalHandlerId)>,
    pub close: Option<gtk::Button>,
    pub client: Option<crate::dash_client::DashClient>,
}
impl Drop for ShelfView {
    fn drop(&mut self) {
        self.scope.close();
        if let Some(client) = &self.client {
            client.cancel_pending();
        }
        if let Some(close) = &self.close
            && let Some(parent) = close
                .parent()
                .and_then(|p| p.downcast::<gtk::Container>().ok())
        {
            parent.remove(close);
        }
        self.page.cancel();
        for (object, id) in self.signals.drain(..) {
            object.disconnect(id);
        }
        self.page.view.hide();
    }
}
pub(crate) struct Shelf {
    pub paned: gtk::Paned,
    pub host: gtk::Box,
    pub extension: RefCell<Option<ShelfView>>,
    pub serial: Cell<u64>,
    pub request: super::snippets::Scope,
    pub opening: RefCell<Option<(String, crate::dash_client::DashClient)>>,
    pub saved: RefCell<Value>,
    pub loaded: Cell<bool>,
    pub save: RefCell<Option<glib::SourceId>>,
    pub css: RefCell<Option<ShelfCss>>,
}
impl Shelf {
    pub fn new() -> Self {
        let paned = gtk::Paned::new(gtk::Orientation::Vertical);
        paned.style_context().add_class("cc-shelf-paned");
        let host = gtk::Box::new(gtk::Orientation::Vertical, 0);
        host.set_no_show_all(true);
        host.hide();
        paned.pack2(&host, false, false);
        Self {
            paned,
            host,
            extension: RefCell::new(None),
            serial: Cell::new(0),
            request: super::snippets::Scope::default(),
            opening: RefCell::new(None),
            saved: RefCell::new(Value::Null),
            loaded: Cell::new(false),
            save: RefCell::new(None),
            css: RefCell::new(None),
        }
    }
    pub fn paint(&self, theme: &crate::theme::ThemeTokens) -> Result<(), glib::Error> {
        let provider = gtk::CssProvider::new();
        provider
            .load_from_data(shelf_separator_css(&Value::Object(theme.values.clone())).as_bytes())?;
        let css = ShelfCss {
            provider,
            screen: gdk::Screen::default(),
        };
        self.css.borrow_mut().take();
        if let Some(screen) = &css.screen {
            gtk::StyleContext::add_provider_for_screen(
                screen,
                &css.provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
            );
        }
        *self.css.borrow_mut() = Some(css);
        Ok(())
    }
    pub fn close_extension(&self) {
        self.request.advance();
        if let Some((_, client)) = self.opening.borrow_mut().take() {
            client.cancel_pending();
        }
        if let Some(view) = self.extension.borrow_mut().take() {
            self.host.remove(&view.page.view);
        }
        if let Some(source) = self.save.borrow_mut().take() {
            source.remove();
        }
        self.sync();
    }
    pub fn sync(&self) {
        self.host.set_visible(self.extension.borrow().is_some());
    }
    pub fn cancel(&self) {
        self.close_extension();
        self.request.close();
        self.css.borrow_mut().take();
    }
    pub fn next_generation(&self) -> u64 {
        let generation = self.serial.get().wrapping_add(1);
        self.serial.set(generation);
        generation
    }
}
impl Drop for Shelf {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(crate) struct ShelfCss {
    provider: gtk::CssProvider,
    screen: Option<gdk::Screen>,
}
impl Drop for ShelfCss {
    fn drop(&mut self) {
        if let Some(screen) = &self.screen {
            gtk::StyleContext::remove_provider_for_screen(screen, &self.provider);
        }
    }
}
