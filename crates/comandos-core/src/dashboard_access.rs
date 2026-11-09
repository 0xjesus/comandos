//! Pure dashboard admission policy. Header names are ASCII case-insensitive;
//! duplicate values retain wire order. Callers supply the token and file fact.
use serde_json::{Map, Value};
use std::net::{IpAddr, Ipv6Addr};
use subtle::ConstantTimeEq;

pub type Headers<'a> = &'a [(&'a str, &'a str)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Delete,
}

#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub method: Method,
    pub path: &'a str,
    pub peer_ip: Option<&'a str>,
    pub headers: Headers<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejection {
    pub status: u16,
    pub message: &'static str,
    pub close: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admission {
    /// None for GET; bounded read length for POST/DELETE. A zero-length body
    /// must be treated as the source's empty JSON object by the future server.
    pub body_length: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityKind {
    Localhost,
    DirectLocal,
    Allowed,
}

const HOST_DENIED: Rejection = Rejection {
    status: 403,
    message: "Host no permitido",
    close: false,
};
const ORIGIN_DENIED: Rejection = Rejection {
    status: 403,
    message: "Origen no permitido",
    close: false,
};
const TOKEN_DENIED: Rejection = Rejection {
    status: 401,
    message: "No autorizado (token requerido para acceso remoto)",
    close: false,
};
const JSON_INVALID: Rejection = Rejection {
    status: 400,
    message: "JSON invalido",
    close: true,
};
const PAYLOAD_TOO_LARGE: Rejection = Rejection {
    status: 413,
    message: "Payload demasiado grande",
    close: true,
};
const INTERNAL_ERROR: Rejection = Rejection {
    status: 500,
    message: "Error interno del tablero",
    close: true,
};

/// The complete source prefix list. Matching deliberately uses raw request
/// paths and starts_with, before the public-asset exception is considered.
pub const API_GET: &[&str] = &[
    "/accounts",
    "/session-config-history",
    "/pane-extensions",
    "/pomodoro",
    "/providers",
    "/optimization/plans",
    "/proxy",
    "/model/status",
    "/sovereignty",
    "/ui-log/summary",
    "/state",
    "/usage/state",
    "/usage/guard",
    "/usage/changes",
    "/usage/provider-compare",
    "/fs/dirs",
    "/models/latest",
    "/news/latest",
    "/news/edition",
    "/push/",
    "/notifs/count",
    "/usage/analytics",
    "/usage/interactions",
    "/usage/experiments",
    "/opencode/models",
    "/model-tiers",
    "/dedication",
    "/active-tab",
    "/tab-models",
    "/session-profiles",
    "/extension-usage",
    "/session-brain",
    "/project-profiles",
    "/events",
    "/conf",
    "/prefs",
    "/ssh",
    "/tabs",
    "/workspace",
    "/notices",
    "/analytics/week",
    "/analytics/resources",
    "/tab-history",
    "/remote-state",
    "/remote-qr.png",
    "/snippets",
    "/tmux-mouse",
    "/commands/catalog",
    "/chains",
    "/webterm-token",
    "/web/status",
    "/news/source",
    "/news/chat",
    "/news/notes",
    "/news/saved",
    "/news/media",
    "/work-marks",
    // Ruta nativa sin heredado (ficha viva): prompts y respuestas de los agentes.
    "/session-feed",
];

fn py_strip(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}'))
}

fn header_name_matches(value: &str, name: &str) -> bool {
    value.eq_ignore_ascii_case(name)
        || (!value.is_ascii() && value.to_lowercase() == name.to_ascii_lowercase())
}

fn first_header<'a>(headers: Headers<'a>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| header_name_matches(key, name))
        .map(|(_, value)| *value)
}

fn header_values<'a>(headers: Headers<'a>, name: &str) -> Vec<&'a str> {
    headers
        .iter()
        .filter(|(key, _)| header_name_matches(key, name))
        .map(|(_, value)| *value)
        .collect()
}

