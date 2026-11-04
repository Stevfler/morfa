//! Undo from recorded data: every pair is checked against the disk right before anything is done, a
//! preview being the same check with no action. Nothing here overwrites, and nothing that changed
//! since the operation is touched unless its path was named by the user. The journal file is treated
//! as untrusted: whatever it says, protected or malformed paths are refused.
use super::store::{fingerprint, Fp, real_parent, real_to_store, Entry, Kind, Pair, Status, Store};
use crate::{
    fsops::{self, trashcan::Window},
    CmdResult,
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path},
};
use unicode_normalization::UnicodeNormalization;

/// These systems' usual disks fold case and Unicode form. On a sensitive disk that only adds a warning.
const FOLDS: bool = cfg!(any(target_os = "macos", windows));

/// The form two spellings of one path share.
pub(crate) fn fold_key(p: &str, fold: bool) -> String {
    if fold {
        p.nfc().collect::<String>().to_lowercase()
    } else {
        p.to_string()
    }
}
fn key(p: &str) -> String {
    fold_key(p, FOLDS)
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    MoveBack,
    TrashOutput,
    Restore,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    Gone,
    Changed,
    /// Same size, times and identity, yet its metadata differs.
    Metadata,
    Unverified,
    Occupied,
    SymlinkParent,
    NotInTrash,
    Unsupported,
    Protected,
}
impl Reason {
    /// The user may name such a path and go ahead; the rest can't be done safely at all.
    pub fn overridable(self) -> bool {
        matches!(self, Reason::Changed | Reason::Metadata | Reason::Unverified)
    }
}

/// One result per path.
pub type Restored = Vec<(String, Result<(), String>)>;

pub struct Env<'a> {
    pub trash: &'a dyn Fn(&Path) -> CmdResult<()>,
    pub restore: &'a dyn Fn(&[Window]) -> Restored,
    pub in_trash: &'a dyn Fn(&[Window]) -> HashSet<String>,
    pub can_restore: bool,
    pub protected: &'a dyn Fn(&Path) -> bool,
}

/// Why an entry can't be undone at all. Restores that the OS can't do are reported per item instead.
pub fn blocked(e: &Entry) -> Option<&'static str> {
    if e.undone_by.is_some() {
        return Some("undone");
    }
    if e.undo_of.is_some() || e.kind == Kind::Restore {
        return Some("undoEntry");
    }
    match e.kind {
        Kind::Delete => return Some("forever"),
        Kind::Compare => return Some("compare"),
        _ => {}
    }
    if !e.pairs.iter().any(|p| p.done) {
        return Some("nothing");
    }
    None
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub action: Action,
    /// Where it is now (empty for a Trash restore).
    pub from: String,
    /// Where undo would leave it (empty when it goes to Trash).
    pub to: String,
    pub reason: Option<Reason>,
    /// A newer journal entry involves this path.
    pub newer: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Newer {
    pub id: u64,
    pub time: u64,
    pub kind: Kind,
    pub label: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub id: u64,
    pub label: String,
    pub kind: Kind,
    pub time: u64,
    pub blocked: Option<&'static str>,
    pub items: Vec<Item>,
    pub newer: Vec<Newer>,
    /// Pairs the operation had; those past the recorded ones stay as they are.
    pub total: u64,
}

fn parent_same(path: &Path, stored: &Option<String>) -> Result<(), Reason> {
    let expected = match stored {
        // Recorded with the disk not answering, so nothing to compare with: every folder on the way
        // must be a real one. Naming a path doesn't get past this.
        Some(s) if s.is_empty() => {
            for a in path.ancestors().skip(1) {
                match a.symlink_metadata() {
                    Ok(m) if m.file_type().is_symlink() => return Err(Reason::SymlinkParent),
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(Reason::SymlinkParent),
                }
            }
            return Ok(());
        }
        Some(s) => s.clone(),
        None => path.parent().map_or(String::new(), |d| d.to_string_lossy().into_owned()),
    };
    match real_parent(path) {
        Some(r) if r == expected => Ok(()),
        _ => Err(Reason::SymlinkParent),
    }
}

/// Absolute, without `..`, and not somewhere protected.
fn allowed(p: &str, env: &Env) -> Result<(), Reason> {
    if p.is_empty() {
        return Ok(());
    }
    let path = Path::new(p);
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) || (env.protected)(path) {
        return Err(Reason::Protected);
    }
    Ok(())
}

