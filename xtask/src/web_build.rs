//! `cargo run -p xtask -- web-build` (Fase 3, T4): compila los crates WASM con el
//! perfil `release-wasm`, pasa `wasm-bindgen` y `wasm-opt -Oz`, nombra cada crate
//! por el hash de su contenido (`<out>/<hash>/…`) y escribe `<out>/manifest.json`.
//! `<out>` es `<directorio de compilación>/web`, resuelto como cargo (preflight R6),
//! y se instala con `comandos install --stage --web <out>` (origen explícito). Solo
//! se reemplaza entero y al final: un build fallido no crea ni toca `<out>`.
use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

pub use comandos_core::web_assets::{MANIFEST_FILE, Manifest};
pub const OUTPUT_OWNER_FILE: &str = ".comandos-web-build.json";

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputOwner {
    producer: String,
    version: u32,
    manifest_sha256: String,
    files: BTreeMap<String, String>,
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn record_output_owner(out: &Path, manifest: &Manifest) -> Result<(), String> {
    manifest.check_paths()?;
    let files = manifest
        .files
        .values()
        .map(|path| file_sha256(&out.join(path)).map(|hash| (path.clone(), hash)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let owner = OutputOwner {
        producer: "comandos-xtask-web-build".into(),
        version: 1,
        manifest_sha256: file_sha256(&out.join(MANIFEST_FILE))?,
        files,
    };
    let bytes = serde_json::to_vec_pretty(&owner).map_err(|e| e.to_string())?;
    fs::write(out.join(OUTPUT_OWNER_FILE), bytes).map_err(|e| e.to_string())
}

/// Cargador generado. Única lógica: instanciar y ceder el control al WASM,
/// que monta, publica sus globales y avisa a la compuerta (`/web/ready`).
pub const BOOT_TEMPLATE: &str = r#"// Generado por `cargo xtask web-build`. No editar.
import init, { boot } from "{{MODULE}}";
const me = document.currentScript || document.querySelector("script[data-k]");
await init({ module_or_path: new URL("{{WASM}}", import.meta.url) });
boot(me ? me.dataset.k || "" : "");
"#;

/// Transport-only loader: native audio mounts eagerly; Rust requests content only at feature use.
pub fn render_content_boot(module_js: &str, wasm: &str) -> String {
    format!(
        r#"// Generated platform transport; Rust owns selection, feature lifecycle and errors.
import init, {{ boot, needs_sound, dependency_failed, install_content_loader }} from "{module_js}";
const me=document.currentScript||document.querySelector("script[data-k]");
const k=me?me.dataset.k||"":"";
await init({{module_or_path:new URL("{wasm}",import.meta.url)}});
if(needs_sound()){{
 try{{
  const sound=await import("./comandos_web_sound.js");
  await sound.default({{module_or_path:new URL("./comandos_web_sound_bg.wasm",import.meta.url)}});
  sound.register_sound();
 }}catch(error){{dependency_failed(k,error);throw error;}}
}}
install_content_loader(async()=>{{
 const content=await import("./comandos_web_content.js");
 await content.default({{module_or_path:new URL("./comandos_web_content_bg.wasm",import.meta.url)}});
 content.register_content();
}});
boot(k);
"#
    )
}

/// Versiones exactas de las herramientas (el crate `wasm-bindgen` va fijado igual).
pub const WASM_BINDGEN_VERSION: &str = "0.2.129";
pub const WASM_OPT_VERSION: &str = "116";

const USAGE: &str = "uso: cargo run -p xtask -- web-build [--crate comandos-web|comandos-term-web|comandos-web-sw|all] [--check-budget] [--out ABSOLUTE-ARTIFACT-DIR]";

const WASM_TARGET: &str = "wasm32-unknown-unknown";
const PROFILE: &str = "release-wasm";
/// Nombre físico del cargador dentro de `<hash>/` (la página carga `/web/<hash>/boot.js`).
const BOOT_FILE: &str = "boot.js";

pub fn render_boot(module_js: &str, wasm: &str) -> String {
    BOOT_TEMPLATE
        .replace("{{MODULE}}", module_js)
        .replace("{{WASM}}", wasm)
}

pub fn render_worker_boot(wasm: &[u8]) -> String {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.encode(wasm);
    format!(
        "// Generated classic WASM worker loader.\nimportScripts({{{{SW_MODULE}}}});\nwasm_bindgen.initSync({{module: Uint8Array.from(atob(\"{bytes}\"), c => c.charCodeAt(0))}});\nwasm_bindgen.boot(true, {{{{PRECACHE}}}});\n"
    )
}

/// Un archivo con su nombre (o ruta relativa) y sus bytes.
pub type NamedFile = (String, Vec<u8>);

/// Módulos de `inline_js`/`module` que wasm-bindgen deja en
/// `snippets/<crate>-<hash>/<archivo>.js` (los usan los crates web en lugar del
/// constructor `Function`, para que una CSP sin `unsafe-eval` funcione). El
/// frente sirve `/web/<hash>/<archivo>` con un solo segmento, así que cada uno
/// pasa a `snippets-<crate>-<hash>-<archivo>.js` junto al cargador (clave de
/// manifiesto `<lib>_snippets-…`) y su `import './snippets/…'` se reescribe;
/// dos rutas que se aplanen al mismo nombre son un error. Un fragmento que nadie importa (LTO
/// quitó sus usos, p. ej. en el arranque vacío) no se copia; un `./snippets/`
/// que queda sin reescribir es un error (el navegador no lo encontraría).
pub fn flatten_snippets(
    js: &str,
    snippets: Vec<NamedFile>,
) -> Result<(String, Vec<NamedFile>), String> {
    let mut out = js.to_string();
    let mut files: Vec<NamedFile> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (rel, bytes) in snippets {
        let flat = rel.replace('/', "-");
        if !seen.insert(flat.clone()) {
            return Err(format!("dos snippets se aplanan a {flat}"));
        }
        let mut found = false;
        for q in ['\'', '"'] {
            let from = format!("{q}./{rel}{q}");
            if out.contains(&from) {
                out = out.replace(&from, &format!("{q}./{flat}{q}"));
                found = true;
            }
        }
        if found {
            files.push((flat, bytes));
        }
    }
    if out.contains("./snippets/") {
        return Err(
            "el módulo JS importa ./snippets/… que wasm-bindgen no dejó en snippets/".to_string(),
        );
    }
    Ok((out, files))
}

/// `(ruta relativa con `/`, bytes)` de cada archivo bajo `dir/snippets`, en orden.
fn read_snippets(dir: &Path) -> Result<Vec<NamedFile>, String> {
    let root = dir.join("snippets");
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for entry in rd {
            let entry = entry.map_err(|e| format!("no se pudo leer {}: {e}", d.display()))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(dir) {
                let rel: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                let bytes = fs::read(&path)
                    .map_err(|e| format!("no se pudo leer {}: {e}", path.display()))?;
                out.push((rel.join("/"), bytes));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// `sha256` de las partes concatenadas, 12 dígitos hexadecimales.
pub fn content_hash(parts: &[&[u8]]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize()
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Salida de `wasm-bindgen`: módulo ES (`web`) o script clásico (`no-modules`, el
/// service worker).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindgenTarget {
    Web,
    NoModules,
}

impl BindgenTarget {
    fn flag(self) -> &'static str {
        match self {
            BindgenTarget::Web => "web",
            BindgenTarget::NoModules => "no-modules",
        }
    }
}

/// Un crate `cdylib` que se compila a WASM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmCrate {
    /// Nombre del paquete (`-p`).
    pub name: String,
    /// Nombre de la biblioteca: el del paquete con `_` (salida de rustc y bindgen).
    pub lib: String,
    pub target: BindgenTarget,
    /// Presupuesto del `.wasm` comprimido con gzip, en bytes.
    pub budget_gzip: usize,
}

impl WasmCrate {
    pub fn new(name: &str, target: BindgenTarget, budget_gzip: usize) -> Self {
        WasmCrate {
            name: name.to_string(),
            lib: name.replace('-', "_"),
            target,
            budget_gzip,
        }
    }

    /// Nombres lógicos del manifiesto: `<lib>.js`, `<lib>_bg.wasm` y, en módulos
    /// ES, `<lib>_boot.js` (el archivo físico es `<hash>/boot.js`).
    fn files(&self) -> (String, String, Option<String>) {
        let boot = (self.target == BindgenTarget::Web).then(|| format!("{}_boot.js", self.lib));
        (
            format!("{}.js", self.lib),
            format!("{}_bg.wasm", self.lib),
            boot,
        )
    }
}

/// Los tres crates de la Fase 3 y sus presupuestos gzip (600/280/64 KiB).
pub fn known_crates() -> Vec<WasmCrate> {
    vec![
        WasmCrate::new("comandos-web", BindgenTarget::Web, 600 * 1024),
        // Includes the owned link action menu and client-scoped touch pane selection.
        // Measured at 272 KiB gzip; retain a bounded 280 KiB ceiling.
        WasmCrate::new("comandos-term-web", BindgenTarget::Web, 280 * 1024),
        WasmCrate::new("comandos-web-sw", BindgenTarget::NoModules, 64 * 1024),
    ]
}

/// Rutas de las herramientas externas.
#[derive(Debug, Clone)]
pub struct Tools {
    pub cargo: PathBuf,
    pub wasm_bindgen: PathBuf,
    pub wasm_opt: PathBuf,
}

impl Tools {
    /// `CARGO` (lo fija cargo), `WASM_BINDGEN` y `WASM_OPT`, o los nombres en `PATH`.
    pub fn from_env() -> Self {
        let pick = |var: &str, default: &str| {
            std::env::var_os(var)
                .filter(|v| !v.is_empty())
                .map_or_else(|| PathBuf::from(default), PathBuf::from)
        };
        Tools {
            cargo: pick("CARGO", "cargo"),
            wasm_bindgen: pick("WASM_BINDGEN", "wasm-bindgen"),
            wasm_opt: pick("WASM_OPT", "wasm-opt"),
        }
    }

    /// Comprueba las versiones exactas; si faltan o no coinciden, el error dice
    /// cómo instalarlas.
    pub fn check(&self) -> Result<(), String> {
        let bindgen = version(&self.wasm_bindgen);
        if bindgen
            .as_deref()
            .map(|v| v.split_whitespace().last() != Some(WASM_BINDGEN_VERSION))
            .unwrap_or(true)
        {
            return Err(format!(
                "wasm-bindgen {WASM_BINDGEN_VERSION} requerido (hallado: {}); instalar con: cargo install --locked wasm-bindgen-cli --version ={WASM_BINDGEN_VERSION}",
                bindgen.unwrap_or_else(|| "ninguno".into())
            ));
        }
        let opt = version(&self.wasm_opt);
        if opt
            .as_deref()
            .map(|v| !wasm_opt_version_ok(v))
            .unwrap_or(true)
        {
            return Err(format!(
                "wasm-opt {WASM_OPT_VERSION} requerido (hallado: {}); instalar con: cargo install --locked wasm-opt --version =0.116.1",
                opt.unwrap_or_else(|| "ninguno".into())
            ));
        }
        Ok(())
    }
}

/// `wasm-opt version 116 (version_116)`: la palabra que sigue a `version` es
/// exactamente `116`.
pub fn wasm_opt_version_ok(line: &str) -> bool {
    line.split_whitespace()
        .skip_while(|w| *w != "version")
        .nth(1)
        == Some(WASM_OPT_VERSION)
}

/// Primera línea de `<tool> --version`, o `None` si no se pudo ejecutar.
fn version(tool: &Path) -> Option<String> {
    let out = Command::new(tool).arg("--version").output().ok()?;
    out.status.success().then_some(())?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().next().map(|l| l.trim().to_string())
}

/// Qué compilar y adónde.
#[derive(Debug, Clone)]
pub struct Options {
    /// `Cargo.toml` del workspace que contiene los crates.
    pub manifest_path: PathBuf,
    /// `CARGO_TARGET_DIR` de la compilación wasm.
    pub target_dir: PathBuf,
    /// Directorio de salida (`<target>/web`).
    pub out: PathBuf,
    pub crates: Vec<WasmCrate>,
    pub check_budget: bool,
    pub tools: Tools,
}

/// Directorio de compilación del workspace, resuelto como cargo (R6).
pub fn target_dir() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("sin directorio de trabajo: {e}"))?;
    let env = std::env::var_os("CARGO_TARGET_DIR");
    Ok(comandos_core::web_assets::target_dir_from(
        env.as_deref().map(Path::new),
        &cwd,
        &workspace_root(),
    ))
}

/// `<directorio de compilación>/web`: lo que escribe `web-build`, lo que se pasa a
/// `install --stage --web` y lo que leerá el `build.rs` de B15.
pub fn out_dir() -> Result<PathBuf, String> {
    target_dir().map(|t| t.join("web"))
}

/// Raíz del workspace: el padre de `xtask/`.
fn workspace_root() -> PathBuf {
    let xtask = Path::new(env!("CARGO_MANIFEST_DIR"));
    xtask.parent().unwrap_or(xtask).to_path_buf()
}

/// Argumentos de `wasm-bindgen` para un crate.
pub fn bindgen_args(c: &WasmCrate, wasm: &Path, out_dir: &Path) -> Vec<OsString> {
    vec![
        "--target".into(),
        c.target.flag().into(),
        "--no-typescript".into(),
        "--out-dir".into(),
        out_dir.into(),
        "--out-name".into(),
        c.lib.clone().into(),
        wasm.into(),
    ]
}

/// Tamaño comprimido con gzip al máximo nivel (como se mide el presupuesto).
pub fn gzip_len(bytes: &[u8]) -> Result<usize, String> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::best());
    enc.write_all(bytes).map_err(|e| format!("gzip: {e}"))?;
    enc.finish()
        .map(|v| v.len())
        .map_err(|e| format!("gzip: {e}"))
}

/// Con `enforce`, error si `gz` supera el presupuesto del crate.
pub fn check_budget(c: &WasmCrate, gz: usize, enforce: bool) -> Result<(), String> {
    if enforce && gz > c.budget_gzip {
        return Err(format!(
            "{}: {} B gzip supera el presupuesto de {} B ({} KiB)",
            c.name,
            gz,
            c.budget_gzip,
            c.budget_gzip / 1024
        ));
    }
    Ok(())
}

/// Corre un comando y devuelve error con su stderr si falla.
fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd
        .output()
        .map_err(|e| format!("{what}: no se pudo ejecutar: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "{what} falló ({}):\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    ))
}

