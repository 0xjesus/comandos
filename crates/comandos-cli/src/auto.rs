//! `comandos auto`: what the Modo auto orchestrator (a Claude Code session the
//! person talks to while driving) uses to see and drive the other sessions,
//! through this machine's ComandOS server. Short plain text, made to be read
//! aloud: no Markdown, no ids unless they are needed to tell two apart.
//!
//! - `sesiones`: who waits, who works, what each one said last.
//! - `leer SESIÓN [N]`: the last N turns of a session and its open question.
//! - `enviar SESIÓN TEXTO…`: writes TEXTO in that session and presses Enter.
//! - `responder SESIÓN NÚMERO [TEXTO…]`: answers the open question with that
//!   option (with TEXTO when the option is «Type something»).
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    time::Duration,
};

const USAGE: &str = "uso: comandos auto sesiones | leer SESIÓN [N] | enviar SESIÓN TEXTO… | responder SESIÓN NÚMERO [TEXTO…]";

/// The orchestrator's own tab: never listed nor driven.
pub const SELF_LABEL: &str = "Modo auto";

fn port() -> u16 {
    std::env::var("COMANDOS_DASH_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(4777)
}

/// One HTTP/1.1 request to the local server: (status, JSON body).
fn call(method: &str, path: &str, body: Option<&Value>) -> Result<(u16, Value), String> {
    let down = || "ComandOS no responde en este equipo.".to_owned();
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port()));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5)).map_err(|_| down())?;
    stream.set_read_timeout(Some(Duration::from_secs(40))).map_err(|_| down())?;
    let payload = body.map(Value::to_string).unwrap_or_default();
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nAccept: application/json\r\n",
        Ipv4Addr::LOCALHOST,
        port()
    );
    if body.is_some() {
        head.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", payload.len()));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).map_err(|_| down())?;
    stream.write_all(payload.as_bytes()).map_err(|_| down())?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).map_err(|_| down())?;
    parse_response(&raw).ok_or_else(down)
}

/// Status and JSON body of a raw HTTP/1.1 response (plain or chunked).
fn parse_response(raw: &[u8]) -> Option<(u16, Value)> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(raw.get(..split)?).ok()?;
    let mut body = raw.get(split + 4..)?.to_vec();
    let status = head.lines().next()?.split_whitespace().nth(1)?.parse().ok()?;
    if head.lines().any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    }) {
        body = dechunk(&body)?;
    }
    let value = if body.iter().all(u8::is_ascii_whitespace) {
        Value::Null
    } else {
        serde_json::from_slice(&body).ok()?
    };
    Some((status, value))
}

fn dechunk(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = body.windows(2).position(|w| w == b"\r\n")?;
        let size = usize::from_str_radix(std::str::from_utf8(body.get(..line_end)?).ok()?.split(';').next()?.trim(), 16).ok()?;
        body = body.get(line_end + 2..)?;
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(body.get(..size)?);
        body = body.get(size + 2..)?;
    }
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Clean, one-line, at most [max] characters (on a word boundary).
fn short(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(a, _)| a).to_owned();
    format!("{cut}…")
}

