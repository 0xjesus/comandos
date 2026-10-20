#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use comandos_app::{
    term::paint::CellGeom,
    ui::overlays::{
        PaneGeometry, frame_layout, grip_rect, gutter_half, gutter_neighbor, gutter_target,
        shell_pill_xy,
    },
};
#[path = "support/t17_oracle.rs"]
mod oracle;
fn numeric_eq(actual: &serde_json::Value, expected: &serde_json::Value) {
    use serde_json::Value;
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => assert_eq!(a.as_f64(), b.as_f64()),
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                numeric_eq(a, b);
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len());
            for (key, value) in a {
                numeric_eq(value, b.get(key).unwrap());
            }
        }
        _ => assert_eq!(actual, expected),
    }
}
#[test]
fn neighbor_max_overlap_and_clamped_drag_leave_both_panes_two_cells() {
    let panes = vec![
        PaneGeometry {
            id: "%1".into(),
            left: 0,
            top: 1,
            width: 40,
            height: 30,
        },
        PaneGeometry {
            id: "%2".into(),
            left: 41,
            top: 1,
            width: 30,
            height: 10,
        },
        PaneGeometry {
            id: "%3".into(),
            left: 41,
            top: 12,
            width: 30,
            height: 19,
        },
    ];
    assert_eq!(gutter_neighbor(&panes, "%1", "v"), Some("%3"));
    assert_eq!(
        gutter_target(&panes, "%1", "v", 100.0),
        Some(serde_json::json!(68.0))
    );
    assert_eq!(
        gutter_target(&panes, "%1", "v", -10.0),
        Some(serde_json::json!(2.0))
    );
    assert_eq!(gutter_half(&panes, "%1", "v"), Some(35));
}
#[test]
fn geometry_queue_has_one_flight_discards_stale_and_keeps_only_latest_request() {
    use comandos_app::ui::overlays::GeometryGate;
    let mut gate = GeometryGate::default();
    let first = gate.request().unwrap();
    for _ in 0..10_000 {
        assert_eq!(gate.request(), None);
    }
    let (publish, next) = gate.finish(first);
    assert!(!publish);
    let second = next.unwrap();
    assert_ne!(first, second);
    assert_eq!(gate.finish(first), (false, None));
    assert_eq!(gate.finish(second), (true, None));
    let last = gate.request().unwrap();
    gate.close();
    assert_eq!(gate.finish(last), (false, None));
    assert_eq!(gate.request(), None);
}
#[test]
fn geometry_parser_owns_real_tmux_command_height_and_active_fields() {
    use comandos_app::ui::overlays::parse_geometry;
    let panes = parse_geometry("%1 0 1 40 grok 29 1\n%2 41 1 38 bash 29 0\n").unwrap();
    assert_eq!(panes.len(), 2);
    assert_eq!(panes[0].geometry.height, 29);
    assert!(panes[0].active);
    assert_eq!(panes[1].command, "bash");
    assert!(parse_geometry("%1 0 bad 40 grok 29 1\n").is_err());
    assert!(parse_geometry("wrong 0 1 40 grok 29 1\n").is_err());
}
struct FakeGeometry {
    mode: comandos_app::config::RunMode,
    socket: std::path::PathBuf,
    layout: String,
    cancel: std::cell::Cell<bool>,
    cancel_read: bool,
    mutations: std::cell::RefCell<Vec<Vec<String>>>,
}
impl comandos_app::ui::clipboard::TmuxIo for FakeGeometry {
    fn mode(&self) -> comandos_app::config::RunMode {
        self.mode
    }
    fn socket(&self) -> &std::path::Path {
        &self.socket
    }
    fn read(
        &self,
        args: &[&str],
    ) -> Result<comandos_app::tmux::TmuxOut, comandos_app::tmux::TmuxError> {
        if self.cancel_read {
            self.cancel.set(true);
        }
        let stdout = if args.first() == Some(&"list-panes") {
            self.layout.clone()
        } else {
            format!("111|$1|42|fixture|{}", args.get(3).unwrap_or(&"%1"))
        };
        Ok(comandos_app::tmux::TmuxOut {
            code: 0,
            stdout,
            stderr: String::new(),
        })
    }
    fn mutate(
        &self,
        args: &[&str],
        _: Option<&[u8]>,
    ) -> Result<comandos_app::tmux::TmuxOut, comandos_app::tmux::TmuxError> {
        self.mutations
            .borrow_mut()
            .push(args.iter().map(|s| s.to_string()).collect());
        Ok(comandos_app::tmux::TmuxOut {
            code: 0,
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}
#[test]
fn real_resize_producer_only_changes_proven_neighbor_and_rechecks_owner_after_read() {
    use comandos_app::ui::overlays::resize_neighbor_when;
    for (mode, neighbor, cancel, expect) in [
        (comandos_app::config::RunMode::Sandbox, "%2", false, true),
        (comandos_app::config::RunMode::Sandbox, "%3", false, false),
        (comandos_app::config::RunMode::Sandbox, "%2", true, false),
        (comandos_app::config::RunMode::Shadow, "%2", false, false),
    ] {
        let fake = FakeGeometry {
            mode,
            socket: std::env::temp_dir().join("gutter-fake-private-S"),
            layout: "%1 0 1 40 grok 29 1\n%2 41 1 38 bash 29 0\n".into(),
            cancel: std::cell::Cell::new(false),
            cancel_read: cancel,
            mutations: std::cell::RefCell::new(vec![]),
        };
        let result = resize_neighbor_when(&fake, "fixture", "%1", "v", neighbor, 100, || {
            !fake.cancel.get()
        });
        assert_eq!(result.is_ok(), expect);
        if expect {
            assert_eq!(
                *fake.mutations.borrow(),
                vec![vec!["resize-pane", "-t", "%1", "-x", "76"]]
            );
        } else {
            assert!(fake.mutations.borrow().is_empty());
        }
    }
}
#[test]
fn original_ast_frames_cards_grips_and_gutters_match_pango_logical_geometry() {
    use comandos_app::ui::overlays::card_rect;
    use serde_json::json;
    for cell in [(8.0, 17.0), (8.5, 17.5), (9.75, 21.25)] {
        for dpr in [1.0, 1.5, 2.0] {
            for focused in [false, true] {
                let panes = vec![
                    PaneGeometry {
                        id: "%1".into(),
                        left: 0,
                        top: 1,
                        width: 40,
                        height: 29,
                    },
                    PaneGeometry {
                        id: "%2".into(),
                        left: 41,
                        top: 1,
                        width: 38,
                        height: 10,
                    },
                    PaneGeometry {
                        id: "%3".into(),
                        left: 41,
                        top: 12,
                        width: 38,
                        height: 18,
                    },
                ];
                let input=panes.iter().map(|p|json!({"id":p.id,"left":p.left,"top":p.top,"width":p.width,"height":p.height})).collect::<Vec<_>>();
                let geom = CellGeom {
                    cell_w: cell.0,
                    cell_h: cell.1,
                    origin_x: 12.0,
                    origin_y: 24.0,
                    dpr,
                    font_size: 14.0,
                };
                let expected = oracle::original(
                    json!({"op":"geometry","panes":input,"geom":{"cell_w":cell.0,"cell_h":cell.1,"origin_x":12.0,"origin_y":24.0},"grid":[80,30],"focused":focused,"active":["%3"],"statuses":[null]}),
                );
                let layout =
                    frame_layout(&panes, &geom, (80, 30), &["%3".to_string()].into(), focused);
                numeric_eq(&json!(layout.frames), &expected["frames"]);
                numeric_eq(&json!(layout.rows), &expected["rows"]);
                numeric_eq(
                    &json!(
                        layout
                            .gutters
                            .iter()
                            .map(|g| json!([g.orientation, g.x, g.y, g.width, g.height, g.pane]))
                            .collect::<Vec<_>>()
                    ),
                    &expected["gutters"],
                );
                assert_eq!(
                    json!(layout.gutters.iter().map(grip_rect).collect::<Vec<_>>()),
                    expected["grips"]
                );
                for p in &panes {
                    assert_eq!(
                        json!(shell_pill_xy(
                            &layout.rows,
                            &p.id,
                            (12.0, 24.0),
                            (p.left, p.top),
                            cell
                        )),
                        expected["pills"][&p.id]
                    );
                }
                for (pid, row) in &layout.rows {
                    assert_eq!(json!(card_rect(*row, cell.1)), expected["cards"][pid]);
                }
                for p in &panes {
                    for orientation in ["v", "h"] {
                        for point in [-10.0, 25.25, 100.0] {
                            let e = oracle::original(
                                json!({"op":"gutter","panes":input,"pane":p.id,"orientation":orientation,"cell":point,"statuses":[null]}),
                            );
                            assert_eq!(
                                json!(gutter_neighbor(&panes, &p.id, orientation)),
                                e["neighbor"]
                            );
                            numeric_eq(
                                &gutter_target(&panes, &p.id, orientation, point).unwrap(),
                                &e["target"],
                            );
                            assert_eq!(json!(gutter_half(&panes, &p.id, orientation)), e["half"]);
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn layout_uses_already_logical_pango_metrics_and_margins_once() {
    let p = vec![PaneGeometry {
        id: "%1".into(),
        left: 0,
        top: 1,
        width: 40,
        height: 19,
    }];
    let geom = CellGeom {
        cell_w: 8.5,
        cell_h: 17.5,
        origin_x: 12.0,
        origin_y: 24.0,
        dpr: 2.0,
        font_size: 14.0,
    };
    let layout = frame_layout(&p, &geom, (80, 20), &["%1".to_string()].into(), true);
    assert_eq!(
        layout.frames,
        vec![serde_json::json!([7.0, 7.0, 348.0, 370.0, true])]
    );
    assert_eq!(
        shell_pill_xy(&layout.rows, "%1", (12.0, 24.0), (0, 1), (8.5, 17.5)),
        (15, 13)
    );
    assert_eq!(grip_rect(&layout.gutters[0]), (344, 13, 24, 358));
    let mut other = geom;
    other.dpr = 1.0;
    assert_eq!(
        layout,
        frame_layout(&p, &other, (80, 20), &["%1".to_string()].into(), true)
    );
}

#[test]
fn hidden_completion_retires_coalesced_flight_and_resume_can_start_fresh() {
    use comandos_app::ui::overlays::GeometryGate;
    let mut gate = GeometryGate::default();
    let first = gate.request().unwrap();
    gate.request();
    let (_, next) = gate.finish(first);
    let stale = next.unwrap();
    gate.pause();
    let fresh = gate.request().unwrap();
    assert_ne!(fresh, stale);
    assert_eq!(gate.finish(stale), (false, None));
    assert_eq!(gate.finish(fresh), (true, None));
}

struct SessionGeometry {
    records: std::cell::RefCell<std::collections::VecDeque<String>>,
    cancel: std::cell::Cell<bool>,
    reads: std::cell::Cell<u8>,
    cancel_at: u8,
    socket: std::path::PathBuf,
}
impl comandos_app::ui::clipboard::TmuxIo for SessionGeometry {
    fn mode(&self) -> comandos_app::config::RunMode {
        comandos_app::config::RunMode::Sandbox
    }
    fn socket(&self) -> &std::path::Path {
        &self.socket
    }
    fn read(
        &self,
        _: &[&str],
    ) -> Result<comandos_app::tmux::TmuxOut, comandos_app::tmux::TmuxError> {
        self.reads.set(self.reads.get() + 1);
        if self.reads.get() == self.cancel_at {
            self.cancel.set(true);
        }
        Ok(comandos_app::tmux::TmuxOut {
            code: 0,
            stdout: self.records.borrow_mut().pop_front().unwrap(),
            stderr: String::new(),
        })
    }
    fn mutate(
        &self,
        _: &[&str],
        _: Option<&[u8]>,
    ) -> Result<comandos_app::tmux::TmuxOut, comandos_app::tmux::TmuxError> {
        panic!("geometry reader cannot mutate tmux")
    }
}
#[test]
fn actual_geometry_job_rejects_wrong_session_recycled_identity_and_cancelled_reads() {
    use comandos_app::ui::overlays::read_geometry_when;
    for (pin, now, cancel_at, expected) in [
        ("111|$1|42|fixture", "111|$1|42|fixture", 0, true),
        ("111|$1|42|other", "111|$1|42|other", 0, false),
        ("111|$1|42|fixture", "111|$2|42|fixture", 0, false),
        ("garbage", "garbage", 0, false),
        ("111|$1|42|fixture", "111|$1|42|fixture", 1, false),
        ("111|$1|42|fixture", "111|$1|42|fixture", 2, false),
        ("111|$1|42|fixture", "111|$1|42|fixture", 3, false),
    ] {
        let fake = SessionGeometry {
            records: std::cell::RefCell::new(
                [pin.into(), "%1 0 1 40 grok 29 1\n".into(), now.into()].into(),
            ),
            cancel: std::cell::Cell::new(false),
            reads: std::cell::Cell::new(0),
            cancel_at,
            socket: std::env::temp_dir().join("session-geometry-private-S"),
        };
        assert_eq!(
            read_geometry_when(&fake, "fixture", || !fake.cancel.get()).is_ok(),
            expected,
            "{pin} → {now}, cancel={cancel_at}"
        );
    }
}

#[path = "support/t17_native.rs"]
mod native;
#[test]
fn actual_native_card_constructor_matches_original_gtk_boundary_for_models_harnesses_and_titles() {
    use serde_json::json;
    let mut cases = vec![];
    for motor in [
        "claude", "codex", "grok", "opencode", "gemini", "acp", "agy",
    ] {
        for state in ["verified", "changing", "detecting"] {
            cases.push(json!({"title":"Fixture <&> 'quoted' 漢","fg":"#abcdef","pane":{"harness":"agy","motor":motor,"state":state,"model":"model <&>","target":"next<&>","effort":"high","tierSym":"S","pane":"%7","hAcct":"fixture-a","mAcct":"fixture-b"}}));
        }
    }
    let actual = native::execute(&json!(cases));
    let original = oracle::original(json!({"op":"cards","cases":cases,"statuses":[null]}));
    assert_eq!(actual, original);
}

#[test]
fn actual_page_cancellation_stops_only_its_handler_retry_once_without_gtk_init() {
    let outcome = native::execute(&serde_json::json!({"op":"cancel"}));
    assert_eq!(
        outcome,
        serde_json::json!({"closed":true,"retry_empty":true,"fired":false,"stops":1,"unregisters":["extensions"]})
    );
}

#[test]
fn hover_follows_current_frame_bounds_without_changing_active_pane() {
    use comandos_app::ui::overlays::FrameLayout;
    let mut layout = FrameLayout::default();
    layout.frames = vec![
        serde_json::json!([0, 0, 100, 200, true]),
        serde_json::json!([110, 0, 100, 200, false]),
    ];
    assert_eq!(layout.frame_at(20., 40.), Some(0));
    assert_eq!(layout.frame_at(120., 40.), Some(1));
    assert_eq!(layout.frame_at(105., 40.), None);
    assert_eq!(layout.frame_at(210., 40.), None);
    assert_eq!(layout.frame_at(20., 200.), None);
    assert_eq!(layout.frames[0][4], true);
    assert_eq!(layout.frames[1][4], false);
    layout.frames[1] = serde_json::json!([80, 0, 70, 200, false]);
    assert_eq!(layout.frame_at(170., 40.), None);
    assert_eq!(layout.frame_at(120., 40.), Some(1));
}

#[test]
fn header_buttons_never_cover_the_model_name() {
    use comandos_app::ui::overlays::{KeysFit, keys_fit};
    // 42-column pane (~380 px): card text "Cobbler | fable-5-1" ~150 px, buttons 270 px.
    assert_eq!(keys_fit(380, 150, 270, 60), KeysFit::Icons);
    assert_eq!(keys_fit(1200, 220, 270, 60), KeysFit::Full);
    assert_eq!(keys_fit(200, 170, 270, 60), KeysFit::Hidden, "sin espacio: se esconden");
    // The edge: exactly the room left after the card text and its gap.
    assert_eq!(keys_fit(500, 214, 270, 60), KeysFit::Full);
    assert_eq!(keys_fit(500, 215, 270, 60), KeysFit::Icons);
}
