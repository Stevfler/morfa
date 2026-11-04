//! Browse archives like folders, extract them and create new ones.
//! Reading: zip, 7z, tar, tar.gz, tar.bz2, tar.xz, tar.zst. Writing: zip, tar.gz.
//!
//! Extraction is all-or-nothing: entries go to a hidden staging folder inside the destination and
//! are moved out only when everything succeeded. Entry names are untrusted, so `clean_rel` rejects
//! anything that could leave the destination, and links and special files are never created.
mod pack;
mod read;
#[cfg(test)]
mod tests;

use crate::fsops::rename::rename_noreplace;
use crate::CmdResult;
use read::{walk, Item, Visitor};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use tauri::{AppHandle, Emitter, Manager, State};

pub(crate) const CANCELLED: &str = "Cancelled";

#[derive(Default)]
pub struct ArchiveState {
    index: Mutex<HashMap<String, (Stamp, Arc<Index>)>>,
    cancelled: Arc<Mutex<HashSet<u64>>>,
}

type Stamp = (u64, Option<SystemTime>);

/// Contents of one archive, keyed by folder ("" is the root). Implicit parent folders are included.
#[derive(Default)]
pub(crate) struct Index {
    dirs: HashMap<String, Vec<Item>>,
}

impl Visitor for Index {
    fn want(&mut self, item: &Item) -> bool {
        self.add(item);
        false
    }
    fn take(&mut self, _: &Item, _: &mut dyn Read) -> CmdResult<()> {
        Ok(())
    }
}

impl Index {
    fn add(&mut self, item: &Item) {
        let mut rel = item.rel.as_str();
        // Register every missing ancestor first.
        while let Some((dir, _)) = rel.rsplit_once('/') {
            let (up, name) = dir.rsplit_once('/').unwrap_or(("", dir));
            let known = self.dirs.get(up).is_some_and(|v| v.iter().any(|i| i.is_dir && i.name() == name));
            if !known {
                self.dirs.entry(up.to_string()).or_default().push(Item::folder(dir.to_string()));
            }
            rel = dir;
        }
        let dir = item.rel.rsplit_once('/').map_or("", |(d, _)| d).to_string();
        let list = self.dirs.entry(dir).or_default();
        // Listed twice (implicit first, or a repeated entry): the later one wins.
        list.retain(|i| i.rel != item.rel);
        list.push(item.clone());
    }

    fn selected_size(&self, rels: &[String]) -> u64 {
        self.dirs.values().flatten().filter(|i| !i.is_dir && in_selection(&i.rel, rels)).map(|i| i.size).sum()
    }
}