fn regex_letter(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, 'İ' | 'ı' | 'ſ' | 'K')
}

fn label_allowed(value: &str) -> bool {
    let count = value.chars().count();
    (1..=63).contains(&count)
        && value.chars().next().is_some_and(regex_letter)
        && value.chars().next_back().is_some_and(regex_letter)
        && value.chars().all(|c| regex_letter(c) || c == '-')
}

fn regex_literal(value: &str, literal: &str) -> bool {
    value.chars().count() == literal.len()
        && value
            .chars()
            .zip(literal.chars())
            .all(|(a, b)| a.eq_ignore_ascii_case(&b) || (a == 'ſ' && b == 's'))
}

fn authority_port(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|b| b.is_ascii_digit())
        && (value.len() <= 4
            || (value.len() == 5
                && value
                    .parse::<u32>()
                    .is_ok_and(|n| (10_000..=65_535).contains(&n))))
}

pub fn authority_matches(kind: AuthorityKind, authority: &str) -> bool {
    let (hostname, port) = if authority.starts_with('[') {
        let Some(end) = authority.find(']') else {
            return false;
        };
        let host = &authority[..=end];
        let suffix = &authority[end + 1..];
        let port = if suffix.is_empty() {
            None
        } else if let Some(port) = suffix.strip_prefix(':') {
            Some(port)
        } else {
            return false;
        };
        (host, port)
    } else {
        authority
            .rsplit_once(':')
            .map_or((authority, None), |(host, port)| (host, Some(port)))
    };
    if hostname.chars().count() > 253 || port.is_some_and(|p| !authority_port(p)) {
        return false;
    }
    if kind != AuthorityKind::Localhost
        && (hostname.eq_ignore_ascii_case("127.0.0.1")
            || hostname.eq_ignore_ascii_case("[::1]")
            || hostname.eq_ignore_ascii_case("[::ffff:127.0.0.1]"))
    {
        return true;
    }
    let labels = hostname.split('.').collect::<Vec<_>>();
    if labels
        .last()
        .is_some_and(|last| regex_literal(last, "localhost"))
    {
        return labels[..labels.len() - 1]
            .iter()
            .all(|label| label_allowed(label));
    }
    kind == AuthorityKind::Allowed
        && labels.len() >= 3
        && regex_literal(labels[labels.len() - 2], "ts")
        && regex_literal(labels[labels.len() - 1], "net")
        && labels[..labels.len() - 2]
            .iter()
            .all(|label| label_allowed(label))
}

fn ipv6_scoped(value: &str) -> Option<Ipv6Addr> {
    let address = if let Some((address, zone)) = value.split_once('%') {
        if zone.is_empty() || zone.contains('%') {
            return None;
        }
        address
    } else {
        value
    };
    address.parse().ok()
}

pub fn ip_is_loopback(value: &str) -> bool {
    let value = py_strip(value);
    match value.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => address.is_loopback(),
        Ok(IpAddr::V6(address)) => {
            address.is_loopback() || address.to_ipv4_mapped().is_some_and(|v| v.is_loopback())
        }
        Err(_) => ipv6_scoped(value).is_some_and(|address| {
            address.is_loopback() || address.to_ipv4_mapped().is_some_and(|v| v.is_loopback())
        }),
    }
}

pub fn xff_is_loopback_chain(value: &str) -> bool {
    !value.is_empty()
        && value
            .split(',')
            .all(|part| !py_strip(part).is_empty() && ip_is_loopback(part))
}

struct SplitUrl {
    scheme: String,
    netloc: String,
    path: String,
    query: String,
    fragment: String,
}

