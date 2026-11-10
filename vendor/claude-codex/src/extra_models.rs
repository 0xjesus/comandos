//! Modelos que `cc-model-sync` (ComandOS) descubre después de compilar el
//! gateway: `<config_dir>/extra-models.json`, con la forma
//! `{"anthropic": ["claude-fable-5-2"], "codex": ["gpt-6-luna"]}`.
//!
//! Las listas fijas de `registry` y de la allowlist de Codex quedan como piso;
//! este archivo solo agrega. Se vuelve a leer cuando cambia su mtime (como
//! mucho cada `RECHECK`), así un modelo nuevo entra sin reiniciar el gateway
//! ni cortar las sesiones que están transmitiendo.
//!
//! Un archivo ausente, roto o con ids raros cuenta como vacío: el gateway
//! nunca deja de arrancar ni de enrutar por culpa de este archivo.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{LazyLock, RwLock},
    time::{Duration, Instant, SystemTime},
};

pub const FILE_NAME: &str = "extra-models.json";
const RECHECK: Duration = Duration::from_secs(5);
const MAX_BYTES: u64 = 64 * 1024;
const MAX_IDS: usize = 64;
const MAX_ID_LEN: usize = 48;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtraModels {
    pub anthropic: Vec<String>,
    pub codex: Vec<String>,
}

fn id_chars_ok(id: &str, extra: &[u8]) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || extra.contains(&b)
        })
        && !id.ends_with('-')
}

/// `claude-<familia>-<versión>`, sin `[1m]` ni otros sufijos de cliente.
pub fn valid_anthropic_id(id: &str) -> bool {
    id.starts_with("claude-") && id_chars_ok(id, &[])
}

/// `gpt-<versión>[-variante]`; el sufijo `-fast` lo agrega el registro.
pub fn valid_codex_id(id: &str) -> bool {
    id.starts_with("gpt-")
        && id.as_bytes().get(4).is_some_and(u8::is_ascii_digit)
        && id_chars_ok(id, b".")
        && !id.ends_with("-fast")
}

fn ids(value: Option<&serde_json::Value>, valid: fn(&str) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Some(items) = value.and_then(serde_json::Value::as_array) else {
        return out;
    };
    for item in items {
        if out.len() >= MAX_IDS {
            break;
        }
        if let Some(id) = item.as_str()
            && valid(id)
            && !out.iter().any(|seen| seen == id)
        {
            out.push(id.to_string());
        }
    }
    out
}

/// Interpreta el contenido del archivo; cualquier cosa inválida se descarta.
pub fn parse(text: &str) -> ExtraModels {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return ExtraModels::default();
    };
    ExtraModels {
        anthropic: ids(value.get("anthropic"), valid_anthropic_id),
        codex: ids(value.get("codex"), valid_codex_id),
    }
}

/// Lee `path` con tope de tamaño; ausente o ilegible es vacío.
pub fn load(path: &Path) -> ExtraModels {
    let Ok(meta) = fs::metadata(path) else {
        return ExtraModels::default();
    };
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return ExtraModels::default();
    }
    fs::read_to_string(path)
        .map(|text| parse(&text))
        .unwrap_or_default()
}

pub fn default_path() -> PathBuf {
    crate::paths::config_dir().join(FILE_NAME)
}

struct Cache {
    path: PathBuf,
    checked: Option<Instant>,
    stamp: Option<(SystemTime, u64)>,
    value: ExtraModels,
}

static CACHE: LazyLock<RwLock<Cache>> = LazyLock::new(|| {
    RwLock::new(Cache {
        path: default_path(),
        checked: None,
        stamp: None,
        value: ExtraModels::default(),
    })
});

/// Los modelos extra vigentes (releídos si el archivo cambió).
pub fn current() -> ExtraModels {
    if let Ok(cache) = CACHE.read()
        && cache.checked.is_some_and(|at| at.elapsed() < RECHECK)
    {
        return cache.value.clone();
    }
    let Ok(mut cache) = CACHE.write() else {
        return ExtraModels::default();
    };
    if cache.checked.is_some_and(|at| at.elapsed() < RECHECK) {
        return cache.value.clone();
    }
    let stamp = fs::metadata(&cache.path)
        .ok()
        .and_then(|meta| Some((meta.modified().ok()?, meta.len())));
    if stamp != cache.stamp || cache.checked.is_none() {
        cache.value = load(&cache.path);
        cache.stamp = stamp;
    }
    cache.checked = Some(Instant::now());
    cache.value.clone()
}

pub fn anthropic() -> Vec<String> {
    current().anthropic
}

pub fn codex() -> Vec<String> {
    current().codex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_ids_and_drops_the_rest() {
        let got = parse(
            r#"{"anthropic": ["claude-fable-5-2", "claude-opus-5-5[1m]", "gpt-6-luna", "claude-fable-5-2", 7],
                "codex": ["gpt-6-luna", "gpt-6.1-sol", "gpt-6-luna-fast", "claude-x", "gpt-x", "GPT-7", "gpt-7;rm"]}"#,
        );
        assert_eq!(got.anthropic, vec!["claude-fable-5-2"]);
        assert_eq!(got.codex, vec!["gpt-6-luna", "gpt-6.1-sol"]);
    }

    #[test]
    fn broken_or_missing_file_is_empty() {
        assert_eq!(parse("{not json"), ExtraModels::default());
        assert_eq!(parse(r#"{"codex": "gpt-6-luna"}"#), ExtraModels::default());
        assert_eq!(
            load(Path::new("/nonexistent/extra-models.json")),
            ExtraModels::default()
        );
    }

    #[test]
    fn caps_the_number_of_ids() {
        let many: Vec<String> = (0..200).map(|n| format!("\"gpt-{n}\"")).collect();
        let got = parse(&format!(r#"{{"codex": [{}]}}"#, many.join(",")));
        assert_eq!(got.codex.len(), MAX_IDS);
    }

    #[test]
    fn loads_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(
            &path,
            r#"{"anthropic": ["claude-fable-5-2"], "codex": ["gpt-6-astra"]}"#,
        )
        .unwrap();
        let got = load(&path);
        assert_eq!(got.anthropic, vec!["claude-fable-5-2"]);
        assert_eq!(got.codex, vec!["gpt-6-astra"]);
    }
}
