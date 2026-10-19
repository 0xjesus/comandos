//! Con el hub bajo demanda (`comandos`) en OpenCode y agy, el selector «MCPs · Skills»
//! sigue listando cada servidor del catálogo, apagado por omisión; encender uno en un
//! panel lo carga completo, porque la configuración base ya no lo declara.
use comandos_runtime::{capabilities::Paths, extension_launch as e};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn root(label: &str) -> Root {
    let root = std::env::temp_dir().join(format!(
        "comandos-hub-selector-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    Root(root)
}
fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}
/// `prepare_launch` exige el lanzador nativo instalado. OpenCode solo comprueba que sea un
/// ELF propio; agy además lo ejecuta para probar el montaje aislado, así que usa el
/// `comandos` de depuración (como `extension_prepare_oracle`) si está compilado.
fn install_helper(root: &Path, namespaces: bool) {
    use std::os::unix::fs::PermissionsExt;
    let native = root.join(".local/share/comandos/bin/comandos");
    let alias = e::helper_for_home(root);
    std::fs::create_dir_all(native.parent().unwrap()).unwrap();
    std::fs::create_dir_all(alias.parent().unwrap()).unwrap();
    let debug = std::env::var_os("COMANDOS_EXTENSION_SESSION_NATIVE_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let exe = std::env::current_exe().unwrap();
            exe.parent().unwrap().parent().unwrap().join("comandos")
        });
    let source = if namespaces && debug.is_file() {
        debug
    } else {
        PathBuf::from("/bin/true")
    };
    std::fs::copy(source, &native).unwrap();
    std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&native, &alias).unwrap();
    e::require_helper(root).unwrap();
}
fn catalog(root: &Path) {
    write(
        root,
        ".config/comandos/extensions/catalog.json",
        r#"{"version":1,"servers":{"alpha":{"command":"never-run"},"beta":{"command":"never-run"},"off":{"command":"never-run","enabled":false}}}"#,
    );
}
fn rows(inv: &Value) -> Vec<(String, Value, Value)> {
    inv["mcps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["name"].as_str().unwrap().to_owned(),
                r["enabled"].clone(),
                r["toggleable"].clone(),
            )
        })
        .collect()
}
fn manifest(bundle: &Value) -> Value {
    serde_json::from_slice(&std::fs::read(bundle["manifest"].as_str().unwrap()).unwrap()).unwrap()
}

#[test]
fn opencode_lists_catalog_servers_off_and_loads_explicit_ones_in_full() {
    let r = root("opencode");
    let root = &r.0;
    let launcher = root.join(".local/bin/cc-extensions");
    let hub = json!({"type":"local","command":[launcher,"hub"],"enabled":true});
    write(
        root,
        ".config/opencode/opencode.json",
        &json!({"mcp":{"comandos":hub}}).to_string(),
    );
    catalog(root);
    install_helper(root, false);
    let registry = json!({"harnesses":{"opencode":{}}});
    let paths = Paths::new(root, root);
    let inv = e::inventory(&registry, "opencode", "main", root, &paths).unwrap();
    let mut listed = rows(&inv);
    listed.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        listed,
        [
            ("alpha".into(), json!(false), json!(true)),
            ("beta".into(), json!(false), json!(true)),
            ("comandos".into(), json!(true), json!(true)),
            ("off".into(), json!(false), json!(false)),
        ]
    );
    let overlay = |selection: Value, op: &str| {
        let bundle = e::prepare_launch(
            &registry,
            "opencode",
            "main",
            root,
            &selection,
            &root.join("runtime"),
            op,
            &paths,
        )
        .unwrap();
        let data = manifest(&bundle);
        let content: Value =
            serde_json::from_str(data["env"]["OPENCODE_CONFIG_CONTENT"].as_str().unwrap()).unwrap();
        content["mcp"].clone()
    };
    // Por omisión: solo el hub; ningún servidor se carga completo.
    let default = overlay(json!({}), "default-op");
    assert_eq!(default["comandos"], json!({"enabled":true}));
    for (name, spec) in default.as_object().unwrap() {
        assert!(spec.get("command").is_none(), "{name}: {spec}");
    }
    // Encendido explícito: la entrada completa viaja en el overlay.
    let explicit = overlay(json!({"mcps":{"alpha":true}}), "explicit-op");
    assert_eq!(
        explicit["alpha"],
        json!({"type":"local","command":[launcher,"serve","alpha"],"enabled":true})
    );
    assert!(explicit["beta"].get("command").is_none());
    assert_eq!(explicit["comandos"], json!({"enabled":true}));
    // Apagar el hub en un panel lo excluye.
    let without = overlay(json!({"mcps":{"comandos":false}}), "nohub-op");
    assert_eq!(without["comandos"], json!({"enabled":false}));
}

#[test]
fn without_the_hub_catalog_servers_keep_their_catalog_default() {
    let r = root("legacy");
    let root = &r.0;
    write(root, ".config/opencode/opencode.json", r#"{"mcp":{}}"#);
    catalog(root);
    let registry = json!({"harnesses":{"opencode":{}}});
    let paths = Paths::new(root, root);
    let inv = e::inventory(&registry, "opencode", "main", root, &paths).unwrap();
    let alpha = rows(&inv).into_iter().find(|r| r.0 == "alpha").unwrap();
    assert_eq!(alpha.1, json!(true));
}

#[test]
fn agy_lists_catalog_servers_off_and_mounts_explicit_ones_in_full() {
    let r = root("agy");
    let root = &r.0;
    let launcher = root.join(".local/bin/cc-extensions");
    write(
        root,
        ".gemini/config/mcp_config.json",
        &json!({"mcpServers":{"comandos":{"command":launcher,"args":["hub"],"disabled":false}}})
            .to_string(),
    );
    write(root, ".gemini/config/skills.json", r#"{"exclude":[]}"#);
    catalog(root);
    install_helper(root, true);
    let registry = json!({"harnesses":{"agy":{}}});
    let paths = Paths::new(root, root);
    let inv = e::inventory(&registry, "agy", "main", root, &paths).unwrap();
    let listed = rows(&inv);
    assert!(listed.contains(&("alpha".into(), json!(false), json!(true))));
    assert!(listed.contains(&("comandos".into(), json!(true), json!(true))));
    let bundle = match e::prepare_launch(
        &registry,
        "agy",
        "main",
        root,
        &json!({"mcps":{"beta":true}}),
        &root.join("runtime"),
        "agy-op",
        &paths,
    ) {
        Ok(bundle) => bundle,
        // Sin espacios de nombres de usuario no hay montaje aislado que comprobar.
        Err(e::LaunchError::Value(reason)) => {
            eprintln!("agy namespace launch unavailable here: {reason}");
            return;
        }
        Err(other) => panic!("{other:?}"),
    };
    let data = manifest(&bundle);
    let target = root.join(".gemini/config/mcp_config.json");
    let mount = data["mounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["target"] == json!(target))
        .unwrap()
        .clone();
    let mounted: Value =
        serde_json::from_slice(&std::fs::read(mount["source"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(
        mounted["mcpServers"],
        json!({
            "comandos":{"command":launcher,"args":["hub"],"disabled":false},
            "beta":{"command":launcher,"args":["serve","beta"],"disabled":false}
        })
    );
}