/// Características WASM que rustc 1.96 activa por defecto en `wasm32-unknown-unknown`;
/// `wasm-opt` 116 las necesita explícitas para validar el módulo.
const WASM_OPT_FEATURES: &[&str] = &[
    "--enable-bulk-memory",
    "--enable-nontrapping-float-to-int",
    "--enable-sign-ext",
    "--enable-mutable-globals",
    "--enable-reference-types",
    "--enable-multivalue",
];

/// Archivos de un crate ya procesado, listos para copiar.
struct Built {
    hash: String,
    files: Vec<(String, String, Vec<u8>)>, // (lógico, físico, bytes)
}

fn build_one(opts: &Options, c: &WasmCrate) -> Result<Built, String> {
    run(
        Command::new(&opts.tools.cargo)
            .arg("build")
            .arg("--manifest-path")
            .arg(&opts.manifest_path)
            .args([
                "--locked",
                "-p",
                &c.name,
                "--profile",
                PROFILE,
                "--target",
                WASM_TARGET,
            ])
            .env("CARGO_TARGET_DIR", &opts.target_dir),
        &format!("cargo build -p {}", c.name),
    )?;
    let raw = opts
        .target_dir
        .join(WASM_TARGET)
        .join(PROFILE)
        .join(format!("{}.wasm", c.lib));
    let work = opts.target_dir.join("web-build").join(&c.name);
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| format!("no se pudo crear {}: {e}", work.display()))?;
    run(
        Command::new(&opts.tools.wasm_bindgen).args(bindgen_args(c, &raw, &work)),
        &format!("wasm-bindgen {}", c.name),
    )?;
    let (js_name, wasm_name, boot_name) = c.files();
    let wasm_path = work.join(&wasm_name);
    run(
        Command::new(&opts.tools.wasm_opt)
            .arg("-Oz")
            .args(WASM_OPT_FEATURES)
            .arg("-o")
            .arg(&wasm_path)
            .arg(&wasm_path),
        &format!("wasm-opt {}", c.name),
    )?;
    let read = |p: &Path| fs::read(p).map_err(|e| format!("no se pudo leer {}: {e}", p.display()));
    let js = read(&work.join(&js_name))?;
    let (js, snippets) = flatten_snippets(
        &String::from_utf8(js).map_err(|e| format!("{js_name} no es UTF-8: {e}"))?,
        read_snippets(&work)?,
    )?;
    let js = js.into_bytes();
    let wasm = read(&wasm_path)?;
    let gz = gzip_len(&wasm)?;
    println!(
        "{}: {} B wasm, {} B gzip (presupuesto {} KiB)",
        c.name,
        wasm.len(),
        gz,
        c.budget_gzip / 1024
    );
    check_budget(c, gz, opts.check_budget)?;
    let worker_boot = (c.name == "comandos-web-sw" && c.target == BindgenTarget::NoModules)
        .then(|| render_worker_boot(&wasm));
    let mut files = vec![
        (js_name.clone(), js_name.clone(), js),
        (wasm_name.clone(), wasm_name.clone(), wasm),
    ];
    if let Some(loader) = worker_boot {
        let gz = gzip_len(loader.as_bytes())?;
        println!("{}: {} B gzip classic synchronous loader", c.name, gz);
        check_budget(c, gz, opts.check_budget)?;
        files.push((
            format!("{}_boot.js", c.lib),
            "worker-boot.js".into(),
            loader.into_bytes(),
        ));
        files.push((
            "comandos_web_sw_component.json".into(),
            "comandos_web_sw_component.json".into(),
            include_bytes!("../../crates/comandos-web-sw/components/sw.json").to_vec(),
        ));
    }
    // Clave lógica con el crate delante: dos WASM que usen el mismo crate
    // (p. ej. `comandos-web-dom`) emiten el mismo nombre plano en sus `<hash>/`
    // respectivos, y el manifiesto debe seguir siendo un mapa fiel.
    for (name, bytes) in snippets {
        files.push((format!("{}_{name}", c.lib), name, bytes));
    }
    if let Some(boot) = boot_name {
        let text = if c.name == "comandos-web" {
            render_content_boot(&format!("./{js_name}"), &format!("./{wasm_name}"))
        } else {
            render_boot(&format!("./{js_name}"), &format!("./{wasm_name}"))
        };
        files.push((boot, BOOT_FILE.to_string(), text.into_bytes()));
    }
    if c.name == "comandos-web" && c.target == BindgenTarget::Web {
        let mut content_gzip = 0;
        let mut sound_gzip = 0;
        for (name, budget) in [
            ("comandos-web-content", 350 * 1024),
            ("comandos-web-sound", 250 * 1024),
        ] {
            let companion = build_one(opts, &WasmCrate::new(name, BindgenTarget::Web, budget))?;
            for (logical, physical, bytes) in companion.files {
                if physical == BOOT_FILE {
                    continue;
                }
                if physical.ends_with(".wasm") {
                    let size = gzip_len(&bytes)?;
                    if name == "comandos-web-content" {
                        content_gzip += size;
                    } else {
                        sound_gzip += size;
                    }
                }
                if let Some((_, _, existing)) = files.iter().find(|(_, path, _)| path == &physical)
                {
                    if existing != &bytes {
                        return Err(format!("companion file collision: {physical}"));
                    }
                    continue;
                }
                files.push((logical, physical, bytes));
            }
        }
        println!(
            "dashboard WASM gzip: initial {} B (main {} + sound {}), deferred {} B, cumulative {} B",
            gz + sound_gzip,
            gz,
            sound_gzip,
            content_gzip,
            gz + sound_gzip + content_gzip
        );
        let mut initial_transport_gzip = 0;
        let mut deferred_transport_gzip = 0;
        for (_, physical, bytes) in &files {
            if physical.ends_with(".js") {
                if physical == "comandos_web_content.js" {
                    deferred_transport_gzip += gzip_len(bytes)?;
                } else {
                    initial_transport_gzip += gzip_len(bytes)?;
                }
            }
        }
        println!(
            "dashboard WASM+JS gzip: initial {} B, deferred {} B, cumulative {} B (JS initial {}, deferred {})",
            gz + sound_gzip + initial_transport_gzip,
            content_gzip + deferred_transport_gzip,
            gz + sound_gzip + content_gzip + initial_transport_gzip + deferred_transport_gzip,
            initial_transport_gzip,
            deferred_transport_gzip
        );
        files.extend(
            native_page_files(
                opts.manifest_path
                    .parent()
                    .ok_or("workspace manifest has no parent")?,
            )?
            .into_iter()
            .map(|(name, bytes)| (name.clone(), name, bytes)),
        );
    }
    if c.name == "comandos-web-sw" {
        let total = files.iter().try_fold(0usize, |sum, (_, _, bytes)| {
            gzip_len(bytes).map(|n| sum + n)
        })?;
        println!(
            "{}: {} B gzip complete worker artifact (including embedded WASM duplication)",
            c.name, total
        );
        check_budget(c, total, opts.check_budget)?;
    }
    if c.name == "comandos-term-web" && c.target == BindgenTarget::Web {
        files.extend(
            crate::native_term_page::files(
                opts.manifest_path
                    .parent()
                    .ok_or("workspace manifest has no parent")?,
            )?
            .into_iter()
            .map(|(name, bytes)| (name.clone(), name, bytes)),
        );
    }
    if c.name == "comandos-web"
        && let Some((_, _, bytes)) = files
            .iter_mut()
            .find(|(name, _, _)| name == comandos_core::web_assets::NATIVE_PAGE_FILE)
    {
        let mut page: comandos_core::web_assets::NativePage =
            serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        for name in [
            "comandos_web_content.js",
            "comandos_web_content_bg.wasm",
            "comandos_web_sound.js",
            "comandos_web_sound_bg.wasm",
        ] {
            page.assets.insert(format!("/{name}"), name.into());
        }
        *bytes = serde_json::to_vec(&page).map_err(|e| e.to_string())?;
    }
    let parts: Vec<&[u8]> = files.iter().map(|(_, _, b)| b.as_slice()).collect();
    Ok(Built {
        hash: content_hash(&parts),
        files,
    })
}

