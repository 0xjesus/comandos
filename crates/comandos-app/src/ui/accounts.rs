//! Account changes belong to a popover operation, never to a session-name cache.
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountStage {
    Loading,
    AwaitingLogin,
    Switching,
    Complete,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountFlow {
    pub operation_id: String,
    pub alias: String,
    pub stage: AccountStage,
    pub attempts: u16,
}
impl AccountFlow {
    /// The original waits 1.5 s before each request, including after a transport failure.
    pub fn next_poll(&mut self) -> Option<std::time::Duration> {
        if self.attempts >= 120
            || matches!(self.stage, AccountStage::Complete | AccountStage::Failed)
        {
            return None;
        }
        self.attempts += 1;
        Some(std::time::Duration::from_millis(1500))
    }
    pub fn adopt_response(&mut self, operation_id: &str, response: &Value) -> bool {
        if self.operation_id.is_empty()
            || operation_id != self.operation_id
            || response.get("operationId").and_then(Value::as_str) != Some(operation_id)
            || response
                .get("alias")
                .and_then(Value::as_str)
                .is_some_and(|alias| alias != self.alias)
        {
            return false;
        }
        self.stage = match response.get("state").and_then(Value::as_str) {
            Some("confirmed") if response.get("ok") == Some(&Value::Bool(true)) => {
                AccountStage::Complete
            }
            Some("confirmed" | "failed" | "rolled_back" | "recovery_required") => {
                AccountStage::Failed
            }
            Some("awaiting_confirmation") => AccountStage::AwaitingLogin,
            _ => AccountStage::Switching,
        };
        true
    }
}
pub fn account_bar_class(percent: i64) -> &'static str {
    if percent >= 95 {
        "full"
    } else if percent >= 85 {
        "warn"
    } else {
        ""
    }
}
/// The reason a rejected account request gives, bounded to one readable line.
pub fn request_error(data: &Value) -> String {
    data.get("error")
        .and_then(Value::as_str)
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(240).collect())
        .unwrap_or_else(|| "No se pudo completar la petición de cuenta".into())
}
pub fn awaiting(alias: &str, english: bool) -> String {
    if english {
        format!(
            "The conversation already runs on {alias}, but the pane asks something (folder trust, login or effort): answer it in its terminal and the switch confirms itself."
        )
    } else {
        format!(
            "La conversación ya corre en {alias}, pero el pane pide algo (confianza de carpeta, login o esfuerzo): respóndelo en su terminal y el cambio se confirma solo."
        )
    }
}
pub fn stage_note(response: &Value, alias: &str, english: bool) -> String {
    if response.get("state").and_then(Value::as_str) == Some("awaiting_confirmation") {
        return awaiting(alias, english);
    }
    match response
        .get("stageCode")
        .and_then(Value::as_str)
        .or_else(|| response.get("state").and_then(Value::as_str))
    {
        Some("validating") => "Validando…",
        Some("waiting") => "Interrumpiendo lo que está haciendo…",
        Some("snapshot") => "Guardando la conversación…",
        Some("applying") => "Abriendo la conversación en la otra cuenta…",
        Some("verifying") => "Confirmando que siguió igual…",
        Some("recovering") => "Recuperando la conversación original…",
        _ => response
            .get("stage")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    }
    .into()
}

use crate::{dash_client::DashClient, ui::snippets::Scope};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};

