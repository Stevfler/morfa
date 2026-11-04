use crate::{
    entry::{entry_from, entry_unresponsive, Entry},
    guard, netfs,
    search::num_threads,
    CmdResult,
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::Duration,
};

#[tauri::command]
pub async fn list_dir(path: String) -> CmdResult<Vec<Entry>> {
    let p = path.clone();
    let (mut out, slow) = guard::guarded(&path, guard::stall_for(&path), move |beat| list_plain(&p, beat)).await?;
    if !slow.is_empty() {
        let d = path.clone();
        let more = tauri::async_runtime::spawn_blocking(move || resolve_slow(&d, slow)).await.map_err(|e| e.to_string())?;
        out.extend(more);
    }
    Ok(out)
}

/// A symlink or mount point in a listing: stat'ing it reaches another filesystem, so a dead share
/// linked from the home folder would otherwise make the home folder itself unlistable.
struct Slow {
    path: PathBuf,
    is_symlink: bool,
}

fn list_plain(path: &str, beat: &guard::Beat) -> CmdResult<(Vec<Entry>, Vec<Slow>)> {
    let mounts: HashSet<PathBuf> = netfs::table().into_iter().filter(|m| netfs::is_network_fs(&m.fs)).map(|m| m.path.into()).collect();
    let (mut out, mut slow) = (vec![], vec![]);
    for e in std::fs::read_dir(path).map_err(|e| format!("{path}: {e}"))?.flatten() {
        beat.tick();
        let p = e.path();
        let is_symlink = e.file_type().is_ok_and(|t| t.is_symlink());
        if is_symlink || mounts.contains(&p) {
            slow.push(Slow { path: p, is_symlink });
        } else if let Some(en) = entry_from(&p) {
            out.push(en);
        }
    }
    Ok((out, slow))
}

/// Paths whose stat didn't return yet. Their stuck thread clears them when it finally does, so
/// each costs one abandoned thread however often the folder is listed.
static DEAD: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(Default::default);
const SLOW_STALL: Duration = Duration::from_millis(1500);
/// Per listing, so that a folder of many dead links opens after a bounded wait.
const SLOW_RETRIES: usize = 4;
/// `DEAD` already bounds this per link; the guard's total bound is what caps the whole.
const SLOW_LIMITS: guard::Limits = guard::Limits { abandoned: 16, running: 32, group: 16, ..guard::LIMITS };

/// Stats on one thread, moving past a stuck entry on a new one. The rest get a placeholder row.
fn resolve_slow(dir: &str, slow: Vec<Slow>) -> Vec<Entry> {
    let dead = |p: &Path| DEAD.lock().unwrap_or_else(|e| e.into_inner()).contains(p);
    let (mut live, stuck): (Vec<Slow>, Vec<Slow>) = slow.into_iter().partition(|s| !dead(&s.path));
    let mut out: Vec<Entry> = stuck.iter().filter_map(|s| entry_unresponsive(&s.path, s.is_symlink)).collect();
    let key = format!("links:{dir}");
    for _ in 0..=SLOW_RETRIES {
        if live.is_empty() {
            break;
        }
        let paths: Arc<Vec<PathBuf>> = Arc::new(live.iter().map(|s| s.path.clone()).collect());
        let results = Arc::new(Mutex::new(Vec::with_capacity(paths.len())));
        let (p2, r2) = (paths.clone(), results.clone());
        let started = Arc::new(AtomicBool::new(false));
        let s2 = started.clone();
        let run = guard::run_keyed(&key, dir, SLOW_STALL, SLOW_LIMITS, move |beat| {
            s2.store(true, Ordering::SeqCst);
            for p in p2.iter() {
                let en = entry_from(p);
                let mut set = DEAD.lock().unwrap_or_else(|e| e.into_inner());
                set.remove(p);
                r2.lock().unwrap_or_else(|e| e.into_inner()).push(en);
                drop(set);
                beat.tick();
            }
            Ok(())
        });
        // Under DEAD's lock, so a stat finishing right now is either counted here or clears its mark.
        let set = DEAD.lock();
        let done = std::mem::take(&mut *results.lock().unwrap_or_else(|e| e.into_inner()));
        let n = done.len();
        out.extend(done.into_iter().flatten());
        live.drain(..n);
        if run.is_ok() {
            return out;
        }
        // Refused, not stuck: nothing here is known dead, and no thread would ever clear a mark.
        if !started.load(Ordering::SeqCst) {
            break;
        }
        if let Some(first) = live.first() {
            let mut set = set.unwrap_or_else(|e| e.into_inner());
            set.insert(first.path.clone());
            drop(set);
            out.extend(entry_unresponsive(&first.path, first.is_symlink));
            live.remove(0);
        }
    }
    out.extend(live.iter().filter_map(|s| entry_unresponsive(&s.path, s.is_symlink)));
    out
}

/// Is this folder on a network filesystem? A filesystem that doesn't answer counts as one.
#[tauri::command]
pub async fn is_remote(path: String) -> CmdResult<bool> {
    if netfs::network_mount_of(Path::new(&path)).is_some() {
        return Ok(true);
    }
    let p = path.clone();
    Ok(guard::guarded(&path, guard::quick_for(&path), move |_| Ok(netfs::is_network_at(Path::new(&p)))).await.unwrap_or(true))
}

