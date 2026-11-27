//! GET `/fs/find?q=<texto>`: busca carpetas de todo el HOME mientras la
//! persona escribe en «Nueva sesión» (web, escritorio y APK).
//!
//! Un índice en memoria de las carpetas hasta [`DEPTH`] niveles (sin ocultas
//! ni carpetas de dependencias o compilación) se arma en el pool de bloqueo y
//! se reusa [`TTL`]; armarlo cuesta decenas de milisegundos y cada consulta
//! solo recorre la lista. El orden premia el nombre exacto, luego el prefijo,
//! el inicio de palabra, la subcadena y por último las letras en orden
//! (`cmdos` → `ComandOS`); los repos de git suben un poco y lo profundo baja.
use std::{
    fs,
    os::unix::ffi::OsStrExt,
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

/// Niveles bajo el HOME que entran al índice.
pub const DEPTH: usize = 5;
/// Cuánto vale un índice antes de rearmarlo.
pub const TTL: Duration = Duration::from_secs(120);
/// Tope de carpetas indexadas (un HOME enorme no se come la memoria).
const CAP: usize = 80_000;
/// Resultados por consulta.
pub const LIMIT: usize = 40;

/// Carpetas que nunca son un proyecto para abrir: dependencias, compilación,
/// cachés y entornos.
const SKIP: &[&str] = &[
    "node_modules",
    "target",
    "build",
    "dist",
    "out",
    "venv",
    "env",
    "__pycache__",
    "site-packages",
    "vendor",
    "Pods",
    "DerivedData",
    "snap",
    "Library",
    "go",
    "worktrees",
];

#[derive(Debug, Clone)]
pub struct Dir {
    /// Relativa al HOME, sin `/` inicial.
    pub rel: String,
    lower: String,
    /// Inicio del último segmento dentro de `rel`.
    name_at: usize,
    pub depth: usize,
    pub repo: bool,
}

#[derive(Debug, Default)]
pub struct Index {
    pub home: String,
    pub dirs: Vec<Dir>,
}

struct Cached {
    built: Instant,
    index: std::sync::Arc<Index>,
}

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

fn skipped(name: &[u8]) -> bool {
    name.first() == Some(&b'.') || SKIP.iter().any(|s| s.as_bytes() == name)
}

/// Recorre `home` a lo ancho, sin seguir enlaces.
pub fn build(home: &str) -> Index {
    let mut dirs = Vec::new();
    let mut level: Vec<String> = vec![String::new()];
    for depth in 1..=DEPTH {
        let mut next = Vec::new();
        for rel in &level {
            let path = if rel.is_empty() {
                home.to_owned()
            } else {
                format!("{home}/{rel}")
            };
            let Ok(entries) = fs::read_dir(&path) else {
                continue;
            };
            let mut children: Vec<(String, bool)> = Vec::new();
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else { continue };
                if !kind.is_dir() {
                    continue;
                }
                let name = entry.file_name();
                if skipped(name.as_bytes()) {
                    continue;
                }
                let Some(name) = name.to_str() else { continue };
                let child = if rel.is_empty() {
                    name.to_owned()
                } else {
                    format!("{rel}/{name}")
                };
                let repo = Path::new(&format!("{home}/{child}/.git")).exists();
                children.push((child, repo));
            }
            children.sort();
            for (child, repo) in children {
                if dirs.len() >= CAP {
                    return Index { home: home.to_owned(), dirs };
                }
                let name_at = child.rfind('/').map_or(0, |i| i + 1);
                dirs.push(Dir {
                    lower: child.to_lowercase(),
                    name_at,
                    depth,
                    repo,
                    rel: child.clone(),
                });
                next.push(child);
            }
        }
        level = next;
    }
    Index { home: home.to_owned(), dirs }
}

static REBUILDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// El índice de `home`. El primero se arma aquí; uno caducado se sigue usando
/// mientras un hilo arma el nuevo, así ninguna tecla espera el disco.
pub fn index(home: &str) -> std::sync::Arc<Index> {
    use std::sync::atomic::Ordering;
    {
        let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = cache.as_ref()
            && c.index.home == home
        {
            if c.built.elapsed() >= TTL && !REBUILDING.swap(true, Ordering::AcqRel) {
                let home = home.to_owned();
                std::thread::spawn(move || {
                    let fresh = std::sync::Arc::new(build(&home));
                    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(Cached { built: Instant::now(), index: fresh });
                    REBUILDING.store(false, Ordering::Release);
                });
            }
            return c.index.clone();
        }
    }
    let index = std::sync::Arc::new(build(home));
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Cached { built: Instant::now(), index: index.clone() });
    index
}

