use comandos_core::dashboard_access as policy;
use serde_json::{Value, json};

fn cases(group: &str) -> Vec<Value> {
    let value: Value = serde_json::from_str(include_str!("dashboard_access_fixture.json")).unwrap();
    value[group].as_array().unwrap().clone()
}

fn rejection(value: Option<policy::Rejection>) -> Value {
    value.map_or(
        Value::Null,
        |value| json!({"status":value.status,"message":value.message,"close":value.close}),
    )
}

fn headers(value: &Value) -> Vec<(&str, &str)> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row[0].as_str().unwrap(), row[1].as_str().unwrap()))
        .collect()
}

fn method(value: &str) -> policy::Method {
    match value {
        "GET" => policy::Method::Get,
        "POST" => policy::Method::Post,
        "DELETE" => policy::Method::Delete,
        _ => panic!("fixture method"),
    }
}

fn request<'a>(row: &'a Value, headers: &'a [(&'a str, &'a str)]) -> policy::Request<'a> {
    policy::Request {
        method: method(row["method"].as_str().unwrap()),
        path: row["path"].as_str().unwrap(),
        peer_ip: row["peer"].as_str(),
        headers,
    }
}

#[test]
fn authority_and_loopback_helpers_keep_exact_source_boundaries() {
    for row in cases("authority") {
        let kind = match row["kind"].as_str().unwrap() {
            "localhost" => policy::AuthorityKind::Localhost,
            "direct" => policy::AuthorityKind::DirectLocal,
            "allowed" => policy::AuthorityKind::Allowed,
            _ => panic!("fixture authority"),
        };
        assert_eq!(
            json!(policy::authority_matches(
                kind,
                row["value"].as_str().unwrap()
            )),
            row["expected"],
            "{row}"
        );
    }
    for row in cases("ip") {
        assert_eq!(
            json!(policy::ip_is_loopback(row["value"].as_str().unwrap())),
            row["expected"],
            "{row}"
        );
    }
    for row in cases("xff") {
        assert_eq!(
            json!(policy::xff_is_loopback_chain(
                row["value"].as_str().unwrap()
            )),
            row["expected"],
            "{row}"
        );
    }
}

#[test]
fn origins_follow_urllib_hostname_port_and_path_rules() {
    for row in cases("origin") {
        assert_eq!(
            json!(policy::origin_matches_host(
                row["origin"].as_str().unwrap(),
                row["host"].as_str().unwrap()
            )),
            row["expected"],
            "{row}"
        );
    }
}

#[test]
fn token_sources_keep_precedence_duplicate_order_and_url_decoding() {
    for row in cases("token") {
        let h = headers(&row["headers"]);
        let path = row["path"].as_str().unwrap();
        let cookies = policy::cookies(&h);
        let actual = json!({"cookieKeys":cookies.keys().collect::<Vec<_>>(),"cookies":cookies,"query":policy::query_token(path),"presented":policy::presented_token(&h,path)});
        assert_eq!(actual, row["expected"], "{row}");
    }
    for row in cases("comparison") {
        let bytes = |key: &str| {
            row[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            json!(policy::token_matches(
                &bytes("presented"),
                &bytes("expected_token")
            )),
            row["expected"],
            "{row}"
        );
    }
}

#[test]
fn security_gate_distinguishes_local_proxy_remote_and_duplicate_headers() {
    for row in cases("gate") {
        let h = headers(&row["headers"]);
        let req = request(&row, &h);
        assert_eq!(
            rejection(policy::security_gate(
                &req,
                row["expected_token"].as_str().unwrap().as_bytes()
            )),
            row["expected"],
            "{row}"
        );
    }
}

#[test]
fn internal_producers_require_the_distinct_direct_local_header_channel() {
    for row in cases("internal") {
        let h = headers(&row["headers"]);
        let req = request(&row, &h);
        assert_eq!(
            json!(policy::internal_producer(
                &req,
                row["expected_token"].as_str().unwrap().as_bytes()
            )),
            row["expected"],
            "{row}"
        );
    }
}

#[test]
fn static_get_admission_keeps_api_prefixes_and_injected_file_existence() {
    for row in cases("get") {
        let h = headers(&row["headers"]);
        let req = request(&row, &h);
        let asset = match policy::public_asset(req.path, row["exists"].as_bool().unwrap()) {
            Ok(value) => json!(value),
            Err(value) => rejection(Some(value)),
        };
        assert_eq!(asset, row["asset"], "{row}");
        let actual = match policy::request_admission(
            &req,
            row["expected_token"].as_str().unwrap().as_bytes(),
            row["exists"].as_bool().unwrap(),
        ) {
            Ok(admit) => json!({"bodyLength":admit.body_length}),
            Err(value) => rejection(Some(value)),
        };
        assert_eq!(actual, row["expected"], "{row}");
    }
}

#[test]
fn mutation_admission_rejects_before_reading_and_preserves_close_differences() {
    for row in cases("body") {
        let h = headers(&row["headers"]);
        let req = request(&row, &h);
        let actual = match policy::request_admission(
            &req,
            row["expected_token"].as_str().unwrap().as_bytes(),
            false,
        ) {
            Ok(admit) => json!({"bodyLength":admit.body_length}),
            Err(value) => rejection(Some(value)),
        };
        assert_eq!(actual, row["expected"], "{row}");
    }
    for row in cases("parsed") {
        let parsed = row["valid"].as_bool().unwrap().then_some(&row["body"]);
        assert_eq!(
            rejection(policy::parsed_body_admission(parsed)),
            row["expected"],
            "{row}"
        );
    }
}

/// Native-only GET routes with no Python branch: a tailnet peer needs the token.
#[test]
fn native_only_gets_require_the_token_remotely() {
    let token = b"fake-correct";
    for path in [
        "/session/message?clientMessageId=abc",
        "/session/message",
        "/usage/board",
        "/credits/openrouter",
    ] {
        let remote = [("Host", "x.ts.net"), ("X-Forwarded-For", "100.64.0.9")];
        let req = policy::Request {
            method: policy::Method::Get,
            path,
            peer_ip: Some("127.0.0.1"),
            headers: &remote,
        };
        let denied = policy::request_admission(&req, token, false).err();
        assert_eq!(denied.map(|r| r.status), Some(401), "{path}");
        let authorized = [
            ("Host", "x.ts.net"),
            ("X-Forwarded-For", "100.64.0.9"),
            ("X-Comandos-Token", "fake-correct"),
        ];
        let req = policy::Request {
            method: policy::Method::Get,
            path,
            peer_ip: Some("127.0.0.1"),
            headers: &authorized,
        };
        assert!(policy::request_admission(&req, token, false).is_ok(), "{path}");
        let local = [("Host", "127.0.0.1:4777")];
        let req = policy::Request {
            method: policy::Method::Get,
            path,
            peer_ip: Some("127.0.0.1"),
            headers: &local,
        };
        assert!(policy::request_admission(&req, token, false).is_ok(), "{path} local");
    }
}
