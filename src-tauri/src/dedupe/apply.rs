//! Moves the files a reviewed plan names to Trash, one at a time. A plan that names anything the scan
//! didn't offer, or every copy of a group, is refused whole. Each file is then looked at again, and a
//! kept copy must still stand, or that file stays.
//!
//! A kept copy counts when it is a regular file of the same size that isn't the file being removed.
//! Its content is read again when it's cheap (up to `REHASH_LIMIT`), and whatever its size if its mtime,
//! ctime or identity changed. A larger unchanged copy is trusted on those, and must still open. What
//! was read stays good for the run only while inode, ctime and mtime stand, and never for a group's
//! last removal.
use super::scan::{hash_full, id_of_stat, is_placeholder, open_file, Cloud, Id, ScanResult};
use crate::{
    cleanup::Protection,
    compare::{mtime_ms, Job, CANCELLED},
    CmdResult,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::atomic::Ordering,
};

pub(super) const REHASH_LIMIT: u64 = 256 << 20;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// The scan this plan was made from; any other is refused.
    pub scan_id: u64,
    pub paths: Vec<String>,
    /// The user went through the confirmation naming count and bytes.
    #[serde(default)]
    pub confirmed: bool,
    /// That confirmation also said deleting these reaches a server.
    #[serde(default)]
    pub confirm_cloud: bool,
    /// And that these are parts of a package or container, each named.
    #[serde(default)]
    pub confirm_whole: bool,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Done,
    Skipped,
    Failed,
    Cancelled,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub path: String,
    pub status: Status,
    /// `gone`, `changed`, `noKeptCopy`, `protected`, `symlinkParent`, `unreadable`, `error`, `cancelled`.
    pub reason: Option<&'static str>,
    pub detail: Option<String>,
}

#[derive(Serialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub outcomes: Vec<Outcome>,
    pub trashed: Vec<String>,
    /// Sizes of what went to Trash.
    pub bytes: u64,
    /// Of those, what stops being on disk once Trash is emptied: other names keep a file alive.
    pub freed: u64,
}

#[derive(Clone, Debug)]
struct Copy {
    path: PathBuf,
    mtime: u64,
    id: Id,
    cloud: bool,
    whole: bool,
}
#[derive(Clone, Debug)]
struct RecGroup {
    hash: String,
    size: u64,
    copies: Vec<Copy>,
}

/// What the last scan found, kept by the backend: a plan can only act on that.
#[derive(Clone)]
pub struct Record {
    id: u64,
    root: String,
    groups: Vec<RecGroup>,
    index: HashMap<String, usize>,
}
impl Record {
    pub fn of(r: &ScanResult) -> Self {
        let mut index = HashMap::new();
        let mut groups = Vec::with_capacity(r.groups.len());
        for (n, g) in r.groups.iter().enumerate() {
            let copies = g
                .files
                .iter()
                .map(|f| {
                    index.insert(f.path.clone(), n);
                    Copy { path: PathBuf::from(&f.path), mtime: f.mtime, id: f.id, cloud: matches!(f.cloud, Some(Cloud::Mirror | Cloud::Provider)), whole: f.whole.is_some() }
                })
                .collect();
            groups.push(RecGroup { hash: g.hash.clone(), size: g.size, copies });
        }
        Record { id: r.scan_id, root: r.root.clone(), groups, index }
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn root(&self) -> &str {
        &self.root
    }
    /// It went to Trash: from now on it can't be a kept copy, nor be asked for again.
    fn forget(&mut self, path: &str) {
        if let Some(n) = self.index.remove(path) {
            self.groups[n].copies.retain(|c| c.path != Path::new(path));
        }
    }
}

pub struct Env<'a> {
    pub trash: &'a dyn Fn(&Path) -> CmdResult<()>,
    pub protection: &'a dyn Fn(&Path) -> Protection,
    pub rehash_limit: u64,
}

/// Refuses the whole plan, with an error the UI translates.
pub(super) fn check(plan: &Plan, record: &Record) -> CmdResult<()> {
    if plan.scan_id != record.id {
        return Err("dedupe-stale".into());
    }
    if !plan.confirmed {
        return Err("dedupe-unconfirmed".into());
    }
    let wanted: HashSet<&str> = plan.paths.iter().map(String::as_str).collect();
    let mut per_group: HashMap<usize, usize> = HashMap::new();
    for p in &wanted {
        let Some(&n) = record.index.get(*p) else { return Err("dedupe-not-in-scan".into()) };
        *per_group.entry(n).or_default() += 1;
    }
    for (n, count) in per_group {
        let g = &record.groups[n];
        if count >= g.copies.len() {
            return Err("dedupe-every-copy".into());
        }
        if !plan.confirm_cloud && g.copies.iter().any(|c| c.cloud && wanted.contains(c.path.to_string_lossy().as_ref())) {
            return Err("dedupe-cloud-unconfirmed".into());
        }
        if !plan.confirm_whole && g.copies.iter().any(|c| c.whole && wanted.contains(c.path.to_string_lossy().as_ref())) {
            return Err("dedupe-whole-unconfirmed".into());
        }
    }
    Ok(())
}

struct Stop(Status, &'static str, Option<String>);
impl Stop {
    fn skip(why: &'static str) -> Self {
        Stop(Status::Skipped, why, None)
    }
}

/// Every folder from the root down is still a real folder.
fn parents_ok(root: &Path, p: &Path) -> Result<(), Stop> {
    let rest = p.strip_prefix(root).map_err(|_| Stop::skip("changed"))?;
    if rest.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(Stop::skip("changed"));
    }
    let mut cur = root.to_path_buf();
    let mut parts = rest.components().peekable();
    while let Some(c) = parts.next() {
        if parts.peek().is_none() {
            break;
        }
        cur.push(c);
        match fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => return Err(Stop::skip("symlinkParent")),
            Ok(m) if m.is_dir() => {}
            _ => return Err(Stop::skip("changed")),
        }
    }
    Ok(())
}