pub(crate) struct Dialog {
    pub pop: gtk::Popover,
    pub rows: gtk::Box,
    pub add: gtk::Button,
    pub note: gtk::Label,
    pub scope: Scope,
    pub client: DashClient,
    pub session: String,
    pub pane: String,
    pub harness: String,
    pub instance: Arc<AtomicBool>,
    pub identity: RefCell<Option<String>>,
    pub flow: RefCell<Option<AccountFlow>>,
    pub busy: Cell<bool>,
    pub timer: RefCell<Option<glib::SourceId>>,
    pub buttons: RefCell<Vec<(gtk::Button, bool)>>,
}
impl Dialog {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        button: &gtk::Button,
        session: &str,
        pane: &str,
        harness: &str,
        instance: Arc<AtomicBool>,
        client: DashClient,
        english: bool,
    ) -> Rc<Self> {
        let pop = gtk::Popover::new(Some(button));
        pop.set_position(gtk::PositionType::Bottom);
        pop.style_context().add_class("acct-pop");
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 6);
        outer.set_margin_start(8);
        outer.set_margin_end(8);
        outer.set_margin_top(8);
        outer.set_margin_bottom(8);
        outer.set_size_request(300, -1);
        let title = gtk::Label::new(Some(if english {
            "THIS PANE'S ACCOUNT"
        } else {
            "CUENTA DE ESTE PANE"
        }));
        title.set_xalign(0.0);
        title.style_context().add_class("acct-title");
        outer.pack_start(&title, false, false, 0);
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 4);
        outer.pack_start(&rows, false, false, 0);
        let add = gtk::Button::with_label(if english {
            "+ Add account"
        } else {
            "+ Añadir cuenta"
        });
        outer.pack_start(&add, false, false, 0);
        let note = gtk::Label::new(Some(if english {
            "Loading accounts…"
        } else {
            "Consultando cuentas…"
        }));
        note.set_xalign(0.0);
        note.set_line_wrap(true);
        note.set_max_width_chars(38);
        note.style_context().add_class("acct-note");
        outer.pack_start(&note, false, false, 0);
        pop.add(&outer);
        let dialog = Rc::new(Self {
            pop,
            rows,
            add,
            note,
            scope: Scope::default(),
            client,
            session: session.into(),
            pane: pane.into(),
            harness: harness.into(),
            instance,
            identity: RefCell::new(None),
            flow: RefCell::new(None),
            busy: Cell::new(false),
            timer: RefCell::new(None),
            buttons: RefCell::new(vec![]),
        });
        let weak = Rc::downgrade(&dialog);
        dialog.pop.connect_closed(move |_| {
            if let Some(d) = weak.upgrade() {
                d.cancel();
            }
        });
        let weak = Rc::downgrade(&dialog);
        dialog.pop.connect_destroy(move |_| {
            if let Some(d) = weak.upgrade() {
                d.cancel();
            }
        });
        dialog
    }
    pub fn cancel(&self) {
        self.scope.close();
        self.client.cancel_pending();
        if let Some(source) = self.timer.borrow_mut().take() {
            source.remove();
        }
    }
    pub fn set_busy(&self, busy: bool, writable: bool) {
        self.busy.set(busy);
        self.add.set_sensitive(!busy && writable);
        for (button, can) in self.buttons.borrow().iter() {
            button.set_sensitive(!busy && writable && *can);
        }
    }
    pub fn fail(&self, message: &str, writable: bool) {
        self.note.set_text(message);
        self.note.style_context().add_class("acct-err");
        self.set_busy(false, writable);
    }
    pub fn fill(
        &self,
        data: &Value,
        current: &str,
        english: bool,
        writable: bool,
        clicked: Rc<dyn Fn(String, bool)>,
    ) {
        for old in self.rows.children() {
            self.rows.remove(&old);
        }
        self.buttons.borrow_mut().clear();
        for account in data
            .get("accounts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(256)
        {
            let alias = account
                .get("alias")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let selectable = account
                .get("selectable")
                .is_some_and(comandos_core::json::truthy);
            let on = alias == current && selectable;
            let can = alias != current || !selectable;
            let button = gtk::Button::new();
            button.set_relief(gtk::ReliefStyle::None);
            button.style_context().add_class("acct-row");
            if on {
                button.style_context().add_class("on");
            }
            let grid = gtk::Grid::new();
            grid.set_column_spacing(8);
            grid.set_row_spacing(3);
            let name = gtk::Label::new(None);
            name.set_xalign(0.0);
            name.set_hexpand(true);
            name.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(alias)));
            let tag = gtk::Label::new(Some(match (english, on, selectable) {
                (true, true, _) => "in use",
                (true, false, true) => "switch →",
                (true, false, false) => "sign in →",
                (false, true, _) => "en uso",
                (false, false, true) => "cambiar →",
                _ => "iniciar sesión →",
            }));
            tag.set_xalign(1.0);
            tag.style_context().add_class("acct-tag");
            grid.attach(&name, 0, 0, 2, 1);
            grid.attach(&tag, 2, 0, 1, 1);
            let identity = account
                .get("identity")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            if let Some(identity) = identity {
                let label = dim_label(identity, 0.0);
                grid.attach(&label, 0, 1, 3, 1);
            }
            for (i, limit) in account
                .get("limits")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(32)
                .enumerate()
            {
                let percent = crate::ui::app_commands::python_int(
                    limit.get("percent").unwrap_or(&Value::Null),
                )
                .unwrap_or(0);
                let label = dim_label(
                    limit
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    0.0,
                );
                let bar = gtk::LevelBar::for_interval(0.0, 100.0);
                bar.set_mode(gtk::LevelBarMode::Continuous);
                for offset in ["low", "high", "full"] {
                    bar.remove_offset_value(Some(offset));
                }
                bar.set_value(percent as f64);
                bar.set_hexpand(true);
                bar.set_valign(gtk::Align::Center);
                bar.style_context().add_class("acct-bar");
                let class = account_bar_class(percent.into());
                if !class.is_empty() {
                    bar.style_context().add_class(class);
                }
                let value = dim_label(&format!("{percent}%"), 1.0);
                let row = i as i32 + if identity.is_some() { 2 } else { 1 };
                grid.attach(&label, 0, row, 1, 1);
                grid.attach(&bar, 1, row, 1, 1);
                grid.attach(&value, 2, row, 1, 1);
            }
            button.add(&grid);
            button.set_tooltip_text(Some(identity.unwrap_or(alias)));
            button.set_sensitive(can && writable);
            if can {
                let clicked = clicked.clone();
                let alias = alias.to_string();
                button.connect_clicked(move |_| clicked(alias.clone(), selectable));
            }
            self.rows.pack_start(&button, false, false, 0);
            self.buttons.borrow_mut().push((button, can));
        }
        self.note.set_text(if english {
            "Same conversation. If it is replying, it is interrupted and switches right away."
        } else {
            "Sigue la misma conversación. Si está respondiendo, se interrumpe y cambia al instante."
        });
        self.rows.show_all();
        self.set_busy(false, writable);
    }
}
impl Drop for Dialog {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn dim_label(text: &str, x: f32) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(x);
    label.style_context().add_class("acct-dim");
    label
}

