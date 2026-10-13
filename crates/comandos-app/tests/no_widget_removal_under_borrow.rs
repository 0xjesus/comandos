//! Removing a GTK widget emits signals that can re-enter our callbacks
//! (`PaneOverlay::reposition` borrows `pills` and `grips`). Draining a
//! `RefCell` while removing its widgets panicked with «RefCell already
//! mutably borrowed» and aborted the app when switching accounts (13-oct).
//! Take the list out first, then remove the widgets.
use std::path::Path;

fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((
                path.display().to_string(),
                std::fs::read_to_string(&path).unwrap(),
            ));
        }
    }
}

#[test]
fn widgets_are_never_removed_while_their_refcell_is_borrowed() {
    let mut files = vec![];
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut offenders = vec![];
    for (name, text) in files {
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !(line.contains("for ") && line.contains(".borrow_mut().drain(")) {
                continue;
            }
            let body = lines[i + 1..].iter().take_while(|l| l.trim() != "}");
            if body
                .into_iter()
                .any(|l| l.contains(".remove(&") || l.contains(".destroy()"))
            {
                offenders.push(format!("{name}:{}", i + 1));
            }
        }
    }
    assert!(offenders.is_empty(), "widget removal under borrow_mut: {offenders:?}");
}