/// `rels` empty means everything.
fn in_selection(rel: &str, rels: &[String]) -> bool {
    rels.is_empty() || rels.iter().any(|s| rel == s || (rel.starts_with(s.as_str()) && rel.as_bytes().get(s.len()) == Some(&b'/')))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveEntry {
    name: String,
    /// Path inside the archive; the frontend turns it into a pseudo-path.
    path: String,
    is_dir: bool,
    is_symlink: bool,
    size: u64,
    modified: u64,
    ext: String,
    hidden: bool,
}

fn stamp(path: &Path) -> CmdResult<Stamp> {
    let m = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((m.len(), m.modified().ok()))
}

fn index_of(state: &Mutex<HashMap<String, (Stamp, Arc<Index>)>>, archive: &str) -> CmdResult<Arc<Index>> {
    let path = Path::new(archive);
    let now = stamp(path)?;
    if let Some((s, idx)) = state.lock().unwrap_or_else(|e| e.into_inner()).get(archive) {
        if *s == now {
            return Ok(idx.clone());
        }
    }
    let mut idx = Index::default();
    walk(path, &mut idx)?;
    idx.dirs.entry(String::new()).or_default();
    let idx = Arc::new(idx);
    let mut cache = state.lock().unwrap_or_else(|e| e.into_inner());
    // Listing a compressed tar means decompressing all of it, so keep a few recent ones.
    if cache.len() >= 8 {
        cache.clear();
    }
    cache.insert(archive.to_string(), (now, idx.clone()));
    Ok(idx)
}

#[tauri::command]
pub async fn archive_list(app: AppHandle, archive: String, rel: String) -> CmdResult<Vec<ArchiveEntry>> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ArchiveState>();
        let idx = index_of(&state.index, &archive)?;
        let items = idx.dirs.get(&rel).ok_or_else(|| format!("\"{rel}\" isn't in this archive"))?;
        Ok(items
            .iter()
            .map(|i| {
                let name = i.name().to_string();
                ArchiveEntry {
                    ext: if i.is_dir { String::new() } else { Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default() },
                    hidden: name.starts_with('.'),
                    name,
                    path: i.rel.clone(),
                    is_dir: i.is_dir,
                    is_symlink: i.is_link,
                    size: i.size,
                    modified: i.modified,
                }
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Progress and cancellation of one running job.
pub(crate) struct Job {
    app: Option<AppHandle>, // None in tests
    cancelled: Arc<Mutex<HashSet<u64>>>,
    id: u64,
    pub(crate) done: u64,
    pub(crate) total: u64,
    last: Instant,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    id: u64,
    done: u64,
    total: u64,
    current: String,
    finished: bool,
    part: Option<f64>,
}

impl Job {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.lock().unwrap_or_else(|e| e.into_inner()).contains(&self.id)
    }
    pub(crate) fn tick(&mut self, current: &str) {
        if self.last.elapsed().as_millis() > 100 {
            self.last = Instant::now();
            if let Some(app) = &self.app {
                let _ = app.emit("op-progress", Progress { id: self.id, done: self.done, total: self.total, current: current.to_string(), finished: false, part: None });
            }
        }
    }
    /// Copy with progress; the cancellation error is recognised by `finish_err`.
    pub(crate) fn copy(&mut self, from: &mut dyn Read, to: &mut dyn Write, current: &str) -> io::Result<()> {
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            if self.is_cancelled() {
                return Err(io::Error::other(CANCELLED));
            }
            let n = match from.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            to.write_all(&buf[..n])?;
            self.done += n as u64;
            self.tick(current);
        }
    }
}

/// Runs `f` as a cancellable job, always clearing its cancel flag afterwards.
fn with_job<T>(app: Option<AppHandle>, cancelled: Arc<Mutex<HashSet<u64>>>, id: u64, f: impl FnOnce(&mut Job) -> CmdResult<T>) -> CmdResult<T> {
    let mut job = Job { app, cancelled: cancelled.clone(), id, done: 0, total: 0, last: Instant::now() };
    let r = f(&mut job);
    let was_cancelled = job.is_cancelled();
    cancelled.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    // A cancel that arrives after the job finished must not hide its result.
    r.map_err(|e| if was_cancelled { CANCELLED.to_string() } else { e })
}

/// Removes the staging path on every exit unless disarmed.
struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        if self.0.is_dir() {
            let _ = fs::remove_dir_all(&self.0);
        } else {
            let _ = fs::remove_file(&self.0);
        }
    }
}

struct Extract<'a> {
    rels: &'a [String],
    /// Folder inside the archive that maps onto the staging root.
    base: &'a str,
    staging: &'a Path,
    job: &'a mut Job,
}

impl Extract<'_> {
    fn out(&self, rel: &str) -> Option<PathBuf> {
        let tail = if self.base.is_empty() { rel } else { rel.strip_prefix(self.base)?.strip_prefix('/')? };
        (!tail.is_empty()).then(|| self.staging.join(tail))
    }
}

impl Visitor for Extract<'_> {
    fn want(&mut self, item: &Item) -> bool {
        // Links could point anywhere, so they are skipped rather than recreated.
        !item.is_link && in_selection(&item.rel, self.rels) && self.out(&item.rel).is_some()
    }
    fn take(&mut self, item: &Item, data: &mut dyn Read) -> CmdResult<()> {
        let Some(out) = self.out(&item.rel) else { return Ok(()) };
        let err = |e: io::Error| format!("{}: {e}", item.rel);
        if item.is_dir {
            return fs::create_dir_all(&out).map_err(err);
        }
        if let Some(dir) = out.parent() {
            fs::create_dir_all(dir).map_err(err)?;
        }
        let mut file = fs::File::create(&out).map_err(err)?;
        self.job.copy(data, &mut file, &item.rel).map_err(err)?;
        if item.modified > 0 {
            let _ = file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_millis(item.modified));
        }
        // Keep only rwx bits: setuid and friends from an archive must not survive.
        #[cfg(unix)]
        if let Some(mode) = item.mode {
            use std::os::unix::fs::PermissionsExt;
            let _ = file.set_permissions(fs::Permissions::from_mode((mode & 0o777) | 0o600));
        }
        Ok(())
    }
}

/// "report.tar.gz" → ("report", ".tar.gz"); "a.txt" → ("a", ".txt"); ".env" → (".env", "").
pub(crate) fn split_name(name: &str) -> (&str, &str) {
    if let Some(i) = name.to_ascii_lowercase().rfind(".tar.") {
        if i > 0 {
            return name.split_at(i);
        }
    }
    match name.rfind('.') {
        Some(i) if i > 0 => name.split_at(i),
        _ => (name, ""),
    }
}