fn sessions() -> Result<Vec<Value>, String> {
    match call("GET", "/session-feed", None)? {
        (200, v) => Ok(v
            .get("sessions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| text(s, "label") != SELF_LABEL)
            .collect()),
        (code, _) => Err(format!("El servidor respondió {code} al listar las sesiones.")),
    }
}

/// The session [wanted] names: its id, or its name (exact, then a unique part).
fn resolve<'a>(all: &'a [Value], wanted: &str) -> Result<&'a Value, String> {
    let w = wanted.trim().to_lowercase();
    if let Some(s) = all.iter().find(|s| text(s, "session") == wanted.trim()) {
        return Ok(s);
    }
    let exact: Vec<&Value> = all.iter().filter(|s| text(s, "label").to_lowercase() == w).collect();
    if exact.len() == 1 {
        return Ok(exact[0]);
    }
    let part: Vec<&Value> = all.iter().filter(|s| text(s, "label").to_lowercase().contains(&w)).collect();
    match part.len() {
        1 => Ok(part[0]),
        0 => Err(format!("No hay ninguna sesión que se llame «{wanted}».")),
        _ => Err(format!(
            "«{wanted}» puede ser {}. Dime cuál.",
            part.iter().map(|s| format!("{} ({})", text(s, "label"), text(s, "session"))).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn state_words(s: &Value) -> &'static str {
    if s.get("ask").is_some_and(|a| !a.is_null()) {
        return "te pregunta algo";
    }
    match text(s, "state") {
        "working" => "está trabajando",
        "waiting" => "te espera",
        "ask" => "te pregunta algo",
        _ if s.get("unread").and_then(Value::as_bool) == Some(true) => "respondió",
        _ => "sin pendientes",
    }
}

fn rank(s: &Value) -> u8 {
    match state_words(s) {
        "te pregunta algo" => 0,
        "te espera" | "respondió" => 1,
        "está trabajando" => 2,
        _ => 3,
    }
}

fn ask_lines(ask: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let q = text(ask, "question");
    if !q.is_empty() {
        out.push(format!("Pregunta: {}", short(q, 400)));
    }
    for o in ask.get("options").and_then(Value::as_array).into_iter().flatten() {
        let key = text(o, "key");
        let label = text(o, "label");
        let tick = match o.get("checked").and_then(Value::as_bool) {
            Some(true) => " (marcada)",
            _ => "",
        };
        let what = match text(o, "kind") {
            "free" => "escribir otra respuesta".to_owned(),
            "chat" => "hablarlo en el chat".to_owned(),
            "submit" => "enviar las marcadas".to_owned(),
            _ => short(label, 140),
        };
        let desc = o
            .get("desc")
            .and_then(Value::as_array)
            .map(|d| d.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
            .filter(|d| !d.is_empty())
            .map(|d| format!(": {}", short(&d, 160)))
            .unwrap_or_default();
        let key = if key.is_empty() { "Enviar".to_owned() } else { key.to_owned() };
        out.push(format!("  {key}. {what}{tick}{desc}"));
    }
    out
}

fn list() -> Result<String, String> {
    let mut all = sessions()?;
    all.sort_by(|a, b| {
        rank(a).cmp(&rank(b)).then(
            b.get("ts").and_then(Value::as_f64).unwrap_or(0.0).total_cmp(&a.get("ts").and_then(Value::as_f64).unwrap_or(0.0)),
        )
    });
    if all.is_empty() {
        return Ok("No hay sesiones abiertas.".into());
    }
    let mut out = Vec::new();
    for s in all.iter().take(20) {
        let mut line = format!("{} [{}]: {}", text(s, "label"), text(s, "session"), state_words(s));
        if let Some(ask) = s.get("ask").filter(|a| !a.is_null()) {
            line.push_str(&format!(". {}", short(text(ask, "question"), 200)));
        } else {
            let reply = text(s, "reply");
            if !reply.is_empty() {
                line.push_str(&format!(". Último: {}", short(reply, 220)));
            }
        }
        out.push(line);
    }
    if all.len() > 20 {
        out.push(format!("Y {} sesiones más sin pendientes.", all.len() - 20));
    }
    Ok(out.join("\n"))
}

fn transcript(session: &str, pane: &str) -> Result<Value, String> {
    match call("POST", "/chat/transcript", Some(&json!({"session": session, "pane": pane})))? {
        (200, v) => Ok(v),
        (code, v) => Err(format!("No pude leer la sesión ({code}): {}", text(&v, "error"))),
    }
}

fn read(wanted: &str, n: usize) -> Result<String, String> {
    let all = sessions()?;
    let s = resolve(&all, wanted)?;
    let (session, pane) = (text(s, "session"), text(s, "pane"));
    let t = transcript(session, pane)?;
    let mut out = vec![format!("{} {}.", text(s, "label"), state_words(s))];
    let turns: Vec<&Value> = t
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|m| matches!(text(m, "r"), "a" | "u"))
        .collect();
    for m in turns.iter().skip(turns.len().saturating_sub(n.max(1))) {
        let who = if text(m, "r") == "u" { "Tú" } else { "El agente" };
        out.push(format!("{who}: {}", short(text(m, "t"), 900)));
    }
    if let Some(ask) = t.get("ask").filter(|a| !a.is_null()) {
        out.extend(ask_lines(ask));
    }
    Ok(out.join("\n"))
}

fn send(wanted: &str, words: &[String]) -> Result<String, String> {
    let message = words.join(" ").trim().to_owned();
    if message.is_empty() {
        return Err("Falta el texto que hay que mandar.".into());
    }
    let all = sessions()?;
    let s = resolve(&all, wanted)?;
    let id = format!("auto-{}", random_id());
    let body = json!({"clientMessageId": id, "session": text(s, "session"), "pane": text(s, "pane"), "text": message, "submit": true});
    match call("POST", "/session/message", Some(&body))? {
        (200 | 202, v) if v.get("ok").and_then(Value::as_bool) != Some(false) => {
            Ok(format!("Listo, se lo mandé a {}.", text(s, "label")))
        }
        (_, v) => Err(format!(
            "No se lo pude mandar a {}: {}",
            text(s, "label"),
            if text(&v, "error").is_empty() { "el servidor lo rechazó" } else { text(&v, "error") }
        )),
    }
}

fn answer(wanted: &str, number: &str, words: &[String]) -> Result<String, String> {
    let all = sessions()?;
    let s = resolve(&all, wanted)?;
    let (session, pane) = (text(s, "session"), text(s, "pane"));
    let t = transcript(session, pane)?;
    let Some(ask) = t.get("ask").filter(|a| !a.is_null()) else {
        return Err(format!("{} no tiene ninguna pregunta abierta.", text(s, "label")));
    };
    let options = ask.get("options").and_then(Value::as_array).cloned().unwrap_or_default();
    let wanted_key = number.trim().trim_end_matches('.');
    let row = options.iter().position(|o| {
        let key = text(o, "key");
        key == wanted_key || (key.is_empty() && wanted_key.eq_ignore_ascii_case("enviar"))
    });
    let Some(row) = row else {
        return Err(format!("La pregunta de {} no tiene la opción {number}.", text(s, "label")));
    };
    let typed = words.join(" ").trim().to_owned();
    let free = text(&options[row], "kind") == "free";
    if free && typed.is_empty() {
        return Err("Esa opción es para escribir: dime el texto.".into());
    }
    let mut body = json!({"session": session, "pane": pane, "sig": text(ask, "sig"), "row": row});
    if free {
        body["text"] = json!(typed);
    }
    match call("POST", "/chat/answer", Some(&body))? {
        (200, _) => Ok(format!("Listo, respondí la opción {number} en {}.", text(s, "label"))),
        (_, v) => Err(format!("No respondí: {}", text(&v, "error"))),
    }
}

fn random_id() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        bytes = (n as u64).to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn run(args: &[String]) -> Result<String, String> {
    let word = |i: usize| args.get(i).map(String::as_str);
    match word(0) {
        Some("sesiones") | Some("sessions") => list(),
        Some("leer") | Some("read") => match word(1) {
            Some(s) => read(s, word(2).and_then(|n| n.parse().ok()).unwrap_or(3)),
            None => Err(USAGE.into()),
        },
        Some("enviar") | Some("send") => match word(1) {
            Some(s) => send(s, args.get(2..).unwrap_or_default()),
            None => Err(USAGE.into()),
        },
        Some("responder") | Some("answer") => match (word(1), word(2)) {
            (Some(s), Some(n)) => answer(s, n, args.get(3..).unwrap_or_default()),
            _ => Err(USAGE.into()),
        },
        _ => Err(USAGE.into()),
    }
}

pub fn main(args: &[String]) -> i32 {
    match run(args) {
        Ok(out) => {
            println!("{out}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_plain_and_chunked_are_read() {
        let plain = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"ok\":true}";
        assert_eq!(parse_response(plain), Some((200, json!({"ok": true}))));
        let chunked = b"HTTP/1.1 409 Conflict\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n{\"a\":\r\n3\r\n12}\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked), Some((409, json!({"a": 12}))));
    }

    #[test]
    fn a_session_is_found_by_id_name_or_a_unique_part() {
        let all = vec![
            json!({"session": "term-1", "label": "Threads0x"}),
            json!({"session": "term-2", "label": "Relotto"}),
            json!({"session": "term-3", "label": "Relotto web"}),
        ];
        assert_eq!(text(resolve(&all, "term-2").unwrap(), "label"), "Relotto");
        assert_eq!(text(resolve(&all, "relotto").unwrap(), "session"), "term-2");
        assert_eq!(text(resolve(&all, "threads").unwrap(), "session"), "term-1");
        assert!(resolve(&all, "lott").unwrap_err().contains("Dime cuál"));
        assert!(resolve(&all, "nada").is_err());
    }

    #[test]
    fn a_question_reads_as_spoken_options() {
        let ask = json!({"question": "¿Qué borro?", "options": [
            {"key": "1", "label": "Logs", "desc": ["Los viejos."], "checked": false, "kind": "option"},
            {"key": "2", "label": "Type something.", "kind": "free"},
            {"key": "", "label": "Submit", "kind": "submit"},
            {"key": "3", "label": "Chat about this", "kind": "chat"}
        ]});
        assert_eq!(
            ask_lines(&ask),
            vec![
                "Pregunta: ¿Qué borro?".to_owned(),
                "  1. Logs: Los viejos.".to_owned(),
                "  2. escribir otra respuesta".to_owned(),
                "  Enviar. enviar las marcadas".to_owned(),
                "  3. hablarlo en el chat".to_owned(),
            ]
        );
    }

    #[test]
    fn long_text_is_cut_on_a_word() {
        assert_eq!(short("uno  dos\ntres cuatro", 12), "uno dos…");
        assert_eq!(short("corto", 12), "corto");
    }
}
