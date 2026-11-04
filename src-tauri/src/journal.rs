//! Operations journal: a local record of finished file operations, for looking back ("where did
//! this file go") and for undo after a restart. See `store` for the file, `chain` for following
//! paths, `undo` for what undo checks.
mod chain;
mod store;
#[cfg(test)]
mod tests;
mod undo;

use crate::{guard, CmdResult};
use chain::{Chain, End};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use store::{clip, fingerprint, real_to_store, Entry, Kind, Pair, Status, Store, MAX_PAIRS};
use tauri::State;

#[derive(Default)]
struct Inner {
    store: Option<Store>,
    /// Undos running now: a second one for the same entry is refused.
    running: HashSet<u64>,
}

#[derive(Default)]
pub struct JournalState(Arc<Mutex<Inner>>);

fn file_path() -> CmdResult<PathBuf> {
    Ok(dirs::config_dir().ok_or("No config folder")?.join("morfa").join("journal.json"))
}

/// The open journal. A file that couldn't be read is tried again; until then nothing may touch it.
fn ready(g: &mut Inner) -> CmdResult<&mut Store> {
    let fresh = match &g.store {
        None => Some(Store::open(file_path()?)),
        Some(s) if s.blocked => Some(s.reopen()),
        Some(_) => None,
    };
    if let Some(s) = fresh {
        g.store = Some(s);
    }
    let s = g.store.as_mut().expect("just opened");
    if s.blocked {
        return Err("journal-unavailable".into());
    }
    Ok(s)
}

fn with<T>(inner: &Mutex<Inner>, f: impl FnOnce(&mut Store) -> CmdResult<T>) -> CmdResult<T> {
    let mut g = inner.lock().unwrap_or_else(|e| e.into_inner());
    f(ready(&mut g)?)
}

/// The cleanup tool's rule for what undo would touch. The item being acted on must also be removable;
/// a place to put something back normally doesn't exist, so its deepest existing folder is resolved
/// and only the static rules apply.
fn is_protected(p: &Path) -> bool {
    use crate::cleanup::{location_protection, protection, Protection};
    if p.symlink_metadata().is_ok() {
        return matches!(protection(p), Protection::Protected);
    }
    let (Some(dir), Some(name)) = (store::real_parent(p), p.file_name()) else { return true };
    matches!(location_protection(&Path::new(&dir).join(name)), Protection::Protected)
}

fn env_real() -> undo::Env<'static> {
    use crate::fsops::trashcan as tc;
    static TRASH: fn(&Path) -> CmdResult<()> = |p| tc::trash_blocking(&[p.to_string_lossy().into_owned()]);
    static PROTECTED: fn(&Path) -> bool = is_protected;
    undo::Env { trash: &TRASH, restore: &tc::restore_since, in_trash: &tc::in_trash, can_restore: tc::CAN_RESTORE, protected: &PROTECTED }
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RecordInput {
    kind: Kind,
    label: String,
    status: Status,
    #[serde(default)]
    error: Option<String>,
    pairs: Vec<(String, String)>,
    /// When the operation began (ms).
    #[serde(default)]
    started: u64,
}

/// Turns what the UI reports into pairs, reading what stands on disk. Whether a pair happened is what
/// the operation said, never guessed from the disk: `failed` means none did. Pairs of a stopped
/// operation without a destination never happened either, unless ending nowhere is the point.
/// `stat` false reads nothing at all, for a disk that stopped answering.
fn build(input: RecordInput, stat: bool, tick: &dyn Fn()) -> Entry {
    let total = input.pairs.len() as u64;
    let pairs: Vec<Pair> = input
        .pairs
        .into_iter()
        .take(MAX_PAIRS)
        .map(|(from, to)| {
            tick();
            let done = input.status != Status::Failed && (!to.is_empty() || input.kind.trashes() || matches!(input.kind, Kind::Delete | Kind::Compare));
            let fp = if stat && done && !to.is_empty() { fingerprint(Path::new(&to), tick) } else { None };
            let (from_real, to_real) = if stat { (real_to_store(&from), real_to_store(&to)) } else { (Some(String::new()), Some(String::new())) };
            Pair { from, to, done, fp, from_real, to_real, undone: false }
        })
        .collect();
    Entry { id: 0, started: input.started, time: 0, kind: input.kind, label: input.label, status: input.status, error: input.error, pairs, total, undone_by: None, undo_of: None }
}