#[tauri::command]
pub async fn stat(path: String) -> CmdResult<Entry> {
    let p = path.clone();
    guard::guarded(&path, guard::quick_for(&path), move |_| entry_from(Path::new(&p)).ok_or_else(|| format!("{p}: not found"))).await
}

const SIZE_STALL: Duration = Duration::from_secs(30);

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
    let p = path.clone();
    let cancelled = move || SIZE_CANCEL.load(Ordering::Relaxed) >= gen;
    // Cancelling answers at once even if the walk is stuck in a dead mount and can't see the flag.
    guard::guarded_cancel(&path, SIZE_STALL, cancelled, move |beat| {
        walk_size(Path::new(&p), &|| {
            beat.tick();
            cancelled()
        })
        .ok_or_else(|| "cancelled".to_string())
    })
    .await
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
    let p = path.clone();
    guard::guarded(&path, guard::stall_for(&path), move |beat| {
        let max = max.min(MAX_READ);
        // Opening a FIFO waits for a writer forever, and a device may never end.
        if !std::fs::metadata(&p).map_err(|e| e.to_string())?.is_file() {
            return Ok(None);
        }
        let mut buf = guard_read(&p, max as u64, beat)?;
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
}

/// Reads up to `max` bytes, beating per chunk so a slow share isn't mistaken for a dead one.
pub fn guard_read(path: &str, max: u64, beat: &guard::Beat) -> CmdResult<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?.take(max);
    let mut buf = vec![];
    let mut chunk = vec![0u8; 256 << 10];
    loop {
        beat.tick();
        match f.read(&mut chunk).map_err(|e| e.to_string())? {
            0 => return Ok(buf),
            n => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

/// Folder completion for the editable path bar. Empty input completes from home.
#[tauri::command]
pub async fn complete_path(input: String) -> CmdResult<Vec<String>> {
    let input = if input.is_empty() { home_dir()? + std::path::MAIN_SEPARATOR_STR } else { input };
    let Some((dir, prefix)) = split_completion(&input, cfg!(windows)) else { return Ok(Vec::new()) };
    let d = dir.clone();
    guard::guarded(&dir, guard::quick_for(&dir), move |beat| {
        let mut out: Vec<String> = std::fs::read_dir(&d)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .filter(|e| {
                beat.tick();
                let n = e.file_name().to_string_lossy().to_lowercase();
                n.starts_with(&prefix) && (prefix.starts_with('.') || !n.starts_with('.'))
            })
            // The type comes with the listing; only a symlink costs a stat, and only a matching one.
            .filter(|e| e.file_type().map(|t| if t.is_symlink() { e.path().is_dir() } else { t.is_dir() }).unwrap_or(false))
            .map(|e| e.path().to_string_lossy().to_string())
            .collect();
        out.sort_by_key(|s| s.to_lowercase());
        out.truncate(12);
        Ok(out)
    })
    .await
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
        let d = std::env::temp_dir().join(format!("morfa-size-{}", std::process::id()));
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

    #[cfg(unix)]
    #[test]
    fn slow_entries_get_resolved_or_a_placeholder() {
        use std::os::unix::fs::symlink;
        let d = std::env::temp_dir().join(format!("morfa-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("real")).unwrap();
        std::fs::write(d.join("f"), b"x").unwrap();
        symlink(d.join("real"), d.join("to-dir")).unwrap();
        symlink(d.join("f"), d.join("to-file")).unwrap();
        let ds = d.to_string_lossy().to_string();
        let beat = guard::run_blocking(&ds, guard::QUICK, {
            let ds = ds.clone();
            move |b| list_plain(&ds, b).map(|(o, s)| (o.len(), s.len(), resolve_slow(&ds, s)))
        })
        .unwrap();
        assert_eq!((beat.0, beat.1), (2, 2), "plain entries vs links");
        let json = serde_json::to_value(&beat.2).unwrap();
        let by_name = |n: &str| json.as_array().unwrap().iter().find(|e| e["name"] == n).unwrap().clone();
        assert_eq!(by_name("to-dir")["isDir"], true);
        assert_eq!(by_name("to-file")["isDir"], false);
        assert_eq!(by_name("to-file")["isSymlink"], true);
        // Marked dead: not stat'ed, and shown as a folder without blocking the listing.
        DEAD.lock().unwrap().insert(d.join("to-file"));
        let r = resolve_slow(&ds, vec![Slow { path: d.join("to-file"), is_symlink: true }]);
        assert_eq!(serde_json::to_value(&r).unwrap()[0]["isDir"], true);
        DEAD.lock().unwrap().remove(&d.join("to-file"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn read_text_skips_fifos_without_blocking() {
        use std::os::unix::ffi::OsStrExt;
        let f = std::env::temp_dir().join(format!("morfa-read-fifo-{}", std::process::id()));
        let _ = std::fs::remove_file(&f);
        let c = std::ffi::CString::new(f.as_os_str().as_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let t = std::time::Instant::now();
        let r = tauri::async_runtime::block_on(read_text(f.to_string_lossy().into(), 100));
        assert_eq!(r, Ok(None));
        assert!(t.elapsed() < Duration::from_secs(2));
        std::fs::remove_file(&f).unwrap();
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
