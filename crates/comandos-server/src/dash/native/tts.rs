//! POST `/tts` `{"text","voice"?,"rate"?}` → `audio/mpeg`: la voz de Edge (el
//! servicio en línea de «Leer en voz alta» de Microsoft, gratis) para el Modo auto
//! del teléfono. Usa el comando `edge-tts` del equipo, que se actualiza con pip
//! cuando Microsoft cambia su servicio, sin rehacer el APK. Sin el comando, o si el
//! servicio falla, responde 503 y el teléfono usa su propia voz.
//!
//! El texto sale hacia Microsoft: solo lo pide el teléfono cuando la persona eligió
//! la voz de Edge. Nunca va en los argumentos del proceso (lo lee de un archivo
//! temporal privado) ni se guarda.
use super::{Answer, Entry, Key, Native, NativeRoute, Verb, reply};
use crate::{Reply, Request};
use http::StatusCode;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

pub const ROUTES: &[Entry] = &[Entry {
    verb: Verb::Post,
    key: Key::Path("/tts"),
    route: NativeRoute::Tts,
}];

/// La voz de partida (español de México).
pub const DEFAULT_VOICE: &str = "es-MX-DaliaNeural";
/// Un aviso del Modo auto ocupa unos cientos de caracteres; esto sobra.
const MAX_TEXT: usize = 3000;
const TIMEOUT: Duration = Duration::from_secs(20);

fn error(status: StatusCode, message: &str) -> Answer {
    reply(status, &json!({"error": message}))
}

/// `es-MX-DaliaNeural`: idioma, región y un nombre que termina en `Neural`.
pub fn valid_voice(voice: &str) -> bool {
    let mut parts = voice.splitn(3, '-');
    let (Some(lang), Some(region), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    (2..=3).contains(&lang.len())
        && lang.bytes().all(|b| b.is_ascii_lowercase())
        && (2..=4).contains(&region.len())
        && region.bytes().all(|b| b.is_ascii_alphanumeric())
        && name.len() <= 40
        && name.ends_with("Neural")
        && name.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// `+0%`, `-10%`, `+25%`: lo que acepta `edge-tts --rate`.
pub fn valid_rate(rate: &str) -> bool {
    let Some(sign) = rate.chars().next().filter(|c| matches!(c, '+' | '-')) else {
        return false;
    };
    let Some(n) = rate[sign.len_utf8()..].strip_suffix('%') else {
        return false;
    };
    (1..=2).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit())
}

/// El texto que se puede leer: sin controles (salvo saltos de línea), no vacío.
pub fn clean_text(text: &str) -> Option<String> {
    let t: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    let t = t.trim();
    (!t.is_empty() && t.chars().count() <= MAX_TEXT).then(|| t.to_owned())
}

/// `edge-tts` en el PATH del servicio o en `~/.local/bin` (pipx, `uv tool`).
fn program() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join(".local/bin"));
    }
    dirs.into_iter()
        .map(|d| d.join("edge-tts"))
        .find(|p| p.is_file())
}

/// Un archivo temporal propio (0600) que se borra al salir de su alcance.
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn temp(ext: &str) -> std::io::Result<Temp> {
    use std::os::unix::fs::OpenOptionsExt;
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let path = dir.join(format!(
        "comandos-tts-{}-{nanos}-{}.{ext}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    Ok(Temp(path))
}

pub async fn answer(_native: &Arc<Native>, request: &Request) -> Answer {
    let Some(body) = request.data.as_ref().and_then(Value::as_object) else {
        return error(StatusCode::BAD_REQUEST, "Se esperaba {text, voice?, rate?}");
    };
    let Some(text) = body.get("text").and_then(Value::as_str).and_then(clean_text) else {
        return error(StatusCode::BAD_REQUEST, "Texto vacío o demasiado largo");
    };
    let voice = body.get("voice").and_then(Value::as_str).unwrap_or(DEFAULT_VOICE);
    let rate = body.get("rate").and_then(Value::as_str).unwrap_or("+0%");
    if !valid_voice(voice) || !valid_rate(rate) {
        return error(StatusCode::BAD_REQUEST, "Voz o velocidad no válidas");
    }
    let Some(program) = program() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "Falta edge-tts en tu equipo");
    };
    let (Ok(input), Ok(output)) = (temp("txt"), temp("mp3")) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "No se pudo preparar la voz");
    };
    if tokio::fs::write(&input.0, text.as_bytes()).await.is_err() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "No se pudo preparar la voz");
    }
    let run = tokio::process::Command::new(program)
        .arg("--voice")
        .arg(voice)
        .arg(format!("--rate={rate}"))
        .arg("--file")
        .arg(&input.0)
        .arg("--write-media")
        .arg(&output.0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .status();
    let ok = matches!(tokio::time::timeout(TIMEOUT, run).await, Ok(Ok(s)) if s.success());
    let audio = if ok { tokio::fs::read(&output.0).await.ok() } else { None };
    match audio.filter(|a| !a.is_empty()) {
        Some(mp3) => Ok(Reply::bytes(StatusCode::OK, "audio/mpeg", mp3)),
        None => error(StatusCode::SERVICE_UNAVAILABLE, "La voz de Edge no respondió"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_edge_voice_names_and_rates_reach_the_command() {
        for ok in ["es-MX-DaliaNeural", "es-US-AlonsoNeural", "en-US-EmmaMultilingualNeural"] {
            assert!(valid_voice(ok), "{ok}");
        }
        for bad in ["", "es-MX", "es-MX-Dalia", "es-MX-Dalia Neural", "--voice", "es-MX-$(id)Neural", "ES-MX-DaliaNeural"] {
            assert!(!valid_voice(bad), "{bad}");
        }
        for ok in ["+0%", "-10%", "+25%"] {
            assert!(valid_rate(ok), "{ok}");
        }
        for bad in ["", "0%", "+100%", "+5", "fast", "+-5%"] {
            assert!(!valid_rate(bad), "{bad}");
        }
    }

    #[test]
    fn the_text_is_cleaned_and_bounded() {
        assert_eq!(clean_text("  Hola\nmundo\u{7}  ").as_deref(), Some("Hola mundo"));
        assert_eq!(clean_text(" \u{1b} "), None);
        assert_eq!(clean_text(&"a".repeat(MAX_TEXT + 1)), None);
    }
}