/// Adds a finished operation. Returns its id.
#[tauri::command]
pub async fn journal_record(state: State<'_, JournalState>, input: RecordInput) -> CmdResult<u64> {
    let first = input.pairs.iter().find_map(|(f, t)| [t, f].into_iter().find(|p| !p.is_empty()).cloned()).unwrap_or_default();
    let inner = state.0.clone();
    // A mount that stopped answering costs the undo checks, not the record.
    let input_copy = input.clone();
    let stall = guard::stall_for(&first);
    let entry = guard::guarded(&first, stall, move |beat| Ok(build(input, true, &|| beat.tick()))).await.ok();
    tauri::async_runtime::spawn_blocking(move || {
        let entry = entry.unwrap_or_else(|| build(input_copy, false, &|| {}));
        with(&inner, |s| {
            let id = s.push(entry);
            s.save()?;
            Ok(id)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    #[serde(default)]
    search: String,
    #[serde(default)]
    kind: Option<Kind>,
    #[serde(default)]
    offset: usize,
    limit: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    id: u64,
    time: u64,
    kind: Kind,
    label: String,
    status: Status,
    error: Option<String>,
    total: u64,
    /// A few pairs, those matching the search first.
    sample: Vec<(String, String)>,
    undone: bool,
    /// Some pairs were undone, the rest can still be.
    partly_undone: bool,
    is_undo: bool,
    blocked: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResult {
    rows: Vec<Row>,
    matched: usize,
    /// All entries, whatever the filter.
    all: usize,
    set_aside: Option<String>,
}

fn row_of(e: &Entry, needle: &str) -> Row {
    let hits = |p: &&Pair| !needle.is_empty() && (p.from.to_lowercase().contains(needle) || p.to.to_lowercase().contains(needle));
    let mut sample: Vec<(String, String)> = e.pairs.iter().filter(hits).take(5).map(|p| (p.from.clone(), p.to.clone())).collect();
    if sample.is_empty() {
        sample = e.pairs.iter().take(5).map(|p| (p.from.clone(), p.to.clone())).collect();
    }
    Row { id: e.id, time: e.time, kind: e.kind, label: e.label.clone(), status: e.status, error: e.error.clone(), total: e.total, sample, undone: e.undone_by.is_some(), partly_undone: e.undone_by.is_none() && e.pairs.iter().any(|p| p.undone), is_undo: e.undo_of.is_some(), blocked: undo::blocked(e) }
}

fn list(s: &Store, q: &ListQuery) -> ListResult {
    let needle = q.search.trim().to_lowercase();
    let matched: Vec<&Entry> = s
        .entries
        .iter()
        .rev()
        .filter(|e| q.kind.is_none_or(|k| e.kind == k))
        .filter(|e| needle.is_empty() || e.label.to_lowercase().contains(&needle) || e.pairs.iter().any(|p| p.from.to_lowercase().contains(&needle) || p.to.to_lowercase().contains(&needle)))
        .collect();
    ListResult {
        matched: matched.len(),
        rows: matched.iter().skip(q.offset).take(q.limit.min(500)).map(|e| row_of(e, &needle)).collect(),
        all: s.entries.len(),
        set_aside: s.set_aside.as_ref().map(|p| p.to_string_lossy().into_owned()),
    }
}

#[tauri::command]
pub async fn journal_list(state: State<'_, JournalState>, query: ListQuery) -> CmdResult<ListResult> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || with(&inner, |s| Ok(list(s, &query)))).await.map_err(|e| e.to_string())?
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Now {
    Here,
    Moved,
    Gone,
    Trash,
    Deleted,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairNow {
    from: String,
    to: String,
    done: bool,
    /// Latest place the journal knows of.
    latest: String,
    now: Now,
}

fn locate(entries: &[Entry], e: &Entry, p: &Pair) -> PairNow {
    // A pair with no destination is followed from its own entry, which sent it to Trash.
    let at = entries.iter().position(|x| x.id == e.id).unwrap_or(0);
    let (start, skip) = if p.to.is_empty() { (&p.from, at) } else { (&p.to, at + 1) };
    let (latest, end) = if p.done { chain::follow(entries.iter().skip(skip), start) } else { (start.clone(), End::Located) };
    let now = match end {
        End::Trash => Now::Trash,
        End::Deleted => Now::Deleted,
        End::Located if Path::new(&latest).symlink_metadata().is_err() => Now::Gone,
        End::Located if &latest == start => Now::Here,
        End::Located => Now::Moved,
    };
    PairNow { from: p.from.clone(), to: p.to.clone(), done: p.done, latest, now }
}

/// Pairs of one entry, with where each is now.
#[tauri::command]
pub async fn journal_entry(state: State<'_, JournalState>, id: u64) -> CmdResult<Vec<PairNow>> {
    let inner = state.0.clone();
    let (entries, e) = tauri::async_runtime::spawn_blocking(move || with(&inner, |s| Ok((s.entries.clone(), s.get(id).cloned().ok_or("journal-not-found")?)))).await.map_err(|e| e.to_string())??;
    let first = e.pairs.first().map(|p| if p.to.is_empty() { p.from.clone() } else { p.to.clone() }).unwrap_or_default();
    guard::guarded(&first, guard::stall_for(&first), move |beat| {
        Ok(e.pairs
            .iter()
            .map(|p| {
                beat.tick();
                locate(&entries, &e, p)
            })
            .collect())
    })
    .await
}

/// Chains of moves and renames for a path or a name.
#[tauri::command]
pub async fn journal_where(state: State<'_, JournalState>, query: String) -> CmdResult<Vec<Chain>> {
    let inner = state.0.clone();
    let mut chains = tauri::async_runtime::spawn_blocking(move || with(&inner, |s| Ok(chain::find(&s.entries, &query)))).await.map_err(|e| e.to_string())??;
    let first = chains.first().map(|c| c.latest.clone()).unwrap_or_default();
    if first.is_empty() {
        return Ok(chains);
    }
    let stat = chains.clone();
    if let Ok(exists) = guard::guarded(&first, guard::stall_for(&first), move |beat| {
        Ok(stat
            .iter()
            .map(|c| {
                beat.tick();
                c.end == End::Located && Path::new(&c.latest).symlink_metadata().is_ok()
            })
            .collect::<Vec<bool>>())
    })
    .await
    {
        for (c, x) in chains.iter_mut().zip(exists) {
            c.exists = x;
        }
    }
    Ok(chains)
}

/// The dry run: what undo would do right now. Changes nothing.
#[tauri::command]
pub async fn journal_preview(state: State<'_, JournalState>, id: u64) -> CmdResult<undo::Preview> {
    let inner = state.0.clone();
    let (entries, e) = tauri::async_runtime::spawn_blocking(move || with(&inner, |s| Ok((s.entries.clone(), s.get(id).cloned().ok_or("journal-not-found")?)))).await.map_err(|e| e.to_string())??;
    let first = e.pairs.first().map(|p| if p.to.is_empty() { p.from.clone() } else { p.to.clone() }).unwrap_or_default();
    guard::guarded(&first, guard::stall_for(&first), move |beat| Ok(undo::preview(&entries, &e, &env_real(), &|| beat.tick()))).await
}

/// Claims the entry, does the undo without the lock held, marks the outcome. Nothing done: nothing
/// marked, so it can be tried again.
fn apply(inner: &Mutex<Inner>, id: u64, named: &HashSet<String>, label: &str, record: bool, env: &undo::Env) -> CmdResult<undo::Report> {
    // Checked and claimed under one lock, so two undos can't both get through.
    let (entries, e) = {
        let mut g = inner.lock().unwrap_or_else(|x| x.into_inner());
        let s = ready(&mut g)?;
        let e = s.get(id).cloned().ok_or("journal-not-found")?;
        let entries = s.entries.clone();
        if e.undone_by.is_some() || g.running.contains(&id) {
            return Err("journal-already-undone".into());
        }
        if let Some(why) = undo::blocked(&e) {
            return Err(format!("journal-blocked: {why}"));
        }
        g.running.insert(id);
        (entries, e)
    };
    let (outcomes, did) = undo::run(&entries, &e, named, env);
    let mut report = undo::Report { applied: !did.is_empty(), ..Default::default() };
    let complete = outcomes.iter().all(|o| o.status == "done") && e.total == e.pairs.len() as u64;
    report.outcomes = outcomes;
    let done = if did.is_empty() {
        Ok(())
    } else {
        // Reads the disk, so before taking the lock.
        let entry = undo::undo_entry(&e, &did, label, complete, true, &|| {});
        with(inner, |s| {
            report.undo_id = undo::mark(s, id, &did, entry, record)?;
            s.save()
        })
    };
    inner.lock().unwrap_or_else(|x| x.into_inner()).running.remove(&id);
    if let Err(err) = done {
        report.save_error = Some(clip(&err, 300));
    }
    Ok(report)
}

/// Runs the undo a preview described. `named`: paths that changed, which the user chose to include.
#[tauri::command]
pub async fn journal_apply(state: State<'_, JournalState>, id: u64, named: Vec<String>, label: String, record: bool) -> CmdResult<undo::Report> {
    let inner = state.0.clone();
    let check = inner.clone();
    let first = tauri::async_runtime::spawn_blocking(move || with(&check, |s| Ok(s.get(id).and_then(|e| e.pairs.first()).map(|p| if p.to.is_empty() { p.from.clone() } else { p.to.clone() })))).await.map_err(|e| e.to_string())??;
    if let Some(p) = first {
        guard::probe(&p).await?;
    }
    tauri::async_runtime::spawn_blocking(move || apply(&inner, id, &named.into_iter().collect(), &label, record, &env_real())).await.map_err(|e| e.to_string())?
}

/// Ctrl+Z undid this operation: journal undo must not apply it again.
#[tauri::command]
pub async fn journal_mark_undone(state: State<'_, JournalState>, id: u64, label: String, record: bool) -> CmdResult<()> {
    let inner = state.0.clone();
    let check = inner.clone();
    let e = tauri::async_runtime::spawn_blocking(move || with(&check, |s| Ok(s.get(id).cloned().ok_or("journal-not-found")?))).await.map_err(|e| e.to_string())??;
    let pairs: Vec<Pair> = e.pairs.iter().filter(|p| p.done && !p.undone).cloned().collect();
    let first = pairs.first().map(|p| if p.to.is_empty() { p.from.clone() } else { p.to.clone() }).unwrap_or_default();
    let (e2, p2, l2) = (e.clone(), pairs.clone(), label.clone());
    let entry = guard::guarded(&first, guard::stall_for(&first), move |beat| Ok(undo::undo_entry(&e2, &p2, &l2, true, true, &|| beat.tick()))).await.ok();
    tauri::async_runtime::spawn_blocking(move || {
        let entry = entry.unwrap_or_else(|| undo::undo_entry(&e, &pairs, &label, true, false, &|| {}));
        with(&inner, |s| {
            undo::mark(s, id, &pairs, entry, record)?;
            s.save()
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Works whatever state the file is in: that's when it's most wanted.
fn clear(inner: &Mutex<Inner>) -> CmdResult<()> {
    let mut g = inner.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(s) = ready(&mut g) {
        return s.clear();
    }
    let path = match &g.store {
        Some(s) => s.path().to_path_buf(),
        None => file_path()?,
    };
    g.store = Some(Store::wipe(path)?);
    Ok(())
}

#[tauri::command]
pub async fn journal_clear(state: State<'_, JournalState>) -> CmdResult<()> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || clear(&inner)).await.map_err(|e| e.to_string())?
}
