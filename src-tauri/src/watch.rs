//! Directory watching. Non-recursive on the open folders only, so on Windows we
//! never hold a handle on ancestors (renaming/deleting a watched folder's
//! parent stays possible as long as that folder itself isn't open). Every
//! `watch` call replaces (drops) the previous watcher, so stale handles are
//! released whenever the frontend's open-folder set changes (`syncWatch`).
use crate::{guard, netfs, CmdResult};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, State};

const DEBOUNCE: Duration = Duration::from_millis(100);

/// The watcher, and the latest request: an older one still resolving paths must not win.
#[derive(Default)]
pub struct WatchState(Arc<(Mutex<Option<RecommendedWatcher>>, AtomicU64)>);

/// The real path of a folder safe to watch; `None` on a network filesystem or one not answering.
/// There, inotify and friends can block or never fire, and panes reload on focus and refresh.
fn watchable(p: &str) -> Option<PathBuf> {
    if netfs::network_mount_of(Path::new(p)).is_some() {
        return None;
    }
    let path = p.to_string();
    guard::run_blocking(p, guard::quick_for(p), move |_| Ok((!netfs::is_network_at(Path::new(&path))).then(|| std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone().into()))))
        .ok()
        .flatten()
}

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
pub async fn watch(app: AppHandle, state: State<'_, WatchState>, paths: Vec<String>) -> CmdResult<Vec<String>> {
    let shared = state.0.clone();
    let my = shared.1.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn_blocking(move || {
        let (slot, latest) = &*shared;
        // Drop the old watcher first so its OS handles are released before re-watching. Checked
        // against the latest request, whose watcher a late-running older one must not clear.
        {
            let mut cur = slot.lock().unwrap_or_else(|e| e.into_inner());
            if latest.load(Ordering::SeqCst) == my {
                *cur = None;
            }
        }
        // Paths that can't be watched are handed back to the caller along with those that fail.
        let mut failed = vec![];
        let mut roots: Vec<(PathBuf, String)> = vec![];
        for p in paths {
            match watchable(&p) {
                Some(canon) => roots.push((canon, p)),
                None => failed.push(p),
            }
        }
        if latest.load(Ordering::SeqCst) != my {
            return Ok(failed);
        }
        let watched: Vec<String> = roots.iter().map(|(_, p)| p.clone()).collect();
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
        for p in &watched {
            if let Err(e) = w.watch(Path::new(p), RecursiveMode::NonRecursive) {
                eprintln!("watch: {p}: {e}");
                failed.push(p.clone());
            }
        }
        let mut cur = slot.lock().unwrap_or_else(|e| e.into_inner());
        if latest.load(Ordering::SeqCst) == my {
            *cur = Some(w);
        }
        Ok(failed)
    })
    .await
    .map_err(|e| e.to_string())?
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
