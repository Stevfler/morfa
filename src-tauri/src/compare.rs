//! Compare the folders of two panes and apply a reviewed plan to them. Scanning never writes and
//! never follows a symlink. Nothing here chooses an action: the plan comes from the user.
mod apply;
mod scan;
pub(crate) use scan::detect_fold;
#[cfg(test)]
mod tests;

use crate::{fsops::rename::same_file, guard, CmdResult};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tauri::{AppHandle, Emitter, State};

pub(crate) use apply::{Plan, Record, SyncReport};
pub(crate) use scan::{CompareResult, Mode};

/// FAT stores times in 2 s steps, and network filesystems round them too.
pub const TOLERANCE_MS: u64 = 2000;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    File,
    Dir,
    Symlink,
    Special,
}

/// What an item looked like, compared again just before acting on it. For a folder: totals over
/// everything inside, and the latest time found there, so a change at any depth shows.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snap {
    pub kind: Kind,
    pub size: u64,
    pub mtime: u64,
    pub files: u64,
    pub errors: u64,
    pub target: Option<String>,
}

/// The cancel flag and the "still alive" beat of a running job.
pub(crate) struct Job<'a> {
    pub cancel: &'a AtomicBool,
    pub beat: &'a (dyn Fn() + Sync),
}
impl Job<'_> {
    pub fn stopped(&self) -> bool {
        (self.beat)();
        self.cancel.load(Ordering::Relaxed)
    }
}

pub(crate) const CANCELLED: &str = "cancelled";
/// An item that exists or may exist, but can't be looked at. Never the same as absent.
pub(crate) const UNREADABLE: &str = "unreadable";
const TEMP_PREFIX: &str = ".morfa-sync-";

/// A temporary name of ours, left by a crash at worst: not part of what's being compared.
pub(crate) fn is_own_temp(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|n| n.starts_with(TEMP_PREFIX) && n.ends_with(".part"))
}
const MAX_DEPTH: usize = 200;

pub(crate) fn mtime_ms(m: &fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64)
}

pub(crate) fn kind_of(m: &fs::Metadata) -> Kind {
    let t = m.file_type();
    if t.is_symlink() {
        Kind::Symlink
    } else if t.is_dir() {
        Kind::Dir
    } else if t.is_file() {
        Kind::File
    } else {
        Kind::Special
    }
}

/// `None`: `p` isn't there. `Err`: cancelled, or `UNREADABLE`. `on_bad` hears of what can't be copied inside a folder.
pub(crate) fn snap(p: &Path, job: &Job, on_bad: &mut dyn FnMut(&Path, &'static str)) -> Result<Option<Snap>, String> {
    let m = match fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(UNREADABLE.into()),
    };
    let mut s = Snap { kind: kind_of(&m), size: 0, mtime: mtime_ms(&m), files: 0, errors: 0, target: None };
    match s.kind {
        Kind::File => s.size = m.len(),
        Kind::Symlink => s.target = fs::read_link(p).ok().map(|t| t.to_string_lossy().into_owned()),
        Kind::Special => {}
        Kind::Dir => add_tree(p, &mut s, 0, job, on_bad)?,
    }
    Ok(Some(s))
}

