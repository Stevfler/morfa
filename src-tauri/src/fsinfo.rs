use crate::{
    entry::{entry_from, Entry},
    search::num_threads,
    CmdResult,
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
};

#[tauri::command]
pub async fn list_dir(path: String) -> CmdResult<Vec<Entry>> {
    tauri::async_runtime::spawn_blocking(move || {
        let rd = std::fs::read_dir(&path).map_err(|e| format!("{path}: {e}"))?;
        Ok(rd.filter_map(|e| e.ok()).filter_map(|e| entry_from(&e.path())).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn stat(path: String) -> CmdResult<Entry> {
    tauri::async_runtime::spawn_blocking(move || entry_from(Path::new(&path)).ok_or_else(|| format!("{path}: not found")))
        .await
        .map_err(|e| e.to_string())?
}

/// Latest cancelled size-request generation: walks started with `gen <= SIZE_CANCEL` stop early.
static SIZE_CANCEL: AtomicU64 = AtomicU64::new(0);

/// Stops every in-flight `dir_size` started with a generation `<= gen` (navigation / re-request).
#[tauri::command]
pub fn cancel_sizes(gen: u64) {
    SIZE_CANCEL.fetch_max(gen, Ordering::Relaxed);
}

/// Real recursive folder size, computed in parallel. Hard links are counted once.
#[tauri::command]
pub async fn dir_size(path: String, gen: Option<u64>) -> CmdResult<(u64, u64)> {
    let gen = gen.unwrap_or(u64::MAX);
    tauri::async_runtime::spawn_blocking(move || walk_size(Path::new(&path), &|| SIZE_CANCEL.load(Ordering::Relaxed) >= gen).ok_or_else(|| "cancelled".to_string()))
        .await
        .map_err(|e| e.to_string())?
}

/// `None` if `cancelled` became true during the walk.
fn walk_size(path: &Path, cancelled: &(dyn Fn() -> bool + Sync)) -> Option<(u64, u64)> {
    let bytes = AtomicU64::new(0);
    let files = AtomicU64::new(0);
    let stop = AtomicBool::new(false);
    let seen = Mutex::new(HashSet::<(u64, u64)>::new());
    ignore::WalkBuilder::new(path).standard_filters(false).follow_links(false).threads(num_threads()).build_parallel().run(|| {
        let (b, f, stop, seen) = (&bytes, &files, &stop, &seen);
        Box::new(move |r| {
            if cancelled() {
                stop.store(true, Ordering::Relaxed);
                return ignore::WalkState::Quit;
            }
            if let Some(m) = r.ok().and_then(|e| e.metadata().ok()) {
                if m.is_file() && first_link(&m, seen) {
                    b.fetch_add(m.len(), Ordering::Relaxed);
                    f.fetch_add(1, Ordering::Relaxed);
                }
            }
            ignore::WalkState::Continue
        })
    });
    (!stop.load(Ordering::Relaxed)).then(|| (bytes.load(Ordering::Relaxed), files.load(Ordering::Relaxed)))
}

/// False for the 2nd+ sighting of a hard-linked file (same dev+inode).
#[cfg(unix)]
fn first_link(m: &std::fs::Metadata, seen: &Mutex<HashSet<(u64, u64)>>) -> bool {
    use std::os::unix::fs::MetadataExt;
    m.nlink() <= 1 || seen.lock().map(|mut s| s.insert((m.dev(), m.ino()))).unwrap_or(true)
}

#[cfg(not(unix))]
fn first_link(_m: &std::fs::Metadata, _seen: &Mutex<HashSet<(u64, u64)>>) -> bool {
    true
}

#[tauri::command]
pub async fn read_text(path: String, max: usize) -> CmdResult<Option<String>> {
    const MAX_READ: usize = 1 << 20; // 1 MiB cap regardless of what the frontend asks for
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Read;
        let max = max.min(MAX_READ);
        let f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let mut buf = Vec::with_capacity(max);
        f.take(max as u64).read_to_end(&mut buf).map_err(|e| e.to_string())?;
        if buf.iter().take(4096).any(|&b| b == 0) {
            return Ok(None); // binary
        }
        // A cut at the byte limit may split a trailing multi-byte char; drop it.
        if let Err(e) = std::str::from_utf8(&buf) {
            if e.error_len().is_none() {
                buf.truncate(e.valid_up_to());
            }
        }
        Ok(Some(String::from_utf8_lossy(&buf).to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Folder completion for the editable path bar.
#[tauri::command]
pub async fn complete_path(input: String) -> CmdResult<Vec<String>> {
    let p = PathBuf::from(&input);
    let (dir, prefix) = if input.ends_with(std::path::MAIN_SEPARATOR) {
        (p.clone(), String::new())
    } else {
        (p.parent().map(Path::to_path_buf).unwrap_or_default(), p.file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default())
    };
    let mut out: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            n.starts_with(&prefix) && (prefix.starts_with('.') || !n.starts_with('.'))
        })
        .map(|e| e.path().to_string_lossy().to_string())
        .collect();
    out.sort_by_key(|s| s.to_lowercase());
    out.truncate(12);
    Ok(out)
}

#[tauri::command]
pub fn home_dir() -> String {
    dirs::home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "/".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_dedupes_hard_links_and_cancels() {
        let d = std::env::temp_dir().join(format!("orbit-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a"), [0u8; 100]).unwrap();
        std::fs::write(d.join("sub/b"), [0u8; 10]).unwrap();
        #[cfg(unix)]
        std::fs::hard_link(d.join("a"), d.join("sub/a2")).unwrap();
        assert_eq!(walk_size(&d, &|| false), Some((110, 2)));
        assert_eq!(walk_size(&d, &|| true), None);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
