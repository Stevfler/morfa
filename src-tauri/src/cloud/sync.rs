//! Two-way reconcile of one pinned path between the server and the local mirror.
//!
//! For each item we compare both sides against the record of the last sync: only the side
//! that changed wins; if both changed, the local file is kept as a "(conflict …)" copy and
//! the server version is downloaded. Deletions go to the system trash / Nextcloud trash bin.
//! A failed listing never counts as "deleted" — the pin is skipped instead.
use super::dav::RemoteItem;
use super::store::{under, Record, SyncDb};
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
    fn upload(&self, rel: &str, src: &Path, mtime_ms: u64) -> Result<Option<String>, String>;
    fn mkcol(&self, rel: &str) -> Result<(), String>;
    fn delete(&self, rel: &str) -> Result<(), String>;
}

impl Remote for super::dav::Dav {
    fn list(&self, rel: &str) -> Result<Vec<RemoteItem>, String> { self.list(rel) }
    fn stat(&self, rel: &str) -> Result<Option<RemoteItem>, String> { self.stat(rel) }
    fn download(&self, rel: &str, dest: &Path, c: &dyn Fn() -> bool) -> Result<(), String> { self.download(rel, dest, c) }
    fn upload(&self, rel: &str, src: &Path, m: u64) -> Result<Option<String>, String> { self.upload(rel, src, m) }
    fn mkcol(&self, rel: &str) -> Result<(), String> { self.mkcol(rel) }
    fn delete(&self, rel: &str) -> Result<(), String> { self.delete(rel) }
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

/// Files that never sync (OS litter, office lock files, our own partial downloads).
pub fn ignored(name: &str) -> bool {
    matches!(name, ".DS_Store" | "Thumbs.db" | "desktop.ini" | ".directory")
        || name.starts_with("~$")
        || name.starts_with(".~lock.")
        || name.ends_with(PART)
}

fn mtime_ms(m: &std::fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn local_path(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').filter(|s| !s.is_empty()).fold(root.to_path_buf(), |p, s| p.join(s))
}

fn local_stat(path: &Path) -> Option<Local> {
    let m = std::fs::symlink_metadata(path).ok()?;
    if m.file_type().is_symlink() {
        return None;
    }
    Some(Local { dir: m.is_dir(), size: if m.is_dir() { 0 } else { m.len() }, mtime: mtime_ms(&m) })
}

fn walk_local(root: &Path, rel: &str, out: &mut BTreeMap<String, Local>) -> Result<(), String> {
    let dir = local_path(root, rel);
    for e in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().into_owned();
        if ignored(&name) {
            continue;
        }
        let child = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        let Some(l) = local_stat(&e.path()) else { continue };
        out.insert(child.clone(), l);
        if l.dir {
            walk_local(root, &child, out)?;
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
        if ignored(name) || !item.link.is_empty() || !under(&item.path, rel) || item.path == rel {
            continue;
        }
        let path = item.path.clone();
        let dir = item.is_dir;
        let unchanged = dir && !item.etag.is_empty() && db.items.get(&path).is_some_and(|rec| rec.dir && rec.etag == item.etag);
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
        let l = local_stat(&local_path(&self.c.root, rel))?;
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
            let parent = dest.parent().ok_or("bad path")?;
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            let name = dest.file_name().ok_or("bad path")?.to_string_lossy();
            let tmp = parent.join(format!(".{name}{PART}"));
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
        let res = self.ensure_remote_parents(rel).and_then(|_| {
            let etag = self.c.remote.upload(rel, &local_path(&self.c.root, rel), l.mtime)?;
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
        let from = local_path(&self.c.root, rel);
        if let Err(e) = std::fs::rename(&from, local_path(&self.c.root, &copy)) {
            return self.fail(rel, e.to_string());
        }
        if let Some(l) = local_stat(&local_path(&self.c.root, &copy)) {
            self.upload(&copy, l);
        }
        self.download(rel, r);
    }

    fn delete_local(&mut self, rel: &str) {
        self.touch(rel);
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
            (Some(r), None, Some(rec)) if r.etag == rec.etag => self.delete_remote(rel),
            (Some(r), None, _) => self.download(rel, r),
            (None, Some(l), Some(rec)) if !l_changed(&l, rec) => self.delete_local(rel),
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
            if let Err(e) = std::fs::create_dir_all(local_path(&self.c.root, rel)) {
                self.fail(rel, e.to_string());
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
        let l_now = local_stat(&path).is_some_and(|l| l.dir);
        let r_now = r.is_some() || self.remote_dirs.contains(rel);
        if rec && r.is_some() && !l_now {
            // Deleted locally: remove on the server once nothing new is left inside.
            match self.c.remote.list(rel) {
                Ok(v) if v.is_empty() => return self.delete_remote(rel),
                Ok(_) => return self.fail(rel, "folder was deleted locally but has new files on the server".into()),
                Err(e) => return self.fail(rel, e),
            }
        }
        if rec && !r_now && l_now {
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
    if !c.root.is_dir() {
        return Err(format!("Local folder {} is missing", c.root.display()));
    }
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
                let unchanged = snapshot.items.get(pin).is_some_and(|r| r.dir && !i.etag.is_empty() && r.etag == i.etag);
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
    let lpin = local_path(&c.root, pin);
    if let Some(l) = local_stat(&lpin) {
        if !pin.is_empty() {
            local.insert(pin.to_string(), l);
        }
        if l.dir && pin_is_dir {
            walk_local(&c.root, pin, &mut local)?;
        }
    }

    let recs: BTreeMap<String, Record> = snapshot.items.iter().filter(|(k, _)| under(k, pin) && !k.is_empty()).map(|(k, v)| (k.clone(), v.clone())).collect();
    let keys: BTreeSet<&String> = remote.keys().chain(local.keys()).chain(recs.keys()).collect();
    let mut run = Run {
        c,
        rep: Report::default(),
        remote_dirs: remote.values().filter(|i| i.is_dir).map(|i| i.path.clone()).chain([String::new()]).collect(),
        touched: HashSet::new(),
    };
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
    let is_dir = |k: &str| remote.get(k).map(|i| i.is_dir).or(local.get(k).map(|l| l.dir)).or(recs.get(k).map(|r| r.dir)).unwrap_or(false);

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
    if !pin.is_empty() && local_stat(&lpin).is_none() && !remote.contains_key(pin) && run.rep.errors.is_empty() {
        let mut db = c.db.lock().unwrap_or_else(|e| e.into_inner());
        db.pins.remove(pin);
        db.drop_under(pin);
        run.rep.changed = true;
    }
    Ok(run.rep)
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
        fn upload(&self, rel: &str, src: &Path, _: u64) -> Result<Option<String>, String> {
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
            fn upload(&self, _: &str, _: &Path, _: u64) -> Result<Option<String>, String> { unreachable!() }
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

    #[test]
    fn ignores_litter() {
        assert!(ignored(".DS_Store") && ignored("~$doc.docx") && ignored(".a.txt.morfa-part"));
        assert!(!ignored("notes.txt"));
    }
}