/// Un nombre de directorio de hash: 12 dígitos hexadecimales en minúscula.
fn is_hash_dir(name: &str) -> bool {
    name.len() == 12 && name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Escribe los archivos de un crate en `<next>/<hash>/`.
fn write_built(next: &Path, b: &Built) -> Result<(), String> {
    let dir = next.join(&b.hash);
    fs::create_dir_all(&dir).map_err(|e| format!("no se pudo crear {}: {e}", dir.display()))?;
    for (_, physical, bytes) in &b.files {
        let p = dir.join(physical);
        fs::write(&p, bytes).map_err(|e| format!("no se pudo escribir {}: {e}", p.display()))?;
    }
    Ok(())
}

/// Copia (sin subir) los archivos regulares de `from` a `to`, que se crea.
fn copy_flat(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("no se pudo crear {}: {e}", to.display()))?;
    let rd = fs::read_dir(from).map_err(|e| format!("no se pudo leer {}: {e}", from.display()))?;
    for entry in rd {
        let entry = entry.map_err(|e| format!("no se pudo leer {}: {e}", from.display()))?;
        if entry.file_type().is_ok_and(|t| t.is_file()) {
            let dest = to.join(entry.file_name());
            fs::copy(entry.path(), &dest)
                .map_err(|e| format!("no se pudo copiar a {}: {e}", dest.display()))?;
        }
    }
    Ok(())
}

