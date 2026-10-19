use crate::{Result, home_dir};
use serde_json::Value;
use std::path::PathBuf;
pub fn run(args: Vec<String>) -> Result<i32> {
    let mut args = args.into_iter();
    let mut home = None;
    let mut catalog = None;
    let mut direct = false;
    loop {
        match args.next().as_deref() {
            Some("--home") => home = Some(PathBuf::from(args.next().ok_or("Missing home")?)),
            Some("--catalog") => {
                catalog = Some(PathBuf::from(args.next().ok_or("Missing catalog")?))
            }
            Some(action @ ("import" | "sync" | "status")) => {
                if args.next().is_some() {
                    return Err("Unexpected arguments".into());
                }
                let home = home.map(Ok).unwrap_or_else(home_dir)?;
                catalog_command(&home, catalog.as_deref(), action)?;
                return Ok(0);
            }
            Some("check") => {
                let home = home.map(Ok).unwrap_or_else(home_dir)?;
                let path = catalog.unwrap_or_else(|| crate::config::catalog_path(&home));
                let catalog = crate::config::read_config(&path)?;
                if catalog["version"] != 1 {
                    return Err("Shared catalog missing or unsupported".into());
                }
                let names = args.collect();
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .max_blocking_threads(8)
                    .enable_all()
                    .build()
                    .map_err(|_| "Runtime unavailable")?;
                let result = runtime.block_on(async {
                    let result = crate::check::run(&home, &catalog, names).await;
                    crate::auth::drain_workers().await;
                    result
                });
                runtime.shutdown_timeout(std::time::Duration::from_millis(100));
                return Ok(i32::from(result?));
            }
            Some("serve") => break,
            // Internal upstream adapter. Public clients always enter through `serve`.
            Some("serve-direct") => {
                direct = true;
                break;
            }
            Some("broker") if args.next().is_none() => {
                let home = home.map(Ok).unwrap_or_else(home_dir)?;
                let path = catalog.unwrap_or_else(|| crate::config::catalog_path(&home));
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| "Runtime unavailable")?;
                runtime.block_on(crate::broker::daemon::run(&home, &path))?;
                runtime.shutdown_timeout(std::time::Duration::from_millis(100));
                return Ok(0);
            }
            // Hub bajo demanda: lo que reciben OpenCode y agy en lugar de cada servidor.
            Some("hub") if args.next().is_none() => {
                let home = home.map(Ok).unwrap_or_else(home_dir)?;
                let path = catalog.unwrap_or_else(|| crate::config::catalog_path(&home));
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .max_blocking_threads(4)
                    .enable_all()
                    .build()
                    .map_err(|_| "Runtime unavailable")?;
                let result = runtime.block_on(crate::hub::serve(&home, &path));
                runtime.shutdown_timeout(std::time::Duration::from_millis(100));
                result?;
                return Ok(0);
            }
            Some("count") if args.next().is_none() => {
                let home = home.map(Ok).unwrap_or_else(home_dir)?;
                crate::tokenizer::count_command(&home)?;
                return Ok(0);
            }
            _ => {
                return Err(
                    "Usage: comandos-extensions [--home PATH] [--catalog PATH] {import|sync|status|count|check [NAME...]|serve NAME|hub|broker}".into(),
                );
            }
        }
    }
    let name = args.next().ok_or("Missing server name")?;
    if args.next().is_some() {
        return Err("Unexpected arguments".into());
    }
    let home = home.map(Ok).unwrap_or_else(home_dir)?;
    let path = catalog.unwrap_or_else(|| crate::config::catalog_path(&home));
    let snapshot = direct.then(broker_snapshot).transpose()?.flatten();
    let preconfigured = snapshot.is_some();
    let spec = snapshot
        .map(Ok)
        .unwrap_or_else(|| server_spec(&path, &name))?;
    if !direct && crate::config::is_shared(&spec, &name) {
        return broker_session(&home, &name, &spec, &path);
    }
    let mut runtime_spec = spec.clone();
    if preconfigured {
        let object = runtime_spec.as_object_mut().ok_or("Invalid server spec")?;
        object.remove("cwd");
        object.remove("env");
    }
    serve_direct(&home, &name, &runtime_spec, &spec)
}

