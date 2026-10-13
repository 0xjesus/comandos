#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use comandos_app::ui::accounts::{AccountFlow, AccountStage};
#[path = "support/t17_oracle.rs"]
mod oracle;
#[test]
fn previous_account_operation_cannot_complete_current_one() {
    let mut flow = AccountFlow {
        operation_id: "new".into(),
        alias: "fixture".into(),
        stage: AccountStage::Switching,
        attempts: 0,
    };
    assert!(!flow.adopt_response("old", &serde_json::json!({"stage":"complete"})));
    assert_eq!(flow.stage, AccountStage::Switching);
}
struct PrivatePane {
    cancel: std::cell::Cell<bool>,
    cancel_during_read: bool,
    socket: std::path::PathBuf,
    record: String,
}
impl comandos_app::ui::clipboard::TmuxIo for PrivatePane {
    fn mode(&self) -> comandos_app::config::RunMode {
        comandos_app::config::RunMode::Sandbox
    }
    fn socket(&self) -> &std::path::Path {
        &self.socket
    }
    fn read(
        &self,
        args: &[&str],
    ) -> Result<comandos_app::tmux::TmuxOut, comandos_app::tmux::TmuxError> {
        if self.cancel_during_read {
            self.cancel.set(true);
        }
        Ok(comandos_app::tmux::TmuxOut {
            code: 0,
            stdout: if args.last()
                == Some(&"#{pid}|#{session_id}|#{session_created}|#{session_name}|#{pane_id}")
            {
                self.record.clone()
            } else {
                String::new()
            },
            stderr: String::new(),
        })
    }
    fn mutate(
        &self,
        _: &[&str],
        _: Option<&[u8]>,
    ) -> Result<comandos_app::tmux::TmuxOut, comandos_app::tmux::TmuxError> {
        panic!("identity lookup cannot mutate fake tmux")
    }
}
#[test]
fn identity_format_matches_existing_tmux_server_contract() {
    let fake = PrivatePane {
        cancel: std::cell::Cell::new(false),
        cancel_during_read: false,
        socket: std::env::temp_dir().join("account-explicit-fake-S"),
        record: "111|$1|42|fixture|%7".into(),
    };
    assert_eq!(
        comandos_app::ui::accounts::pane_identity(&fake, "fixture", "%7", None).unwrap(),
        fake.record
    );
    assert!(comandos_app::ui::accounts::pane_identity(&fake, "other", "%7", None).is_err());
    assert!(
        comandos_app::ui::accounts::pane_identity(
            &fake,
            "fixture",
            "%7",
            Some("111|$2|42|fixture|%7")
        )
        .is_err()
    );
}
#[test]
fn owner_cancelled_inside_pane_read_cannot_authorize_http() {
    let fake = PrivatePane {
        cancel: std::cell::Cell::new(false),
        cancel_during_read: true,
        socket: std::env::temp_dir().join("account-explicit-fake-S"),
        record: "111|$1|42|fixture|%7".into(),
    };
    assert!(
        comandos_app::ui::accounts::pane_identity_when(&fake, "fixture", "%7", None, || !fake
            .cancel
            .get())
        .is_err()
    );
}
#[test]
fn forged_payload_identity_and_nonfinal_success_cannot_complete() {
    let mut flow = AccountFlow {
        operation_id: "new".into(),
        alias: "fixture".into(),
        stage: AccountStage::Switching,
        attempts: 0,
    };
    assert!(!flow.adopt_response(
        "new",
        &serde_json::json!({"operationId":"old","alias":"fixture","state":"confirmed","ok":true})
    ));
    assert!(!flow.adopt_response(
        "new",
        &serde_json::json!({"operationId":"new","alias":"other","state":"confirmed","ok":true})
    ));
    assert!(flow.adopt_response(
        "new",
        &serde_json::json!({"operationId":"new","state":"awaiting_confirmation","ok":true})
    ));
    assert_eq!(flow.stage, AccountStage::AwaitingLogin);
    assert!(flow.adopt_response(
        "new",
        &serde_json::json!({"operationId":"new","state":"applying","stage":"complete","ok":true})
    ));
    assert_eq!(flow.stage, AccountStage::Switching);
    assert!(flow.adopt_response(
        "new",
        &serde_json::json!({"operationId":"new","state":"confirmed","ok":true})
    ));
    assert_eq!(flow.stage, AccountStage::Complete);
}
#[test]
fn polling_is_bounded_and_network_errors_do_not_decide_outcome() {
    let mut flow = AccountFlow {
        operation_id: "fixture-id".into(),
        alias: "fixture".into(),
        stage: AccountStage::Switching,
        attempts: 0,
    };
    for expected in 1..=120 {
        assert_eq!(
            flow.next_poll(),
            Some(std::time::Duration::from_millis(1500))
        );
        assert_eq!(flow.attempts, expected);
        assert_eq!(flow.stage, AccountStage::Switching);
    }
    assert_eq!(flow.next_poll(), None);
    let mut flow = AccountFlow {
        stage: AccountStage::Complete,
        ..flow
    };
    flow.attempts = 0;
    assert_eq!(flow.next_poll(), None);
}
#[test]
fn quota_thresholds_match_original() {
    for (pct, class) in [
        (-1, ""),
        (0, ""),
        (84, ""),
        (85, "warn"),
        (94, "warn"),
        (95, "full"),
        (120, "full"),
    ] {
        assert_eq!(comandos_app::ui::accounts::account_bar_class(pct), class);
    }
    assert_eq!(
        oracle::original(
            serde_json::json!({"op":"quota","percents":[-1,0,84,85,94,95,120],"statuses":[null]})
        ),
        serde_json::json!(["", "", "", "warn", "warn", "full", "full"])
    );
}
#[test]
fn actual_original_wait_body_matches_native_poll_delays_stages_and_terminal_states() {
    use comandos_app::ui::accounts::stage_note;
    use serde_json::json;
    for english in [true, false] {
        for end in [
            "confirmed",
            "failed",
            "rolled_back",
            "recovery_required",
            "awaiting_confirmation",
        ] {
            let statuses = vec![
                Some(json!({"operationId":"fixture-id","state":"snapshot"})),
                None,
                Some(json!({"operationId":"fixture-id","state":end,"ok":end=="confirmed"})),
            ];
            let expected =
                oracle::original(json!({"op":"account","english":english,"statuses":statuses}));
            let mut flow = AccountFlow {
                operation_id: "fixture-id".into(),
                alias: "fixture".into(),
                stage: AccountStage::Switching,
                attempts: 0,
            };
            let mut delays = vec![];
            let mut notes = vec![];
            while let Some(delay) = flow.next_poll() {
                delays.push(delay.as_secs_f64());
                if let Some(Some(status)) =
                    statuses.get((flow.attempts as usize - 1).min(statuses.len() - 1))
                {
                    assert!(flow.adopt_response("fixture-id", status));
                    if matches!(flow.stage, AccountStage::Complete | AccountStage::Failed) {
                        break;
                    }
                    notes.push(stage_note(status, &flow.alias, english));
                }
            }
            assert_eq!(json!(delays), expected["delays"]);
            assert_eq!(json!(notes), expected["notes"]);
            if end == "awaiting_confirmation" {
                assert_eq!(flow.attempts, 120);
                assert_eq!(expected["result"]["awaiting"], true);
            } else {
                assert_eq!(flow.attempts, 3);
                assert_eq!(expected["result"]["state"], end);
            }
        }
    }
}

#[test]
fn confirmed_without_ok_is_terminal_failure_instead_of_120_more_requests() {
    let mut flow = AccountFlow {
        operation_id: "fixture-id".into(),
        alias: "fixture".into(),
        stage: AccountStage::Switching,
        attempts: 0,
    };
    let data = serde_json::json!({"operationId":"fixture-id","state":"confirmed","ok":false});
    let original = oracle::original(serde_json::json!({"op":"account","statuses":[data]}));
    assert_eq!(original["delays"], serde_json::json!([1.5]));
    assert!(flow.adopt_response("fixture-id", &data));
    assert_eq!(flow.stage, AccountStage::Failed);
    assert_eq!(flow.next_poll(), None);
}

#[test]
fn a_rejected_switch_shows_the_server_reason() {
    use comandos_app::ui::accounts::request_error;
    let data = serde_json::json!({"ok":false,"error":"el panel tiene una operación pendiente\n o necesita recuperación"});
    assert_eq!(
        request_error(&data),
        "el panel tiene una operación pendiente o necesita recuperación"
    );
    assert_eq!(
        request_error(&serde_json::json!({})),
        "No se pudo completar la petición de cuenta"
    );
}
