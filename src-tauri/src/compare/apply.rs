//! Carries out a reviewed plan, one item at a time: each is looked at again first and skipped if it
//! changed since the scan. Nothing is overwritten in place: the new copy is finished under a temporary
//! name, the old item goes to Trash, then the copy takes its name.
use super::{is_own_temp, scan::{Class, CompareResult}, snap, Job, Kind, Snap, CANCELLED, TEMP_PREFIX, UNREADABLE};
use crate::{cloud::names::safe_rel, fsops::rename::rename_noreplace, transfer::copy::link, CmdResult};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Op {
    CopyLeftToRight,
    CopyRightToLeft,
    DeleteLeft,
    DeleteRight,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub left_rel: String,
    pub right_rel: String,
    pub op: Op,
    /// As scanned; `None` means it must still be absent.
    pub left: Option<Snap>,
    pub right: Option<Snap>,
    /// The user chose to replace something newer, or of another type.
    #[serde(default)]
    pub replace_newer: bool,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// The scan this plan was made from; any other is refused.
    pub scan_id: u64,
    pub left_root: String,
    pub right_root: String,
    pub actions: Vec<Action>,
    /// The user went through the extra confirmation for deletes.
    #[serde(default)]
    pub confirm_deletes: bool,
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
    pub rel: String,
    pub op: Op,
    pub status: Status,
    /// `changed`, `unreadable`, `nameClash`, `notInScan`, `symlinkParent`, `newerDest`, `deleteNotConfirmed`, `badPath`,
    /// `specialSkipped`, `error`, `oldInTrash`.
    pub reason: Option<&'static str>,
    pub detail: Option<String>,
}

#[derive(Serialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub outcomes: Vec<Outcome>,
    /// New items where nothing stood: undo moves them to Trash.
    pub created: Vec<String>,
    /// New items whose predecessor is in Trash.
    pub replaced: Vec<String>,
    /// Now in Trash, nothing in their place.
    pub trashed: Vec<String>,
    /// Bytes of copies that landed under their final name.
    pub bytes: u64,
    /// Temporary items of ours that couldn't be removed.
    pub leftovers: Vec<String>,
}

