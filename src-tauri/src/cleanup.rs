//! Disk clean-up: space map + clean-up categories from one same-filesystem scan,
//! and a protection-checked delete. Uses `ignore`'s parallel walker (already a dependency).
mod categories;
mod guard;
mod index;
mod protect;
mod scan;

use crate::CmdResult;
use index::{Index, Node};
pub(crate) use guard::protection;
pub use protect::Protection;
use serde::Serialize;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, State};

const CHILDREN: usize = 40;

#[derive(Default)]
pub struct CleanupState {
    gen: Arc<AtomicU64>,
    index: Arc<Mutex<Option<Index>>>,
}

#[derive(Serialize, Clone)]
struct ProgressEvent {
    id: u64,
    files: u64,
    bytes: u64,
}

#[derive(Serialize)]
pub struct Report {
    root: Node,
    children: Vec<Node>,
    categories: Vec<categories::Category>,
    elapsed_ms: u64,
}

#[derive(Serialize, Default)]
pub struct DeleteReport {
    removed: Vec<String>,
    /// `[path, reason]`
    rejected: Vec<(String, String)>,
    freed: u64,
}

/// Scan `root` (one filesystem). Progress: `cleanup-progress` {id, files, bytes} ~5×/s.
/// `Err("cancelled")` if `cleanup_cancel` or a newer scan superseded it.
#[tauri::command]
pub async fn cleanup_scan(app: AppHandle, state: State<'_, CleanupState>, root: String, id: u64) -> CmdResult<Report> {
    crate::guard::probe(&root).await?;
    let (gen, slot) = (state.gen.clone(), state.index.clone());
    gen.store(id, Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let prog = Arc::new(scan::Progress { files: AtomicU64::new(0), bytes: AtomicU64::new(0) });
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (p2, d2, a2) = (prog.clone(), done.clone(), app.clone());
        let ticker = std::thread::spawn(move || {
            while !d2.load(Ordering::Relaxed) {
                let _ = a2.emit("cleanup-progress", ProgressEvent { id, files: p2.files.load(Ordering::Relaxed), bytes: p2.bytes.load(Ordering::Relaxed) });
                std::thread::park_timeout(Duration::from_millis(200));
            }
        });
        let root = Path::new(&root);
        let ix = scan::scan(root, &prog, &|| gen.load(Ordering::Relaxed) != id);
        done.store(true, Ordering::Relaxed);
        ticker.thread().unpark();
        let _ = ticker.join();
        let ix = ix.ok_or("cancelled")?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let report = Report { root: ix.node(root), children: ix.children(root, CHILDREN), categories: categories::categorize(&ix, now), elapsed_ms: started.elapsed().as_millis() as u64 };
        *slot.lock().map_err(|e| e.to_string())? = Some(ix);
        Ok(report)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cleanup_cancel(state: State<'_, CleanupState>) {
    state.gen.store(0, Ordering::SeqCst);
}

/// Children of a folder from the last scan's index (largest first).
#[tauri::command]
pub fn cleanup_children(state: State<'_, CleanupState>, path: String) -> CmdResult<Vec<Node>> {
    let g = state.index.lock().map_err(|e| e.to_string())?;
    let ix = g.as_ref().ok_or("no scan yet")?;
    Ok(ix.children(Path::new(&path), CHILDREN))
}

/// Trash (default) or permanently delete. Protection is re-checked here regardless of the UI.
#[tauri::command]
pub async fn cleanup_delete(state: State<'_, CleanupState>, paths: Vec<String>, permanent: bool) -> CmdResult<DeleteReport> {
    let slot = state.index.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut r = DeleteReport::default();
        let ix = slot.lock().map_err(|e| e.to_string())?;
        for p in paths {
            let path = Path::new(&p);
            if let Some(why) = guard::refuse(path, ix.as_ref()) {
                r.rejected.push((p, why.into()));
                continue;
            }
            let size = ix.as_ref().map_or(0, |i| i.size_of(path));
            let res = if permanent { crate::fsops::remove_any(path).map_err(|e| e.to_string()) } else { crate::fsops::trashcan::trash_blocking(std::slice::from_ref(&p)) };
            match res {
                Ok(()) => {
                    r.freed += size;
                    r.removed.push(p);
                }
                Err(e) => r.rejected.push((p, e)),
            }
        }
        Ok(r)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Permanently empty the user's Trash (Linux & Windows). macOS: `Err` starting `UNSUPPORTED_EMPTY_TRASH:`.
#[tauri::command]
pub async fn cleanup_empty_trash() -> CmdResult<()> {
    #[cfg(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))))]
    {
        tauri::async_runtime::spawn_blocking(|| {
            let items = trash::os_limited::list().map_err(|e| e.to_string())?;
            trash::os_limited::purge_all(items).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android")))))]
    {
        Err("UNSUPPORTED_EMPTY_TRASH: emptying Trash isn't supported here — use Show in Trash".into())
    }
}