fn serve_direct(
    home: &std::path::Path,
    name: &str,
    spec: &Value,
    catalog_spec: &Value,
) -> Result<i32> {
    if crate::broker::direct_stdio(spec) {
        crate::exec_direct(spec)?;
        return Err("Extension command failed".into());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(4)
        .enable_all()
        .build()
        .map_err(|_| "Runtime unavailable")?;
    let result = runtime.block_on(async {
        let result = crate::serve::serve_with_catalog_spec(home, name, spec, catalog_spec).await;
        // A running refresh must persist its rotated token before process exit.
        crate::auth::drain_workers().await;
        result
    });
    runtime.shutdown_timeout(std::time::Duration::from_millis(100));
    result?;
    Ok(0)
}

/// Private snapshot handed to a single broker-owned HTTP/filtered-stdio adapter. It never
/// appears in argv or logs, and prevents a catalog change from replacing the keyed spec
/// between registry admission and process startup.
pub(crate) const BROKER_UPSTREAM_SPEC: &str = "COMANDOS_BROKER_UPSTREAM_SPEC";

fn broker_snapshot() -> Result<Option<Value>> {
    let Some(raw) = std::env::var_os(BROKER_UPSTREAM_SPEC) else {
        return Ok(None);
    };
    let raw = raw
        .into_string()
        .map_err(|_| "Invalid broker upstream snapshot")?;
    if raw.len() > 8 * 1024 * 1024 {
        return Err("Broker upstream snapshot too large".into());
    }
    let value: Value =
        serde_json::from_str(&raw).map_err(|_| "Invalid broker upstream snapshot")?;
    if !value.is_object() {
        return Err("Invalid broker upstream snapshot".into());
    }
    Ok(Some(value))
}

/// Adapts HTTP and filtered stdio to the broker's stdio multiplexer. The registry keeps
/// the original spec in its identity, then applies the returned private environment after
/// resolving the authoritative cwd/environment. The adapter removes cwd/env for execution
/// while retaining this original snapshot for metadata's catalog configuration digest.
pub(crate) fn broker_upstream_spec(
    home: &std::path::Path,
    catalog: &std::path::Path,
    name: &str,
    spec: &Value,
) -> Result<(Value, Vec<(String, String)>)> {
    if crate::broker::direct_stdio(spec) {
        return Ok((spec.clone(), Vec::new()));
    }
    let executable = std::env::current_exe().map_err(|_| "Extension executable unavailable")?;
    let mut args = Vec::new();
    // The standalone binary consumes extension arguments directly; the multicall CLI
    // first dispatches `ext`. Both forms are used by private integration fixtures.
    if executable
        .file_name()
        .is_none_or(|name| name != "comandos-extensions")
    {
        args.push("ext".to_owned());
    }
    args.extend([
        "--home".into(),
        home.to_string_lossy().into_owned(),
        "--catalog".into(),
        catalog.to_string_lossy().into_owned(),
        "serve-direct".into(),
        name.into(),
    ]);
    let snapshot = serde_json::to_string(spec).map_err(|_| "Invalid server spec")?;
    Ok((
        serde_json::json!({"command":executable,"args":args}),
        vec![(BROKER_UPSTREAM_SPEC.into(), snapshot)],
    ))
}
/// Servidor `name` del catálogo, habilitado. Lo usan `serve` y el broker.
pub(crate) fn server_spec(path: &std::path::Path, name: &str) -> Result<Value> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "Catalog unavailable")?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Catalog unavailable")?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Catalog too large".into());
    }
    let mut catalog: Value = crate::config::parse_json(&bytes)?;
    if catalog["version"] != 1 {
        return Err("Unsupported catalog".into());
    }
    catalog["servers"]
        .get_mut(name)
        .filter(|s| {
            // Python: `not spec` o `not spec.get('enabled', True)` ⇒ no disponible.
            s.as_object().is_some_and(|o| !o.is_empty())
                && s.get("enabled").is_none_or(crate::py_truthy)
        })
        .map(Value::take)
        .ok_or_else(|| format!("Server unavailable: {name}"))
}
/// Public shared sessions remain under the broker's authority, including startup and
/// reconnect. Failure never creates a direct duplicate beside an existing upstream.
fn broker_session(
    home: &std::path::Path,
    name: &str,
    spec: &Value,
    catalog: &std::path::Path,
) -> Result<i32> {
    use crate::broker::{attach_request, client, socket_path};
    let socket = socket_path();
    let mut request = attach_request(name, spec, catalog).ok_or("Invalid broker attach")?;
    request["home"] = serde_json::json!(home);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(4)
        .enable_all()
        .build()
        .map_err(|_| "Runtime unavailable")?;
    let relayed = runtime.block_on(async {
        // The client owns stdin while starting, attaching and reconnecting, so EOF can
        // cancel an unused session without leaving a waiting wrapper behind.
        client::serve(home, catalog, &socket, &request).await
    });
    // El hilo bloqueado en stdin no debe retrasar la salida.
    runtime.shutdown_timeout(std::time::Duration::from_millis(100));
    relayed
}
pub fn catalog_command(
    home: &std::path::Path,
    path: Option<&std::path::Path>,
    action: &str,
) -> Result<()> {
    use crate::{auth, catalog, config, skills};
    use serde_json::json;
    let default = config::catalog_path(home);
    let path = path.unwrap_or(&default);
    let launcher = home
        .join(".local/bin/cc-extensions")
        .to_string_lossy()
        .into_owned();
    let result = match action {
        "import" => {
            let _lock = config::SyncLock::new(home)?;
            if path.exists() || path.is_symlink() {
                return Err("Catalog already exists; refusing to overwrite it".into());
            }
            let c = catalog::import_catalog(home)?;
            let imported = auth::import_credentials(home, &c)?;
            config::save_json(path, &c)?;
            catalog::save_snapshot(home, &c, None)?;
            json!({"servers":c["servers"].as_object().ok_or("Invalid catalog")?.len(),"credentials_imported":imported})
        }
        "sync" => {
            let _lock = config::SyncLock::new(home)?;
            let original =
                config::read_bytes(path)?.ok_or("Shared catalog missing or unsupported")?;
            let mut c = config::Document::parse(path, Some(&original))?.data;
            if c["version"] != 1 {
                return Err("Shared catalog missing or unsupported".into());
            }
            let mut inputs = json!({});
            let revised = catalog::reconcile(home, &c, &launcher, &mut inputs)?;
            if !config::toml::python_equal(&revised, &c) {
                c = revised;
                config::replace_config(home, path, Some(&original), &config::json_bytes(&c)?)?;
            }
            auth::import_credentials(home, &c)?;
            let mut observed = json!({});
            let configs = catalog::sync_configs(home, &c, &launcher, &mut observed, Some(&inputs))?;
            let skills = skills::sync_skills(home)?;
            catalog::save_snapshot(home, &c, Some(&observed))?;
            json!({"configurations_changed":configs.len(),"skill_links_changed":skills.len()})
        }
        "status" => {
            let c = config::read_config(path)?;
            if c["version"] != 1 {
                return Err("Shared catalog missing or unsupported".into());
            }
            let credentials =
                config::read_config(&home.join(".config/comandos/extensions/credentials.json"))?;
            let mut servers = c["servers"]
                .as_object()
                .ok_or("Invalid catalog")?
                .iter()
                .collect::<Vec<_>>();
            servers.sort_by_key(|(n, _)| *n);
            json!({"servers":servers.into_iter().map(|(n,s)|json!({"name":n,"enabled":s.get("enabled").cloned().unwrap_or(json!(true)),"credential_saved":credentials.get(n).is_some(),"transport":if s["command"].as_str().is_some_and(|s|!s.is_empty()){json!("stdio")}else{s.get("transport").cloned().unwrap_or(json!("http"))}})).collect::<Vec<_>>(),"targets":catalog::targets(home)?.iter().map(|t|&t.path).collect::<Vec<_>>()})
        }
        _ => return Err("Unknown command".into()),
    };
    if matches!(action, "import" | "sync") {
        // json.dumps del Python: separadores ", " y ": ", orden de inserción.
        let body = result
            .as_object()
            .ok_or("Invalid command result")?
            .iter()
            .map(|(k, v)| {
                Ok(format!(
                    "{}: {}",
                    crate::python_json::dumps(&json!(k), true, false)?,
                    crate::python_json::dumps(v, true, false)?
                ))
            })
            .collect::<std::result::Result<Vec<_>, String>>()?
            .join(", ");
        println!("{{{body}}}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).map_err(|_| "Invalid command result")?
        );
    }
    Ok(())
}