struct Fail(Status, &'static str, Option<String>);
impl Fail {
    fn skip(why: &'static str) -> Self {
        Fail(Status::Skipped, why, None)
    }
    fn error(e: impl ToString) -> Self {
        let e = e.to_string();
        if e == CANCELLED {
            Fail(Status::Cancelled, "cancelled", None)
        } else if e == UNREADABLE {
            Fail::skip("unreadable")
        } else {
            Fail(Status::Failed, "error", Some(e))
        }
    }
}

/// Should replacing `dst` with `src` take an explicit choice? Only ever asked of items that differ, so
/// the scan's tolerance has no say: newer by any margin counts, and so does an unknown time.
pub fn needs_choice(src: &Snap, dst: &Snap) -> bool {
    src.kind != dst.kind || dst.mtime > src.mtime || dst.mtime == 0 || src.mtime == 0
}

/// What the last scan found, kept by the backend: a plan can only act on that.
#[derive(Clone)]
pub struct Record {
    id: u64,
    left_root: String,
    right_root: String,
    items: HashMap<(String, String), (Option<Snap>, Option<Snap>)>,
}
impl Record {
    pub fn of(r: &CompareResult) -> Self {
        let acts = |c: Class| !matches!(c, Class::Same | Class::Skipped);
        let items = r.items.iter().filter(|i| acts(i.class)).map(|i| ((i.left_rel.clone(), i.right_rel.clone()), (i.left.clone(), i.right.clone()))).collect();
        Record { id: r.scan_id, left_root: r.left_root.clone(), right_root: r.right_root.clone(), items }
    }
    fn allows(&self, a: &Action) -> bool {
        let side_exists = match a.op {
            Op::CopyLeftToRight | Op::DeleteLeft => a.left.is_some(),
            Op::CopyRightToLeft | Op::DeleteRight => a.right.is_some(),
        };
        side_exists && self.items.get(&(a.left_rel.clone(), a.right_rel.clone())).is_some_and(|(l, r)| *l == a.left && *r == a.right)
    }
}

/// The path of `rel`, provided every folder on the way is a real folder that's still there.
fn resolve(root: &Path, rel: &str) -> Result<PathBuf, Fail> {
    if rel.is_empty() || !safe_rel(rel) {
        return Err(Fail::skip("badPath"));
    }
    let mut cur = root.to_path_buf();
    let mut parts = rel.split('/').peekable();
    while let Some(seg) = parts.next() {
        cur.push(seg);
        if parts.peek().is_some() {
            match fs::symlink_metadata(&cur) {
                Ok(m) if m.file_type().is_symlink() => return Err(Fail::skip("symlinkParent")),
                Ok(m) if m.is_dir() => {}
                _ => return Err(Fail::skip("changed")),
            }
        }
    }
    Ok(cur)
}

struct Run<'a, 'j> {
    job: &'a Job<'j>,
    trash: &'a dyn Fn(&Path) -> CmdResult<()>,
    report: &'a mut dyn FnMut(u64, u64, &Path),
    id: u64,
    seq: usize,
    done: u64,
    total: u64,
    out: SyncReport,
}

/// `report(done, total, current)` is called often; throttling is the caller's.
pub fn run(plan: &Plan, record: &Record, id: u64, job: &Job, trash: &dyn Fn(&Path) -> CmdResult<()>, report: &mut dyn FnMut(u64, u64, &Path)) -> CmdResult<SyncReport> {
    // The roots were canonical when scanned; if one became a symlink, or nested, it's all off.
    let (l, r) = super::check_roots(&plan.left_root, &plan.right_root)?;
    if l != Path::new(&plan.left_root) || r != Path::new(&plan.right_root) {
        return Err("compare-changed".into());
    }
    if plan.scan_id != record.id || plan.left_root != record.left_root || plan.right_root != record.right_root {
        return Err("compare-stale".into());
    }
    let total = plan
        .actions
        .iter()
        .filter_map(|a| match a.op {
            Op::CopyLeftToRight => a.left.as_ref(),
            Op::CopyRightToLeft => a.right.as_ref(),
            _ => None,
        })
        .map(|s| s.size)
        .sum();
    let mut run = Run { job, trash, report, id, seq: 0, done: 0, total, out: SyncReport::default() };
    for a in &plan.actions {
        let rel = if matches!(a.op, Op::CopyRightToLeft | Op::DeleteRight) { &a.right_rel } else { &a.left_rel };
        let result = if job.cancel.load(Ordering::Relaxed) { Err(Fail::error(CANCELLED)) } else { run.one(plan, record, a, &l, &r) };
        let (status, reason, detail) = match result {
            Ok(note) => (Status::Done, note.as_ref().map(|_| "specialSkipped"), note),
            Err(Fail(s, why, d)) => (s, Some(why), d),
        };
        run.out.outcomes.push(Outcome { rel: rel.clone(), op: a.op, status, reason, detail });
    }
    Ok(run.out)
}

impl Run<'_, '_> {
    fn snap_now(&self, p: &Path) -> Result<Option<Snap>, Fail> {
        snap(p, self.job, &mut |_, _| {}).map_err(Fail::error)
    }

    fn unchanged(&self, p: &Path, expected: &Option<Snap>) -> Result<(), Fail> {
        if self.snap_now(p)? == *expected { Ok(()) } else { Err(Fail::skip("changed")) }
    }

    fn one(&mut self, plan: &Plan, record: &Record, a: &Action, l: &Path, r: &Path) -> Result<Option<String>, Fail> {
        let (lp, rp) = (resolve(l, &a.left_rel)?, resolve(r, &a.right_rel)?);
        if !record.allows(a) {
            return Err(Fail::skip("notInScan"));
        }
        let left = End { root: l, rel: &a.left_rel, path: lp, want: &a.left };
        let right = End { root: r, rel: &a.right_rel, path: rp, want: &a.right };
        match a.op {
            Op::CopyLeftToRight => self.copy(&left, &right, a.replace_newer),
            Op::CopyRightToLeft => self.copy(&right, &left, a.replace_newer),
            Op::DeleteLeft => self.delete(plan, &left, &right),
            Op::DeleteRight => self.delete(plan, &right, &left),
        }
    }

    fn delete(&mut self, plan: &Plan, it: &End, other: &End) -> Result<Option<String>, Fail> {
        if !plan.confirm_deletes {
            return Err(Fail::skip("deleteNotConfirmed"));
        }
        if it.want.is_none() {
            return Err(Fail::skip("changed"));
        }
        self.unchanged(&it.path, it.want)?;
        recheck(it, other)?;
        (self.trash)(&it.path).map_err(Fail::error)?;
        self.out.trashed.push(it.path.to_string_lossy().into());
        Ok(None)
    }

    /// As scanned; and if it should be absent but answers, says whether that's the name itself or another spelling.
    fn dest_unchanged(&self, dst: &End) -> Result<(), Fail> {
        let now = self.snap_now(&dst.path)?;
        if now == *dst.want {
            return Ok(());
        }
        if dst.want.is_none() {
            let name = dst.path.file_name();
            let listed = dst.path.parent().and_then(|d| fs::read_dir(d).ok()).is_some_and(|l| l.flatten().any(|e| Some(e.file_name().as_os_str()) == name));
            if !listed {
                return Err(Fail::skip("nameClash"));
            }
        }
        Err(Fail::skip("changed"))
    }

    /// Removes a temporary item we created; what can't be removed is reported.
    fn discard(&mut self, temp: &Path) {
        if discard_tree(temp).is_err() {
            self.out.leftovers.push(temp.to_string_lossy().into());
        }
    }

    fn copy(&mut self, src: &End, dst: &End, replace: bool) -> Result<Option<String>, Fail> {
        let Some(s) = src.want else { return Err(Fail::skip("changed")) };
        if s.kind == Kind::Special {
            return Err(Fail::skip("badPath"));
        }
        if dst.want.as_ref().is_some_and(|d| needs_choice(s, d)) && !replace {
            return Err(Fail::skip("newerDest"));
        }
        self.unchanged(&src.path, src.want)?;
        self.dest_unchanged(dst)?;
        let parent = dst.path.parent().ok_or_else(|| Fail::skip("badPath"))?;
        let temp = loop {
            self.seq += 1;
            let t = parent.join(format!("{TEMP_PREFIX}{}-{}-{}.part", std::process::id(), self.id, self.seq));
            if fs::symlink_metadata(&t).is_err() {
                break t;
            }
        };
        recheck(src, dst)?;
        let before = self.done;
        let mut skipped = vec![];
        let mut made = false;
        let filled = self.copy_node(&src.path, &temp, &mut skipped, &mut made).map_err(Fail::error);
        // Whatever goes wrong before the rename leaves both sides as they were.
        let checked = filled.and_then(|()| self.unchanged(&src.path, src.want)).and_then(|()| self.dest_unchanged(dst)).and_then(|()| recheck(src, dst));
        if let Err(e) = checked {
            if made {
                self.discard(&temp);
            }
            return Err(e);
        }
        let name = dst.path.to_string_lossy().into_owned();
        if dst.want.is_some() {
            if let Err(e) = (self.trash)(&dst.path) {
                self.discard(&temp);
                return Err(Fail::error(e));
            }
        }
        // The parents may have been swapped while Trash was working.
        let landed = recheck(src, dst).map_err(|Fail(_, why, _)| why.to_string()).and_then(|()| rename_noreplace(&temp, &dst.path).map_err(|e| e.to_string()));
        if let Err(e) = landed {
            self.discard(&temp);
            if dst.want.is_some() {
                self.out.trashed.push(name);
                return Err(Fail(Status::Failed, "oldInTrash", Some(e)));
            }
            return Err(Fail::error(e));
        }
        self.out.bytes += self.done - before;
        if dst.want.is_some() {
            self.out.replaced.push(name);
        } else {
            self.out.created.push(name);
        }
        Ok((!skipped.is_empty()).then(|| skipped.join("\n")))
    }

    /// Symlinks are recreated as links; special files inside a folder are left out and named.
    /// Every creation is exclusive; `made` says the top item is ours to remove.
    fn copy_node(&mut self, src: &Path, dst: &Path, skipped: &mut Vec<String>, made: &mut bool) -> CmdResult<()> {
        if self.job.cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let meta = fs::symlink_metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
        let t = meta.file_type();
        if t.is_symlink() {
            link(src, dst).map_err(|e| e.to_string())?;
            *made = true;
            return Ok(());
        }
        if t.is_dir() {
            fs::create_dir(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
            *made = true;
            for e in fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
                let e = e.map_err(|e| format!("{}: {e}", src.display()))?;
                if is_own_temp(&e.file_name()) {
                    continue;
                }
                self.copy_node(&e.path(), &dst.join(e.file_name()), skipped, &mut true)?;
            }
            let _ = fs::set_permissions(dst, meta.permissions());
            return Ok(());
        }
        if !t.is_file() {
            skipped.push(src.to_string_lossy().into_owned());
            return Ok(());
        }
        let mut r = fs::File::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
        let mut w = OpenOptions::new().write(true).create_new(true).open(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        *made = true;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            if self.job.cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
            let n = r.read(&mut buf).map_err(|e| format!("{}: {e}", src.display()))?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", dst.display()))?;
            self.done += n as u64;
            (self.report)(self.done, self.total, src);
        }
        let _ = w.set_permissions(meta.permissions());
        if let Ok(m) = meta.modified() {
            let _ = w.set_modified(m);
        }
        Ok(())
    }
}

/// One side of an action.
struct End<'a> {
    root: &'a Path,
    rel: &'a str,
    path: PathBuf,
    want: &'a Option<Snap>,
}

