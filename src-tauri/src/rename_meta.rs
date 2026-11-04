//! Metadata for batch-rename tokens, and the check of the names a batch would produce. Everything
//! read from a file is untrusted: it's parsed from a bounded amount of bytes, and leaves here only
//! sanitised. Nothing here writes; renames go through `move_pairs_report`.
mod check;
mod read;
mod sanitize;
#[cfg(test)]
mod tests;

use crate::{guard, CmdResult};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::State;

pub use check::{Row, RowCheck};
pub use read::FileMeta;

/// Running calls' cancel flags.
#[derive(Default)]
pub struct RenameMetaState(Mutex<HashMap<u64, Arc<AtomicBool>>>);
impl RenameMetaState {
    fn begin(&self, id: u64) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(id, flag.clone());
        flag
    }
    fn end(&self, id: u64) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    }
}

/// Reads one batch of files. `image` and `audio` ask for those parsers; the modified time always comes.
#[tauri::command]
pub async fn rename_meta_read(state: State<'_, RenameMetaState>, id: u64, paths: Vec<String>, image: bool, audio: bool) -> CmdResult<Vec<FileMeta>> {
    let Some(first) = paths.first().cloned() else { return Ok(vec![]) };
    let flag = state.begin(id);
    let (worker, watcher) = (flag.clone(), flag.clone());
    let r = guard::guarded_cancel(&first, guard::stall_for(&first), move || watcher.load(Ordering::Relaxed), move |beat| {
        let mut out = Vec::with_capacity(paths.len());
        for p in &paths {
            if worker.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let tick = || beat.tick();
            out.push(read::read_meta(p, image, audio, &worker, &tick));
            beat.tick();
        }
        Ok(out)
    })
    .await;
    // A read given up on as stuck stops by itself if it ever wakes up.
    flag.store(true, Ordering::Relaxed);
    state.end(id);
    r
}

/// Judges the names a batch would produce, against each other and the disk.
#[tauri::command]
pub async fn rename_check(state: State<'_, RenameMetaState>, id: u64, rows: Vec<Row>) -> CmdResult<Vec<RowCheck>> {
    let Some(first) = rows.first().map(|r| r.path.clone()) else { return Ok(vec![]) };
    let flag = state.begin(id);
    let (worker, watcher) = (flag.clone(), flag.clone());
    let r = guard::guarded_cancel(&first, guard::stall_for(&first), move || watcher.load(Ordering::Relaxed), move |beat| {
        let tick = || beat.tick();
        check::check_rows(&rows, &crate::compare::detect_fold, &check::name_max, &worker, &tick)
    })
    .await;
    flag.store(true, Ordering::Relaxed);
    state.end(id);
    r
}

#[tauri::command]
pub fn rename_cancel(state: State<'_, RenameMetaState>, id: u64) {
    if let Some(f) = state.0.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
        f.store(true, Ordering::Relaxed);
    }
}
