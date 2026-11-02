//! Directory watching. Non-recursive on the open folders only, so on Windows we
//! never hold a handle on ancestors (renaming/deleting a watched folder's
//! parent stays possible as long as that folder itself isn't open). Every
//! `watch` call replaces (drops) the previous watcher, so stale handles are
//! released whenever the frontend's open-folder set changes (`syncWatch`).
use crate::CmdResult;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{mpsc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Emitter, State};

const DEBOUNCE: Duration = Duration::from_millis(100);

#[derive(Default)]
pub struct WatchState(Mutex<Option<RecommendedWatcher>>);

/// Map an event path (possibly canonical, e.g. FSEvents' `/private/var/..`) back
/// to the root as the UI registered it.
fn ui_path(roots: &[(PathBuf, String)], p: &Path) -> String {
    for (canon, req) in roots {
        if let Ok(rest) = p.strip_prefix(canon) {
            return if rest.as_os_str().is_empty() { req.clone() } else { Path::new(req).join(rest).to_string_lossy().into() };
        }
    }
    p.to_string_lossy().into()
}

#[tauri::command]
pub fn watch(app: AppHandle, state: State<'_, WatchState>, paths: Vec<String>) -> CmdResult<Vec<String>> {
    // Drop the old watcher first so its OS handles are released before re-watching.
    *state.0.lock().unwrap() = None;
    let roots: Vec<(PathBuf, String)> = paths.iter().map(|p| (std::fs::canonicalize(p).unwrap_or_else(|_| p.into()), p.clone())).collect();
    let (tx, rx) = mpsc::channel::<String>();
    let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
        Ok(ev) if !matches!(ev.kind, notify::EventKind::Access(_)) => {
            for p in ev.paths.iter().filter_map(|p| p.parent()) {
                let _ = tx.send(ui_path(&roots, p));
            }
        }
        Ok(_) => {}
        Err(e) => eprintln!("watch: {e}"),
    })
    .map_err(|e| e.to_string())?;
    // Debounce thread: exits once the watcher (holding `tx`) is dropped.
    std::thread::spawn(move || {
        while let Ok(first) = rx.recv() {
            let mut dirs = BTreeSet::from([first]);
            while let Ok(d) = rx.recv_timeout(DEBOUNCE) {
                dirs.insert(d);
            }
            let _ = app.emit("fs-changed", dirs.into_iter().collect::<Vec<_>>());
        }
    });
    // Watch what we can; one bad folder (deleted, no permission) must not
    // leave the others unwatched. Failed paths are returned to the caller.
    let mut failed = vec![];
    for p in &paths {
        if let Err(e) = w.watch(Path::new(p), RecursiveMode::NonRecursive) {
            eprintln!("watch: {p}: {e}");
            failed.push(p.clone());
        }
    }
    *state.0.lock().unwrap() = Some(w);
    Ok(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_canonical_back() {
        let roots = vec![(PathBuf::from("/private/var/x"), "/var/x".to_string())];
        assert_eq!(ui_path(&roots, Path::new("/private/var/x")), "/var/x");
        assert_eq!(ui_path(&roots, Path::new("/private/var/x/sub")), "/var/x/sub");
        assert_eq!(ui_path(&roots, Path::new("/other")), "/other");
    }
}