/// Completa en `next` (que ya tiene los directorios de hash de `fresh`) la salida
/// nueva: une `fresh` sobre `old` (compilar un crate conserva los demás), copia de
/// `out` los directorios de hash de `old` que siguen vivos y escribe
/// `next/manifest.json`. Una entrada vieja cuyo directorio falta se descarta con
/// aviso. `out` no se toca: la salida es solo de `web-build`, así que lo que el
/// manifiesto no referencia no pasa a la nueva.
pub fn assemble(
    out: &Path,
    next: &Path,
    old: Manifest,
    fresh: Manifest,
) -> Result<Manifest, String> {
    let mut merged = Manifest::default();
    for (logical, path) in old.files {
        if fresh.files.contains_key(&logical) {
            continue;
        }
        let Some(hash) = path.split('/').next().filter(|h| is_hash_dir(h)) else {
            eprintln!("aviso: {logical} → {path} no tiene forma <hash>/…; se descarta");
            continue;
        };
        let have = next.join(hash);
        if !have.is_dir() {
            let from = out.join(hash);
            if !from.is_dir() {
                eprintln!(
                    "aviso: {logical} → {path} falta en {}; se descarta",
                    out.display()
                );
                continue;
            }
            copy_flat(&from, &have)?;
        }
        merged.files.insert(logical, path);
    }
    merged.files.extend(fresh.files);
    let text = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?;
    let path = next.join(MANIFEST_FILE);
    fs::write(&path, format!("{text}\n"))
        .map_err(|e| format!("no se pudo escribir {}: {e}", path.display()))?;
    record_output_owner(next, &merged)?;
    Ok(merged)
}

