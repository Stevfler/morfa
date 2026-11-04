//! Find duplicate files in a folder and move the extras the user picked to Trash. Scanning never
//! writes and never follows a symlink. Nothing here chooses what goes: the plan comes from the user,
//! and it's checked against what the scan found. Undo goes through `compare_undo`.
mod apply;
mod scan;
#[cfg(test)]
mod tests;

use crate::{
    cleanup::protection as protection_of,
    cloud::CloudState,
    compare::Job,
    guard, CmdResult,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tauri::{AppHandle, Emitter, State};

pub(crate) use apply::{Plan, Record, Report};
pub(crate) use scan::{Phase, ScanResult};

#[derive(Default)]
/// Running jobs' cancel flags, the one scan a plan may refer to, and whether a removal is running.
pub struct DedupeState {
    jobs: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    record: Mutex<Option<Record>>,
    busy: AtomicBool,
}
impl DedupeState {
    fn begin(&self, id: u64) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.jobs.lock().unwrap_or_else(|e| e.into_inner()).insert(id, flag.clone());
        flag
    }
    fn end(&self, id: u64) {
        self.jobs.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ScanProgress {
    id: u64,
    phase: Phase,
    done: u64,
    total: u64,
    bytes: u64,
    current: String,
}

/// Same shape as the progress of a transfer: drives the op row.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OpProgress {
    id: u64,
    done: u64,
    total: u64,
    current: String,
    finished: bool,
}

/// The flag is released last, once the record is back in place, whichever way the call ends.
struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[tauri::command]
pub async fn dedupe_scan(app: AppHandle, state: State<'_, DedupeState>, cloud: State<'_, CloudState>, id: u64, root: String, min_size: u64) -> CmdResult<ScanResult> {
    if state.busy.load(Ordering::SeqCst) {
        return Err("dedupe-busy".into());
    }
    guard::probe(&root).await?;
    let mirrors = cloud.mirror_roots();
    let flag = state.begin(id);
    let (worker, watcher) = (flag.clone(), flag.clone());
    let stall = guard::stall_for(&root);
    let path = root.clone();
    let r = guard::guarded_cancel(&path, stall, move || watcher.load(Ordering::Relaxed), move |beat| {
        let canon = std::fs::canonicalize(&root).map_err(|e| format!("{root}: {e}"))?;
        if !canon.is_dir() {
            return Err(format!("{root}: not a folder"));
        }
        let beat = beat.clone();
        let tick = move || beat.tick();
        let job = Job { cancel: &worker, beat: &tick };
        let mut last = Instant::now();
        let mut report = |phase: Phase, done: u64, total: u64, bytes: u64, current: &Path| {
            if last.elapsed().as_millis() > 100 {
                last = Instant::now();
                let _ = app.emit("dedupe-progress", ScanProgress { id, phase, done, total, bytes, current: current.to_string_lossy().into() });
            }
        };
        let home = dirs::home_dir().map(|h| std::fs::canonicalize(&h).unwrap_or(h));
        // In the worker: a mirror on a dead mount can't hold up the command. Both spellings count.
        let mirrors: Vec<_> = mirrors.into_iter().flat_map(|m| [std::fs::canonicalize(&m).ok(), Some(m)]).flatten().collect();
        let opts = scan::Options { min_size, mirrors: &mirrors, home: home.as_deref(), protection: &protection_of };
        scan::scan(&canon, &opts, &job, &mut report)
    })
    .await;
    // A walk given up on as stuck stops by itself if it ever wakes up.
    flag.store(true, Ordering::Relaxed);
    state.end(id);
    let mut r = r?;
    r.scan_id = id;
    *state.record.lock().unwrap_or_else(|e| e.into_inner()) = Some(Record::of(&r));
    Ok(r)
}

/// Cancel stops before the next file and still reports. A call that stops making progress (a dead
/// share) is given up on: its cancel flag is set, the scan's record dropped, and the UI asks for a new scan.
#[tauri::command]
pub async fn dedupe_remove(app: AppHandle, state: State<'_, DedupeState>, id: u64, plan: Plan) -> CmdResult<Report> {
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("dedupe-busy".into());
    }
    let _busy = Busy(&state.busy);
    let mut record = state.record.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or("dedupe-stale")?;
    let root = record.root().to_string();
    guard::probe(&root).await?;
    let flag = state.begin(id);
    let worker = flag.clone();
    let done = guard::guarded(&root, guard::stall_for(&root) * 3, move |beat| {
        let beat = beat.clone();
        let tick = move || beat.tick();
        let job = Job { cancel: &worker, beat: &tick };
        let mut last = Instant::now();
        let mut report = |done: u64, total: u64, current: &Path| {
            job.stopped();
            if last.elapsed().as_millis() > 100 {
                last = Instant::now();
                let _ = app.emit("op-progress", OpProgress { id, done, total, current: current.to_string_lossy().into(), finished: false });
            }
        };
        let trash = |p: &Path| crate::fsops::trashcan::trash_blocking(&[p.to_string_lossy().into_owned()]);
        let out = apply::run(&plan, &mut record, &job, &apply::Env { trash: &trash, protection: &protection_of, rehash_limit: apply::REHASH_LIMIT }, &mut report);
        let _ = app.emit("op-progress", OpProgress { id, done: 0, total: 0, current: String::new(), finished: true });
        Ok((out, record))
    })
    .await;
    state.end(id);
    let mut slot = state.record.lock().unwrap_or_else(|e| e.into_inner());
    let (out, record) = match done {
        Ok(d) => d,
        Err(e) => {
            flag.store(true, Ordering::Relaxed);
            // Some of it may have gone: what the record says can't be relied on.
            *slot = None;
            return Err(e);
        }
    };
    // What went to Trash can't serve as a kept copy any more; unless a new scan has replaced the record.
    if slot.as_ref().is_some_and(|r| r.id() == record.id()) {
        *slot = Some(record);
    }
    out
}

#[tauri::command]
pub fn dedupe_cancel(state: State<'_, DedupeState>, id: u64) {
    if let Some(flag) = state.jobs.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}
