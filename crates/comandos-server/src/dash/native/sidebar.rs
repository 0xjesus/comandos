//! Account-wide OpenRouter credits for the sidebar «Consumos» view.
use super::{Answer, Entry, Key, Native, NativeRoute, Verb, reply};
use crate::Request;
use http::{Method, StatusCode};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

pub const ROUTES: &[Entry] = &[
    Entry {
        verb: Verb::Get,
        key: Key::Path("/credits/openrouter"),
        route: NativeRoute::Sidebar,
    },
    Entry {
        verb: Verb::Post,
        key: Key::Path("/credits/openrouter"),
        route: NativeRoute::Sidebar,
    },
];
#[derive(Default)]
pub struct CreditsCache(pub tokio::sync::Mutex<Option<(Instant, Value)>>);
fn error(status: StatusCode, message: &str) -> Answer {
    reply(status, &json!({"error":message}))
}
fn now() -> i64 {
    super::wall_clock_ms() / 1000
}
fn disconnected(message: &str) -> Value {
    json!({"connected":false,"updated_at":now(),"error":message})
}
fn credit_data(value: &Value, at: i64) -> Result<Value, &'static str> {
    let credits = value["data"]["total_credits"]
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0)
        .ok_or("Invalid credits response")?;
    let usage = value["data"]["total_usage"]
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0)
        .ok_or("Invalid credits response")?;
    Ok(
        json!({"connected":true,"total_credits":credits,"total_usage":usage,"remaining":credits-usage,"updated_at":at}),
    )
}
const CREDITS_URL: &str = "https://openrouter.ai/api/v1/credits";
async fn fetch_credits(key: &str) -> Result<Value, &'static str> {
    if key.is_empty() || key.len() > 4096 || key.chars().any(char::is_whitespace) {
        return Err("Enter a valid OpenRouter management key");
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Credits service unavailable")?;
    let mut response = client
        .get(CREDITS_URL)
        .bearer_auth(key)
        .send()
        .await
        .map_err(|_| "Unable to reach OpenRouter; try again later")?;
    if !response.status().is_success() {
        return Err(if matches!(response.status().as_u16(), 401 | 403) {
            "OpenRouter requires a valid management key"
        } else {
            "OpenRouter credits unavailable; try again later"
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Unable to read OpenRouter response")?
    {
        if bytes.len() + chunk.len() > 16384 {
            return Err("OpenRouter response exceeds limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "Invalid OpenRouter response")?;
    credit_data(&value, now())
}
fn key_path(home: &Path) -> PathBuf {
    home.join(".config/comandos/openrouter-management-key")
}
fn read_key(path: &Path) -> Result<Option<String>, &'static str> {
    match fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => {
            let meta = file
                .metadata()
                .map_err(|_| "Cannot read saved management key")?;
            if !meta.is_file() || meta.permissions().mode() & 0o077 != 0 {
                return Err("Saved management key must have private permissions");
            }
            let mut key = String::new();
            file.take(4097)
                .read_to_string(&mut key)
                .map_err(|_| "Cannot read saved management key")?;
            if key.len() > 4096 {
                return Err("Invalid saved management key");
            }
            Ok(Some(key.trim().into()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Cannot read saved management key"),
    }
}
fn save_key(path: &Path, key: &str) -> Result<(), &'static str> {
    let parent = path.parent().ok_or("Cannot save management key")?;
    fs::create_dir_all(parent).map_err(|_| "Cannot save management key")?;
    let nonce = format!(
        ".openrouter-key-{}-{}",
        std::process::id(),
        super::wall_clock_ms()
    );
    let temp = parent.join(nonce);
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|_| "Cannot save management key")?;
        file.write_all(key.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|_| "Cannot save management key")?;
        fs::rename(&temp, path).map_err(|_| "Cannot save management key")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
pub async fn answer(native: &Arc<Native>, request: &Request) -> Answer {
    let mut cache = native.credits.0.lock().await;
    if request.method == Method::GET
        && let Some((at, value)) = &*cache
        && at.elapsed() < Duration::from_secs(60)
    {
        return reply(StatusCode::OK, value);
    }
    let path = key_path(&native.options().home);
    let posted = request.method == Method::POST;
    let key = if posted {
        match request
            .data
            .as_ref()
            .and_then(|v| v.get("key"))
            .and_then(Value::as_str)
        {
            Some(v) => v.trim().to_owned(),
            None => return error(StatusCode::BAD_REQUEST, "Management key required"),
        }
    } else if let Ok(v) = std::env::var("OPENROUTER_MANAGEMENT_KEY") {
        v.trim().to_owned()
    } else {
        let path = path.clone();
        match tokio::task::spawn_blocking(move || read_key(&path)).await {
            Ok(Ok(Some(v))) => v,
            Ok(Ok(None)) => {
                let v =
                    disconnected("Connect an OpenRouter management key to view account credits");
                *cache = Some((Instant::now(), v.clone()));
                return reply(StatusCode::OK, &v);
            }
            _ => {
                let v = disconnected("Cannot read private OpenRouter management key");
                *cache = Some((Instant::now(), v.clone()));
                return reply(StatusCode::OK, &v);
            }
        }
    };
    let value = match fetch_credits(&key).await {
        Ok(v) => v,
        Err(e) => {
            let v = disconnected(e);
            if !posted {
                *cache = Some((Instant::now(), v.clone()));
            }
            return reply(
                if posted {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::OK
                },
                &v,
            );
        }
    };
    if posted {
        let result = tokio::task::spawn_blocking(move || save_key(&path, &key)).await;
        if !matches!(result, Ok(Ok(()))) {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Validated management key could not be saved",
            );
        }
    }
    *cache = Some((Instant::now(), value.clone()));
    reply(StatusCode::OK, &value)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};
    fn temp() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "comandos-sidebar-{}-{}-{}",
            std::process::id(),
            super::super::wall_clock_ms(),
            {
                static NEXT: std::sync::atomic::AtomicUsize =
                    std::sync::atomic::AtomicUsize::new(0);
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            }
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn sidebar_private_key_storage_and_no_fake_balances() {
        let root = temp();
        let path = key_path(&root);
        assert!(read_key(&path).unwrap().is_none());
        save_key(&path, "test-management-key").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            read_key(&path).unwrap().as_deref(),
            Some("test-management-key")
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_key(&path).is_err());
        fs::remove_file(&path).unwrap();
        symlink("/etc/passwd", &path).unwrap();
        assert!(read_key(&path).is_err());
        let v = disconnected("Connect management key");
        assert_eq!(v["connected"], false);
        assert!(v.get("remaining").is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn sidebar_routes_are_exact_and_registered() {
        for method in [Method::GET, Method::POST] {
            assert_eq!(
                super::super::route(&method, "/credits/openrouter"),
                Some(NativeRoute::Sidebar)
            );
        }
        assert_ne!(
            super::super::route(&Method::POST, "/fs/explorer"),
            Some(NativeRoute::Sidebar)
        );
    }
    #[test]
    fn sidebar_credits_require_real_numbers() {
        let v = credit_data(
            &json!({"data":{"total_credits":20.0,"total_usage":7.5}}),
            10,
        )
        .unwrap();
        assert_eq!(v["remaining"], 12.5);
        assert_eq!(v["updated_at"], 10);
        assert!(credit_data(&json!({"data":{"total_credits":20}}), 10).is_err());
        assert!(credit_data(&json!({"data":{"total_credits":-1,"total_usage":0}}), 10).is_err());
    }
}