/// Is the thing at `to` what the operation left there?
fn dest_check(p: &Pair, base: Option<&Fp>, tick: &dyn Fn()) -> Result<(), Reason> {
    let to = Path::new(&p.to);
    parent_same(to, &p.to_real)?;
    let Some(now) = fingerprint(to, tick) else { return Err(Reason::Gone) };
    let Some(then) = &p.fp else { return Err(Reason::Unverified) };
    if !then.complete || !now.complete {
        return Err(Reason::Unverified);
    }
    if &now != then {
        if !now.same_content(then) {
            return Err(Reason::Changed);
        }
        // The baseline only excuses ctime: the operation's own record still decides about content.
        if base != Some(&now) {
            return Err(Reason::Metadata);
        }
    }
    Ok(())
}

fn free_check(p: &Pair) -> Result<(), Reason> {
    let (from, to) = (Path::new(&p.from), Path::new(&p.to));
    parent_same(from, &p.from_real)?;
    // A case-only rename: on a case-insensitive disk `from` is the very file at `to`.
    if from.symlink_metadata().is_ok() && !fsops::rename::same_file(from, to) {
        return Err(Reason::Occupied);
    }
    Ok(())
}

/// Hard failures first: an occupied place stays occupied whatever the user names.
fn check_move(p: &Pair, base: Option<&Fp>, tick: &dyn Fn()) -> Result<(), Reason> {
    let hard = free_check(p);
    let dest = dest_check(p, base, tick);
    match (hard, dest) {
        (_, Err(r @ (Reason::Gone | Reason::SymlinkParent))) => Err(r),
        (Err(r), _) => Err(r),
        (Ok(()), d) => d,
    }
}

fn check_restore(p: &Pair, trashed: &HashSet<String>, env: &Env) -> Result<(), Reason> {
    if !env.can_restore {
        return Err(Reason::Unsupported);
    }
    parent_same(Path::new(&p.from), &p.from_real)?;
    if Path::new(&p.from).symlink_metadata().is_ok() {
        return Err(Reason::Occupied);
    }
    if !trashed.contains(&p.from) {
        return Err(Reason::NotInTrash);
    }
    Ok(())
}

/// Seconds its copy in Trash can have been deleted within: from the operation's start until it was
/// recorded, and before the next time the journal saw that path trashed. A copy outside isn't ours.
/// Too close to tell apart leaves an empty window, and nothing restored.
pub(crate) fn window(entries: &[Entry], e: &Entry, path: &str) -> Window {
    let lo = (e.started / 1000) as i64;
    let mut hi = (e.time / 1000) as i64 + 2;
    let k = key(path);
    let next = entries
        .iter()
        .filter(|x| (x.started, x.id) > (e.started, e.id) && (x.kind.trashes() || x.kind == Kind::Compare))
        .find(|x| x.pairs.iter().any(|p| p.done && p.to.is_empty() && key(&p.from) == k));
    if let Some(x) = next {
        hi = hi.min((x.started / 1000) as i64 - 1);
    }
    (path.to_string(), lo, hi)
}

/// The recorded pairs an undo still has to act on, in the order to do them: last change first.
fn targets(e: &Entry) -> Vec<&Pair> {
    let mut seen = HashSet::new();
    let done = e.pairs.iter().filter(|p| p.done && !p.undone);
    if e.kind.moves() {
        done.rev().collect()
    } else if e.kind.outputs() {
        // A compressed archive is the output of each of its sources.
        done.filter(|p| !p.to.is_empty() && seen.insert(p.to.as_str())).rev().collect()
    } else {
        done.filter(|p| !p.from.is_empty() && seen.insert(p.from.as_str())).rev().collect()
    }
}

fn action_of(k: Kind) -> Action {
    if k.moves() {
        Action::MoveBack
    } else if k.outputs() {
        Action::TrashOutput
    } else {
        Action::Restore
    }
}

/// What an undo of a later operation left at `p.to`. Coming back changes ctime by itself, so an item
/// that matches this and has the content first recorded isn't reported for its ctime.
fn baseline<'a>(entries: &'a [Entry], e: &Entry, p: &Pair) -> Option<&'a Fp> {
    let by_id: HashMap<u64, &Entry> = entries.iter().map(|x| (x.id, x)).collect();
    entries
        .iter()
        .rev()
        .filter(|x| (x.started, x.id) > (e.started, e.id) && x.undo_of.and_then(|o| by_id.get(&o)).is_some_and(|o| (o.started, o.id) > (e.started, e.id)))
        .find_map(|x| x.pairs.iter().find(|q| q.done && q.to == p.to).and_then(|q| q.fp.as_ref()))
}

fn check(entries: &[Entry], e: &Entry, p: &Pair, trashed: &HashSet<String>, env: &Env, tick: &dyn Fn()) -> Result<(), Reason> {
    allowed(&p.from, env)?;
    allowed(&p.to, env)?;
    let k = e.kind;
    match action_of(k) {
        Action::MoveBack => check_move(p, baseline(entries, e, p), tick),
        Action::TrashOutput => dest_check(p, baseline(entries, e, p), tick),
        Action::Restore => check_restore(p, trashed, env),
    }
}