/// Pone `next` en el lugar de `out`. Si `out` existe, `renameat2(RENAME_EXCHANGE)`
/// los intercambia de una vez y la salida vieja acaba en `trash`, que se borra; si el
/// sistema de archivos no lo admite, dos `rename` (con una ventana sin `out`).
pub fn swap_into_place(next: &Path, out: &Path, trash: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    use nix::fcntl::{AT_FDCWD, RenameFlags, renameat2};
    let err = |e: &dyn std::fmt::Display| format!("no se pudo instalar {}: {e}", out.display());
    validate_output(out)?;
    validate_output(next)?;
    if fs::symlink_metadata(next).is_err() {
        return Err("staged output is missing".into());
    }
    if fs::symlink_metadata(trash).is_ok() {
        return Err("refusing to remove pre-existing swap trash".into());
    }
    let staged = fs::canonicalize(next).map_err(|e| e.to_string())?;
    let destination = if out.exists() {
        fs::canonicalize(out).map_err(|e| e.to_string())?
    } else {
        fs::canonicalize(out.parent().ok_or("output has no parent")?)
            .map_err(|e| e.to_string())?
            .join(out.file_name().ok_or("output has no name")?)
    };
    if staged.starts_with(&destination)
        || destination.starts_with(&staged)
        || trash.starts_with(next)
        || trash.starts_with(out)
    {
        return Err("swap paths must be distinct and disjoint".into());
    }
    if out.symlink_metadata().is_err() {
        return fs::rename(next, out).map_err(|e| err(&e));
    }
    #[cfg(target_os = "linux")]
    if renameat2(AT_FDCWD, next, AT_FDCWD, out, RenameFlags::RENAME_EXCHANGE).is_ok() {
        // Tras el intercambio, `next` es la salida vieja.
        fs::rename(next, trash).map_err(|e| err(&e))?;
        let _ = fs::remove_dir_all(trash);
        return Ok(());
    }
    swap_with_renames(next, out, trash)?;
    let _ = fs::remove_dir_all(trash);
    Ok(())
}

