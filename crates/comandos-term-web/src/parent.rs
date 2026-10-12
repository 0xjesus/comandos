//! Validated parent commands; event origin AND source are checked by the browser adapter.
use serde_json::Value;
#[derive(Debug, PartialEq)]
pub enum Command {
    Theme(String),
    CursorStyle(String),
    ButtonStyle(String),
    Session(String),
    SelectPane(String),
    Interaction {
        known: bool,
        busy: bool,
        selecting: bool,
    },
    Key(String),
    Paste(String),
    Ctrl,
    Focus,
    Mode(bool),
    /// The parent page draws the keys and composer; hide this page's own and keep
    /// focus where the parent has it.
    Dock(bool),
    /// A toolbar action button pressed from the parent's bar.
    Action(String),
}
pub fn toolbar_action(action: &str) -> bool {
    matches!(action, "mode" | "panes" | "history" | "copy" | "paste" | "keyboard" | "ctrl")
}
pub fn button_style(style: &str) -> bool {
    matches!(style, "sutil" | "arcade" | "tecla" | "pixel" | "consola")
}
pub fn pane_id(pane: &str) -> bool {
    pane.strip_prefix('%')
        .is_some_and(|tail| !tail.is_empty() && tail.bytes().all(|c| c.is_ascii_digit()))
}
pub fn command(data: &Value) -> Option<Command> {
    if data.get("source")?.as_str()? != "comandos" {
        return None;
    }
    let text = |key| data.get(key)?.as_str().map(str::to_owned);
    Some(match data.get("type")?.as_str()? {
        "theme" => {
            let s = text("theme")?;
            crate::page_theme::theme(&s)?;
            Command::Theme(s)
        }
        "cursor-style" => {
            let style = text("style")?;
            crate::cursor_style(&style)?;
            Command::CursorStyle(style)
        }
        "button-style" => {
            let s = text("style")?;
            if !button_style(&s) {
                return None;
            }
            Command::ButtonStyle(s)
        }
        "session" => Command::Session(text("session")?),
        "dock" => Command::Dock(comandos_web_dom::api::js_truthy(&data["on"])),
        "select-pane" => {
            let s = text("pane")?;
            if !pane_id(&s) {
                return None;
            }
            Command::SelectPane(s)
        }
        "interaction-state" => Command::Interaction {
            known: comandos_web_dom::api::js_truthy(&data["known"]),
            busy: comandos_web_dom::api::js_truthy(&data["busy"]),
            selecting: comandos_web_dom::api::js_truthy(&data["selecting"]),
        },
        "toolbar" => {
            let t = data.get("term")?;
            match t.get("type")?.as_str()? {
                "toolbar" => {
                    let key = t.get("key")?.as_str()?;
                    toolbar_key(key)?;
                    Command::Key(key.into())
                }
                "paste" => Command::Paste(t.get("text")?.as_str()?.into()),
                "ctrl" => Command::Ctrl,
                "focus" => Command::Focus,
                "mode" => Command::Mode(comandos_web_dom::api::js_truthy(&t["selecting"])),
                "action" => {
                    let action = t.get("action")?.as_str()?;
                    if !toolbar_action(action) {
                        return None;
                    }
                    Command::Action(action.into())
                }
                _ => return None,
            }
        }
        _ => return None,
    })
}
pub fn toolbar_key(key: &str) -> Option<&'static [u8]> {
    Some(match key {
        "escape" => b"\x1b",
        "ctrlc" => b"\x03",
        "yes" => b"\r",
        "no" => b"\x1b",
        "backspace" => b"\x7f",
        "tab" => b"\t",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "right" => b"\x1b[C",
        "left" => b"\x1b[D",
        _ => return None,
    })
}
/// ASCII Ctrl mapping used by the armed toolbar (including @, [, \, ], ^ and _).
pub fn control_byte(data: &[u8]) -> Option<u8> {
    let [byte] = data else { return None };
    if *byte == b' ' {
        Some(0)
    } else if (64..=95).contains(byte) || (97..=122).contains(byte) {
        Some(*byte & 31)
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_and_toolbar_match_legacy() {
        assert_eq!(toolbar_key("escape"), Some(&b"\x1b"[..]));
        for (text, byte) in [(b"@", 0), (b"[", 27), (b"_", 31), (b"c", 3), (b" ", 0)] {
            assert_eq!(control_byte(text), Some(byte));
        }
        assert_eq!(control_byte(b"1"), None);
        assert_eq!(control_byte(b"abc"), None);
    }
    #[test]
    fn dock_and_actions_validate() {
        let m = |v| command(&v);
        assert_eq!(
            m(serde_json::json!({"source":"comandos","type":"dock","on":true})),
            Some(Command::Dock(true))
        );
        assert_eq!(
            m(serde_json::json!({"source":"comandos","type":"toolbar","term":{"type":"action","action":"history"}})),
            Some(Command::Action("history".into()))
        );
        assert!(
            m(serde_json::json!({"source":"comandos","type":"toolbar","term":{"type":"action","action":"x');"}}))
                .is_none()
        );
    }
    #[test]
    fn cursor_style_messages_validate_before_applying() {
        assert_eq!(
            command(
                &serde_json::json!({"source":"comandos","type":"cursor-style","style":"ibeam"})
            ),
            Some(Command::CursorStyle("ibeam".into()))
        );
        assert!(
            command(
                &serde_json::json!({"source":"comandos","type":"cursor-style","style":"invalid"})
            )
            .is_none()
        );
    }
    #[test]
    fn invalid_messages_cannot_mutate_page() {
        assert!(
            command(&serde_json::json!({"source":"other","type":"theme","theme":"dia"})).is_none()
        );
        assert!(
            command(
                &serde_json::json!({"source":"comandos","type":"select-pane","pane":"%1;kill"})
            )
            .is_none()
        );
        assert_eq!(
            command(&serde_json::json!({"source":"comandos","type":"select-pane","pane":"%123"})),
            Some(Command::SelectPane("%123".into()))
        );
        assert!(
            command(&serde_json::json!({"source":"comandos","type":"theme","theme":"unknown"}))
                .is_none()
        );
        assert!(!button_style("arbitrary"));
        assert!(!pane_id("%"));
    }
}