fn windows(entries: &[Entry], e: &Entry) -> Vec<Window> {
    targets(e).iter().map(|p| window(entries, e, &p.from)).collect()
}

fn trashed_of(entries: &[Entry], e: &Entry, env: &Env) -> HashSet<String> {
    if !e.kind.trashes() || !env.can_restore {
        return HashSet::new();
    }
    (env.in_trash)(&windows(entries, e))
}

fn item_of(k: Kind, p: &Pair, reason: Option<Reason>) -> Item {
    match action_of(k) {
        Action::MoveBack => Item { action: Action::MoveBack, from: p.to.clone(), to: p.from.clone(), reason, newer: false },
        Action::TrashOutput => Item { action: Action::TrashOutput, from: p.to.clone(), to: String::new(), reason, newer: false },
        Action::Restore => Item { action: Action::Restore, from: String::new(), to: p.from.clone(), reason, newer: false },
    }
}

/// Marks items whose paths a later entry involves and lists those entries. An entry since undone in
/// full, and its undo, cancel out; a partly undone one still counts.
fn newer_of(entries: &[Entry], e: &Entry, items: &mut [Item]) -> Vec<Newer> {
    let mut exact: HashMap<String, Vec<usize>> = HashMap::new();
    let mut above: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, it) in items.iter().enumerate() {
        for p in [&it.from, &it.to] {
            if p.is_empty() {
                continue;
            }
            let k = key(p);
            exact.entry(k.clone()).or_default().push(i);
            for a in Path::new(&k).ancestors().skip(1) {
                above.entry(a.to_string_lossy().into_owned()).or_default().push(i);
            }
        }
    }
    let by_id: HashMap<u64, &Entry> = entries.iter().map(|x| (x.id, x)).collect();
    let cancelled = |x: &Entry| x.undone_by.is_some() || x.undo_of.and_then(|o| by_id.get(&o)).is_some_and(|o| o.undone_by.is_some() && (o.started, o.id) > (e.started, e.id));
    let mut hits: HashSet<usize> = HashSet::new();
    let mut out = vec![];
    for later in entries.iter().filter(|x| (x.started, x.id) > (e.started, e.id) && x.undo_of != Some(e.id) && !cancelled(x)) {
        let mut touched = false;
        for p in later.pairs.iter().filter(|p| p.done) {
            for side in [&p.from, &p.to] {
                if side.is_empty() {
                    continue;
                }
                let k = key(side);
                if let Some(v) = above.get(&k) {
                    touched = true;
                    hits.extend(v);
                }
                for a in Path::new(&k).ancestors() {
                    if let Some(v) = a.to_str().and_then(|s| exact.get(s)) {
                        touched = true;
                        hits.extend(v);
                    }
                }
            }
        }
        if touched && out.len() < 20 {
            out.push(Newer { id: later.id, time: later.time, kind: later.kind, label: later.label.clone() });
        }
    }
    for i in hits {
        items[i].newer = true;
    }
    out
}

pub fn preview(entries: &[Entry], e: &Entry, env: &Env, tick: &dyn Fn()) -> Preview {
    let b = blocked(e);
    let mut items = vec![];
    if b.is_none() {
        let trashed = trashed_of(entries, e, env);
        items = targets(e)
            .into_iter()
            .map(|p| {
                tick();
                item_of(e.kind, p, check(entries, e, p, &trashed, env, tick).err())
            })
            .collect();
    }
    let newer = newer_of(entries, e, &mut items);
    Preview { id: e.id, label: e.label.clone(), kind: e.kind, time: e.time, blocked: b, items, newer, total: e.total }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub from: String,
    pub to: String,
    /// `done`, `skipped` or `failed`.
    pub status: &'static str,
    pub reason: Option<Reason>,
    pub detail: Option<String>,
}

#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub outcomes: Vec<Outcome>,
    /// Something happened.
    pub applied: bool,
    pub undo_id: Option<u64>,
    /// The journal couldn't be saved.
    pub save_error: Option<String>,
}