fn swap_with_renames(next: &Path, out: &Path, trash: &Path) -> Result<(), String> {
    fs::rename(out, trash).map_err(|e| format!("no se pudo instalar {}: {e}", out.display()))?;
    if let Err(error) = fs::rename(next, out) {
        return match fs::rename(trash, out) {
            Ok(()) => Err(format!("no se pudo instalar {}: {error}", out.display())),
            Err(restore) => Err(format!(
                "no se pudo instalar {}: {error}; salida anterior en {}: {restore}",
                out.display(),
                trash.display()
            )),
        };
    }
    Ok(())
}

/// Compila los crates de `opts` y, solo si todo salió bien, sustituye `<out>` por
/// la salida nueva con su `manifest.json`. Un fallo (cargo, bindgen, wasm-opt o
/// presupuesto) no crea ni toca `<out>`. Los temporales viven en
/// `<target>/web-build/`, fuera de `<out>`.
pub fn build(opts: &Options) -> Result<Manifest, String> {
    validate_output(&opts.out)?;
    opts.tools.check()?;
    let built = opts
        .crates
        .iter()
        .map(|c| build_one(opts, c))
        .collect::<Result<Vec<_>, _>>()?;
    let path = opts.out.join(MANIFEST_FILE);
    let old = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("{} ilegible: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Manifest::default(),
        Err(e) => return Err(format!("no se pudo leer {}: {e}", path.display())),
    };
    let work = opts.target_dir.join("web-build");
    let pid = std::process::id();
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let next = work.join(format!("out.{pid}.{nonce}"));
    fs::create_dir(&next).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut fresh = Manifest::default();
        for b in &built {
            write_built(&next, b)?;
            for (logical, physical, _) in &b.files {
                fresh
                    .files
                    .insert(logical.clone(), format!("{}/{physical}", b.hash));
            }
        }
        let merged = assemble(&opts.out, &next, old, fresh)?;
        if let Some(parent) = opts.out.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("no se pudo crear {}: {e}", parent.display()))?;
        }
        swap_into_place(&next, &opts.out, &work.join(format!("old.{pid}.{nonce}")))?;
        Ok(merged)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&next);
    }
    result
}

/// `(--crate elegido o None = todos, --check-budget)`.
pub fn parse_args(args: &[String]) -> Result<(Option<String>, bool, Option<PathBuf>), String> {
    let mut krate = None;
    let mut budget = false;
    let mut output = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--check-budget" => budget = true,
            "--out" => output = Some(PathBuf::from(it.next().ok_or(USAGE)?)),
            "--crate" => {
                let v = it.next().ok_or(USAGE)?;
                if v == "all" {
                    krate = None;
                } else if known_crates().iter().any(|c| &c.name == v) {
                    krate = Some(v.clone());
                } else {
                    return Err(format!("crate desconocido: {v}\n{USAGE}"));
                }
            }
            _ => return Err(USAGE.to_string()),
        }
    }
    Ok((krate, budget, output))
}