fn add_tree(dir: &Path, sum: &mut Snap, depth: usize, job: &Job, on_bad: &mut dyn FnMut(&Path, &'static str)) -> Result<(), String> {
    if job.stopped() {
        return Err(CANCELLED.into());
    }
    let list = if depth >= MAX_DEPTH { None } else { fs::read_dir(dir).ok() };
    let Some(list) = list else {
        sum.errors += 1;
        on_bad(dir, "unreadable");
        return Ok(());
    };
    for e in list {
        let Ok(e) = e else {
            sum.errors += 1;
            on_bad(dir, "unreadable");
            continue;
        };
        if is_own_temp(&e.file_name()) {
            continue;
        }
        let path = e.path();
        let Ok(m) = fs::symlink_metadata(&path) else {
            sum.errors += 1;
            on_bad(&path, "unreadable");
            continue;
        };
        sum.mtime = sum.mtime.max(mtime_ms(&m));
        match kind_of(&m) {
            Kind::Dir => add_tree(&path, sum, depth + 1, job, on_bad)?,
            Kind::Special => {
                sum.files += 1;
                on_bad(&path, "special");
            }
            Kind::File => {
                sum.files += 1;
                sum.size += m.len();
            }
            Kind::Symlink => sum.files += 1,
        }
    }
    Ok(())
}

/// Is `inner` the same as, or anywhere below, `outer`? Compared by identity as well as by path, so
/// a case-insensitive disk or a second spelling can't hide it.
fn within(inner: &Path, outer: &Path) -> bool {
    inner.starts_with(outer) || inner.ancestors().any(|a| same_file(a, outer))
}

/// Canonical folders that may be compared. Errors starting with `compare-` are translated by the UI.
pub(crate) fn check_roots(left: &str, right: &str) -> CmdResult<(PathBuf, PathBuf)> {
    let open = |p: &str| {
        let c = fs::canonicalize(p).map_err(|e| format!("{p}: {e}"))?;
        if c.is_dir() {
            Ok(c)
        } else {
            Err(format!("{p}: not a folder"))
        }
    };
    let (l, r) = (open(left)?, open(right)?);
    if l == r || same_file(&l, &r) {
        return Err("compare-same".into());
    }
    if within(&l, &r) || within(&r, &l) {
        return Err("compare-nested".into());
    }
    Ok((l, r))
}

#[derive(Default)]
/// Running jobs' cancel flags, and the one scan a plan may refer to.
pub struct CompareState(Mutex<HashMap<u64, Arc<AtomicBool>>>, Mutex<Option<Arc<Record>>>);
impl CompareState {
    fn begin(&self, id: u64) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(id, flag.clone());
        flag
    }
    fn end(&self, id: u64) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ScanProgress {
    id: u64,
    files: u64,
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

#[tauri::command]
pub async fn compare_check(left: String, right: String) -> CmdResult<()> {
    guard::probe(&left).await?;
    guard::probe(&right).await?;
    let stall = guard::quick_for(&left).max(guard::quick_for(&right));
    let path = left.clone();
    guard::guarded(&path, stall, move |_| check_roots(&left, &right).map(|_| ())).await
}

#[tauri::command]
pub async fn compare_scan(app: AppHandle, state: State<'_, CompareState>, id: u64, left: String, right: String, mode: Mode) -> CmdResult<CompareResult> {
    guard::probe(&left).await?;
    guard::probe(&right).await?;
    let flag = state.begin(id);
    let stall = guard::stall_for(&left).max(guard::stall_for(&right));
    let (worker, watcher) = (flag.clone(), flag.clone());
    let path = left.clone();
    let r = guard::guarded_cancel(&path, stall, move || watcher.load(Ordering::Relaxed), move |beat| {
        let (l, r) = check_roots(&left, &right)?;
        let beat = beat.clone();
        let tick = move || beat.tick();
        let job = Job { cancel: &worker, beat: &tick };
        let mut last = Instant::now();
        let mut report = |files: u64, bytes: u64, current: &str| {
            if last.elapsed().as_millis() > 100 {
                last = Instant::now();
                let _ = app.emit("compare-progress", ScanProgress { id, files, bytes, current: current.into() });
            }
        };
        scan::scan(&l, &r, mode, &job, &mut report)
    })
    .await;
    // A walk given up on as stuck stops by itself if it ever wakes up.
    flag.store(true, Ordering::Relaxed);
    state.end(id);
    let mut r = r?;
    r.scan_id = id;
    *state.1.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(Record::of(&r)));
    Ok(r)
}

/// Past the probes it runs to the end, like a transfer: abandoning it would misreport what it did.
#[tauri::command]
pub async fn compare_sync(app: AppHandle, state: State<'_, CompareState>, id: u64, plan: Plan) -> CmdResult<SyncReport> {
    guard::probe(&plan.left_root).await?;
    guard::probe(&plan.right_root).await?;
    let record = state.1.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or("compare-stale")?;
    let flag = state.begin(id);
    let worker = flag.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let noop = || {};
        let job = Job { cancel: &worker, beat: &noop };
        let mut last = Instant::now();
        let mut report = |done: u64, total: u64, current: &Path| {
            if last.elapsed().as_millis() > 100 {
                last = Instant::now();
                let _ = app.emit("op-progress", OpProgress { id, done, total, current: current.to_string_lossy().into(), finished: false });
            }
        };
        let trash = |p: &Path| crate::fsops::trashcan::trash_blocking(&[p.to_string_lossy().into_owned()]);
        let out = apply::run(&plan, &record, id, &job, &trash, &mut report);
        let _ = app.emit("op-progress", OpProgress { id, done: 0, total: 0, current: String::new(), finished: true });
        out
    })
    .await
    .map_err(|e| e.to_string());
    state.end(id);
    r?
}

/// Reverses what a sync reported; see `apply::undo`.
#[tauri::command]
pub async fn compare_undo(created: Vec<String>, replaced: Vec<String>, trashed: Vec<String>) -> CmdResult<()> {
    if let Some(first) = created.iter().chain(&replaced).chain(&trashed).next() {
        guard::probe(first).await?;
    }
    tauri::async_runtime::spawn_blocking(move || {
        let trash = |p: &Path| crate::fsops::trashcan::trash_blocking(&[p.to_string_lossy().into_owned()]);
        let restore = |p: &str| crate::fsops::trashcan::restore_blocking(&[p.to_string()]);
        apply::undo(&created, &replaced, &trashed, &trash, &restore)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn compare_cancel(state: State<'_, CompareState>, id: u64) {
    if let Some(flag) = state.0.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}
