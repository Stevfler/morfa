//! Two-way reconcile of one pinned path between the server and the local mirror.
//!
//! For each item we compare both sides against the record of the last sync: only the side
//! that changed wins; if both changed, the local file is kept as a "(conflict …)" copy and
//! the server version is downloaded. Deletions go to the system trash locally. On the server they
//! go to the Nextcloud trash bin; generic WebDAV and SFTP have none, there they are final.
//! A failed listing never counts as "deleted" — the pin is skipped instead.
//!
//! Absence is only believed in small doses. A pin whose whole side vanished (disk not mounted,
//! `rm -rf`, an empty answer from the server) or a mass deletion (see `is_mass`) is held: nothing
//! is deleted and no record is lost until the user confirms or restores (`resolve_hold`). A symlink
//! or unreadable item is never "absent", and names the local disk can't keep apart are left out.
use super::dav::RemoteItem;
use super::names::{safe_name, safe_rel};
use super::store::{under, Hold, Record, SyncDb};
use unicode_normalization::UnicodeNormalization;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::Mutex,
    time::UNIX_EPOCH,
};

pub trait Remote {
    fn list(&self, rel: &str) -> Result<Vec<RemoteItem>, String>;
    fn stat(&self, rel: &str) -> Result<Option<RemoteItem>, String>;
    fn download(&self, rel: &str, dest: &Path, cancelled: &dyn Fn() -> bool) -> Result<(), String>;
    /// `tick` is called as the transfer makes progress.
    fn upload(&self, rel: &str, src: &Path, mtime_ms: u64, tick: &dyn Fn()) -> Result<Option<String>, String>;
    fn mkcol(&self, rel: &str) -> Result<(), String>;
    fn delete(&self, rel: &str) -> Result<(), String>;
    /// A folder's ETag changes when anything at any depth inside it changes (Nextcloud does this;
    /// a plain WebDAV server or SFTP doesn't), so an unchanged folder needn't be walked.
    fn recursive_etags(&self) -> bool {
        false
    }
}

impl Remote for super::dav::Dav {
    fn list(&self, rel: &str) -> Result<Vec<RemoteItem>, String> { self.list(rel) }
    fn stat(&self, rel: &str) -> Result<Option<RemoteItem>, String> { self.stat(rel) }
    fn download(&self, rel: &str, dest: &Path, c: &dyn Fn() -> bool) -> Result<(), String> { self.download(rel, dest, c) }
    fn upload(&self, rel: &str, src: &Path, m: u64, t: &dyn Fn()) -> Result<Option<String>, String> { self.upload(rel, src, m, t) }
    fn mkcol(&self, rel: &str) -> Result<(), String> { self.mkcol(rel) }
    fn delete(&self, rel: &str) -> Result<(), String> { self.delete(rel) }
    fn recursive_etags(&self) -> bool { self.recursive_etags() }
}

pub struct Ctx<'a> {
    pub remote: &'a dyn Remote,
    pub root: PathBuf,
    pub db: &'a Mutex<SyncDb>,
    pub cancelled: &'a dyn Fn() -> bool,
    /// Move a local file/folder to the system trash.
    pub trash: &'a dyn Fn(&Path) -> Result<(), String>,
    /// Called with the rel path before each transfer.
    pub progress: &'a dyn Fn(&str),
}

#[derive(Default, Debug)]
pub struct Report {
    pub errors: Vec<String>,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug)]
struct Local {
    dir: bool,
    size: u64,
    mtime: u64,
}

pub const PART: &str = ".morfa-part";

/// Deletions in one direction in one run are held past this: more than `MASS_MIN` items and over
/// a quarter of the pin's records, or more than `MASS_MAX` whatever the share.
const MASS_MIN: usize = 10;
const MASS_MAX: usize = 200;
fn is_mass(count: usize, total: usize) -> bool {
    count > MASS_MAX || (count > MASS_MIN && count * 4 > total)
}

/// A short unique temp name: never over the file name limit, whatever the real name's length.
pub fn part_name() -> String {
    use sha2::Digest;
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let seed = format!("{}-{:?}-{}", std::process::id(), std::time::SystemTime::now(), N.fetch_add(1, Ordering::Relaxed));
    let hex: String = sha2::Sha256::digest(seed).iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!(".morfa-{hex}{PART}")
}

/// What the local disk treats as the same name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fold {
    pub case: bool,
    pub unicode: bool,
}

/// Asked of the disk itself, since a Mac volume can be case-sensitive and a Linux USB disk not.
/// If the folder can't be written to, the platform's usual behaviour is assumed.
fn probe_fold(root: &Path) -> Fold {
    let usual = cfg!(any(target_os = "macos", windows));
    let tag = part_name();
    let name = |s: &str| root.join(format!("{tag}-{s}{PART}"));
    let made = name("aB\u{e9}");
    if std::fs::File::create(&made).is_err() {
        return Fold { case: usual, unicode: usual };
    }
    let exists = |s: &str| std::fs::symlink_metadata(name(s)).is_ok();
    let fold = Fold { case: exists("Ab\u{e9}"), unicode: exists("aBe\u{301}") };
    let _ = std::fs::remove_file(&made);
    fold
}

pub(crate) fn fold_key(s: &str, f: Fold) -> String {
    let s: String = if f.unicode { s.nfc().collect() } else { s.to_string() };
    if f.case { s.to_lowercase() } else { s }
}

/// Paths that would land on one local file, grouped; each group has at least two spellings.
fn collisions<'k>(keys: impl Iterator<Item = &'k String>, f: Fold) -> Vec<BTreeSet<String>> {
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for k in keys {
        groups.entry(fold_key(k, f)).or_default().insert(k.clone());
    }
    groups.into_values().filter(|g| g.len() > 1).collect()
}

/// Files that never sync (OS litter, office lock files, our own partial downloads and conversions).
pub fn ignored(name: &str) -> bool {
    matches!(name, ".DS_Store" | "Thumbs.db" | "desktop.ini" | ".directory")
        || name.starts_with("~$")
        || name.starts_with(".~lock.")
        || name.ends_with(PART)
        || name.starts_with(crate::convert::naming::TEMP_PREFIX)
}

fn mtime_ms(m: &std::fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Where `rel` lives under `root`. Whatever `rel` holds, the result stays under `root`: unsafe
/// segments are dropped here as the last line of defence, callers reject them earlier.
pub fn local_path(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').filter(|s| safe_name(s)).fold(root.to_path_buf(), |p, s| p.join(s))
}

/// Refuses a mirror path that passes through a symlink on this device: writing through one could
/// land outside the mirror folder.
fn no_symlink_on_the_way(root: &Path, rel: &str) -> Result<(), String> {
    let mut p = root.to_path_buf();
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        p.push(seg);
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_symlink() => return Err(format!("{} is a symbolic link, not syncing through it", p.display())),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(format!("{}: {e}", p.display())),
        }
    }
    Ok(())
}

/// `Ok(None)` only when the item really isn't there. A symlink or a failed stat is an error:
/// neither may ever read as "deleted here".
fn local_stat(path: &Path) -> Result<Option<Local>, String> {
    let m = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if m.file_type().is_symlink() {
        return Err(format!("{} is a symbolic link", path.display()));
    }
    Ok(Some(Local { dir: m.is_dir(), size: if m.is_dir() { 0 } else { m.len() }, mtime: mtime_ms(&m) }))
}

