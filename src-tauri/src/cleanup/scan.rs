//! One parallel, same-filesystem walk that builds the size index and collects category hits.
use super::index::{FileHit, Index};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Mutex,
    },
};

/// Pseudo / runtime filesystems never worth sizing (also usually other mounts).
const SKIP: &[&str] = &["/proc", "/sys", "/dev", "/run"];
/// Files at least this big are kept individually in the index (smaller ones are aggregated).
pub const KEEP_FILE: u64 = 4 << 20;

pub struct Progress {
    pub files: AtomicU64,
    pub bytes: AtomicU64,
}

/// Per-thread partial results, merged after the walk.
#[derive(Default)]
struct Part {
    own: HashMap<PathBuf, (u64, u64)>,
    dirs: Vec<PathBuf>,
    files: Vec<FileHit>,
}

struct Sink<'a>(Part, &'a mpsc::Sender<Part>);
impl Drop for Sink<'_> {
    fn drop(&mut self) {
        let _ = self.1.send(std::mem::take(&mut self.0));
    }
}

/// On-disk (allocated) size; hard links after the first sighting count as 0.
#[cfg(unix)]
fn disk_size(m: &std::fs::Metadata, seen: &Mutex<HashSet<(u64, u64)>>) -> u64 {
    use std::os::unix::fs::MetadataExt;
    if m.nlink() > 1 && !seen.lock().map(|mut s| s.insert((m.dev(), m.ino()))).unwrap_or(true) {
        return 0;
    }
    m.blocks() * 512
}
#[cfg(not(unix))]
fn disk_size(m: &std::fs::Metadata, _seen: &Mutex<HashSet<(u64, u64)>>) -> u64 {
    m.len()
}

/// `None` when cancelled.
pub fn scan(root: &Path, prog: &Progress, cancelled: &(dyn Fn() -> bool + Sync)) -> Option<Index> {
    let (tx, rx) = mpsc::channel::<Part>();
    let stop = AtomicBool::new(false);
    let seen = Mutex::new(HashSet::new());
    let skip_root = root.to_path_buf();
    ignore::WalkBuilder::new(root)
        .standard_filters(false)
        .follow_links(false)
        .same_file_system(true)
        .threads(crate::search::num_threads())
        .filter_entry(move |e| e.path() == skip_root || !SKIP.iter().any(|s| e.path() == Path::new(s)))
        .build_parallel()
        .run(|| {
            let mut sink = Sink(Part::default(), &tx);
            let (stop, seen) = (&stop, &seen);
            Box::new(move |r| {
                if cancelled() {
                    stop.store(true, Ordering::Relaxed);
                    return ignore::WalkState::Quit;
                }
                let Ok(e) = r else { return ignore::WalkState::Continue };
                let Some(ft) = e.file_type() else { return ignore::WalkState::Continue };
                if ft.is_dir() {
                    sink.0.dirs.push(e.into_path());
                    return ignore::WalkState::Continue;
                }
                let Ok(m) = e.metadata() else { return ignore::WalkState::Continue };
                let size = disk_size(&m, seen);
                prog.files.fetch_add(1, Ordering::Relaxed);
                prog.bytes.fetch_add(size, Ordering::Relaxed);
                let parent = e.path().parent().unwrap_or(root).to_path_buf();
                let o = sink.0.own.entry(parent).or_default();
                o.0 += size;
                o.1 += 1;
                if size >= KEEP_FILE || is_log(e.path()) {
                    let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
                    sink.0.files.push(FileHit { path: e.into_path(), size, mtime });
                }
                ignore::WalkState::Continue
            })
        });
    drop(tx);
    if stop.load(Ordering::Relaxed) {
        return None;
    }
    let mut all = Part::default();
    for p in rx {
        for (k, (b, f)) in p.own {
            let o = all.own.entry(k).or_default();
            o.0 += b;
            o.1 += f;
        }
        all.dirs.extend(p.dirs);
        all.files.extend(p.files);
    }
    Some(Index::build(root.to_path_buf(), all.dirs, all.own, all.files))
}

/// `*.log`, rotated `*.log.1`, `*.log.2.gz`, `*.old`.
pub fn is_log(p: &Path) -> bool {
    let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    n.ends_with(".log") || n.contains(".log.") || (n.ends_with(".old") && n.len() > 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_roll_up() {
        let d = std::env::temp_dir().join(format!("morfa-cleanup-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        std::fs::write(d.join("a/b/x.bin"), vec![1u8; 100_000]).unwrap();
        std::fs::write(d.join("a/y.log"), b"hello").unwrap();
        std::fs::write(d.join("z"), vec![2u8; 10_000]).unwrap();
        let prog = Progress { files: AtomicU64::new(0), bytes: AtomicU64::new(0) };
        let ix = scan(&d, &prog, &|| false).unwrap();
        assert_eq!(prog.files.load(Ordering::Relaxed), 3);
        let a = ix.total(&d.join("a"));
        assert!(a >= ix.total(&d.join("a/b")) && ix.total(&d.join("a/b")) >= 100_000);
        assert_eq!(ix.total(&d), prog.bytes.load(Ordering::Relaxed));
        assert!(ix.files.iter().any(|f| f.path.ends_with("y.log")));
        let kids = ix.children(&d, 10);
        assert_eq!(kids[0].name, "a");
        assert!(scan(&d, &prog, &|| true).is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }
}

/// Read-only smoke run against $HOME: `cargo test home_smoke -- --ignored --nocapture`.
#[cfg(test)]
#[test]
#[ignore]
fn home_smoke() {
    let h = dirs::home_dir().unwrap();
    let prog = Progress { files: AtomicU64::new(0), bytes: AtomicU64::new(0) };
    let t = std::time::Instant::now();
    let ix = scan(&h, &prog, &|| false).unwrap();
    println!("{} files, {} MB, {:?}", prog.files.load(Ordering::Relaxed), prog.bytes.load(Ordering::Relaxed) >> 20, t.elapsed());
    for c in super::categories::categorize(&ix, 1_800_000_000) {
        println!("{:12} {:>8} MB {:>4} items {:?}", c.id, c.total >> 20, c.items.len(), c.items.first().map(|i| (&i.path, i.protected)));
    }
}