/// Move `from` to `dir/name`, picking "name (2)" etc. Never overwrites.
pub(crate) fn place(from: &Path, dir: &Path, name: &str) -> CmdResult<PathBuf> {
    let (stem, ext) = split_name(name);
    for n in 1..10_000 {
        let target = dir.join(if n == 1 { name.to_string() } else { format!("{stem} ({n}){ext}") });
        match rename_noreplace(from, &target) {
            Ok(()) => return Ok(target),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", target.display())),
        }
    }
    Err(format!("No free name for \"{name}\""))
}

/// Unpack `rels` (empty: everything) relative to folder `base` into `dest`.
/// With `wrap`, several top-level items are kept together in a folder named after the archive.
pub(crate) fn extract(job: &mut Job, idx: &Index, archive: &Path, rels: &[String], base: &str, dest: &Path, wrap: bool) -> CmdResult<Vec<String>> {
    job.total = idx.selected_size(rels);
    let staging = dest.join(format!(".morfa-extract-{}-{}", std::process::id(), job.id));
    fs::create_dir(&staging).map_err(|e| format!("{}: {e}", dest.display()))?;
    let guard = Staging(staging.clone());
    walk(archive, &mut Extract { rels, base, staging: &staging, job })?;
    if job.is_cancelled() {
        return Err(CANCELLED.into());
    }
    let items: Vec<PathBuf> = fs::read_dir(&staging).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
    if wrap && items.len() > 1 {
        let name = archive.file_name().unwrap_or_default().to_string_lossy().to_string();
        let target = place(&staging, dest, split_name(&name).0)?;
        return Ok(vec![target.to_string_lossy().into()]);
    }
    let mut out = vec![];
    for item in &items {
        let name = item.file_name().unwrap_or_default().to_string_lossy().to_string();
        match place(item, dest, &name) {
            Ok(t) => out.push(t.to_string_lossy().to_string()),
            Err(e) => {
                // Don't leave half of it behind: put back what was already moved.
                for (done, orig) in out.iter().zip(&items) {
                    let _ = fs::rename(done, orig);
                }
                return Err(e);
            }
        }
    }
    drop(guard);
    Ok(out)
}

#[tauri::command]
pub async fn archive_extract(app: AppHandle, id: u64, archive: String, rels: Vec<String>, base: String, dest: String, wrap: bool) -> CmdResult<Vec<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ArchiveState>();
        let idx = index_of(&state.index, &archive)?;
        with_job(Some(app.clone()), state.cancelled.clone(), id, |job| extract(job, &idx, Path::new(&archive), &rels, &base, Path::new(&dest), wrap))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn archive_pack(app: AppHandle, id: u64, sources: Vec<String>, dest: String, format: String) -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ArchiveState>();
        with_job(Some(app.clone()), state.cancelled.clone(), id, |job| pack::pack(job, &sources, Path::new(&dest), &format))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn archive_cancel(state: State<'_, ArchiveState>, id: u64) {
    state.cancelled.lock().unwrap_or_else(|e| e.into_inner()).insert(id);
}

/// Unpack one file to a temp folder so the system can open it; returns its path.
#[tauri::command]
pub async fn archive_open(app: AppHandle, archive: String, rel: String) -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ArchiveState>();
        let idx = index_of(&state.index, &archive)?;
        let root = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("morfa").join("archive-open");
        sweep(&root);
        let s = stamp(Path::new(&archive))?;
        let key = Sha256::digest(format!("{archive}\0{rel}\0{}\0{:?}", s.0, s.1));
        let dir = root.join(key.iter().take(12).map(|b| format!("{b:02x}")).collect::<String>());
        let name = rel.rsplit('/').next().unwrap_or(&rel);
        let file = dir.join(name);
        if file.exists() {
            return Ok(file.to_string_lossy().to_string());
        }
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let base = rel.rsplit_once('/').map_or("", |(d, _)| d);
        // Id 0 is never given to a visible op, so nothing can cancel this.
        with_job(None, Default::default(), 0, |job| extract(job, &idx, Path::new(&archive), std::slice::from_ref(&rel), base, &dir, false))?;
        Ok(file.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Drop files opened from archives more than a day ago.
fn sweep(root: &Path) {
    for e in fs::read_dir(root).into_iter().flatten().flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a > Duration::from_secs(86_400));
        if old {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}
