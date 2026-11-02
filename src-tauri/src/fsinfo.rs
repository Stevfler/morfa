use crate::{
    entry::{entry_from, Entry},
    search::num_threads,
    CmdResult,
};
use std::{
    collections::HashSet,
    path::Path,
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

/// Folder completion for the editable path bar. Empty input completes from home.
#[tauri::command]
pub async fn complete_path(input: String) -> CmdResult<Vec<String>> {
    let input = if input.is_empty() { home_dir()? + std::path::MAIN_SEPARATOR_STR } else { input };
    let Some((dir, prefix)) = split_completion(&input, cfg!(windows)) else { return Ok(Vec::new()) };
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

/// Splits typed text into (directory to list, lowercase name prefix). `None` = nothing listable
/// (relative text, or a bare `\\server\` UNC root). On Windows `/` counts as a separator and
/// `X:` means `X:\` (not that drive's cwd).
fn split_completion(input: &str, windows: bool) -> Option<(String, String)> {
    let mut s = if windows { input.replace('/', "\\") } else { input.to_string() };
    let sep = if windows { '\\' } else { '/' };
    if windows && s.len() == 2 && s.as_bytes()[0].is_ascii_alphabetic() && s.as_bytes()[1] == b':' {
        s.push('\\');
    }
    let idx = s.rfind(sep)?;
    let (dir, prefix) = (&s[..=idx], &s[idx + 1..]);
    if windows && dir.starts_with("\\\\") && dir[2..].matches('\\').count() < 2 {
        return None; // `\\server\` lists shares, which read_dir can't enumerate
    }
    Some((dir.to_string(), prefix.to_lowercase()))
}

/// The user's home directory, or an error if the OS doesn't report one.
#[tauri::command]
pub fn home_dir() -> CmdResult<String> {
    dirs::home_dir().map(|p| p.to_string_lossy().to_string()).ok_or_else(|| "no home directory".into())
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

    fn sc(i: &str, w: bool) -> Option<(String, String)> {
        split_completion(i, w)
    }
    fn some(d: &str, p: &str) -> Option<(String, String)> {
        Some((d.into(), p.into()))
    }

    #[test]
    fn completion_split_unix() {
        assert_eq!(sc("/home/u/Do", false), some("/home/u/", "do"));
        assert_eq!(sc("/home/", false), some("/home/", ""));
        assert_eq!(sc("/", false), some("/", ""));
        assert_eq!(sc("rel", false), None);
        assert_eq!(sc("C:\\x", false), None);
    }

    #[test]
    fn completion_split_windows() {
        assert_eq!(sc("C:", true), some("C:\\", ""));
        assert_eq!(sc("c:/Users/Bo", true), some("c:\\Users\\", "bo"));
        assert_eq!(sc("C:\\Users\\", true), some("C:\\Users\\", ""));
        assert_eq!(sc("C:/", true), some("C:\\", ""));
        assert_eq!(sc("\\\\srv\\share\\Pro", true), some("\\\\srv\\share\\", "pro"));
        assert_eq!(sc("//srv/share/", true), some("\\\\srv\\share\\", ""));
        assert_eq!(sc("\\\\srv\\sha", true), None);
        assert_eq!(sc("Users", true), None);
    }
}