fn subsequence(hay: &str, needle: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// Puntos de un término contra una carpeta; `None` si no aparece.
fn term_score(dir: &Dir, term: &str) -> Option<i64> {
    let name = &dir.lower[dir.name_at..];
    if name == term {
        return Some(1000);
    }
    if name.starts_with(term) {
        return Some(700);
    }
    if let Some(at) = name.find(term) {
        let boundary = name[..at].ends_with(['-', '_', '.', ' ']);
        return Some(if boundary { 520 } else { 380 });
    }
    if term.contains('/') || dir.lower.contains(term) {
        if let Some(at) = dir.lower.find(term) {
            let segment = at == 0 || dir.lower[..at].ends_with('/');
            return Some(if segment { 300 } else { 220 });
        }
    }
    if subsequence(name, term) {
        return Some(140);
    }
    if subsequence(&dir.lower, term) {
        return Some(40);
    }
    None
}

/// Las [`LIMIT`] mejores carpetas para `query`, de mayor a menor.
pub fn search<'a>(index: &'a Index, query: &str, limit: usize) -> Vec<(i64, &'a Dir)> {
    let query = query.trim().trim_start_matches("~/").to_lowercase();
    let terms: Vec<&str> = query.split_whitespace().collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(i64, &Dir)> = index
        .dirs
        .iter()
        .filter_map(|dir| {
            let mut total = 0;
            for term in &terms {
                total += term_score(dir, term)?;
            }
            total += if dir.repo { 60 } else { 0 };
            total -= dir.depth as i64 * 12 + (dir.rel.len() - dir.name_at) as i64 / 3;
            Some((total, dir))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.rel.cmp(&b.1.rel)));
    hits.truncate(limit);
    hits
}

/// La respuesta de `/fs/find`.
pub fn answer(home: &str, query: &str, limit: usize) -> Value {
    let started = Instant::now();
    let index = index(home);
    let results: Vec<Value> = search(&index, query, limit)
        .into_iter()
        .map(|(score, dir)| {
            json!({
                "path": format!("{}/{}", index.home, dir.rel),
                "short": format!("~/{}", dir.rel),
                "name": &dir.rel[dir.name_at..],
                "repo": dir.repo,
                "score": score,
            })
        })
        .collect();
    json!({
        "q": query,
        "home": index.home,
        "indexed": index.dirs.len(),
        "ms": started.elapsed().as_millis() as u64,
        "results": results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Home(std::path::PathBuf);
    impl Home {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn home() -> Home {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let t = Home(std::env::temp_dir().join(format!(
            "fs-find-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        )));
        for d in [
            "codebase/0xJesus/ComandOS/crates/comandos-app",
            "codebase/0xJesus/ComandOS/node_modules/x",
            "codebase/0xJesus/ComandOS/.git",
            "codebase/0xJesus/SAVA/SAVA-WebApps",
            "codebase/other/commander",
            ".hidden/ComandOS",
            "Downloads",
        ] {
            fs::create_dir_all(t.path().join(d)).unwrap();
        }
        t
    }

    #[test]
    fn finds_by_name_prefix_and_scattered_letters_without_hidden_or_deps() {
        let t = home();
        let h = t.path().to_str().unwrap();
        let index = build(h);
        assert!(index.dirs.iter().all(|d| !d.rel.contains("node_modules") && !d.rel.contains(".hidden")));
        let names = |q: &str| search(&index, q, 5).iter().map(|(_, d)| d.rel.clone()).collect::<Vec<_>>();
        assert_eq!(names("comandos")[0], "codebase/0xJesus/ComandOS", "exact name and a repo first");
        assert_eq!(names("sava web")[0], "codebase/0xJesus/SAVA/SAVA-WebApps", "every term must match");
        assert!(names("cmdos").contains(&"codebase/0xJesus/ComandOS".to_owned()), "letters in order");
        assert!(names("~/codebase/0xjesus/sava").contains(&"codebase/0xJesus/SAVA".to_owned()), "a typed path works too");
        assert!(names("zzz").is_empty());
        assert!(search(&index, "  ", 5).is_empty());
    }

    #[test]
    fn the_answer_carries_full_and_short_paths() {
        let t = home();
        let h = t.path().to_str().unwrap().to_owned();
        let v = answer(&h, "downloads", 3);
        assert_eq!(v["results"][0]["short"], "~/Downloads");
        assert_eq!(v["results"][0]["path"], format!("{h}/Downloads"));
        assert_eq!(v["results"][0]["repo"], false);
    }
}
