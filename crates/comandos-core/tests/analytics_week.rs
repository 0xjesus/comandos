use comandos_core::{analytics_week as a, json::workspace_loads};
use serde_json::{Value, json};
fn rows(input: &Value) -> a::WeekRows {
    a::WeekRows::from_rows(
        input["turns"].as_array().unwrap(),
        input["spans"].as_array().unwrap(),
        comandos_core::focus::float(&input["now"]).unwrap(),
    )
}
fn week<'a>(input: &'a Value, rows: &'a a::WeekRows) -> a::WeekInput<'a> {
    a::WeekInput {
        now: comandos_core::focus::float(&input["now"]).unwrap(),
        offset: input["offset"].as_i64().unwrap(),
        limits: input["limits"].as_array().unwrap(),
        rows,
        snapshots: input["snapshots"].as_array().unwrap(),
        records: input["records"].as_array().unwrap(),
        tz_name: input["tz_name"].as_str().unwrap(),
    }
}
#[test]
fn aliases_projects_and_duration_labels_follow_reference() {
    assert_eq!(a::account_of(&json!(" unknown ")).unwrap(), "main");
    assert_eq!(a::account_of(&json!(" Work ")).unwrap(), "Work");
    assert_eq!(a::project_of(&json!("/x/ComandOS///")).unwrap(), "ComandOS");
    assert_eq!(a::project_of(&Value::Null).unwrap(), "Sin carpeta");
    assert_eq!(
        a::pomodoro_project(&json!(" ComandOS ⎇ branch ⫽30")).unwrap(),
        "ComandOS"
    );
    assert_eq!(a::pomodoro_project(&json!("X⫽30")).unwrap(), "X⫽30");
    assert_eq!(a::fmt_left(&json!(3599.9)).unwrap(), "59 min");
}
#[test]
fn sidebar_keeps_accounts_and_opencode_sessions_models_separate() {
    let accounts = vec![json!({"id":"claude:main","provider":"claude","alias":"main","week":70})];
    let original = accounts.clone();
    let turns = vec![
        json!({"provider":"groq","agent":"opencode","account":"unknown","finished":100,"tokens":100,"cost":0.5,"session":"s","model":"m"}),
        json!({"provider":"opencode","account":"main","finished":99,"tokens":200,"cost":0.25,"session":"s","model":"m"}),
        json!({"provider":"opencode","account":"work","finished":99,"tokens":400,"cost":2,"session":"s","model":"other"}),
    ];
    let out = a::sidebar_accounts(
        &accounts,
        &[json!({"provider":"claude","account":"main","plan":"Max"})],
        &turns,
        100.,
    )
    .unwrap();
    assert_eq!(accounts, original);
    assert_eq!(out[0]["plan"], "Max");
    assert_eq!(out[1]["id"], "opencode:main");
    assert_eq!(
        out[1]["measured"],
        json!({"sessions":1,"tokens":300,"costUsd":0.75,"models":["m"]})
    );
    assert_eq!(out[2]["measured"]["tokens"], 400);
}
#[test]
fn analytics_fixed_source_reference_preserves_dst_and_account_models() {
    let f = workspace_loads(include_str!("analytics_week_fixture.json")).unwrap();
    for (case, row) in f.as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let result = match row["kind"].as_str().unwrap() {
            "week" => a::build_week(&week(input, &rows(input))),
            "sessions" => a::sessions(
                input["turns"].as_array().unwrap(),
                input["spans"].as_array().unwrap(),
                input["tz_name"].as_str().unwrap(),
            )
            .map(Value::Array),
            "sidebar" => a::sidebar_accounts(
                input["accounts"].as_array().unwrap(),
                input["limits"].as_array().unwrap(),
                input["turns"].as_array().unwrap(),
                comandos_core::focus::float(&input["now"]).unwrap(),
            )
            .map(Value::Array),
            "account" => a::account_of(input).map(Value::String),
            "project" => a::project_of(input).map(Value::String),
            "pomodoro_project" => a::pomodoro_project(input).map(Value::String),
            "left" => a::fmt_left(input).map(Value::String),
            _ => panic!("unknown kind"),
        };
        if let Some(e) = row["error"].as_str() {
            assert_eq!(result.unwrap_err().kind, e, "{row}");
        } else {
            assert_json_close(
                &result.unwrap_or_else(|e| panic!("case {case} unexpected {e:?}")),
                &row["expected"],
                &format!("case {case}"),
            );
        }
    }
}
#[test]
fn empty_week_has_all_observable_model_fields_without_accounts() {
    let input = json!({"now":1790854560.,"offset":0,"limits":[],"turns":[],"spans":[],"snapshots":[],"records":[],"tz_name":a::TZ});
    let out = a::build_week(&week(&input, &rows(&input))).unwrap();
    assert_eq!(out["days"][0], json!(["2026-10-01", "jue", "1 oct"]));
    assert_eq!(out["week"]["label"], "24 sep – 1 oct");
    for key in ["accounts", "sessions", "waste", "pomodoros"] {
        assert_eq!(out[key], json!([]));
    }
    assert_eq!(out["lastWeek"], json!({}));
}
fn assert_json_close(actual: &Value, expected: &Value, path: &str) {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => {
            if a.as_str() == b.as_str() {
                return;
            }
            if !a.as_str().contains(['.', 'e', 'E']) && !b.as_str().contains(['.', 'e', 'E']) {
                assert_eq!(a.as_str(), b.as_str(), "{path}");
                return;
            }
            let x = a.as_str().parse::<f64>().unwrap();
            let y = b.as_str().parse::<f64>().unwrap();
            assert!(
                (x.is_nan() && y.is_nan())
                    || x == y
                    || (x.is_finite() && y.is_finite() && (x - y).abs() <= 1e-12 * y.abs().max(1.)),
                "{path}: {actual} != {expected}"
            );
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}");
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                assert_json_close(x, y, &format!("{path}/{i}"));
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len(), "{path}");
            for (k, v) in b {
                assert_json_close(a.get(k).unwrap(), v, &format!("{path}/{k}"));
            }
        }
        _ => assert_eq!(actual, expected, "{path}"),
    }
}
#[test]
fn week_rows_do_not_depend_on_turn_and_span_interleaving() {
    let turns = vec![
        json!({"provider":"claude","account":"main","git_root":"/r/a","pane_pwd":"/r/a","started":1790850000,"finished":1790850100,"tokens":10}),
        json!({"provider":"codex","account":"w","git_root":null,"pane_pwd":"/r/b","started":1790851000,"finished":1790851200,"tokens":20}),
    ];
    let spans = vec![
        json!({"provider":"grok","account":"main","git_root":"/r/c","started":1790849000,"finished":1790849500}),
        json!({"provider":"claude","account":"main","git_root":"/r/a","started":1790850100,"finished":1790850300}),
    ];
    let input = json!({"now":1790854560.,"offset":0,"limits":[],"snapshots":[],"records":[],"tz_name":a::TZ});
    let ordered = a::WeekRows::from_rows(&turns, &spans, 1790854560.);
    let mut mixed = a::WeekRows::new(1790854560.);
    mixed.push_span(&spans[0]);
    mixed.push_turn(&turns[0]);
    mixed.push_span(&spans[1]);
    mixed.push_turn(&turns[1]);
    let out = a::build_week(&week(&input, &ordered)).unwrap();
    assert_eq!(out, a::build_week(&week(&input, &mixed)).unwrap());
    assert_eq!(
        out["sessions"].as_array().unwrap().len(),
        a::sessions(&turns, &spans, a::TZ)
            .unwrap()
            .iter()
            .filter(|s| s["d"] == "2026-10-01")
            .count()
    );
}
#[test]
fn sidebar_counts_a_new_opencode_account_only_from_the_turn_that_adds_it() {
    let accounts = vec![json!({"id":"claude:main","provider":"claude","alias":"main"})];
    // `opencode:w` + alias `main` y `opencode` + alias `w:main` dan la misma cuenta.
    let turns = vec![
        json!({"provider":"opencode:w","account":null,"finished":100,"tokens":7}),
        json!({"provider":"codex","account":"x","finished":100,"tokens":"no es un número"}),
        json!({"provider":"opencode","account":"w:main","finished":100,"tokens":5,"model":"m"}),
        json!({"provider":"opencode:w","account":null,"finished":100,"tokens":11}),
        json!({"provider":"claude","account":"main","finished":100,"tokens":3}),
    ];
    let out = a::sidebar_accounts(&accounts, &[], &turns, 100.).unwrap();
    assert_eq!(out[1]["id"], "opencode:w:main");
    assert_eq!(out[1]["alias"], "w:main");
    assert_eq!(
        out[1]["measured"],
        json!({"sessions":0,"tokens":16,"costUsd":0.0,"models":["m"]})
    );
    assert_eq!(out[0]["measured"]["tokens"], 3);
    // Un turno contado que lanza gana al de después; uno que no se cuenta, no lanza.
    let mut bad = turns.clone();
    bad.push(json!({"provider":"claude","account":"main","finished":"x"}));
    bad.insert(
        4,
        json!({"provider":"claude","account":"main","finished":100,"tokens":[1]}),
    );
    let err = a::sidebar_accounts(&accounts, &[], &bad, 100.).unwrap_err();
    assert_eq!(err.kind, "TypeError");
    let rows = a::WeekRows::from_rows(&bad, &[], 100.);
    assert_eq!(
        a::sidebar_accounts_from(&accounts, &[], &rows)
            .unwrap_err()
            .kind,
        "TypeError"
    );
}
/// Lo de la columna (`build_week` + `sidebar_accounts_from`) con todas las filas.
fn sidebar_full(input: &Value, rows: &a::WeekRows) -> Option<Value> {
    let out = a::build_week(&week(input, rows)).ok()?;
    a::sidebar_accounts_from(
        out["accounts"].as_array().unwrap(),
        input["limits"].as_array().unwrap(),
        rows,
    )
    .ok()
    .map(Value::Array)
}
/// `sidebar_week` sobre las filas que lee el atajo de la columna: las que terminan
/// desde `sidebar_since` (el `finished_at >= ?` de la base), sin fotos de cuota
/// en la semana actual y sin registros de Pomodoro.
fn sidebar_fast(input: &Value) -> Option<Value> {
    let now = comandos_core::focus::float(&input["now"]).unwrap();
    let offset = input["offset"].as_i64().unwrap();
    let since = a::sidebar_since(now, offset, input["tz_name"].as_str().unwrap()).ok()?;
    let keep = |rows: &Value| -> Vec<Value> {
        rows.as_array()
            .unwrap()
            .iter()
            .filter(|r| r["finished"].as_f64().is_some_and(|f| f >= since))
            .cloned()
            .collect()
    };
    let rows = a::WeekRows::from_rows(&keep(&input["turns"]), &keep(&input["spans"]), now);
    let none: Vec<Value> = vec![];
    a::sidebar_week(&a::WeekInput {
        now,
        offset,
        limits: input["limits"].as_array().unwrap(),
        rows: &rows,
        snapshots: if offset < 0 {
            input["snapshots"].as_array().unwrap()
        } else {
            &none
        },
        records: &none,
        tz_name: input["tz_name"].as_str().unwrap(),
    })
    .ok()
    .map(Value::Array)
}
#[test]
fn sidebar_week_matches_full_week_on_reference_cases() {
    let f = workspace_loads(include_str!("analytics_week_fixture.json")).unwrap();
    let mut compared = 0;
    for (case, row) in f.as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        if row["kind"] != "week" || !matches!(input["offset"].as_i64(), Some(0 | -1)) {
            continue;
        }
        let Some(full) = sidebar_full(input, &rows(input)) else {
            continue;
        };
        assert_eq!(sidebar_fast(input), Some(full), "caso {case}");
        compared += 1;
    }
    assert!(compared >= 20, "{compared}");
}
/// Cadenas de turnos que cruzan el corte de `sidebar_since` y la medianoche que
/// abre la ventana, turnos viejos (fuera de la ventana) y OpenCode: las cuentas
/// de la columna salen iguales con las filas recortadas.
#[test]
fn sidebar_week_ignores_rows_before_its_cutoff() {
    let now = 1790864160.;
    for offset in [0, -1] {
        let open = a::sidebar_since(now, offset, a::TZ).unwrap();
        let mut turns = vec![];
        let mut spans = vec![];
        let mut seed: u64 = 7;
        let mut rand = |n: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let accounts = [
            ("claude", "main"),
            ("claude", "relotto"),
            ("codex", "main"),
            ("opencode", "main"),
        ];
        // Turnos densos alrededor del corte (huecos de 0 a 30 min) y dispersos en 17 días.
        let mut at = open - 6. * 3600.;
        while at < open + 6. * 3600. {
            let (p, acc) = accounts[rand(4) as usize];
            let len = 60. + rand(1500) as f64;
            turns.push(json!({"provider":p,"agent":p,"account":acc,"git_root":format!("/r/{}",rand(3)),"pane_pwd":"/r/x","started":at,"finished":at+len,"tokens":rand(100000),"cost":rand(100) as f64/100.,"session":format!("s{}",rand(5)),"model":format!("m{}",rand(3))}));
            at += len + rand(1800) as f64;
        }
        for i in 0..400 {
            let start = now - 17. * 86400. + (i as f64) * 3670. + rand(600) as f64;
            let (p, acc) = accounts[rand(4) as usize];
            if rand(5) == 0 {
                spans.push(json!({"provider":p,"account":acc,"git_root":"/r/1","started":start,"finished":start+900.}));
            } else {
                turns.push(json!({"provider":p,"agent":p,"account":acc,"git_root":null,"pane_pwd":format!("/r/{}",rand(3)),"started":start,"finished":start+rand(2400) as f64,"tokens":rand(50000),"cost":0.1,"session":"s","model":"m"}));
            }
        }
        // Una sesión larga que empieza antes del corte y sigue dentro de la ventana.
        turns.push(json!({"provider":"claude","account":"main","git_root":"/r/0","pane_pwd":"/r/0","started":open-7200.,"finished":open+7200.,"tokens":5}));
        let input = json!({"now":now,"offset":offset,"limits":[
            {"provider":"claude","account":"main","window":"7d","scope":"","percent":36.0,"resets_at":now+86400.,"plan":"max"},
            {"provider":"codex","account":"main","window":"5h","scope":"","percent":12.5,"resets_at":now+3600.}
        ],"turns":turns,"spans":spans,"snapshots":[
            {"provider":"claude","account":"main","window":"7d","scope":"","percent":40,"resets_at":now-2.*86400.}
        ],"records":[],"tz_name":a::TZ});
        let full = sidebar_full(&input, &rows(&input)).unwrap();
        assert!(full.as_array().unwrap().len() >= 4, "{full}");
        assert_eq!(sidebar_fast(&input), Some(full), "offset {offset}");
    }
}