/// Temp files older than this were left by a crash or a lost connection.
pub const STALE_PART: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

fn reclaim_local_part(path: &Path, name: &str) {
    if !name.starts_with('.') || !name.ends_with(PART) {
        return;
    }
    let Ok(m) = std::fs::symlink_metadata(path) else { return };
    if m.is_file() && m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > STALE_PART) {
        let _ = std::fs::remove_file(path);
    }
}

/// `unreadable` collects what's there but can't be synced: symlinks and items that fail to stat.
fn walk_local(root: &Path, rel: &str, out: &mut BTreeMap<String, Local>, unreadable: &mut BTreeSet<String>) -> Result<(), String> {
    let dir = local_path(root, rel);
    for e in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().into_owned();
        // Couldn't be mapped back to this path by `local_path`, so it stays out of the sync.
        if ignored(&name) || !safe_name(&name) {
            reclaim_local_part(&e.path(), &name);
            continue;
        }
        let child = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        match local_stat(&e.path()) {
            Ok(Some(l)) => {
                out.insert(child.clone(), l);
                if l.dir {
                    walk_local(root, &child, out, unreadable)?;
                }
            }
            Ok(None) => {}
            Err(_) => {
                unreadable.insert(child);
            }
        }
    }
    Ok(())
}

/// List the remote tree under the folder `rel`. Sub-folders whose ETag matches the record are
/// not listed again: their content is taken from the records (Nextcloud folder ETags change
/// whenever anything inside changes).
fn walk_remote(r: &dyn Remote, db: &SyncDb, rel: &str, out: &mut BTreeMap<String, RemoteItem>, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        return Err("Cancelled".into());
    }
    for item in r.list(rel)? {
        let name = item.path.rsplit('/').next().unwrap_or("");
        if ignored(name) || !item.link.is_empty() || !under(&item.path, rel) || item.path == rel || !safe_rel(&item.path) {
            continue;
        }
        // Never followed during a walk: a link can point back up and never end.
        if item.symlink && item.is_dir {
            continue;
        }
        let path = item.path.clone();
        let dir = item.is_dir;
        let unchanged = r.recursive_etags() && dir && !item.etag.is_empty() && db.items.get(&path).is_some_and(|rec| rec.dir && rec.etag == item.etag);
        out.insert(path.clone(), item);
        if unchanged {
            for (k, rec) in db.items.range(format!("{path}/")..) {
                if !under(k, &path) {
                    break;
                }
                out.insert(k.clone(), RemoteItem { path: k.clone(), is_dir: rec.dir, size: rec.size, etag: rec.etag.clone(), ..Default::default() });
            }
        } else if dir {
            walk_remote(r, db, &path, out, cancelled)?;
        }
    }
    Ok(())
}

fn conflict_name(rel: &str) -> String {
    let (dir, name) = rel.rsplit_once('/').map(|(d, n)| (format!("{d}/"), n)).unwrap_or((String::new(), rel));
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let secs = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{dir}{stem} (conflict {}){ext}", super::stamp(secs))
}

struct Run<'a, 'b> {
    c: &'a Ctx<'b>,
    rep: Report,
    /// Remote folders known to exist (so uploads don't MKCOL every parent).
    remote_dirs: HashSet<String>,
    /// Folders with any action or error beneath: their ETag is stale after this run.
    touched: HashSet<String>,
    /// Deletions this run may carry out; anything else that looks deleted is held.
    allow_server: BTreeSet<String>,
    allow_local: BTreeSet<String>,
    local_held: bool,
}

impl Run<'_, '_> {
    fn touch(&mut self, rel: &str) {
        let mut p = rel;
        while let Some((parent, _)) = p.rsplit_once('/') {
            self.touched.insert(parent.to_string());
            p = parent;
        }
        self.touched.insert(String::new());
        self.rep.changed = true;
    }
    fn fail(&mut self, rel: &str, e: String) {
        self.touch(rel);
        self.rep.errors.push(format!("{rel}: {e}"));
    }
    fn record(&self, rel: &str, rec: Option<Record>) {
        let mut db = self.c.db.lock().unwrap_or_else(|e| e.into_inner());
        match rec {
            // Unpinned meanwhile: leave the file, just stop tracking it.
            Some(r) if db.pin_of(rel).is_some() => {
                db.items.insert(rel.into(), r);
            }
            _ => {
                db.items.remove(rel);
            }
        }
    }
    fn file_record(&self, rel: &str, etag: String) -> Option<Record> {
        let l = local_stat(&local_path(&self.c.root, rel)).ok().flatten()?;
        Some(Record { dir: false, etag, size: l.size, mtime: l.mtime })
    }

    fn ensure_remote_parents(&mut self, rel: &str) -> Result<(), String> {
        let mut need = vec![];
        let mut p = rel;
        while let Some((parent, _)) = p.rsplit_once('/') {
            if self.remote_dirs.contains(parent) {
                break;
            }
            need.push(parent.to_string());
            p = parent;
        }
        for d in need.into_iter().rev() {
            self.c.remote.mkcol(&d)?;
            self.remote_dirs.insert(d);
        }
        Ok(())
    }

    fn download(&mut self, rel: &str, r: &RemoteItem) {
        (self.c.progress)(rel);
        self.touch(rel);
        let dest = local_path(&self.c.root, rel);
        let res = (|| {
            no_symlink_on_the_way(&self.c.root, rel)?;
            let parent = dest.parent().ok_or("bad path")?;
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            let tmp = parent.join(part_name());
            if let Err(e) = self.c.remote.download(rel, &tmp, self.c.cancelled) {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
            if r.modified > 0 {
                if let Ok(f) = std::fs::File::options().write(true).open(&tmp) {
                    let _ = f.set_modified(UNIX_EPOCH + std::time::Duration::from_millis(r.modified));
                }
            }
            std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())
        })();
        match res {
            Ok(()) => self.record(rel, self.file_record(rel, r.etag.clone())),
            Err(e) => self.fail(rel, e),
        }
    }

    fn upload(&mut self, rel: &str, l: Local) {
        (self.c.progress)(rel);
        self.touch(rel);
        let res = no_symlink_on_the_way(&self.c.root, rel).and_then(|_| self.ensure_remote_parents(rel)).and_then(|_| {
            let etag = self.c.remote.upload(rel, &local_path(&self.c.root, rel), l.mtime, &|| {})?;
            match etag {
                Some(e) => Ok(e),
                None => Ok(self.c.remote.stat(rel)?.map(|i| i.etag).unwrap_or_default()),
            }
        });
        match res {
            Ok(etag) => self.record(rel, Some(Record { dir: false, etag, size: l.size, mtime: l.mtime })),
            Err(e) => self.fail(rel, e),
        }
    }

    /// Both sides changed: keep the local edit as a conflict copy (uploaded too), take the server's.
    fn conflict(&mut self, rel: &str, r: &RemoteItem) {
        let copy = conflict_name(rel);
        if let Err(e) = no_symlink_on_the_way(&self.c.root, rel) {
            return self.fail(rel, e);
        }
        let from = local_path(&self.c.root, rel);
        if let Err(e) = std::fs::rename(&from, local_path(&self.c.root, &copy)) {
            return self.fail(rel, e.to_string());
        }
        if let Ok(Some(l)) = local_stat(&local_path(&self.c.root, &copy)) {
            self.upload(&copy, l);
        }
        self.download(rel, r);
    }