fn check_bracketed_netloc(value: &str) -> Result<(), ()> {
    if value.contains('[') != value.contains(']') {
        return Err(());
    }
    if !value.contains('[') {
        return Ok(());
    }
    let hostinfo = value.rsplit_once('@').map_or(value, |(_, host)| host);
    let (before, bracketed) = hostinfo.split_once('[').ok_or(())?;
    if !before.is_empty() {
        return Err(());
    }
    let (hostname, suffix) = bracketed.split_once(']').ok_or(())?;
    if !suffix.is_empty() && !suffix.starts_with(':') {
        return Err(());
    }
    if let Some(future) = hostname.strip_prefix('v') {
        let (version, address) = future.split_once('.').ok_or(())?;
        if version.is_empty()
            || !version.bytes().all(|c| c.is_ascii_hexdigit())
            || address.is_empty()
        {
            return Err(());
        }
    } else if ipv6_scoped(hostname).is_none() {
        return Err(());
    }
    Ok(())
}

fn split_url(value: &str) -> Result<SplitUrl, ()> {
    // Match the bound Python 3.10 urllib parser, including leading C0 removal,
    // embedded tab/CR/LF removal, empty delimiters, and non-letter schemes.
    let cleaned = value
        .trim_start_matches(|c| c <= '\u{20}')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect::<String>();
    let mut remaining = cleaned.as_str();
    let mut scheme = String::new();
    if let Some((candidate, suffix)) = remaining.split_once(':')
        && !candidate.is_empty()
        && candidate
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
    {
        scheme = candidate.to_ascii_lowercase();
        remaining = suffix;
    }
    let mut netloc = String::new();
    if let Some(authority) = remaining.strip_prefix("//") {
        let end = authority.find(['/', '?', '#']).unwrap_or(authority.len());
        netloc = authority[..end].into();
        remaining = &authority[end..];
        check_bracketed_netloc(&netloc)?;
        // All Unicode 13 NFKC characters introducing urllib's forbidden
        // authority delimiters. The table is bound to the source oracle.
        if netloc.chars().any(|c| {
            matches!(
                c as u32,
                8263 | 8264
                    | 8265
                    | 8448
                    | 8449
                    | 8453
                    | 8454
                    | 10868
                    | 65043
                    | 65046
                    | 65109
                    | 65110
                    | 65119
                    | 65131
                    | 65283
                    | 65295
                    | 65306
                    | 65311
                    | 65312
            )
        }) {
            return Err(());
        }
    }
    let (remaining, fragment) = remaining.split_once('#').unwrap_or((remaining, ""));
    let (path, query) = remaining.split_once('?').unwrap_or((remaining, ""));
    Ok(SplitUrl {
        scheme,
        netloc,
        path: path.into(),
        query: query.into(),
        fragment: fragment.into(),
    })
}

fn hostname_port(netloc: &str) -> Result<(String, Option<u16>), ()> {
    let hostinfo = netloc.rsplit_once('@').map_or(netloc, |(_, host)| host);
    let (hostname, port) = if let Some(bracketed) = hostinfo.strip_prefix('[') {
        let (hostname, suffix) = bracketed.split_once(']').ok_or(())?;
        (hostname, suffix.strip_prefix(':').unwrap_or(""))
    } else {
        hostinfo.split_once(':').unwrap_or((hostinfo, ""))
    };
    let port = if port.is_empty() {
        None
    } else {
        if port.len() > 4300 || !port.bytes().all(|c| c.is_ascii_digit()) {
            return Err(());
        }
        let digits = port.trim_start_matches('0');
        Some(if digits.is_empty() {
            0
        } else {
            digits.parse::<u16>().map_err(|_| ())?
        })
    };
    let hostname = if let Some((address, zone)) = hostname.split_once('%') {
        format!("{}%{zone}", address.to_lowercase())
    } else {
        hostname.to_lowercase()
    };
    Ok((hostname, port))
}