/// Only an absent directory or an existing artifact output may be replaced.
pub fn validate_output(out: &Path) -> Result<(), String> {
    if !out.is_absolute() {
        return Err("--out must be an absolute artifact directory".into());
    }
    if out.parent().is_none() || out.file_name().is_none() {
        return Err("--out must name an artifact directory, never a filesystem root".into());
    }
    if fs::symlink_metadata(out).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("--out cannot replace a symlink".into());
    }
    if !out.exists() {
        return Ok(());
    }
    if !out.is_dir() {
        return Err("--out must be a directory".into());
    }
    let regular = |p: &Path| fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file());
    if !regular(&out.join(MANIFEST_FILE)) || !regular(&out.join(OUTPUT_OWNER_FILE)) {
        return Err(format!(
            "{} lacks a regular manifest and verified web-build ownership record; use a fresh --out",
            out.display()
        ));
    }
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(out.join(MANIFEST_FILE)).map_err(|e| e.to_string())?)
            .map_err(|e| format!("invalid output manifest: {e}"))?;
    manifest.check_paths()?;
    if manifest.files.is_empty() {
        return Err("owned output manifest is empty".into());
    }
    let owner: OutputOwner =
        serde_json::from_slice(&fs::read(out.join(OUTPUT_OWNER_FILE)).map_err(|e| e.to_string())?)
            .map_err(|e| format!("invalid output ownership record: {e}"))?;
    if owner.producer != "comandos-xtask-web-build"
        || owner.version != 1
        || owner.manifest_sha256 != file_sha256(&out.join(MANIFEST_FILE))?
    {
        return Err("output ownership does not match the manifest".into());
    }
    let mut expected = BTreeSet::new();
    let mut directories = BTreeSet::new();
    for path in manifest.files.values() {
        let Some((dir, file)) = path.split_once('/') else {
            return Err("output asset path lacks hash directory".into());
        };
        if !is_hash_dir(dir)
            || file.contains('/')
            || file.is_empty()
            || !fs::symlink_metadata(out.join(dir)).is_ok_and(|m| m.file_type().is_dir())
            || !expected.insert(path.clone())
            || !regular(&out.join(path))
        {
            return Err("output contains invalid, missing, duplicate or linked assets".into());
        }
        let Some(hash) = owner.files.get(path) else {
            return Err("output asset is not owned".into());
        };
        if &file_sha256(&out.join(path))? != hash {
            return Err("owned output asset changed".into());
        }
        directories.insert(dir.to_string());
    }
    if expected != owner.files.keys().cloned().collect() {
        return Err("ownership inventory differs from manifest".into());
    }
    for entry in fs::read_dir(out).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink()
            || !((name == MANIFEST_FILE || name == OUTPUT_OWNER_FILE) && kind.is_file()
                || directories.contains(&name) && kind.is_dir())
        {
            return Err(format!(
                "refusing to replace non-artifact output {}",
                out.display()
            ));
        }
        if kind.is_dir() {
            for child in fs::read_dir(entry.path()).map_err(|e| e.to_string())? {
                let child = child.map_err(|e| e.to_string())?;
                let path = format!("{name}/{}", child.file_name().to_string_lossy());
                if !child.file_type().map_err(|e| e.to_string())?.is_file()
                    || !expected.contains(&path)
                {
                    return Err("refusing to remove an unowned output child".into());
                }
            }
        }
    }
    Ok(())
}