    fn delete_local(&mut self, rel: &str) {
        self.touch(rel);
        if let Err(e) = no_symlink_on_the_way(&self.c.root, rel) {
            return self.fail(rel, e);
        }
        match (self.c.trash)(&local_path(&self.c.root, rel)) {
            Ok(()) => self.record(rel, None),
            Err(e) => self.fail(rel, e),
        }
    }

    fn delete_remote(&mut self, rel: &str) {
        self.touch(rel);
        match self.c.remote.delete(rel) {
            Ok(()) => self.record(rel, None),
            Err(e) => self.fail(rel, e),
        }
    }

    fn file(&mut self, rel: &str, r: Option<&RemoteItem>, l: Option<Local>, rec: Option<&Record>) {
        let l_changed = |l: &Local, rec: &Record| l.size != rec.size || l.mtime != rec.mtime;
        match (r, l, rec) {
            (Some(r), Some(l), Some(rec)) => match (r.etag != rec.etag, l_changed(&l, rec)) {
                (false, false) => {}
                (true, false) => self.download(rel, r),
                (false, true) => self.upload(rel, l),
                (true, true) => self.conflict(rel, r),
            },
            // Both appeared independently: same size → assume same content.
            (Some(r), Some(l), None) if r.size == l.size => self.record(rel, Some(Record { dir: false, etag: r.etag.clone(), size: l.size, mtime: l.mtime })),
            (Some(r), Some(_), None) => self.conflict(rel, r),
            (Some(r), None, Some(rec)) if r.etag == rec.etag => {
                if self.allow_server.contains(rel) {
                    self.delete_remote(rel)
                }
            }
            (Some(r), None, _) => self.download(rel, r),
            (None, Some(l), Some(rec)) if !l_changed(&l, rec) => {
                if self.allow_local.contains(rel) {
                    self.delete_local(rel)
                }
            }
            (None, Some(l), _) => self.upload(rel, l),
            (None, None, Some(_)) => self.record(rel, None),
            (None, None, None) => {}
        }
    }

    /// Before children: create folders that are new on one side.
    fn dir_pre(&mut self, rel: &str, r: bool, l: bool, rec: bool) {
        if rec {
            return;
        }
        if r && !l {
            self.touch(rel);
            let made = no_symlink_on_the_way(&self.c.root, rel).and_then(|_| std::fs::create_dir_all(local_path(&self.c.root, rel)).map_err(|e| e.to_string()));
            if let Err(e) = made {
                self.fail(rel, e);
            }
        } else if l && !r {
            self.touch(rel);
            match self.ensure_remote_parents(rel).and_then(|_| self.c.remote.mkcol(rel)) {
                Ok(()) => {
                    self.remote_dirs.insert(rel.into());
                }
                Err(e) => self.fail(rel, e),
            }
        }
    }

    /// After children: finish folder deletions and refresh the folder record.
    fn dir_post(&mut self, rel: &str, r: Option<&RemoteItem>, rec: bool) {
        let path = local_path(&self.c.root, rel);
        let l_now = match local_stat(&path) {
            Ok(l) => l.is_some_and(|l| l.dir),
            Err(e) => return self.fail(rel, e),
        };
        let r_now = r.is_some() || self.remote_dirs.contains(rel);
        if rec && r.is_some() && !l_now {
            if !self.allow_server.contains(rel) {
                return;
            }
            // Deleted locally: remove on the server once nothing new is left inside.
            match self.c.remote.list(rel) {
                // Our own temp files don't count as new files.
                Ok(v) if v.iter().all(|i| i.path.ends_with(PART)) => return self.delete_remote(rel),
                Ok(_) => return self.fail(rel, "folder was deleted locally but has new files on the server".into()),
                Err(e) => return self.fail(rel, e),
            }
        }
        if rec && !r_now && l_now && !self.local_held {
            // Deleted on the server: drop the local folder if all its files went with it.
            let empty = std::fs::read_dir(&path).map(|mut d| d.next().is_none()).unwrap_or(false);
            if empty {
                self.touch(rel);
                let _ = std::fs::remove_dir(&path);
                return self.record(rel, None);
            }
        }
        if !l_now && !r_now {
            return self.record(rel, None);
        }
        let etag = match r {
            Some(r) if !self.touched.contains(rel) => r.etag.clone(),
            _ => String::new(),
        };
        self.record(rel, Some(Record { dir: true, etag, size: 0, mtime: 0 }));
    }
}