pub fn origin_matches_host(origin: &str, host: &str) -> bool {
    let Ok(o) = split_url(origin) else {
        return false;
    };
    let Ok(h) = split_url(&format!("//{host}")) else {
        return false;
    };
    let Ok((o_hostname, o_port)) = hostname_port(&o.netloc) else {
        return false;
    };
    let Ok((h_hostname, h_port)) = hostname_port(&h.netloc) else {
        return false;
    };
    if !matches!(o.scheme.as_str(), "http" | "https")
        || o_hostname.is_empty()
        || h_hostname.is_empty()
        || !o.path.is_empty()
        || !o.query.is_empty()
        || !o.fragment.is_empty()
        || o.netloc.contains('@')
    {
        return false;
    }
    let default = if o.scheme == "http" { 80 } else { 443 };
    o_hostname == h_hostname
        && o_port.filter(|p| *p != 0).unwrap_or(default)
            == h_port.filter(|p| *p != 0).unwrap_or(default)
}

pub fn cookies(headers: Headers<'_>) -> Map<String, Value> {
    let mut out = Map::new();
    for part in first_header(headers, "Cookie").unwrap_or("").split(';') {
        let part = py_strip(part);
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        if !key.is_empty() {
            out.insert(key.into(), Value::String(value.into()));
        }
    }
    out
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(a), Some(b)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((a * 16 + b) as u8);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn query_token(path: &str) -> String {
    let Some((_, query)) = path.split_once('?') else {
        return String::new();
    };
    for field in query.split('&') {
        let (key, value) = field.split_once('=').unwrap_or((field, ""));
        if value.is_empty() {
            continue;
        }
        if percent_decode(key) == "token" {
            return py_strip(&percent_decode(value)).into();
        }
    }
    String::new()
}

/// Python URL splitting for route matching; path bytes remain percent-encoded.
pub fn request_target_parts(target: &str) -> Option<(String, String)> {
    split_url(target)
        .ok()
        .map(|parts| (parts.path, parts.query))
}

/// Python parse_qsl semantics with ordered duplicate decoded keys.
pub fn query_pairs(query: &str, keep_blank_values: bool) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|field| !field.is_empty())
        .filter_map(|field| {
            let (key, value) = field.split_once('=').unwrap_or((field, ""));
            if value.is_empty() && !keep_blank_values {
                return None;
            }
            Some((percent_decode(key), percent_decode(value)))
        })
        .collect()
}

pub fn presented_token(headers: Headers<'_>, path: &str) -> String {
    if let Some(bearer) = first_header(headers, "Authorization")
        .unwrap_or("")
        .strip_prefix("Bearer ")
    {
        return py_strip(bearer).into();
    }
    let header = py_strip(first_header(headers, "X-Comandos-Token").unwrap_or(""));
    if !header.is_empty() {
        return header.into();
    }
    if let Some(cookie) = cookies(headers)
        .get("cc_token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        return cookie.into();
    }
    query_token(path)
}

/// Supplied expected bytes must be UTF-8, as access_token().encode("utf-8")
/// requires. Malformed presented bytes, including surrogatepass bytes, deny
/// without panic. Maintained subtle compares equal-length contents in constant
/// time; different lengths are observable, as with the source compare_digest.
pub fn token_matches(presented: &[u8], expected: &[u8]) -> bool {
    std::str::from_utf8(expected).is_ok() && bool::from(presented.ct_eq(expected))
}

pub fn security_gate(request: &Request<'_>, expected_token: &[u8]) -> Option<Rejection> {
    let hosts = header_values(request.headers, "Host");
    if hosts.len() != 1 || !authority_matches(AuthorityKind::Allowed, hosts[0]) {
        return Some(HOST_DENIED);
    }
    let host = hosts[0];
    let origins = header_values(request.headers, "Origin");
    if origins.len() > 1
        || origins
            .first()
            .is_some_and(|origin| !origin_matches_host(origin, host))
    {
        return Some(ORIGIN_DENIED);
    }
    let forwarded = header_values(request.headers, "X-Forwarded-For");
    let local_peer = request.peer_ip.is_some_and(ip_is_loopback);
    let direct =
        local_peer && forwarded.is_empty() && authority_matches(AuthorityKind::DirectLocal, host);
    let dev_proxy = local_peer
        && !forwarded.is_empty()
        && authority_matches(AuthorityKind::Localhost, host)
        && xff_is_loopback_chain(&forwarded.join(","));
    if direct
        || dev_proxy
        || token_matches(
            presented_token(request.headers, request.path).as_bytes(),
            expected_token,
        )
    {
        None
    } else {
        Some(TOKEN_DENIED)
    }
}