/// Does the undo. `named` holds destinations the user agreed to touch although they changed.
pub fn run(entries: &[Entry], e: &Entry, named: &HashSet<String>, env: &Env) -> (Vec<Outcome>, Vec<Pair>) {
    let mut outcomes = vec![];
    let mut did = vec![];
    let trashed = trashed_of(entries, e, env);
    let mut to_restore: Vec<&Pair> = vec![];
    for p in targets(e) {
        let mut res = check(entries, e, p, &trashed, env, &|| {});
        if let Err(r) = res {
            if r.overridable() && named.contains(if action_of(e.kind) == Action::Restore { &p.from } else { &p.to }) {
                res = Ok(());
            }
        }
        let it = item_of(e.kind, p, None);
        if let Err(r) = res {
            outcomes.push(Outcome { from: it.from, to: it.to, status: "skipped", reason: Some(r), detail: None });
            continue;
        }
        let acted = match action_of(e.kind) {
            Action::MoveBack => fsops::move_all(&[(p.to.clone(), p.from.clone())]),
            Action::TrashOutput => (env.trash)(Path::new(&p.to)),
            Action::Restore => {
                to_restore.push(p);
                continue;
            }
        };
        match acted {
            Ok(()) => {
                did.push(p.clone());
                outcomes.push(Outcome { from: it.from, to: it.to, status: "done", reason: None, detail: None });
            }
            Err(d) => outcomes.push(Outcome { from: it.from, to: it.to, status: "failed", reason: None, detail: Some(d) }),
        }
    }
    if !to_restore.is_empty() {
        let wanted: Vec<Window> = to_restore.iter().map(|p| window(entries, e, &p.from)).collect();
        let results: HashMap<String, Result<(), String>> = (env.restore)(&wanted).into_iter().collect();
        for p in to_restore {
            match results.get(&p.from) {
                Some(Ok(())) => {
                    did.push(p.clone());
                    outcomes.push(Outcome { from: String::new(), to: p.from.clone(), status: "done", reason: None, detail: None });
                }
                Some(Err(d)) => outcomes.push(Outcome { from: String::new(), to: p.from.clone(), status: "failed", reason: None, detail: Some(d.clone()) }),
                None => outcomes.push(Outcome { from: String::new(), to: p.from.clone(), status: "failed", reason: Some(Reason::NotInTrash), detail: None }),
            }
        }
    }
    (outcomes, did)
}

/// The record of having undone `pairs` of `e`, with what stands now. Reads the disk unless `stat` is
/// false, so never call it holding the journal's lock.
pub fn undo_entry(e: &Entry, pairs: &[Pair], label: &str, complete: bool, stat: bool, tick: &dyn Fn()) -> Entry {
    let landed = |from: &str, to: &str| {
        tick();
        if stat {
            Pair { from: from.to_string(), to: to.to_string(), done: true, fp: fingerprint(Path::new(to), tick), from_real: real_to_store(from), to_real: real_to_store(to), ..Default::default() }
        } else {
            Pair { from: from.to_string(), to: to.to_string(), done: true, from_real: Some(String::new()), to_real: Some(String::new()), ..Default::default() }
        }
    };
    let (kind, new): (Kind, Vec<Pair>) = if e.kind.moves() {
        (e.kind, pairs.iter().map(|p| landed(&p.to, &p.from)).collect())
    } else if e.kind.outputs() {
        (Kind::Trash, pairs.iter().map(|p| Pair { from: p.to.clone(), to: String::new(), done: true, from_real: if stat { real_to_store(&p.to) } else { Some(String::new()) }, ..Default::default() }).collect())
    } else if e.kind.trashes() {
        (Kind::Restore, pairs.iter().map(|p| landed("", &p.from)).collect())
    } else {
        (e.kind, vec![])
    };
    Entry { id: 0, started: 0, time: 0, kind, label: label.to_string(), status: if complete { Status::Done } else { Status::Partial }, error: None, total: new.len() as u64, pairs: new, undone_by: None, undo_of: Some(e.id) }
}

/// These pairs were undone: note it per pair, add the record. Once all are, the entry is closed.
pub fn mark(store: &mut Store, id: u64, pairs: &[Pair], undo: Entry, record: bool) -> CmdResult<Option<u64>> {
    let Some(e) = store.get(id) else { return Err("journal-not-found".into()) };
    if e.undone_by.is_some() {
        return Err("journal-already-undone".into());
    }
    let kind = e.kind;
    let new_id = if record { Some(store.push(undo)) } else { None };
    if let Some(x) = store.entries.iter_mut().find(|x| x.id == id) {
        for p in x.pairs.iter_mut() {
            // One archive can stand for many pairs, and a file trashed twice over for two.
            let same = |d: &Pair| if kind.moves() { d.from == p.from && d.to == p.to } else if kind.outputs() { d.to == p.to } else { d.from == p.from };
            if pairs.iter().any(same) {
                p.undone = true;
            }
        }
        if x.pairs.iter().filter(|p| p.done).all(|p| p.undone) {
            // With the journal off no entry stands for it: 0.
            x.undone_by = Some(new_id.unwrap_or(0));
        }
    }
    Ok(new_id)
}
