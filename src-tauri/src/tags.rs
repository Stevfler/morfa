//! Finder-style colour labels and free-form tags.
//!
//! Colours are just tags with well-known names ("Red", "Blue", ...), so one list
//! per file covers both. On Linux the list lives in the freedesktop xattr
//! `user.xdg.tags`; everywhere (and when xattrs are unsupported) it is mirrored in
//! the app's own JSON store, which also serves as the index for "all files tagged X".
use crate::entry::{entry_from, Entry};
use crate::CmdResult;
use std::collections::HashMap;
use std::path::Path;

mod db;
mod xattr;

/// Tags for each path; paths without tags are omitted.
#[tauri::command]
pub async fn tags_get(paths: Vec<String>) -> CmdResult<HashMap<String, Vec<String>>> {
    blocking(move || {
        let db = db::load();
        Ok(paths
            .into_iter()
            .filter_map(|p| {
                let t = read(&db, &p);
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
                // xattr is best-effort: read-only or foreign filesystems fall back to the db.
                let _ = xattr::write(Path::new(p), &tags);
                if tags.is_empty() {
                    db.remove(p);
                } else {
                    db.insert(p.clone(), tags.clone());
                }
            }
            true
        })
    })
    .await
}

/// Every tag in use with how many files carry it.
#[tauri::command]
pub async fn tags_all() -> CmdResult<HashMap<String, usize>> {
    blocking(|| {
        let db = db::load();
        let mut out = HashMap::new();
        for t in db.keys().flat_map(|p| read(&db, p)) {
            *out.entry(t).or_insert(0) += 1;
        }
        Ok(out)
    })
    .await
}

/// Files carrying `tag`; drops entries for files that no longer exist or lost the tag.
#[tauri::command]
pub async fn tags_find(tag: String) -> CmdResult<Vec<Entry>> {
    blocking(move || {
        let db = db::load();
        Ok(db
            .keys()
            .filter(|p| read(&db, p).contains(&tag))
            .filter_map(|p| entry_from(Path::new(p)))
            .collect())
    })
    .await
}

/// The xattr wins when present (other apps may have edited it); otherwise the db.
fn read(db: &db::Db, path: &str) -> Vec<String> {
    xattr::read(Path::new(path)).unwrap_or_else(|| db.get(path).cloned().unwrap_or_default())
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
