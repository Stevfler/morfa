//! Finder-style colour labels and free-form tags.
//!
//! Colours are just tags with well-known names ("Red", "Blue", ...), so one list
//! per file covers both. On Linux the list lives in the freedesktop xattr
//! `user.xdg.tags`; everywhere (and when xattrs are unsupported) it is mirrored in
//! the app's own JSON store, which also serves as the index for "all files tagged X".
use crate::entry::{entry_from, entry_unresponsive, Entry};
use crate::{guard, netfs, CmdResult};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod db;
mod xattr;

/// One stuck path ends the batch: the rest answer from the db, or as placeholders.
const STALL: Duration = Duration::from_secs(5);

/// Runs `each` over `paths` on one guarded thread, returning results until it stalls, in order.
fn batch<R: Send + 'static>(key: &str, paths: Vec<String>, each: impl Fn(&str) -> R + Send + 'static) -> Vec<R> {
    let results = Arc::new(Mutex::new(Vec::with_capacity(paths.len())));
    let r2 = results.clone();
    let _ = guard::run_keyed(key, key, STALL, guard::LIMITS, move |beat| {
        for p in &paths {
            let r = each(p);
            r2.lock().unwrap_or_else(|e| e.into_inner()).push(r);
            beat.tick();
        }
        Ok(())
    });
    let mut done = results.lock().unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *done)
}

/// Tags stored in xattrs, for paths where asking is safe. On a share the db alone answers: this
/// runs per row, and over the whole db for the sidebar's counts.
fn xattrs(paths: Vec<String>) -> HashMap<String, Vec<String>> {
    if !cfg!(target_os = "linux") {
        return HashMap::new(); // a no-op there
    }
    let paths: Vec<String> = paths.into_iter().filter(|p| netfs::network_mount_of(Path::new(p)).is_none()).collect();
    batch("tags-xattr", paths, |p| (p.to_string(), xattr::read(Path::new(p)))).into_iter().filter_map(|(p, t)| Some((p, t?))).collect()
}

/// The xattr wins when present (other apps may have edited it); otherwise the db.
fn merged(db: &db::Db, xa: &HashMap<String, Vec<String>>, path: &str) -> Vec<String> {
    xa.get(path).or_else(|| db.get(path)).cloned().unwrap_or_default()
}

/// Tags for each path; paths without tags are omitted.
#[tauri::command]
pub async fn tags_get(paths: Vec<String>) -> CmdResult<HashMap<String, Vec<String>>> {
    blocking(move || {
        let db = db::load();
        let xa = xattrs(paths.clone());
        Ok(paths
            .into_iter()
            .filter_map(|p| {
                let t = merged(&db, &xa, &p);
                (!t.is_empty()).then_some((p, t))
            })
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn tags_set(paths: Vec<String>, tags: Vec<String>) -> CmdResult<()> {
    blocking(move || {
        let tags = normalize(tags);
        db::update(|db| {
            for p in &paths {
                if tags.is_empty() {
                    db.remove(p);
                } else {
                    db.insert(p.clone(), tags.clone());
                }
            }
            true
        })?;
        // Outside the db's lock. xattr is best-effort: read-only or foreign filesystems have the db.
        let local: Vec<String> = paths.into_iter().filter(|p| netfs::network_mount_of(Path::new(p)).is_none()).collect();
        batch("tags-xattr", local, move |p| {
            let _ = xattr::write(Path::new(p), &tags);
        });
        Ok(())
    })
    .await
}

/// Every tag in use with how many files carry it.
#[tauri::command]
pub async fn tags_all() -> CmdResult<HashMap<String, usize>> {
    blocking(|| {
        let db = db::load();
        let xa = xattrs(db.keys().cloned().collect());
        let mut out = HashMap::new();
        for t in db.keys().flat_map(|p| merged(&db, &xa, p)) {
            *out.entry(t).or_insert(0) += 1;
        }
        Ok(out)
    })
    .await
}

/// Files carrying `tag`; drops entries for files that no longer exist or lost the tag. After the
/// first stall, the rest are listed without asking: a share that's away costs one wait, not one per file.
#[tauri::command]
pub async fn tags_find(tag: String) -> CmdResult<Vec<Entry>> {
    blocking(move || {
        let db = db::load();
        let xa = xattrs(db.keys().cloned().collect());
        let paths: Vec<String> = db.keys().filter(|p| merged(&db, &xa, p).contains(&tag)).cloned().collect();
        let found = batch("tags-find", paths.clone(), |p| entry_from(Path::new(p)));
        let rest = paths[found.len()..].iter().filter_map(|p| entry_unresponsive(Path::new(p), false));
        let mut out: Vec<Entry> = found.into_iter().flatten().collect();
        out.extend(rest);
        Ok(out)
    })
    .await
}

fn normalize(tags: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for t in tags {
        let t = t.trim().replace(',', " ");
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}
