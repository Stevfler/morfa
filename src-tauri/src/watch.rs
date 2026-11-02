use crate::CmdResult;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{path::Path, sync::Mutex};
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
pub struct WatchState(Mutex<Option<RecommendedWatcher>>);

#[tauri::command]
pub fn watch(app: AppHandle, state: State<'_, WatchState>, paths: Vec<String>) -> CmdResult<()> {
    let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            if matches!(ev.kind, notify::EventKind::Access(_)) {
                return;
            }
            let dirs: Vec<String> = ev.paths.iter().filter_map(|p| p.parent()).map(|p| p.to_string_lossy().to_string()).collect();
            let _ = app.emit("fs-changed", dirs);
        }
    })
    .map_err(|e| e.to_string())?;
    for p in &paths {
        let _ = w.watch(Path::new(p), RecursiveMode::NonRecursive);
    }
    *state.0.lock().unwrap() = Some(w);
    Ok(())
}