pub fn internal_producer(request: &Request<'_>, expected_token: &[u8]) -> bool {
    request.peer_ip.is_some_and(ip_is_loopback)
        && header_values(request.headers, "X-Forwarded-For").is_empty()
        && header_values(request.headers, "Origin").is_empty()
        && token_matches(
            py_strip(first_header(request.headers, "X-Comandos-Token").unwrap_or("")).as_bytes(),
            expected_token,
        )
}

pub fn public_asset(path: &str, existing_file: bool) -> Result<bool, Rejection> {
    let parts = split_url(path).map_err(|_| INTERNAL_ERROR)?;
    let Some(path) = parts.path.strip_prefix('/') else {
        return Ok(false);
    };
    let basename = path
        .strip_suffix(".css")
        .or_else(|| path.strip_suffix(".js"));
    let lexical = basename.is_some_and(|path| {
        path.split('/').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        })
    });
    Ok(lexical && existing_file)
}

fn content_length(headers: Headers<'_>, method: Method) -> Result<usize, Rejection> {
    let raw = first_header(headers, "Content-Length").unwrap_or("0");
    // int() rejects ASCII file/group/record/unit separators even though str's
    // strip() accepts them. Keep that HTTP string distinction at this boundary.
    if raw.contains(['\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}']) {
        return Err(JSON_INVALID);
    }
    let trimmed = py_strip(raw);
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    if digits.chars().filter(|c| *c != '_').count() > 4300 {
        return Err(JSON_INVALID);
    }
    let integer =
        crate::focus::integer_string(&Value::String(raw.into())).map_err(|_| JSON_INVALID)?;
    if integer.starts_with('-') {
        return Err(if method == Method::Delete {
            PAYLOAD_TOO_LARGE
        } else {
            JSON_INVALID
        });
    }
    let cap = if method == Method::Delete {
        "64000"
    } else {
        "20000000"
    };
    if integer.len() > cap.len() || (integer.len() == cap.len() && integer.as_str() > cap) {
        return Err(PAYLOAD_TOO_LARGE);
    }
    integer.parse().map_err(|_| JSON_INVALID)
}

pub fn request_admission(
    request: &Request<'_>,
    expected_token: &[u8],
    existing_asset_file: bool,
) -> Result<Admission, Rejection> {
    if request.method == Method::Get {
        let hosts = header_values(request.headers, "Host");
        if hosts.len() != 1 || !authority_matches(AuthorityKind::Allowed, hosts[0]) {
            return Err(HOST_DENIED);
        }
        if API_GET
            .iter()
            .any(|prefix| request.path.starts_with(prefix))
            && !public_asset(request.path, existing_asset_file)?
            && let Some(rejection) = security_gate(request, expected_token)
        {
            return Err(rejection);
        }
        return Ok(Admission { body_length: None });
    }
    if let Some(mut rejection) = security_gate(request, expected_token) {
        rejection.close = true;
        return Err(rejection);
    }
    Ok(Admission {
        body_length: Some(content_length(request.headers, request.method)?),
    })
}
/// None means JSON decoding failed. A parsed non-object differs from a decoder
/// failure in whether the response must close the connection.
pub fn parsed_body_admission(parsed: Option<&Value>) -> Option<Rejection> {
    match parsed {
        None => Some(JSON_INVALID),
        Some(value) if value.is_object() => None,
        Some(_) => Some(Rejection {
            status: 400,
            message: "El cuerpo debe ser un objeto JSON",
            close: false,
        }),
    }
}