pub fn pane_identity(
    tmux: &impl crate::ui::clipboard::TmuxIo,
    session: &str,
    pane: &str,
    expected: Option<&str>,
) -> Result<String, String> {
    pane_identity_when(tmux, session, pane, expected, || true)
}
pub fn pane_identity_when(
    tmux: &impl crate::ui::clipboard::TmuxIo,
    session: &str,
    pane: &str,
    expected: Option<&str>,
    allowed: impl Fn() -> bool,
) -> Result<String, String> {
    if !allowed() {
        return Err("Account view closed".into());
    }
    let socket = tmux.socket().to_path_buf();
    if !crate::tab_actions::valid_session(session) || !crate::ui::app_commands::valid_pane(pane) {
        return Err("Invalid account pane".into());
    }
    let out = tmux
        .read(&[
            "display-message",
            "-p",
            "-t",
            pane,
            "#{pid}|#{session_id}|#{s/[$]//:session_id}|#{session_name}|#{pane_id}",
        ])
        .map_err(|_| "Account pane unavailable".to_string())?;
    if !allowed() || tmux.socket() != socket {
        return Err("Account view closed or socket replaced".into());
    }
    let value = out.stdout.trim();
    let fields = value.split('|').collect::<Vec<_>>();
    if !out.ok()
        || fields.len() != 5
        || fields.get(3).copied() != Some(session)
        || fields.get(4).copied() != Some(pane)
        || fields
            .first()
            .is_none_or(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
        || fields.get(1).is_none_or(|s| {
            s.strip_prefix('$')
                .is_none_or(|id| id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()))
        })
        || fields
            .get(2)
            .is_none_or(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
        || expected.is_some_and(|expected| expected != value)
    {
        return Err("Account pane identity changed".into());
    }
    Ok(value.to_string())
}
