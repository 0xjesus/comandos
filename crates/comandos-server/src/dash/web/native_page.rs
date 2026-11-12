//! Explicit compiled-page admission; no legacy source reads or implicit fallback.
use super::{Manifest, Selection, registry::Resolved};
use comandos_core::web_assets::{
    NATIVE_PAGE_FILE, NATIVE_PAGE_VERSION, NativePage, native_index_components,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
#[derive(Debug)]
pub struct Plan {
    pub ids: Vec<String>,
    pub bundle: NativePage,
}
pub fn bundle(manifest: &Manifest, web_dir: &Path) -> Result<NativePage, String> {
    let rel = manifest.path(NATIVE_PAGE_FILE);
    if rel.is_empty() {
        return Err("native page bundle missing".into());
    }
    let bytes = fs::read(web_dir.join(rel)).map_err(|e| e.to_string())?;
    let bundle: NativePage = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let hash = super::registry::sha256_hex(
        comandos_web_view::index_page::shell("es")
            .into_string()
            .as_bytes(),
    );
    if bundle.version != NATIVE_PAGE_VERSION
        || bundle.template_sha256 != hash
        || bundle.components != native_index_components()?
    {
        return Err("native bundle does not match compiled template/inventory".into());
    }
    for root in [
        "/comandos_web_sound.js",
        "/comandos_web_sound_bg.wasm",
        "/comandos_web_content.js",
        "/comandos_web_content_bg.wasm",
        "/workspace.css",
        "/buttons.css",
        "/manifest.webmanifest",
        "/icon-192.png",
        "/icon-512.png",
    ] {
        if !bundle.assets.contains_key(root) {
            return Err(format!("native asset absent: {root}"));
        }
    }
    for (url, logical) in &bundle.assets {
        if !url.starts_with('/')
            || url.contains(['?', '#', '\\', '\0'])
            || url.split('/').any(|v| v == ".." || v == ".")
        {
            return Err(format!("invalid native asset URL: {url}"));
        }
        let rel = manifest.path(logical);
        if rel.is_empty() || !web_dir.join(rel).is_file() {
            return Err(format!("native asset absent: {logical}"));
        }
    }
    Ok(bundle)
}
pub fn admit(
    reg: &Resolved,
    sel: &Selection,
    manifest: &Manifest,
    web_dir: &Path,
) -> Result<Plan, String> {
    manifest.check_files(web_dir)?;
    let bundle = bundle(manifest, web_dir)?;
    let entries: BTreeMap<_, _> = reg.entries().iter().map(|e| (e.id.as_str(), e)).collect();
    if entries.len() != reg.entries().len() {
        return Err("duplicate native component metadata".into());
    }
    let mut visited = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    let mut ids = Vec::new();
    fn visit(
        id: &str,
        entries: &BTreeMap<&str, &super::registry::Entry>,
        sel: &Selection,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        ids: &mut Vec<String>,
    ) -> Result<(), String> {
        if visited.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id.into()) {
            return Err(format!("native component dependency cycle: {id}"));
        }
        let e = entries
            .get(id)
            .ok_or_else(|| format!("native component missing: {id}"))?;
        if !sel.on.contains(id) {
            return Err(format!("native component not selected on: {id}"));
        }
        for dep in &e.deps {
            visit(dep, entries, sel, visiting, visited, ids)?;
        }
        visiting.remove(id);
        visited.insert(id.into());
        ids.push(id.into());
        Ok(())
    }
    for id in &bundle.components {
        visit(id, &entries, sel, &mut visiting, &mut visited, &mut ids)?;
    }
    Ok(Plan { ids, bundle })
}
pub fn render(plan: &Plan, manifest: &Manifest, nonce: &str, embedded: bool) -> Vec<u8> {
    let mut page = comandos_web_view::index_page::shell("es").into_string();
    if embedded {
        // GTK's WebKit rejects this mobile-browser viewport key. Its desktop
        // app needs the supported viewport settings; browsers retain keyboard
        // resizing without changing the canonical packaged template.
        page = page.replacen(", interactive-widget=resizes-content", "", 1);
    }
    for (root, logical) in &plan.bundle.assets {
        let target = format!("/web/{}", manifest.path(logical));
        let stem = root.trim_start_matches('/');
        for old in [
            format!("href=\"{root}\""),
            format!("href=\"{stem}\""),
            format!("href=\"{root}?v=sin-ia1\""),
            format!("href=\"{root}?v=2\""),
            format!("href=\"{stem}?v=an2\""),
        ] {
            page = page.replace(&old, &format!("href=\"{target}\""));
        }
    }
    let meta = format!(
        "<meta name=\"comandos-web\" content=\"{}\"><meta name=\"comandos-web-mode\" content=\"native\">",
        comandos_web_view::escape::attr_esc(&plan.ids.join(" "))
    );
    page = page.replacen("</head>", &format!("{meta}</head>"), 1);
    let boot = format!(
        "<script type=\"module\" data-k=\"{}\" src=\"/web/{}\"></script>",
        comandos_web_view::escape::attr_esc(nonce),
        comandos_web_view::escape::attr_esc(&manifest.path("comandos_web_boot.js"))
    );
    page = page.replacen("</body>", &format!("{boot}</body>"), 1);
    page.into_bytes()
}
pub fn alias(manifest: &Manifest, web_dir: &Path, dash_dir: &Path, url: &str) -> Option<String> {
    if dash_dir.join(url.trim_start_matches('/')).is_file() {
        return None;
    }
    let bundle = bundle(manifest, web_dir).ok()?;
    let logical = bundle.assets.get(url)?;
    let path = manifest.path(logical);
    (!path.is_empty()).then_some(path)
}