/// Every parent on both sides is still a real folder.
fn recheck(a: &End, b: &End) -> Result<(), Fail> {
    resolve(a.root, a.rel)?;
    resolve(b.root, b.rel)?;
    Ok(())
}

/// Removes a tree of ours, copied read-only folders included. Never through a symlink.
pub(super) fn discard_tree(p: &Path) -> std::io::Result<()> {
    let m = fs::symlink_metadata(p)?;
    if m.is_dir() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o700));
        }
        for e in fs::read_dir(p)? {
            discard_tree(&e?.path())?;
        }
        fs::remove_dir(p)
    } else {
        fs::remove_file(p)
    }
}

/// Undoes a sync as far as it can, attempting everything and reporting what failed. The original comes
/// back from Trash while the new copy is only moved aside: Trash restores the newest item of a path, so
/// trashing the copy first would bring back the copy. If nothing came back, the copy returns to its place.
pub fn undo(created: &[String], replaced: &[String], trashed: &[String], trash: &dyn Fn(&Path) -> CmdResult<()>, restore: &dyn Fn(&str) -> CmdResult<()>) -> CmdResult<()> {
    const UNSUPPORTED: &str = "UNSUPPORTED_RESTORE:";
    let mut errors = vec![];
    for p in created {
        if let Err(e) = trash(Path::new(p)) {
            errors.push(format!("{p}: {e}"));
        }
    }
    let back = |p: &str| match restore(p) {
        Ok(()) if fs::symlink_metadata(p).is_ok() => Ok(()),
        Ok(()) => Err("not found in Trash".to_string()),
        Err(e) => Err(e),
    };
    for (n, p) in replaced.iter().enumerate() {
        let path = Path::new(p);
        let aside = path.with_file_name(format!("{TEMP_PREFIX}undo-{}-{n}.part", std::process::id()));
        if let Err(e) = rename_noreplace(path, &aside) {
            errors.push(format!("{p}: {e}"));
            continue;
        }
        match back(p) {
            Ok(()) => {
                if let Err(e) = trash(&aside) {
                    errors.push(format!("{p}: the original is back, but the new copy stayed at {}: {e}", aside.display()));
                }
            }
            Err(e) => {
                if let Err(e2) = rename_noreplace(&aside, path) {
                    errors.push(format!("{p}: the new copy stayed at {}: {e2}", aside.display()));
                }
                if e.starts_with(UNSUPPORTED) {
                    return Err(e);
                }
                errors.push(format!("{p}: {e}"));
            }
        }
    }
    for p in trashed {
        if let Err(e) = back(p) {
            if e.starts_with(UNSUPPORTED) {
                return Err(e);
            }
            errors.push(format!("{p}: {e}"));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
}
