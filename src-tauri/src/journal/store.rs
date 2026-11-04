//! The journal's data and its file: JSON in the config folder, replaced atomically, owner-only on
//! unix, bounded. Holds paths and sizes and times, never file contents. A file that can't be read is
//! set aside and the journal starts fresh.
use crate::{compare::mtime_ms, CmdResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_OPS: usize = 2000;
pub const MAX_PAIRS: usize = 500;
/// Across the whole file, so rewriting it stays cheap: a few megabytes at worst.
pub const MAX_TOTAL_PAIRS: usize = 50_000;
const VERSION: u32 = 1;
/// A journal can't honestly grow past this; a larger file is not ours to load.
const MAX_FILE: u64 = 128 << 20;
/// Unreadable files kept for a look.
const KEEP_ASIDE: usize = 3;
const MAX_LABEL: usize = 300;
const MAX_ERROR: usize = 500;
/// Items looked at per folder fingerprint. Past this a folder can't be vouched for.
pub const MAX_WALK: u32 = 20_000;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Move,
    Copy,
    Rename,
    BatchRename,
    Trash,
    Delete,
    Create,
    Extract,
    Compress,
    Convert,
    Compare,
    Dedupe,
    /// Only as the record of an undo.
    Restore,
}
impl Kind {
    pub fn moves(self) -> bool {
        matches!(self, Kind::Move | Kind::Rename | Kind::BatchRename)
    }
    pub fn outputs(self) -> bool {
        matches!(self, Kind::Copy | Kind::Create | Kind::Extract | Kind::Compress | Kind::Convert)
    }
    pub fn trashes(self) -> bool {
        matches!(self, Kind::Trash | Kind::Dedupe)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Done,
    Partial,
    Failed,
    Cancelled,
}

/// What stood at a path when it was recorded. A folder adds digests of every item below it, in name
/// order, so edits that keep the totals still show: `digest` of what looks like content, `meta` with
/// ctimes as well.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Fp {
    pub dir: bool,
    pub link: bool,
    pub size: u64,
    pub mtime: u64,
    pub count: u32,
    pub complete: bool,
    #[serde(default)]
    pub ino: u64,
    /// Nanoseconds; 0 where the OS has no such thing.
    #[serde(default)]
    pub ctime: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub digest: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub meta: String,
}
impl Fp {
    /// Everything but ctimes agrees: the contents look the same.
    pub fn same_content(&self, o: &Fp) -> bool {
        (self.dir, self.link, self.size, self.mtime, self.count, self.ino, &self.digest) == (o.dir, o.link, o.size, o.mtime, o.count, o.ino, &o.digest)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Pair {
    /// Empty when nothing came from anywhere (a new file).
    pub from: String,
    /// Empty when nothing is left at a known place (Trash, deleted).
    pub to: String,
    /// It really happened. A failed operation lists what it tried too.
    pub done: bool,
    /// Already undone. The other pairs of the entry can still be tried.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub undone: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fp: Option<Fp>,
    /// Real folder of `from`/`to` when it differs from the written one. Empty string: unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_real: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_real: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: u64,
    /// When it began: what the entries are ordered by, as it's the order the changes began in.
    #[serde(default)]
    pub started: u64,
    /// When it was recorded.
    pub time: u64,
    pub kind: Kind,
    pub label: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub pairs: Vec<Pair>,
    /// Pairs the operation had; `pairs` keeps at most `MAX_PAIRS`.
    pub total: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undone_by: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_of: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct File {
    version: u32,
    next_id: u64,
    entries: Vec<Entry>,
}

pub struct Store {
    path: PathBuf,
    pub entries: Vec<Entry>,
    next_id: u64,
    /// Where an unreadable file was put, until the journal is cleared.
    pub set_aside: Option<PathBuf>,
    /// The file couldn't be read right now: writing would destroy it. Opening again retries.
    pub blocked: bool,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

pub fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(unix)]
fn owner_only(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn owner_only(_p: &Path) {}

impl Store {
    pub fn open(path: PathBuf) -> Self {
        let mut s = Store { path, entries: vec![], next_id: 1, set_aside: None, blocked: false };
        if fs::metadata(&s.path).is_ok_and(|m| m.is_file() && m.len() > MAX_FILE) {
            s.put_aside();
            return s;
        }
        let bytes = match fs::read(&s.path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return s,
            // A busy disk or a permission slip says nothing about the file: it stays.
            Err(_) => {
                s.blocked = true;
                return s;
            }
        };
        match serde_json::from_slice::<File>(&bytes) {
            Ok(f) if f.version == VERSION => {
                owner_only(&s.path);
                s.next_id = f.entries.iter().map(|e| e.id + 1).max().unwrap_or(1).max(f.next_id);
                s.entries = f.entries;
                s.entries.sort_by_key(|e| (e.started, e.id));
                s.trim();
            }
            _ => s.put_aside(),
        }
        s
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn reopen(&self) -> Self {
        Store::open(self.path.clone())
    }

    /// Set-aside files, oldest first (names hold the time).
    fn asides(&self) -> Vec<PathBuf> {
        let stem = self.path.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
        let prefix = format!("{stem}.corrupt-");
        let Some(Ok(items)) = self.path.parent().map(fs::read_dir) else { return vec![] };
        let mut out: Vec<PathBuf> = items.flatten().filter(|i| i.file_name().to_string_lossy().starts_with(&prefix)).map(|i| i.path()).collect();
        out.sort();
        out
    }

    fn put_aside(&mut self) {
        let stem = self.path.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
        let at = now_ms();
        let to = (0..1000).map(|n| self.path.with_file_name(format!("{stem}.corrupt-{at}-{n:03}.json"))).find(|p| p.symlink_metadata().is_err());
        let Some(to) = to else {
            self.blocked = true;
            return;
        };
        if fs::rename(&self.path, &to).is_ok() {
            owner_only(&to);
            self.set_aside = Some(to);
            let all = self.asides();
            for old in &all[..all.len().saturating_sub(KEEP_ASIDE)] {
                let _ = fs::remove_file(old);
            }
        } else {
            self.blocked = true;
        }
    }

    /// Temp file in the same folder, flushed, renamed over: a crash leaves the old file or the new one.
    pub fn save(&self) -> CmdResult<()> {
        if self.blocked {
            return Err("journal file can't be read or moved aside".into());
        }
        let dir = self.path.parent().ok_or("no folder for the journal")?;
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let data = serde_json::to_vec(&File { version: VERSION, next_id: self.next_id, entries: self.entries.clone() }).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("tmp");
        let _ = fs::remove_file(&tmp); // left by a crash; create_new below never follows anything there
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let written = (|| {
            use std::io::Write;
            let mut f = opts.open(&tmp)?;
            f.write_all(&data)?;
            f.sync_all()?;
            drop(f);
            fs::rename(&tmp, &self.path)
        })();
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(e.to_string());
        }
        #[cfg(unix)]
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    }

    fn trim(&mut self) {
        let extra = self.entries.len().saturating_sub(MAX_OPS);
        self.entries.drain(..extra);
        let mut pairs: usize = self.entries.iter().map(|e| e.pairs.len()).sum();
        let mut drop = 0;
        // The newest always stays, however large.
        while pairs > MAX_TOTAL_PAIRS && drop + 1 < self.entries.len() {
            pairs -= self.entries[drop].pairs.len();
            drop += 1;
        }
        self.entries.drain(..drop);
    }

    /// Adds it in memory, in order of start, giving it its id and time. The caller saves.
    pub fn push(&mut self, mut e: Entry) -> u64 {
        e.id = self.next_id;
        self.next_id += 1;
        e.time = now_ms();
        e.started = if e.started == 0 { e.time } else { e.started.min(e.time) };
        e.label = clip(&e.label, MAX_LABEL);
        e.error = e.error.map(|x| clip(&x, MAX_ERROR));
        e.total = e.total.max(e.pairs.len() as u64);
        e.pairs.truncate(MAX_PAIRS);
        let id = e.id;
        let at = self.entries.partition_point(|x| (x.started, x.id) <= (e.started, e.id));
        self.entries.insert(at, e);
        self.trim();
        id
    }

    pub fn get(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Clearing when the file can't be read: it goes, whatever it holds. Ids start past any used so
    /// far, as undo items may still point at old ones.
    pub fn wipe(path: PathBuf) -> CmdResult<Store> {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        let mut s = Store::open(path);
        s.next_id = now_ms();
        s.clear()?;
        Ok(s)
    }

    pub fn clear(&mut self) -> CmdResult<()> {
        self.entries.clear();
        for old in self.asides() {
            let _ = fs::remove_file(old);
        }
        // The name earlier builds used.
        let _ = fs::remove_file(self.path.with_extension("corrupt.json"));
        self.set_aside = None;
        self.save()
    }
}

#[cfg(unix)]
fn identity(m: &fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (m.ino(), (m.ctime() as u64).wrapping_mul(1_000_000_000).wrapping_add(m.ctime_nsec() as u64))
}
#[cfg(not(unix))]
fn identity(_m: &fs::Metadata) -> (u64, u64) {
    (0, 0)
}

/// Fingerprint without following links. `None`: nothing there.
pub fn fingerprint(p: &Path, tick: &dyn Fn()) -> Option<Fp> {
    let m = fs::symlink_metadata(p).ok()?;
    let (ino, ctime) = identity(&m);
    let mut fp = Fp { dir: m.is_dir(), link: m.file_type().is_symlink(), size: if m.is_dir() { 0 } else { m.len() }, mtime: mtime_ms(&m), count: 0, complete: true, ino, ctime, digest: String::new(), meta: String::new() };
    if !fp.dir {
        return Some(fp);
    }
    let mut rows: Vec<(String, u64, u64, u64)> = vec![];
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(items) = fs::read_dir(&d) else {
            fp.complete = false;
            continue;
        };
        for item in items {
            tick();
            if fp.count >= MAX_WALK {
                fp.complete = false;
                return Some(fp);
            }
            let Ok(item) = item else {
                fp.complete = false;
                continue;
            };
            let path = item.path();
            let Ok(m) = fs::symlink_metadata(&path) else {
                fp.complete = false;
                continue;
            };
            fp.count += 1;
            fp.mtime = fp.mtime.max(mtime_ms(&m));
            let size = if m.is_dir() { 0 } else { m.len() };
            fp.size += size;
            let rel = path.strip_prefix(p).map_or(String::new(), |r| r.to_string_lossy().into_owned());
            rows.push((rel, size, mtime_ms(&m), identity(&m).1));
            if m.is_dir() {
                stack.push(path);
            }
        }
    }
    rows.sort();
    let (mut h, mut m) = (Sha256::new(), Sha256::new());
    for (rel, size, mtime, ctime) in &rows {
        h.update(rel.as_bytes());
        h.update([0]);
        h.update(size.to_le_bytes());
        h.update(mtime.to_le_bytes());
        m.update(ctime.to_le_bytes());
    }
    let hex = |d: &[u8]| d.iter().take(16).map(|b| format!("{b:02x}")).collect::<String>();
    fp.digest = hex(&h.clone().finalize());
    m.update(h.finalize());
    fp.meta = hex(&m.finalize());
    Some(fp)
}

/// The real location of `p`'s folder, resolving links, with the part that doesn't exist yet kept as
/// written. `None` if a link on the way points nowhere.
pub fn real_parent(p: &Path) -> Option<String> {
    let parent = p.parent()?;
    let mut rest: Vec<&std::ffi::OsStr> = vec![];
    let mut cur = parent;
    loop {
        if fs::symlink_metadata(cur).is_ok() {
            let mut real = fs::canonicalize(cur).ok()?;
            real.extend(rest.iter().rev());
            return Some(real.to_string_lossy().into_owned());
        }
        rest.push(cur.file_name()?);
        cur = cur.parent()?;
    }
}

/// The real folder to store: nothing when it's the written one, empty when it can't be told.
pub fn real_to_store(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let p = Path::new(path);
    match real_parent(p) {
        Some(r) if p.parent().is_some_and(|d| d.as_os_str() == r.as_str()) => None,
        Some(r) => Some(r),
        None => Some(String::new()),
    }
}