/// Sync one pinned path (file or folder; "" = the whole account).
pub fn sync_pin(c: &Ctx, pin: &str) -> Result<Report, String> {
    if !safe_rel(pin) {
        return Err("Invalid path".into());
    }
    // Through a symlink to wherever the folder really is; if that's gone, nothing happens.
    let root = std::fs::canonicalize(&c.root).map_err(|e| format!("Local folder {} is unavailable: {e}", c.root.display()))?;
    if !root.is_dir() {
        return Err(format!("Local folder {} is missing", c.root.display()));
    }
    let c = &Ctx { root, remote: c.remote, db: c.db, cancelled: c.cancelled, trash: c.trash, progress: c.progress };
    // A symlink or unreadable item on the way to the pin, or the pin itself: the local side is
    // unknown, so nothing may be concluded from it.
    no_symlink_on_the_way(&c.root, pin)?;
    let snapshot = c.db.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut remote = BTreeMap::new();
    let top = if pin.is_empty() {
        None
    } else {
        Some(c.remote.stat(pin)?)
    };
    let pin_is_dir = match &top {
        None => true,
        Some(Some(i)) => i.is_dir,
        Some(None) => snapshot.items.get(pin).is_none_or(|r| r.dir),
    };
    match &top {
        Some(Some(i)) => {
            remote.insert(pin.to_string(), i.clone());
            if i.is_dir {
                let unchanged = c.remote.recursive_etags() && snapshot.items.get(pin).is_some_and(|r| r.dir && !i.etag.is_empty() && r.etag == i.etag);
                if unchanged {
                    for (k, rec) in snapshot.items.range(format!("{pin}/")..) {
                        if !under(k, pin) {
                            break;
                        }
                        remote.insert(k.clone(), RemoteItem { path: k.clone(), is_dir: rec.dir, size: rec.size, etag: rec.etag.clone(), ..Default::default() });
                    }
                } else {
                    walk_remote(c.remote, &snapshot, pin, &mut remote, c.cancelled)?;
                }
            }
        }
        Some(None) => {}
        None => walk_remote(c.remote, &snapshot, "", &mut remote, c.cancelled)?,
    }

    let mut local = BTreeMap::new();
    let mut unreadable = BTreeSet::new();
    let lpin = local_path(&c.root, pin);
    if let Some(l) = local_stat(&lpin)? {
        if !pin.is_empty() {
            local.insert(pin.to_string(), l);
        }
        if l.dir && pin_is_dir {
            walk_local(&c.root, pin, &mut local, &mut unreadable)?;
        }
    }

    let recs: BTreeMap<String, Record> = snapshot.items.iter().filter(|(k, _)| under(k, pin) && !k.is_empty() && safe_rel(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    // Left out of the sync entirely, with everything inside: nothing transferred, nothing deleted.
    let mut excluded: BTreeMap<String, String> = BTreeMap::new();
    for u in unreadable {
        // A link the user made next to synced files is their business, unless it replaces one.
        if remote.keys().chain(recs.keys()).any(|k| under(k, &u)) {
            excluded.insert(u, "is a symbolic link or can't be read here; left out of the sync, nothing is deleted".to_string());
        }
    }
    for group in collisions(remote.keys().chain(local.keys()), probe_fold(&c.root)) {
        let names = group.iter().cloned().collect::<Vec<_>>().join(", ");
        for k in group {
            excluded.insert(k, format!("names differ only by case or Unicode form and can't coexist on this disk ({names}); left out of the sync, nothing is deleted"));
        }
    }
    let keys: BTreeSet<&String> = remote.keys().chain(local.keys()).chain(recs.keys()).filter(|k| !excluded.keys().any(|x| under(k, x))).collect();
    let is_dir_of = |k: &str| remote.get(k).map(|i| i.is_dir).or(local.get(k).map(|l| l.dir)).or(recs.get(k).map(|r| r.dir)).unwrap_or(false);

    // What would be deleted, before anything happens.
    let (mut to_server, mut to_local) = (BTreeSet::new(), BTreeSet::new());
    for k in &keys {
        let Some(rec) = recs.get(*k) else { continue };
        match (remote.get(*k), local.get(*k)) {
            (Some(r), None) if is_dir_of(k) || r.etag == rec.etag => {
                to_server.insert((*k).clone());
            }
            (None, Some(l)) if !l.dir && l.size == rec.size && l.mtime == rec.mtime => {
                to_local.insert((*k).clone());
            }
            _ => {}
        }
    }
    let total = recs.len();
    let gone = |present: bool, any_below: bool| !recs.is_empty() && (!present || (recs.keys().any(|k| k != pin) && !any_below));
    let local_gone = gone(pin.is_empty() || local.contains_key(pin), local.keys().any(|k| k != pin));
    let remote_gone = gone(pin.is_empty() || remote.contains_key(pin), remote.keys().any(|k| k != pin));
    let earlier = |to_server: bool| snapshot.holds.iter().find(|h| h.pin == pin && h.to_server == to_server);
    let split = |planned: BTreeSet<String>, to_server: bool, side_gone: bool| {
        // What a hold lists stays held whatever the counts say now, until the user answers or the
        // item returns (it's then no longer planned). A confirmation covers exactly the items
        // that were shown; anything that vanished since is judged by itself.
        let (listed, rest): (BTreeSet<String>, BTreeSet<String>) = planned.into_iter().partition(|k| earlier(to_server).is_some_and(|h| h.items.contains(k)));
        let confirmed = earlier(to_server).is_some_and(|h| h.confirmed);
        let (mut allowed, mut held) = if confirmed { (listed, BTreeSet::new()) } else { (BTreeSet::new(), listed) };
        if !rest.is_empty() && (side_gone || is_mass(rest.len(), total)) {
            held.extend(rest);
        } else {
            allowed.extend(rest);
        }
        (allowed, held)
    };
    let (allow_server, held_server) = split(to_server, true, local_gone);
    let (allow_local, held_local) = split(to_local, false, remote_gone);
    let mut holds = vec![];
    for (to_server, items, side_gone) in [(true, &held_server, local_gone), (false, &held_local, remote_gone)] {
        if !items.is_empty() {
            let was_gone = earlier(to_server).is_some_and(|h| h.gone && !h.confirmed);
            holds.push(Hold { pin: pin.to_string(), to_server, items: items.clone(), gone: side_gone || was_gone, confirmed: false });
        }
    }
    let holds_changed = {
        let mut db = c.db.lock().unwrap_or_else(|e| e.into_inner());
        let pins = db.pins.clone();
        let before = db.holds.clone();
        db.holds.retain(|h| h.pin != pin && pins.contains(&h.pin));
        db.holds.extend(holds.iter().cloned());
        before != db.holds
    };
    let mut held_errors = vec![];
    for h in &holds {
        let shown = if pin.is_empty() { "/" } else { pin };
        let side = if h.to_server { "on the server" } else { "on this device" };
        let why = if h.gone { "the other side looks empty or missing" } else { "that's too many at once" };
        held_errors.push(format!("{shown}: {} deletions {side} are on hold ({why}). Confirm or restore them", h.items.len()));
    }
    if (local_gone && !held_server.is_empty()) || (remote_gone && !held_local.is_empty()) {
        // One side can't be trusted at all: no transfers into or from it either.
        return Ok(Report { errors: held_errors, changed: holds_changed });
    }
    let mut run = Run {
        c,
        rep: Report::default(),
        remote_dirs: remote.values().filter(|i| i.is_dir).map(|i| i.path.clone()).chain([String::new()]).collect(),
        touched: HashSet::new(),
        local_held: !held_local.is_empty(),
        allow_server,
        allow_local,
    };
    run.rep.changed = holds_changed;
    run.rep.errors = held_errors;
    for k in held_server.iter().chain(&held_local) {
        // Their parents' ETags can't count as settled, but nothing happened yet.
        let mut p = k.as_str();
        while let Some((parent, _)) = p.rsplit_once('/') {
            run.touched.insert(parent.to_string());
            p = parent;
        }
    }
    for (k, why) in &excluded {
        run.fail(k, why.clone());
    }
    if let Some((parent, _)) = pin.rsplit_once('/') {
        // Ancestors of a pin exist on the server (they were listed to reach it).
        let mut p = parent;
        loop {
            run.remote_dirs.insert(p.to_string());
            match p.rsplit_once('/') {
                Some((pp, _)) => p = pp,
                None => break,
            }
        }
    }
    let is_dir = is_dir_of;

    for k in &keys {
        if (c.cancelled)() {
            return Err("Cancelled".into());
        }
        let (r, l, rec) = (remote.get(*k), local.get(*k).copied(), recs.get(*k));
        let mismatch = matches!((r, l), (Some(r), Some(l)) if r.is_dir != l.dir);
        if mismatch {
            run.fail(k, "is a folder on one side and a file on the other".into());
            continue;
        }
        if is_dir(k) {
            run.dir_pre(k, r.is_some(), l.is_some(), rec.is_some());
        } else {
            run.file(k, r, l, rec);
        }
    }
    for k in keys.iter().rev() {
        if is_dir(k) && !run.rep.errors.iter().any(|e| e.starts_with(&format!("{k}: "))) {
            run.dir_post(k, remote.get(*k), recs.contains_key(*k));
        }
    }
    // The pinned item is gone on both sides: nothing left to keep in sync.
    if !pin.is_empty() && matches!(local_stat(&lpin), Ok(None)) && !remote.contains_key(pin) && run.rep.errors.is_empty() {
        let mut db = c.db.lock().unwrap_or_else(|e| e.into_inner());
        db.pins.remove(pin);
        db.drop_under(pin);
        run.rep.changed = true;
    }
    Ok(run.rep)
}

/// The user's answer to a hold. Confirm: the next run deletes exactly the held items. Restore: their
/// records are dropped, so the next run brings them back from the side that still has them.
///
/// `digest` is what the user was shown (`hold_digest`). If the hold changed since, nothing happens.
pub fn resolve_hold(db: &mut SyncDb, pin: &str, to_server: bool, confirm: bool, digest: &str) -> Result<(), String> {
    let stale = "The list of held items has changed. Review it again";
    let i = db.holds.iter().position(|h| h.pin == pin && h.to_server == to_server).ok_or(stale)?;
    if hold_digest(&db.holds[i].items) != digest {
        return Err(stale.into());
    }
    if confirm {
        db.holds[i].confirmed = true;
    } else {
        let h = db.holds.remove(i);
        for k in &h.items {
            db.items.remove(k);
        }
    }
    Ok(())
}

pub fn hold_digest(items: &BTreeSet<String>) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for k in items {
        h.update(k.as_bytes());
        h.update([0]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap};

    /// In-memory server: path → (content, etag). Folders have content None.
    /// Content (None = folder) and ETag counter.
    type Node = (Option<Vec<u8>>, u32);
    #[derive(Default)]
    struct Fake {
        files: RefCell<HashMap<String, Node>>,
        n: RefCell<u32>,
        /// Folder ETags say nothing about what's deeper, as on a plain WebDAV server.
        flat: std::cell::Cell<bool>,
    }
    impl Fake {
        fn put(&self, rel: &str, data: Option<&[u8]>) {
            *self.n.borrow_mut() += 1;
            let n = *self.n.borrow();
            let mut f = self.files.borrow_mut();
            f.insert(rel.into(), (data.map(|d| d.to_vec()), n));
            // Parent folder ETags change too, as on Nextcloud.
            let mut p = rel;
            while let Some((parent, _)) = p.rsplit_once('/') {
                f.insert(parent.into(), (None, n));
                p = parent;
            }
        }
        fn get(&self, rel: &str) -> Option<Vec<u8>> {
            self.files.borrow().get(rel).and_then(|f| f.0.clone())
        }
        fn item(rel: &str, v: &Node) -> RemoteItem {
            RemoteItem { path: rel.into(), is_dir: v.0.is_none(), size: v.0.as_ref().map(|d| d.len() as u64).unwrap_or(0), etag: v.1.to_string(), ..Default::default() }
        }
    }
    impl Remote for Fake {
        fn list(&self, rel: &str) -> Result<Vec<RemoteItem>, String> {
            Ok(self.files.borrow().iter().filter(|(k, _)| k.rsplit_once('/').map(|x| x.0).unwrap_or("") == rel && k.as_str() != rel).map(|(k, v)| Fake::item(k, v)).collect())
        }
        fn stat(&self, rel: &str) -> Result<Option<RemoteItem>, String> {
            Ok(self.files.borrow().get(rel).map(|v| Fake::item(rel, v)))
        }
        fn download(&self, rel: &str, dest: &Path, _: &dyn Fn() -> bool) -> Result<(), String> {
            std::fs::write(dest, self.get(rel).ok_or("404")?).map_err(|e| e.to_string())
        }
        fn upload(&self, rel: &str, src: &Path, _: u64, _: &dyn Fn()) -> Result<Option<String>, String> {
            self.put(rel, Some(&std::fs::read(src).unwrap()));
            Ok(Some(self.n.borrow().to_string()))
        }
        fn mkcol(&self, rel: &str) -> Result<(), String> {
            if !self.files.borrow().contains_key(rel) {
                self.put(rel, None);
            }
            Ok(())
        }
        fn delete(&self, rel: &str) -> Result<(), String> {
            self.files.borrow_mut().retain(|k, _| !under(k, rel));
            Ok(())
        }
        fn recursive_etags(&self) -> bool {
            !self.flat.get()
        }
    }

    struct Env {
        fake: Fake,
        root: PathBuf,
        db: Mutex<SyncDb>,
    }
    impl Env {
        fn new(tag: &str) -> Env {
            let root = std::env::temp_dir().join(format!("morfa-sync-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Env { fake: Fake::default(), root, db: Mutex::default() }
        }
        fn pin(&self, p: &str) {
            self.db.lock().unwrap().pins.insert(p.into());
        }
        fn sync(&self, pin: &str) -> Report {
            let trash = |p: &Path| if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }.map_err(|e| e.to_string());
            let ctx = Ctx { remote: &self.fake, root: self.root.clone(), db: &self.db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
            let r = sync_pin(&ctx, pin).unwrap();
            assert!(r.errors.is_empty(), "{:?}", r.errors);
            r
        }
        fn local(&self, rel: &str) -> PathBuf {
            local_path(&self.root, rel)
        }
        fn write(&self, rel: &str, data: &str) {
            let p = self.local(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, data).unwrap();
            // Make sure the mtime differs from the recorded one.
            let f = std::fs::File::options().write(true).open(&p).unwrap();
            f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
        }
    }
    impl Drop for Env {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn downloads_only_pinned() {
        let e = Env::new("pinned");
        e.fake.put("Docs/a.txt", Some(b"A"));
        e.fake.put("Docs/sub/b.txt", Some(b"B"));
        e.fake.put("Other/c.txt", Some(b"C"));
        e.pin("Docs");
        e.sync("Docs");
        assert_eq!(std::fs::read(e.local("Docs/sub/b.txt")).unwrap(), b"B");
        assert!(!e.local("Other").exists());
        let db = e.db.lock().unwrap();
        assert!(db.items.contains_key("Docs/a.txt") && db.items.contains_key("Docs"));
    }

    #[test]
    fn two_way_changes() {
        let e = Env::new("twoway");
        e.fake.put("D/a.txt", Some(b"A"));
        e.fake.put("D/b.txt", Some(b"B"));
        e.pin("D");
        e.sync("D");
        // Local edit + new local file → uploaded; remote edit → downloaded.
        e.write("D/a.txt", "A2");
        e.write("D/new.txt", "N");
        e.fake.put("D/b.txt", Some(b"B2"));
        e.sync("D");
        assert_eq!(e.fake.get("D/a.txt").unwrap(), b"A2");
        assert_eq!(e.fake.get("D/new.txt").unwrap(), b"N");
        assert_eq!(std::fs::read(e.local("D/b.txt")).unwrap(), b"B2");
        // Nothing to do on a second pass.
        assert!(!e.sync("D").changed);
    }

    #[test]
    fn deletions_both_ways() {
        let e = Env::new("del");
        e.fake.put("D/a.txt", Some(b"A"));
        e.fake.put("D/b.txt", Some(b"B"));
        e.fake.put("D/sub/c.txt", Some(b"C"));
        e.pin("D");
        e.sync("D");
        std::fs::remove_file(e.local("D/a.txt")).unwrap();
        e.fake.delete("D/b.txt").unwrap();
        std::fs::remove_dir_all(e.local("D/sub")).unwrap();
        e.sync("D");
        assert!(e.fake.get("D/a.txt").is_none());
        assert!(!e.local("D/b.txt").exists());
        assert!(e.fake.stat("D/sub").unwrap().is_none());
    }

    #[test]
    fn conflict_keeps_both() {
        let e = Env::new("conflict");
        e.fake.put("f.txt", Some(b"v1"));
        e.pin("f.txt");
        e.sync("f.txt");
        e.write("f.txt", "local");
        e.fake.put("f.txt", Some(b"remote"));
        e.pin("");
        e.sync("");
        assert_eq!(std::fs::read(e.local("f.txt")).unwrap(), b"remote");
        let copy = std::fs::read_dir(&e.root).unwrap().filter_map(|d| d.ok()).find(|d| d.file_name().to_string_lossy().contains("(conflict")).unwrap();
        assert_eq!(std::fs::read(copy.path()).unwrap(), b"local");
    }

    #[test]
    fn listing_failure_deletes_nothing() {
        struct Down;
        impl Remote for Down {
            fn list(&self, _: &str) -> Result<Vec<RemoteItem>, String> { Err("offline".into()) }
            fn stat(&self, _: &str) -> Result<Option<RemoteItem>, String> { Err("offline".into()) }
            fn download(&self, _: &str, _: &Path, _: &dyn Fn() -> bool) -> Result<(), String> { unreachable!() }
            fn upload(&self, _: &str, _: &Path, _: u64, _: &dyn Fn()) -> Result<Option<String>, String> { unreachable!() }
            fn mkcol(&self, _: &str) -> Result<(), String> { unreachable!() }
            fn delete(&self, _: &str) -> Result<(), String> { unreachable!() }
        }
        let e = Env::new("offline");
        e.fake.put("D/a.txt", Some(b"A"));
        e.pin("D");
        e.sync("D");
        let trash = |_: &Path| -> Result<(), String> { panic!("must not delete") };
        let ctx = Ctx { remote: &Down, root: e.root.clone(), db: &e.db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
        assert!(sync_pin(&ctx, "D").is_err());
        assert!(e.local("D/a.txt").exists());
    }

    /// A server without Nextcloud's recursive folder ETags: a change deep inside must still be found.
    #[test]
    fn flat_etags_still_see_deep_changes() {
        let e = Env::new("flat");
        e.fake.flat.set(true);
        e.fake.put("D/sub/a.txt", Some(b"A"));
        e.pin("D");
        e.sync("D");
        e.fake.files.borrow_mut().insert("D/sub/a.txt".into(), (Some(b"A2".to_vec()), 999));
        e.sync("D");
        assert_eq!(std::fs::read(e.local("D/sub/a.txt")).unwrap(), b"A2");
    }

    #[test]
    fn local_path_cannot_leave_the_root() {
        let root = Path::new("/mirror");
        for evil in ["../x", "a/../../x", "..", "/etc/passwd", "a/./b/../c", "..\\..\\x", "a\0b/c"] {
            let p = local_path(root, evil);
            assert!(p.starts_with(root) && !p.components().any(|c| matches!(c, std::path::Component::ParentDir)), "{evil}: {}", p.display());
        }
    }

    /// A server (or fake) naming things outside the mirror gets nothing written there.
    #[test]
    fn hostile_listing_writes_nothing_outside() {
        struct Evil;
        impl Remote for Evil {
            fn list(&self, _: &str) -> Result<Vec<RemoteItem>, String> {
                let f = |p: &str| RemoteItem { path: p.into(), etag: "1".into(), size: 1, ..Default::default() };
                Ok(vec![f("../morfa-escape.txt"), f("D/../../morfa-escape.txt"), f("D/ok.txt"), f("/tmp/morfa-escape.txt")])
            }
            fn stat(&self, r: &str) -> Result<Option<RemoteItem>, String> { Ok(Some(RemoteItem { path: r.into(), is_dir: true, ..Default::default() })) }
            fn download(&self, _: &str, d: &Path, _: &dyn Fn() -> bool) -> Result<(), String> { std::fs::write(d, b"x").map_err(|e| e.to_string()) }
            fn upload(&self, _: &str, _: &Path, _: u64, _: &dyn Fn()) -> Result<Option<String>, String> { Ok(None) }
            fn mkcol(&self, _: &str) -> Result<(), String> { Ok(()) }
            fn delete(&self, _: &str) -> Result<(), String> { Ok(()) }
        }
        let e = Env::new("evil");
        let root = e.root.join("mirror");
        std::fs::create_dir_all(&root).unwrap();
        let trash = |_: &Path| -> Result<(), String> { panic!("must not delete") };
        let ctx = Ctx { remote: &Evil, root: root.clone(), db: &e.db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
        e.pin("D");
        assert!(sync_pin(&ctx, "D").unwrap().errors.is_empty());
        assert!(root.join("D/ok.txt").exists());
        assert!(!e.root.join("morfa-escape.txt").exists());
        assert!(!Path::new("/tmp/morfa-escape.txt").exists());
        assert!(sync_pin(&ctx, "../D").is_err(), "a hostile pin is refused");
    }

    /// A symlink in the mirror folder must not let a download land outside it.
    #[cfg(unix)]
    #[test]
    fn does_not_write_through_local_symlinks() {
        let e = Env::new("symlink");
        let outside = e.root.join("outside");
        let root = e.root.join("mirror");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(root.join("D")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("D/link")).unwrap();
        e.fake.put("D/link/x.txt", Some(b"X"));
        e.pin("D");
        let trash = |_: &Path| -> Result<(), String> { Ok(()) };
        let ctx = Ctx { remote: &e.fake, root, db: &e.db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
        let rep = sync_pin(&ctx, "D").unwrap();
        assert!(!rep.errors.is_empty());
        assert!(!outside.join("x.txt").exists());
    }

    /// A linked folder is not descended in a walk, so a loop can't make it endless.
    #[test]
    fn walk_skips_folder_symlinks() {
        struct Linky;
        impl Remote for Linky {
            fn list(&self, r: &str) -> Result<Vec<RemoteItem>, String> {
                assert_eq!(r, "", "only the root is walked");
                Ok(vec![RemoteItem { path: "loop".into(), is_dir: true, symlink: true, ..Default::default() }])
            }
            fn stat(&self, _: &str) -> Result<Option<RemoteItem>, String> { Ok(None) }
            fn download(&self, _: &str, _: &Path, _: &dyn Fn() -> bool) -> Result<(), String> { unreachable!() }
            fn upload(&self, _: &str, _: &Path, _: u64, _: &dyn Fn()) -> Result<Option<String>, String> { unreachable!() }
            fn mkcol(&self, _: &str) -> Result<(), String> { unreachable!() }
            fn delete(&self, _: &str) -> Result<(), String> { unreachable!() }
        }
        let e = Env::new("linky");
        let trash = |_: &Path| -> Result<(), String> { Ok(()) };
        let ctx = Ctx { remote: &Linky, root: e.root.clone(), db: &e.db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
        sync_pin(&ctx, "").unwrap();
        assert!(!e.local("loop").exists());
    }

    fn answer(e: &Env, to_server: bool, confirm: bool) {
        let mut db = e.db.lock().unwrap();
        let d = hold_digest(&db.holds.iter().find(|h| h.to_server == to_server).unwrap().items);
        resolve_hold(&mut db, "D", to_server, confirm, &d).unwrap();
    }

    /// N1: files being copied back: the count falls under the limit, the rest must stay held.
    #[test]
    fn hold_stays_while_files_are_coming_back() {
        let e = Env::new("n1back");
        many(&e, 40);
        let moved = e.root.with_extension("away");
        let _ = std::fs::remove_dir_all(&moved);
        std::fs::rename(e.local("D"), &moved).unwrap();
        assert!(!raw(&e).errors.is_empty());
        std::fs::create_dir(e.local("D")).unwrap();
        for i in 0..32 {
            let n = format!("f{i}.txt");
            std::fs::rename(moved.join(&n), e.local(&format!("D/{n}"))).unwrap();
        }
        assert!(!raw(&e).errors.is_empty());
        assert_eq!(server_count(&e), 40, "8 of 40 is under the limit but they were held");
        let db = e.db.lock().unwrap();
        assert_eq!(db.holds[0].items.len(), 8, "returned items left the hold");
        drop(db);
        for i in 32..40 {
            let n = format!("f{i}.txt");
            std::fs::rename(moved.join(&n), e.local(&format!("D/{n}"))).unwrap();
        }
        // Copies get new dates, so this run may upload; nothing may be deleted, and the hold clears.
        raw(&e);
        assert_eq!(server_count(&e), 40);
        assert!(e.db.lock().unwrap().holds.is_empty());
        std::fs::remove_dir_all(&moved).unwrap();
    }

    /// N1: a new file makes the side look alive again; the held files must stay held.
    #[test]
    fn new_file_does_not_release_a_hold() {
        let e = Env::new("n1new");
        many(&e, 5);
        std::fs::remove_dir_all(e.local("D")).unwrap();
        assert!(!raw(&e).errors.is_empty());
        e.write("D/new.txt", "N");
        assert!(!raw(&e).errors.is_empty());
        for i in 0..5 {
            assert!(e.fake.get(&format!("D/f{i}.txt")).is_some());
        }
        assert_eq!(e.fake.get("D/new.txt").unwrap(), b"N", "other work goes on");
        assert_eq!(e.db.lock().unwrap().holds[0].items.len(), 5);
        answer(&e, true, false);
        assert!(raw(&e).errors.is_empty());
        assert!(e.local("D/f0.txt").exists());
    }

    #[test]
    fn answer_to_a_changed_hold_is_refused() {
        let e = Env::new("digest");
        many(&e, 5);
        std::fs::remove_dir_all(e.local("D")).unwrap();
        raw(&e);
        let shown = hold_digest(&e.db.lock().unwrap().holds[0].items);
        // Another item joins before the user answers.
        e.db.lock().unwrap().holds[0].items.remove("D/f0.txt");
        assert!(resolve_hold(&mut e.db.lock().unwrap(), "D", true, true, &shown).is_err());
        assert!(!e.db.lock().unwrap().holds[0].confirmed);
        assert_eq!(server_count(&e), 5);
        assert!(resolve_hold(&mut e.db.lock().unwrap(), "D", true, false, "nope").is_err());
        assert_eq!(e.db.lock().unwrap().items.len(), 6, "records untouched");
    }

    #[test]
    fn part_files_do_not_block_a_folder_delete() {
        let e = Env::new("partdir");
        e.fake.put("D/sub/a.txt", Some(b"A"));
        e.fake.put("D/keep.txt", Some(b"K"));
        e.pin("D");
        e.sync("D");
        e.fake.put("D/sub/.morfa-0.morfa-part", Some(b"t"));
        std::fs::remove_dir_all(e.local("D/sub")).unwrap();
        e.sync("D");
        assert!(e.fake.stat("D/sub").unwrap().is_none());
    }

    #[test]
    fn stale_local_parts_are_reclaimed() {
        let e = Env::new("partlocal");
        e.fake.put("D/a.txt", Some(b"A"));
        e.pin("D");
        e.sync("D");
        let old = e.local("D/.morfa-0.morfa-part");
        let fresh = e.local("D/.morfa-1.morfa-part");
        std::fs::write(&old, b"t").unwrap();
        std::fs::write(&fresh, b"t").unwrap();
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(std::time::SystemTime::now() - STALE_PART * 2).unwrap();
        e.sync("D");
        assert!(!old.exists() && fresh.exists());
        assert!(e.fake.get("D/.morfa-1.morfa-part").is_none(), "never uploaded");
    }

    fn many(e: &Env, n: usize) {
        for i in 0..n {
            e.fake.put(&format!("D/f{i}.txt"), Some(b"x"));
        }
        e.pin("D");
        e.sync("D");
    }
    fn raw(e: &Env) -> Report {
        let trash = |p: &Path| if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }.map_err(|e| e.to_string());
        let ctx = Ctx { remote: &e.fake, root: e.root.clone(), db: &e.db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
        sync_pin(&ctx, "D").unwrap()
    }
    fn server_count(e: &Env) -> usize {
        e.fake.files.borrow().values().filter(|v| v.0.is_some()).count()
    }

    /// B1: `rm -rf mirror/D` must not wipe the server. Restore brings everything back.
    #[test]
    fn removed_pin_folder_is_held_then_restored() {
        let e = Env::new("b1rm");
        many(&e, 50);
        std::fs::remove_dir_all(e.local("D")).unwrap();
        let r = raw(&e);
        assert!(!r.errors.is_empty());
        assert_eq!(server_count(&e), 50);
        assert_eq!(e.db.lock().unwrap().items.len(), 51, "records survive");
        assert_eq!(e.db.lock().unwrap().holds.len(), 1);
        // Across a restart: the state round-trips, and a second run changes nothing.
        let saved = serde_json::to_string(&*e.db.lock().unwrap()).unwrap();
        *e.db.lock().unwrap() = serde_json::from_str(&saved).unwrap();
        raw(&e);
        assert_eq!(server_count(&e), 50);
        answer(&e, true, false);
        assert!(raw(&e).errors.is_empty());
        assert_eq!(std::fs::read_dir(e.local("D")).unwrap().count(), 50);
        assert_eq!(server_count(&e), 50);
        assert!(e.db.lock().unwrap().holds.is_empty());
    }

    /// B1: a mirror on an unmounted disk looks like an empty folder.
    #[test]
    fn emptied_pin_folder_is_held_even_when_small_and_nothing_is_written() {
        let e = Env::new("b1empty");
        many(&e, 3);
        std::fs::remove_dir_all(e.local("D")).unwrap();
        std::fs::create_dir(e.local("D")).unwrap();
        e.fake.put("D/f0.txt", Some(b"changed"));
        assert!(!raw(&e).errors.is_empty());
        assert_eq!(server_count(&e), 3);
        assert_eq!(std::fs::read_dir(e.local("D")).unwrap().count(), 0, "no transfers into an untrusted side");
    }

    #[test]
    fn confirm_applies_only_what_was_shown() {
        let e = Env::new("b1ok");
        many(&e, 50);
        for i in 0..20 {
            std::fs::remove_file(e.local(&format!("D/f{i}.txt"))).unwrap();
        }
        assert!(!raw(&e).errors.is_empty());
        assert_eq!(server_count(&e), 50, "20 of 50 is over the limit");
        answer(&e, true, true);
        std::fs::remove_file(e.local("D/f20.txt")).unwrap();
        raw(&e);
        assert_eq!(server_count(&e), 29, "the 20 shown, plus f20 which is small enough on its own");
        assert!(e.db.lock().unwrap().holds.is_empty());
    }

    #[test]
    fn small_deletions_still_go_through() {
        let e = Env::new("b1small");
        many(&e, 50);
        for i in 0..10 {
            std::fs::remove_file(e.local(&format!("D/f{i}.txt"))).unwrap();
        }
        assert!(raw(&e).errors.is_empty());
        assert_eq!(server_count(&e), 40);
        assert!(is_mass(11, 43) && !is_mass(11, 44) && !is_mass(10, 10) && is_mass(201, 100_000));
    }

    /// Non-blocking 2: the server answering with nothing holds the local Trash side too.
    #[test]
    fn empty_server_is_held_and_restore_uploads() {
        let e = Env::new("srvgone");
        many(&e, 3);
        e.fake.files.borrow_mut().clear();
        assert!(!raw(&e).errors.is_empty());
        assert!(e.local("D/f0.txt").exists());
        assert!(!e.db.lock().unwrap().holds[0].to_server);
        answer(&e, false, false);
        assert!(raw(&e).errors.is_empty());
        assert_eq!(server_count(&e), 3);
    }

    /// B2: the folder moved to another disk and symlinked back must not read as deleted.
    #[cfg(unix)]
    #[test]
    fn symlinked_pin_folder_deletes_nothing() {
        let e = Env::new("b2pin");
        many(&e, 50);
        let moved = e.root.with_extension("moved");
        let _ = std::fs::remove_dir_all(&moved);
        std::fs::rename(e.local("D"), &moved).unwrap();
        std::os::unix::fs::symlink(&moved, e.local("D")).unwrap();
        let trash = |_: &Path| -> Result<(), String> { panic!("must not trash") };
        let ctx = Ctx { remote: &e.fake, root: e.root.clone(), db: &e.db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
        assert!(sync_pin(&ctx, "D").is_err());
        assert_eq!(server_count(&e), 50);
        std::fs::remove_dir_all(&moved).unwrap();
    }

    /// B2: a file or subfolder inside the pin replaced by a link blocks only that item's deletion.
    #[cfg(unix)]
    #[test]
    fn symlink_inside_a_pin_is_an_error_and_blocks_deletion() {
        let e = Env::new("b2item");
        e.fake.put("D/sub/a.txt", Some(b"A"));
        e.fake.put("D/keep.txt", Some(b"K"));
        e.pin("D");
        e.sync("D");
        let moved = e.root.with_extension("sub");
        let _ = std::fs::remove_dir_all(&moved);
        std::fs::rename(e.local("D/sub"), &moved).unwrap();
        std::os::unix::fs::symlink(&moved, e.local("D/sub")).unwrap();
        let r = raw(&e);
        assert!(r.errors.iter().any(|x| x.contains("D/sub")), "{:?}", r.errors);
        assert_eq!(e.fake.get("D/sub/a.txt").unwrap(), b"A");
        std::fs::remove_dir_all(&moved).unwrap();
    }

    /// B2: a symlinked mirror root with pin "": followed to the real folder, or refused if dangling.
    #[cfg(unix)]
    #[test]
    fn symlinked_mirror_root_deletes_nothing() {
        let e = Env::new("b2root");
        let real = e.root.join("real");
        let link = e.root.join("link");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        for i in 0..3 {
            e.fake.put(&format!("f{i}.txt"), Some(b"x"));
        }
        e.pin("");
        let trash = |_: &Path| -> Result<(), String> { panic!("must not trash") };
        let ctx = Ctx { remote: &e.fake, root: link.clone(), db: &e.db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
        assert!(sync_pin(&ctx, "").unwrap().errors.is_empty());
        assert!(!sync_pin(&ctx, "").unwrap().changed);
        assert_eq!(server_count(&e), 3);
        std::fs::remove_dir_all(&real).unwrap();
        assert!(sync_pin(&ctx, "").is_err(), "dangling root");
        assert_eq!(server_count(&e), 3);
    }

    /// QA note 6: a pin reached through a symlinked parent must not upload what's outside the mirror.
    #[cfg(unix)]
    #[test]
    fn nested_pin_under_a_symlinked_parent_is_refused() {
        let e = Env::new("nested");
        let outside = e.root.with_extension("out");
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(outside.join("p")).unwrap();
        std::fs::write(outside.join("p/secret.txt"), b"s").unwrap();
        std::os::unix::fs::symlink(&outside, e.local("a")).unwrap();
        e.fake.put("a/p/x.txt", Some(b"X"));
        e.pin("a/p");
        let trash = |_: &Path| -> Result<(), String> { Ok(()) };
        let ctx = Ctx { remote: &e.fake, root: e.root.clone(), db: &e.db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
        assert!(sync_pin(&ctx, "a/p").is_err());
        assert!(e.fake.get("a/p/secret.txt").is_none());
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn finds_collisions_by_the_disks_rules() {
        let k = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let both = Fold { case: true, unicode: true };
        let v = k(&["D/Report.txt", "D/report.txt", "D/other.txt"]);
        assert_eq!(collisions(v.iter(), both).len(), 1);
        assert!(collisions(v.iter(), Fold { case: false, unicode: false }).is_empty());
        let v = k(&["caf\u{e9}", "cafe\u{301}"]);
        assert_eq!(collisions(v.iter(), both).len(), 1);
        assert!(collisions(v.iter(), Fold { case: true, unicode: false }).is_empty());
    }

    /// B3: on a disk that folds case, both spellings stay on the server and neither lands locally.
    #[test]
    fn case_collision_is_excluded_on_a_folding_disk() {
        let e = Env::new("b3");
        if !probe_fold(&e.root).case {
            return; // This disk keeps them apart, so there's nothing to collide; see the unit test above.
        }
        e.fake.put("D/Report.txt", Some(b"upper"));
        e.fake.put("D/report.txt", Some(b"lower"));
        e.fake.put("D/fine.txt", Some(b"F"));
        e.pin("D");
        for _ in 0..2 {
            let r = raw(&e);
            assert_eq!(r.errors.iter().filter(|x| x.contains("can't coexist")).count(), 2, "{:?}", r.errors);
            assert_eq!(e.fake.get("D/Report.txt").unwrap(), b"upper");
            assert_eq!(e.fake.get("D/report.txt").unwrap(), b"lower");
            assert!(e.local("D/fine.txt").exists());
            assert!(!e.local("D/report.txt").exists());
        }
    }

    #[test]
    fn temp_names_are_short_and_unique() {
        assert_ne!(part_name(), part_name());
        assert!(part_name().len() < 64 && ignored(&part_name()));
    }

    #[test]
    fn long_names_download() {
        let e = Env::new("long");
        let name = "x".repeat(250);
        e.fake.put(&format!("D/{name}"), Some(b"L"));
        e.pin("D");
        e.sync("D");
        assert!(e.local(&format!("D/{name}")).exists());
    }

    #[test]
    fn ignores_litter() {
        assert!(ignored(".DS_Store") && ignored("~$doc.docx") && ignored(".a.txt.morfa-part"));
        assert!(ignored(".morfa-convert-12-3.webp"));
        assert!(!ignored("notes.txt"));
    }
}
