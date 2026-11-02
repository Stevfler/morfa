//! Copy/move with progress reporting and cancellation.
mod copy;

use crate::fsops::remove_any;
use crate::CmdResult;
use serde::Serialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
struct Ops {
    running: HashSet<u64>,
    cancelled: HashSet<u64>,
}

#[derive(Default)]
pub struct OpsState(Arc<Mutex<Ops>>);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    id: u64,
    done: u64,
    total: u64,
    current: String,
    finished: bool,
}

/// Completed (source, destination) pairs, plus the error that stopped the op (if any), so the
/// UI can offer undo for the part that did happen.
#[derive(Serialize)]
pub struct TransferResult {
    pairs: Vec<(String, String)>,
    error: Option<String>,
}

pub(crate) struct Ctx {
    app: Option<AppHandle>, // None in tests
    ops: Arc<Mutex<Ops>>,
    id: u64,
    done: u64,
    total: u64,
    last: Instant,
}

impl Ctx {
    fn cancelled(&self) -> bool {
        self.ops.lock().unwrap_or_else(|e| e.into_inner()).cancelled.contains(&self.id)
    }
    fn tick(&mut self, current: &Path, force: bool) {
        if force || self.last.elapsed().as_millis() > 100 {
            self.last = Instant::now();
            let Some(app) = &self.app else { return };
            let _ = app.emit("op-progress", Progress { id: self.id, done: self.done, total: self.total, current: current.to_string_lossy().into(), finished: false });
        }
    }
}

/// Canonical form of a path that may not exist yet: canonicalize its parent, keep the name.
/// Symlinks in the final component are not followed.
fn canon(p: &Path) -> PathBuf {
    match (p.parent().and_then(|d| fs::canonicalize(d).ok()), p.file_name()) {
        (Some(d), Some(n)) => d.join(n),
        _ => fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()),
    }
}

/// Try an instant same-filesystem rename onto the reserved `target`. Returns (renamed, target);
/// when not renamed, `target` is a reserved placeholder for the copy fallback.
#[cfg(not(windows))]
fn rename_into(src: &Path, _meta: &fs::Metadata, _dest: &Path, target: PathBuf) -> CmdResult<(bool, PathBuf)> {
    // POSIX rename atomically replaces our own placeholder (same kind: file/file, empty dir/dir).
    Ok((fs::rename(src, &target).is_ok(), target))
}

/// Windows can't rename onto the placeholder, and std's rename would replace a file that appears
/// once it's freed, so free it and use a non-replacing rename, re-reserving on a lost race.
#[cfg(windows)]
fn rename_into(src: &Path, meta: &fs::Metadata, dest: &Path, mut target: PathBuf) -> CmdResult<(bool, PathBuf)> {
    for _ in 0..100 {
        let _ = remove_any(&target);
        match crate::fsops::rename::rename_noreplace(src, &target) {
            Ok(()) => return Ok((true, target)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => target = copy::reserve(src, meta, dest)?,
            Err(_) => return Ok((false, copy::reserve(src, meta, dest)?)),
        }
    }
    Ok((false, copy::reserve(src, meta, dest)?))
}

fn cleanup(items: &[(PathBuf, PathBuf)]) {
    for (_, t) in items {
        let _ = remove_any(t);
    }
}

fn run(cx: &mut Ctx, sources: &[String], dest: &Path, moving: bool, pairs: &mut Vec<(String, String)>) -> CmdResult<()> {
    let dest_c = fs::canonicalize(dest).unwrap_or_else(|_| canon(dest));
    let mut slow: Vec<(PathBuf, PathBuf)> = vec![];
    let fast = (|| {
        for s in sources {
            if cx.cancelled() {
                return Err("Cancelled".to_string());
            }
            let src = PathBuf::from(s);
            let src_c = canon(&src);
            if dest_c.starts_with(&src_c) {
                return Err(format!("Can't put \"{}\" inside itself", src.file_name().unwrap_or_default().to_string_lossy()));
            }
            if moving && src_c.parent() == Some(dest_c.as_path()) {
                continue; // already there
            }
            let meta = fs::symlink_metadata(&src).map_err(|e| format!("{}: {e}", src.display()))?;
            let mut target = copy::reserve(&src, &meta, dest)?;
            if moving {
                match rename_into(&src, &meta, dest, target)? {
                    (true, t) => {
                        pairs.push((s.clone(), t.to_string_lossy().into()));
                        continue;
                    }
                    (false, t) => target = t,
                }
            }
            slow.push((src, target));
        }
        Ok(())
    })();
    if let Err(e) = fast {
        cleanup(&slow);
        return Err(e);
    }
    let mut total = 0;
    for (s, _) in &slow {
        match copy::size_of(s, cx) {
            Some(n) => total += n,
            None => {
                cleanup(&slow);
                return Err("Cancelled".into());
            }
        }
    }
    cx.total = total;
    for (i, (src, target)) in slow.iter().enumerate() {
        cx.tick(src, true);
        if let Err(e) = copy::fill(src, target, cx) {
            cleanup(&slow[i..]);
            return Err(e);
        }
        if moving {
            if let Err(e) = remove_any(src) {
                cleanup(&slow[i + 1..]);
                // The copy is complete: report it so undo can remove it again.
                pairs.push((src.to_string_lossy().into(), target.to_string_lossy().into()));
                return Err(format!("Copied but couldn't remove \"{}\": {e}", src.display()));
            }
        }
        pairs.push((src.to_string_lossy().into(), target.to_string_lossy().into()));
    }
    Ok(())
}

/// Copy or move items into `dest`. Returns completed (source, destination) pairs (for undo)
/// and the error that stopped the op, if any.
#[tauri::command]
pub async fn transfer(app: AppHandle, state: State<'_, OpsState>, id: u64, sources: Vec<String>, dest: String, mode: String) -> CmdResult<TransferResult> {
    let ops = state.0.clone();
    ops.lock().unwrap_or_else(|e| e.into_inner()).running.insert(id);
    let job = tauri::async_runtime::spawn_blocking(move || {
        let mut cx = Ctx { app: Some(app), ops, id, done: 0, total: 0, last: Instant::now() };
        let mut pairs = vec![];
        let error = run(&mut cx, &sources, Path::new(&dest), mode == "move", &mut pairs).err();
        let _ = cx.app.as_ref().map(|a| a.emit("op-progress", Progress { id, done: cx.total, total: cx.total, current: String::new(), finished: true }));
        TransferResult { pairs, error }
    })
    .await;
    let mut o = state.0.lock().unwrap_or_else(|e| e.into_inner());
    o.running.remove(&id);
    o.cancelled.remove(&id);
    job.map_err(|e| e.to_string())
}

/// Request cancellation; ignored for ops that aren't running (so ids never linger).
#[tauri::command]
pub fn cancel_op(state: State<'_, OpsState>, id: u64) {
    let mut o = state.0.lock().unwrap_or_else(|e| e.into_inner());
    if o.running.contains(&id) {
        o.cancelled.insert(id);
    }
}
