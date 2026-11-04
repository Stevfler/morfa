//! Optional index of file and folder names, for search as you type. Off by default. Holds names and
//! modification times, never contents, and never leaves this computer. See `engine` for how it stays
//! current; when it can't answer for a folder, `search` walks the folder as it always did.
//!
//! Custom structure rather than SQLite: the only query is "substring of a name", which neither B-trees
//! nor stock FTS tokens help with (LIKE '%x%' scans; a trigram FTS is several times the size of the
//! names), so SQLite would add a C dependency to do the same scan slower.
mod apply;
mod engine;
mod query;
mod store;
#[cfg(test)]
mod tests;
mod tree;
mod walk;

use crate::CmdResult;
pub use engine::{Engine, Reason, Request, Status};
pub use query::Query;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
pub struct IndexState(OnceLock<Arc<Engine>>);

impl IndexState {
    pub fn init(&self, app: AppHandle) {
        use tauri::Manager;
        let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("morfa");
        let dir = cache.join("file-index");
        let private = [Some(cache), app.path().app_cache_dir().ok()].into_iter().flatten().collect();
        let emit = move |s: &Status| {
            let _ = app.emit("index-status", s);
        };
        let _ = self.0.set(Engine::start(dir, private, Box::new(emit)));
    }
    pub fn engine(&self) -> Option<Arc<Engine>> {
        self.0.get().cloned()
    }
}

fn real_path(p: &Path) -> std::io::Result<PathBuf> {
    let r = std::fs::canonicalize(p)?;
    // Windows' verbatim form (`\\?\C:\x`) wouldn't compare with anything else.
    #[cfg(windows)]
    if let Some(s) = r.to_str().and_then(|s| s.strip_prefix(r"\\?\")).filter(|s| !s.starts_with("UNC")) {
        return Ok(PathBuf::from(s));
    }
    Ok(r)
}

/// Whether the index can answer for this pane folder, and its real path if so.
pub async fn choose(engine: &Engine, pane: &str, gitignore: bool) -> Result<PathBuf, Reason> {
    if gitignore {
        return Err(Reason::Gitignore); // the index doesn't know what's ignored
    }
    engine.ready()?;
    if crate::netfs::network_mount_of(Path::new(pane)).is_some() {
        return Err(Reason::NotCovered);
    }
    let p = pane.to_string();
    let real = crate::guard::guarded(pane, crate::guard::quick_for(pane), move |_| real_path(Path::new(&p)).map_err(|e| e.to_string())).await.map_err(|_| Reason::NotCovered)?;
    engine.covers(&real)?;
    Ok(real)
}

/// Off the main thread: turning off removes a file, on a disk that may be slow.
#[tauri::command]
pub async fn index_configure(state: State<'_, IndexState>, enabled: bool, roots: Vec<String>, excludes: Vec<String>) -> CmdResult<()> {
    if let Some(e) = state.engine() {
        tauri::async_runtime::spawn_blocking(move || e.configure(enabled.then_some(Request { roots, excludes }))).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[tauri::command]
pub fn index_status(state: State<'_, IndexState>) -> Option<Status> {
    state.engine().map(|e| e.status())
}
#[tauri::command]
pub fn index_pause(state: State<'_, IndexState>, paused: bool) {
    if let Some(e) = state.engine() {
        e.pause(paused);
    }
}
#[tauri::command]
pub fn index_cancel(state: State<'_, IndexState>) {
    if let Some(e) = state.engine() {
        e.cancel();
    }
}
#[tauri::command]
pub fn index_rebuild(state: State<'_, IndexState>) {
    if let Some(e) = state.engine() {
        e.rebuild();
    }
}
/// Deletes the file and the copy in memory. Returns once they're gone.
#[tauri::command]
pub async fn index_delete(state: State<'_, IndexState>) -> CmdResult<()> {
    if let Some(e) = state.engine() {
        tauri::async_runtime::spawn_blocking(move || e.delete_now()).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}