/// Punto de entrada del subcomando; devuelve el código de salida.
pub fn main(args: &[String]) -> i32 {
    let (krate, check_budget, output) = match parse_args(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let ws = workspace_root();
    // `all` compila los crates que ya existen; uno explícito debe existir.
    let present = |c: &WasmCrate| ws.join("crates").join(&c.name).join("Cargo.toml").is_file();
    let crates: Vec<WasmCrate> = match &krate {
        Some(name) => known_crates()
            .into_iter()
            .filter(|c| &c.name == name)
            .collect(),
        None => known_crates().into_iter().filter(present).collect(),
    };
    if let Some(missing) = crates.iter().find(|c| !present(c)) {
        eprintln!("error: crates/{} no existe todavía", missing.name);
        return 1;
    }
    if crates.is_empty() {
        eprintln!("error: no hay crates WASM en el workspace (llegan con A6, B1 y B9)");
        return 1;
    }
    let result = target_dir().and_then(|target_dir| {
        let opts = Options {
            manifest_path: ws.join("Cargo.toml"),
            out: output.clone().unwrap_or_else(|| target_dir.join("web")),
            target_dir,
            crates,
            check_budget,
            tools: Tools::from_env(),
        };
        if output.is_some() {
            validate_output(&opts.out)?;
        }
        build(&opts).map(|m| (opts.out, m))
    });
    match result {
        Ok((out, m)) => {
            println!(
                "{} archivos en {}",
                m.files.len(),
                out.join(MANIFEST_FILE).display()
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Bundle only static native assets and their CSS dependencies; own legacy JS
/// is never copied. All URLs in CSS/PWA resolve within the versioned directory.
pub fn native_page_files(workspace: &Path) -> Result<Vec<NamedFile>, String> {
    use comandos_core::web_assets::{
        NATIVE_PAGE_FILE, NATIVE_PAGE_VERSION, NativePage, native_index_components,
    };
    use std::collections::{BTreeMap, BTreeSet};
    let roots = [
        "/workspace.css",
        "/buttons.css",
        "/manifest.webmanifest",
        "/icon-192.png",
        "/icon-512.png",
    ];
    let urls =
        regex::Regex::new(r#"url\(\s*['\"]?([^'\")]+)['\"]?\s*\)"#).map_err(|e| e.to_string())?;
    let mut source = BTreeMap::new();
    let mut pending = roots
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    while let Some(url) = pending.pop_first() {
        if source.contains_key(&url) {
            continue;
        }
        let rel = url.trim_start_matches('/');
        if rel
            .split('/')
            .any(|s| s.is_empty() || s == ".." || s == ".")
        {
            return Err(format!("invalid native asset {url}"));
        }
        let source_path = if rel.starts_with("assets/") {
            workspace.join(rel)
        } else {
            workspace.join("dash").join(rel)
        };
        let bytes = fs::read(source_path).map_err(|e| format!("native asset {url}: {e}"))?;
        if url.ends_with(".css") {
            let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
            for captures in urls.captures_iter(text) {
                if let Some(m) = captures.get(1) {
                    let value = m.as_str();
                    if !value.starts_with("data:") && !value.contains("://") {
                        pending.insert(format!("/{}", value.trim_start_matches('/')));
                    }
                }
            }
        }
        source.insert(url, bytes);
    }
    let mut assets = BTreeMap::new();
    for url in source.keys() {
        let suffix = url.rsplit('/').next().unwrap_or("asset");
        let physical = format!("native-{}-{suffix}", content_hash(&[url.as_bytes()]));
        assets.insert(url.clone(), physical);
    }
    let mut files = Vec::new();
    for (url, bytes) in source {
        let physical = assets.get(&url).ok_or("native mapping missing")?.clone();
        let bytes = if url.ends_with(".css") {
            let mut text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            let edits = urls
                .captures_iter(&text)
                .filter_map(|caps| caps.get(1).map(|m| m.as_str().to_string()))
                .collect::<Vec<_>>();
            for original in edits {
                let root = format!("/{}", original.trim_start_matches('/'));
                if let Some(target) = assets.get(&root) {
                    text = text
                        .replace(&format!("url('{original}')"), &format!("url('./{target}')"))
                        .replace(
                            &format!("url(\"{original}\")"),
                            &format!("url(\"./{target}\")"),
                        )
                        .replace(&format!("url({original})"), &format!("url(./{target})"));
                }
            }
            text.into_bytes()
        } else if url == "/manifest.webmanifest" {
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if let Some(icons) = manifest
                .get_mut("icons")
                .and_then(serde_json::Value::as_array_mut)
            {
                for icon in icons {
                    if let Some(src) = icon.get("src").and_then(serde_json::Value::as_str)
                        && let Some(target) = assets.get(src)
                    {
                        let target = format!("./{target}");
                        if let Some(object) = icon.as_object_mut() {
                            object.insert("src".into(), serde_json::Value::String(target));
                        }
                    }
                }
            }
            if let Some(object) = manifest.as_object_mut() {
                object.insert(
                    "start_url".into(),
                    serde_json::Value::String("/?web=native".into()),
                );
            }
            serde_json::to_vec(&manifest).map_err(|e| e.to_string())?
        } else {
            bytes
        };
        files.push((physical, bytes));
    }
    let page = NativePage {
        version: NATIVE_PAGE_VERSION,
        template_sha256: format!(
            "{:x}",
            sha2::Sha256::digest(
                comandos_web_view::index_page::shell("es")
                    .into_string()
                    .as_bytes()
            )
        ),
        components: native_index_components()?,
        assets,
    };
    files.push((
        NATIVE_PAGE_FILE.into(),
        serde_json::to_vec(&page).map_err(|e| e.to_string())?,
    ));
    Ok(files)
}

#[cfg(test)]
mod swap_transaction_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "comandos-web-swap-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn portable_swap_keeps_old_tree_until_new_tree_is_installed() {
        let root = Fixture::new();
        let (out, next, trash) = (
            root.0.join("out"),
            root.0.join("next"),
            root.0.join("trash"),
        );
        fs::create_dir(&out).unwrap();
        fs::create_dir(&next).unwrap();
        fs::write(out.join("version"), b"old").unwrap();
        fs::write(next.join("version"), b"new").unwrap();
        swap_with_renames(&next, &out, &trash).unwrap();
        assert_eq!(fs::read(out.join("version")).unwrap(), b"new");
        assert_eq!(fs::read(trash.join("version")).unwrap(), b"old");
    }

    #[test]
    fn failed_second_rename_restores_previous_output() {
        let root = Fixture::new();
        let (out, next, trash) = (
            root.0.join("out"),
            root.0.join("missing-next"),
            root.0.join("trash"),
        );
        fs::create_dir(&out).unwrap();
        fs::write(out.join("version"), b"old").unwrap();
        assert!(swap_with_renames(&next, &out, &trash).is_err());
        assert_eq!(fs::read(out.join("version")).unwrap(), b"old");
        assert!(!trash.exists());
    }
}