/// The file about to go is what the scan hashed. Returns how many names it has now.
fn target_unchanged(root: &Path, c: &Copy, size: u64) -> Result<u64, Stop> {
    parents_ok(root, &c.path)?;
    let m = match fs::symlink_metadata(&c.path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Stop::skip("gone")),
        Err(_) => return Err(Stop::skip("unreadable")),
    };
    if !m.is_file() || m.len() != size || mtime_ms(&m) != c.mtime || is_placeholder(&c.path, &m) {
        return Err(Stop::skip("changed"));
    }
    let id = id_of_stat(&c.path, &m).map_err(|_| Stop::skip("unreadable"))?;
    if id.dev != c.id.dev || id.ino != c.id.ino || id.ctime != c.id.ctime {
        return Err(Stop::skip("changed"));
    }
    Ok(id.links)
}

type Seen = (PathBuf, u64, u64, u64);

/// Is there still a copy that survives this removal? `verified` remembers contents already read this
/// run; `fresh` ignores it. A copy that can't be opened doesn't count, whatever its stats say.
#[allow(clippy::too_many_arguments)]
fn kept_copy(root: &Path, g: &RecGroup, target: &Copy, plan: &HashSet<&str>, verified: &mut HashSet<Seen>, fresh: bool, limit: u64, job: &Job) -> Result<(), Stop> {
    for c in &g.copies {
        if c.path == target.path || plan.contains(c.path.to_string_lossy().as_ref()) || parents_ok(root, &c.path).is_err() {
            continue;
        }
        let Ok(m) = fs::symlink_metadata(&c.path) else { continue };
        if !m.is_file() || m.len() != g.size || is_placeholder(&c.path, &m) {
            continue;
        }
        let Ok(id) = id_of_stat(&c.path, &m) else { continue };
        // A second name of the target isn't another copy unless the scan said so; it says the world moved.
        if id.dev == target.id.dev && id.ino == target.id.ino {
            continue;
        }
        let mtime = mtime_ms(&m);
        let same = mtime == c.mtime && id.dev == c.id.dev && id.ino == c.id.ino && id.ctime == c.id.ctime;
        if same && g.size > limit {
            if open_file(&c.path).is_ok() {
                return Ok(());
            }
            continue;
        }
        let key = (c.path.clone(), id.ino, id.ctime, mtime);
        if !fresh && verified.contains(&key) {
            return Ok(());
        }
        match hash_full(&c.path, &id, g.size, job, &mut |_| {}) {
            Ok(h) if h == g.hash => {
                verified.insert(key);
                return Ok(());
            }
            Ok(_) => {}
            Err(e) if e == CANCELLED => return Err(Stop(Status::Cancelled, "cancelled", None)),
            Err(_) => {}
        }
    }
    Err(Stop::skip("noKeptCopy"))
}

/// `report(done, total, current)` is called often; throttling is the caller's.
pub fn run(plan: &Plan, record: &mut Record, job: &Job, env: &Env, report: &mut dyn FnMut(u64, u64, &Path)) -> CmdResult<Report> {
    check(plan, record)?;
    let root = PathBuf::from(&record.root);
    let wanted: HashSet<&str> = plan.paths.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    let order: Vec<&str> = plan.paths.iter().map(String::as_str).filter(|p| seen.insert(*p)).collect();
    let total = order.iter().filter_map(|p| record.index.get(*p)).map(|&n| record.groups[n].size).sum();
    let mut out = Report::default();
    let mut done = 0;
    let mut verified = HashSet::new();
    let mut left: HashMap<usize, usize> = HashMap::new();
    for p in &order {
        *left.entry(record.index[*p]).or_default() += 1;
    }
    for p in order {
        let n = record.index[p];
        let fresh = left.get(&n) == Some(&1);
        *left.entry(n).or_default() -= 1;
        let group = record.groups[n].clone();
        let Some(copy) = group.copies.iter().find(|c| c.path == Path::new(p)) else { continue };
        report(done, total, &copy.path);
        let result = (|| -> Result<u64, Stop> {
            if job.cancel.load(Ordering::Relaxed) {
                return Err(Stop(Status::Cancelled, "cancelled", None));
            }
            if (env.protection)(&copy.path) == Protection::Protected {
                return Err(Stop::skip("protected"));
            }
            target_unchanged(&root, copy, group.size)?;
            kept_copy(&root, &group, copy, &wanted, &mut verified, fresh, env.rehash_limit, job)?;
            // Reading a kept copy can take a while.
            let links = target_unchanged(&root, copy, group.size)?;
            // A worker waking up after being given up on as stuck must not touch anything.
            if job.cancel.load(Ordering::Relaxed) {
                return Err(Stop(Status::Cancelled, "cancelled", None));
            }
            (env.trash)(&copy.path).map_err(|e| Stop(Status::Failed, "error", Some(e)))?;
            Ok(links)
        })();
        done += group.size;
        let (status, reason, detail) = match result {
            Ok(links) => {
                out.trashed.push(p.to_string());
                out.bytes += group.size;
                if links <= 1 {
                    out.freed += group.size;
                }
                record.forget(p);
                (Status::Done, None, None)
            }
            Err(Stop(s, why, d)) => (s, Some(why), d),
        };
        out.outcomes.push(Outcome { path: p.to_string(), status, reason, detail });
    }
    Ok(out)
}
