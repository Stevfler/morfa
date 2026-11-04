//! The rules' state machine, free of threads and clocks so a test can drive it: scans go in, jobs
//! come out, one at a time. `run_job` holds the only code that touches a source file.
use super::{
    filter::{self, Spec},
    ledger::{self, Ledger},
    rules::{self, Failure, PathEnv, Recipe, Rule},
};
use crate::convert::{routes::Via, ConvertOpts};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

/// Conversions started per rule in a minute before it pauses and asks.
pub const RATE_PER_MIN: usize = 30;
/// Files waiting at once before a rule pauses and asks.
pub const MASS_ASK: usize = 100;
pub const MAX_ATTEMPTS: u32 = 3;
/// Looks at a file that won't open, a few seconds apart, before listing it as failed.
pub const MAX_OPEN_TRIES: u32 = 20;
const RETRY_MS: u64 = 30_000;
const MINUTE: u64 = 60_000;
const SCAN_BUSY_MS: u64 = 1000;
const SCAN_IDLE_MS: u64 = 30_000;
const SCAN_MAX: usize = 200_000;
const BATCH_FILES: usize = 20;
const BATCH_MS: u64 = 60_000;

pub type ViaFn = dyn Fn(&str, &str) -> Option<Via> + Send + Sync;
/// Converts one file; returns every file written. Cancelled by the closure.
pub type Encoder = dyn Fn(&Path, &ConvertOpts, &dyn Fn() -> bool) -> Result<Vec<PathBuf>, String> + Send + Sync;

#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
    /// Device and inode, where the OS gives them stably.
    pub id: String,
    /// Looks like an output of one of the rules: never offered as "already here".
    pub ours: bool,
}
impl Seen {
    fn path_key(&self, rule: &str) -> String {
        ledger::key(rule, &self.path, self.size, self.mtime)
    }
    fn id_key(&self, rule: &str) -> String {
        if self.id.is_empty() { String::new() } else { ledger::key(rule, &format!("#{}", self.id), self.size, self.mtime) }
    }
    fn produced_id(&self) -> String {
        output_id(&self.id, self.size, self.mtime)
    }
}
fn output_id(id: &str, size: u64, mtime: i64) -> String {
    if id.is_empty() { String::new() } else { format!("#{id}:{size}:{mtime}") }
}

/// A written file, as it stood when the job ended.
#[derive(Clone, Debug)]
pub struct Output {
    pub path: String,
    pub id: String,
    pub size: u64,
    pub mtime: i64,
}

/// What was last written from a source, to spot a source that was still being written.
struct Recent {
    size: u64,
    at: u64,
    outs: Vec<Output>,
}
const RECENT_MS: u64 = 5 * 60_000;

/// Files present at the baseline. Never shrunk by a scan's absence: an empty mountpoint or an
/// unreadable subfolder isn't evidence of anything.
#[derive(Clone)]
struct Base {
    size: u64,
    mtime: i64,
    id: String,
    /// May be proposed for the opt-in.
    offer: bool,
    /// In the latest complete scan.
    present: bool,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Hold {
    Mass,
    Rate,
}

struct Cand {
    seen: Seen,
    since: u64,
    /// Found cut off: looked at again from this time.
    retry_at: u64,
    incomplete: bool,
}
impl Cand {
    fn new(seen: Seen, since: u64) -> Self {
        Cand { seen, since, retry_at: 0, incomplete: false }
    }
}
struct Queued {
    seen: Seen,
    attempts: u32,
    not_before: u64,
}

#[derive(Default)]
struct Batch {
    started: u64,
    pairs: Vec<(String, String)>,
    failed: Vec<(String, String)>,
    cancelled: bool,
    done: u64,
}

/// A rule's runtime state; gone when it's disabled, which is what stops its queue.
#[derive(Default)]
struct Rt {
    baselined: bool,
    baseline: HashMap<String, Base>,
    /// Identities with their version, to know a renamed file. The value: may it be offered.
    ids: HashMap<String, bool>,
    cands: HashMap<String, Cand>,
    queue: VecDeque<Queued>,
    hold: Option<Hold>,
    approved: bool,
    approved_high: usize,
    starts: VecDeque<u64>,
    unavailable: Option<String>,
    no_engine: Option<String>,
    next_scan: u64,
    dirty: bool,
    batch: Batch,
    unopenable: HashMap<String, u32>,
    incomplete_tries: HashMap<String, u32>,
    recent: HashMap<String, Recent>,
}
impl Rt {
    /// Waiting to be worked on; cut-off files sit apart until they change.
    fn pending(&self) -> usize {
        self.cands.values().filter(|c| !c.incomplete).count() + self.queue.len()
    }
    fn remember(&mut self, s: &Seen, offer: bool, present: bool) {
        self.baseline.insert(s.path.clone(), Base { size: s.size, mtime: s.mtime, id: s.id.clone(), offer, present });
        if !s.id.is_empty() {
            self.ids.insert(s.produced_id(), offer);
        }
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BatchEvent {
    pub rule_id: String,
    pub recipe_name: String,
    pub pairs: Vec<(String, String)>,
    pub failed: Vec<(String, String)>,
    pub cancelled: bool,
    pub started: u64,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Running {
    pub file: String,
    pub done: u64,
    pub total: u64,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RuleView {
    pub id: String,
    pub folder: String,
    pub recipe_id: String,
    pub recipe_name: String,
    pub enabled: bool,
    pub subfolders: bool,
    pub flag: Option<String>,
    pub next_to_source: bool,
    pub unavailable: Option<String>,
    pub no_engine: Option<String>,
    pub hold: Option<Hold>,
    pub pending: usize,
    pub incomplete: usize,
    pub backlog: usize,
    pub last_run: Option<u64>,
    pub done: u64,
    pub failed: u64,
    pub failures: Vec<FailureView>,
    pub running: Option<Running>,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FailureView {
    pub path: String,
    pub error: String,
    pub at: u64,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub paused_all: bool,
    /// Another Morfa window runs the rules; this one only shows them.
    pub standby: bool,
    /// The record of past work overflowed, so the opt-in isn't offered.
    pub ledger_full: bool,
    pub rules: Vec<RuleView>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Saved {
    paused_all: bool,
    rules: Vec<Rule>,
}

pub struct ScanSpec {
    pub rule_id: String,
    pub folder: String,
    pub subfolders: bool,
    pub spec: Spec,
    /// Where each rule writes, for telling outputs apart.
    pub own: Vec<OwnOutput>,
    /// First look at this folder: clear what a dead process left. Also the recipe's output folder.
    pub sweep: Option<Option<String>>,
}

#[derive(Clone)]
pub struct OwnOutput {
    pub dir: String,
    pub deep: bool,
    pub format: String,
}

#[derive(Default)]
pub struct ScanOut {
    pub files: Vec<Seen>,
    pub no_engine: Option<String>,
    /// Some subfolder couldn't be read: absence means nothing this time.
    pub partial: bool,
}

pub struct Job {
    pub rule_id: String,
    pub seen: Seen,
    pub opts: ConvertOpts,
    pub cancel: Arc<AtomicBool>,
}

struct RunningJob {
    rule_id: String,
    path: String,
    cancel: Arc<AtomicBool>,
    user_cancelled: Arc<AtomicBool>,
}

pub enum Outcome {
    Written(Vec<Output>),
    /// Quiet, but cut off by its own format's account: keeps waiting, never failed.
    Incomplete,
    Failed(String),
    /// The file changed: back to waiting, it wasn't an attempt.
    NotReady,
    /// Unchanged but can't be opened: maybe still locked, so it waits, a limited number of times.
    NotOpenable,
    /// The source's folder went away mid-way: nothing happened to the file, do it when it returns.
    Gone,
    OutputGone,
    Cancelled,
}

pub struct Core {
    dir: Option<PathBuf>,
    pub rules: Vec<Rule>,
    rt: HashMap<String, Rt>,
    recipes: Vec<Recipe>,
    /// Nothing runs before the first list of recipes: without it every rule would look orphaned.
    synced: bool,
    pub paused_all: bool,
    ledger: Ledger,
    running: Option<RunningJob>,
    events: Vec<BatchEvent>,
    last_rule: usize,
    /// Holds the lock file: the OS releases it however this process ends.
    lock: Option<std::fs::File>,
    owner: bool,
    quitting: bool,
    pub driver_on: bool,
    pub last_sweep: u64,
    #[cfg(test)]
    ledger_cap: Option<usize>,
}

fn permanent(e: &str) -> bool {
    ["NO_ENGINE", "OVER_LIMIT", "LIMIT_IMAGES_ONLY", "BAD_TEMPLATE", "Can't convert", "Can't save"].iter().any(|p| e.starts_with(p))
}

fn spec_of(o: &ConvertOpts) -> Spec {
    Spec { format: o.format.clone(), next_to_source: o.out_dir.is_none(), limited: o.max_bytes.is_some() }
}

impl Core {
    /// `dir`: where rules and the ledger live; `None` keeps everything in memory. Nothing is
    /// written, and no lock taken, until there is a rule.
    pub fn load(dir: Option<PathBuf>) -> Self {
        let saved = Self::read_saved(dir.as_ref());
        let ledger = Ledger::open(dir.as_ref().map(|d| d.join("autoconvert-ledger.jsonl")));
        let mut c = Core { owner: dir.is_none(), dir, rules: saved.rules, rt: HashMap::new(), recipes: vec![], synced: false, paused_all: saved.paused_all, ledger, running: None, events: vec![], last_rule: 0, lock: None, quitting: false, driver_on: false, last_sweep: 0, #[cfg(test)] ledger_cap: None };
        if !c.rules.is_empty() {
            c.acquire();
        }
        c
    }

    #[cfg(test)]
    pub fn load_capped(dir: PathBuf, cap: usize) -> Self {
        let mut c = Self::load(Some(dir.clone()));
        c.ledger = Ledger::with_cap(Some(dir.join("autoconvert-ledger.jsonl")), cap);
        c.ledger_cap = Some(cap);
        c
    }

    fn read_saved(dir: Option<&PathBuf>) -> Saved {
        dir.map(|d| d.join("autoconvert.json"))
            .and_then(|f| {
                let text = std::fs::read_to_string(&f).ok()?;
                serde_json::from_str::<Saved>(&text)
                    .map_err(|e| {
                        // Kept for inspection; rules are off until someone creates them anyway.
                        eprintln!("autoconvert: unreadable rules: {e}");
                        let _ = std::fs::rename(&f, f.with_extension("json.bad"));
                    })
                    .ok()
            })
            .unwrap_or_default()
    }

    /// Only one process may run rules: two have separate records and can feed each other forever.
    fn acquire(&mut self) -> bool {
        if self.owner {
            return true;
        }
        let Some(d) = &self.dir else { return true };
        let _ = std::fs::create_dir_all(d);
        let Ok(f) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(d.join("autoconvert.lock")) else { return false };
        // A child being spawned holds a copy of the descriptor until its exec, for an instant.
        let mut tries = 0;
        while f.try_lock().is_err() {
            tries += 1;
            if tries >= 5 {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        self.lock = Some(f);
        self.owner = true;
        true
    }

    /// The other instance may have gone: take over, from what it left on disk.
    pub fn try_take_over(&mut self, env: &dyn PathEnv) -> bool {
        if self.owner || self.rules.is_empty() || !self.acquire() {
            return false;
        }
        let saved = Self::read_saved(self.dir.as_ref());
        self.rules = saved.rules;
        self.paused_all = saved.paused_all;
        self.ledger = Ledger::open(self.dir.as_ref().map(|d| d.join("autoconvert-ledger.jsonl")));
        self.rt.clear();
        if self.synced {
            self.sync_recipes(self.recipes.clone(), env);
        }
        true
    }

    pub fn is_standby(&self) -> bool {
        !self.owner && !self.rules.is_empty()
    }

    pub fn save(&self) {
        let Some(d) = &self.dir else { return };
        let f = d.join("autoconvert.json");
        if !self.owner || (self.rules.is_empty() && !self.paused_all && !f.exists()) {
            return;
        }
        let _ = std::fs::create_dir_all(d);
        let saved = Saved { paused_all: self.paused_all, rules: self.rules.clone() };
        let Ok(text) = serde_json::to_string_pretty(&saved) else { return };
        let tmp = d.join("autoconvert.json.tmp");
        if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &f)) {
            eprintln!("autoconvert: can't save rules: {e}");
        }
    }

    /// The app is closing: stop the engine's child now; the file isn't counted as dealt with.
    pub fn quit(&mut self) -> bool {
        self.quitting = true;
        if let Some(j) = &self.running {
            j.cancel.store(true, Ordering::SeqCst);
        }
        self.running.is_some()
    }
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn ledger_paths_if_tight(&mut self, now: u64) -> Option<Vec<String>> {
        if !self.owner || !self.ledger.wants_sweep() || now.saturating_sub(self.last_sweep) < 3_600_000 {
            return None;
        }
        self.last_sweep = now;
        Some(self.ledger.paths())
    }
    pub fn ledger_drop_gone(&mut self, live: &dyn Fn(&str) -> bool) {
        self.ledger.drop_gone(live);
    }

    pub fn recipe(&self, id: &str) -> Option<&Recipe> {
        self.recipes.iter().find(|r| r.id == id)
    }
    fn rule(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.id == id)
    }

    /// Drops the rule's watcher and queue; a conversion in flight is cancelled.
    fn stop(&mut self, id: &str) {
        self.flush(id);
        self.rt.remove(id);
        if let Some(j) = self.running.as_ref().filter(|j| j.rule_id == id) {
            j.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Takes the recipes as they are now. A rule whose recipe is gone, or which is refused, is
    /// switched off and flagged. Enabled rules start from a fresh baseline, once.
    pub fn sync_recipes(&mut self, recipes: Vec<Recipe>, env: &dyn PathEnv) {
        self.recipes = recipes;
        self.synced = true;
        if !self.owner {
            return;
        }
        let checks: Vec<(String, Result<(), &'static str>)> = self.rules.iter().filter(|r| r.enabled).map(|r| (r.id.clone(), rules::check_rule(&r.folder, r.subfolders, self.recipe(&r.recipe_id), env))).collect();
        for (id, res) in checks {
            match res {
                Err(code) if !rules::SOFT.contains(&code) => {
                    self.stop(&id);
                    if let Some(r) = self.rules.iter_mut().find(|r| r.id == id) {
                        r.enabled = false;
                        r.flag = Some(code.into());
                    }
                }
                _ => {
                    self.rt.entry(id).or_default();
                }
            }
        }
        for r in &mut self.rules {
            if !self.recipes.iter().any(|x| x.id == r.recipe_id) {
                if r.flag.is_none() {
                    r.flag = Some("recipeMissing".into());
                }
                continue;
            }
            if let Some(x) = self.recipes.iter().find(|x| x.id == r.recipe_id) {
                r.recipe_name.clone_from(&x.name);
            }
        }
        self.save();
    }

    /// A new rule is always off. Only a refusal keeps it from existing; a folder must be there now.
    pub fn add(&mut self, folder: String, recipe_id: String, subfolders: bool, id: String, env: &dyn PathEnv) -> Result<String, String> {
        rules::check_rule(&folder, subfolders, self.recipe(&recipe_id), env)?;
        if !self.acquire() {
            return Err("otherInstance".into());
        }
        let real = rules::check_folder(&folder, env)?;
        if self.rules.iter().any(|r| r.recipe_id == recipe_id && rules::check_folder(&r.folder, env).is_ok_and(|x| x == real)) {
            return Err("duplicate".into());
        }
        let recipe_name = self.recipe(&recipe_id).map(|r| r.name.clone()).unwrap_or_default();
        self.rules.push(Rule { id: id.clone(), folder, recipe_id, recipe_name, enabled: false, subfolders, flag: None, stats: Default::default() });
        self.save();
        Ok(id)
    }

    /// Changing what a rule does starts it over, baseline included.
    pub fn update(&mut self, id: &str, recipe_id: Option<String>, subfolders: Option<bool>, env: &dyn PathEnv) -> Result<(), String> {
        if !self.owner {
            return Err("otherInstance".into());
        }
        let r = self.rule(id).ok_or("gone")?;
        let (folder, recipe_id, subfolders, enabled) = (r.folder.clone(), recipe_id.unwrap_or_else(|| r.recipe_id.clone()), subfolders.unwrap_or(r.subfolders), r.enabled);
        match rules::check_rule(&folder, subfolders, self.recipe(&recipe_id), env) {
            Err(code) if !rules::SOFT.contains(&code) => return Err(code.into()),
            _ => {}
        }
        self.stop(id);
        let name = self.recipe(&recipe_id).map(|r| r.name.clone()).unwrap_or_default();
        if let Some(r) = self.rules.iter_mut().find(|r| r.id == id) {
            r.recipe_id = recipe_id;
            r.recipe_name = name;
            r.subfolders = subfolders;
            r.flag = None;
        }
        if enabled {
            self.rt.insert(id.to_string(), Rt::default());
        }
        self.save();
        Ok(())
    }

    pub fn set_enabled(&mut self, id: &str, on: bool, env: &dyn PathEnv) -> Result<(), String> {
        if !self.owner {
            return Err("otherInstance".into());
        }
        let r = self.rule(id).ok_or("gone")?;
        if on {
            match rules::check_rule(&r.folder, r.subfolders, self.recipe(&r.recipe_id), env) {
                Err(code) if !rules::SOFT.contains(&code) => return Err(code.into()),
                _ => {}
            }
        }
        self.stop(id);
        if on {
            self.rt.insert(id.to_string(), Rt::default());
        }
        if let Some(r) = self.rules.iter_mut().find(|r| r.id == id) {
            r.enabled = on;
            if on {
                r.flag = None;
            }
        }
        self.save();
        Ok(())
    }

    pub fn delete(&mut self, id: &str) {
        if !self.owner {
            return;
        }
        self.stop(id);
        self.rules.retain(|r| r.id != id);
        self.save();
    }

    pub fn set_paused(&mut self, paused: bool) {
        if !self.owner {
            return;
        }
        self.paused_all = paused;
        self.save();
    }

    pub fn clear_failures(&mut self, id: &str) {
        if let Some(r) = self.rules.iter_mut().find(|r| r.id == id) {
            r.stats.failures.clear();
            r.stats.failed = 0;
        }
        self.save();
    }

    /// The answer to a pause: go on at the capped rate, or leave the waiting files alone.
    pub fn resolve(&mut self, id: &str, proceed: bool) {
        let Some(rt) = self.rt.get_mut(id) else { return };
        rt.hold = None;
        if proceed {
            rt.approved = true;
            rt.approved_high = rt.pending();
        } else {
            let cands: Vec<Seen> = rt.cands.drain().map(|(_, c)| c.seen).collect();
            let queued: Vec<Seen> = rt.queue.drain(..).map(|q| q.seen).collect();
            for s in cands.iter().chain(&queued) {
                let offer = !s.ours;
                rt.remember(s, offer, true);
            }
            rt.approved = false;
            rt.approved_high = 0;
        }
    }

    /// The opt-in: what was in the folder at the baseline gets processed too.
    pub fn process_existing(&mut self, id: &str, now: u64) {
        if self.ledger.lossy() {
            return; // can't tell what was already dealt with: fail safe
        }
        let Some(rt) = self.rt.get_mut(id) else { return };
        let offered: Vec<String> = rt.baseline.iter().filter(|(_, b)| b.offer && b.present).map(|(p, _)| p.clone()).collect();
        for path in offered {
            if let Some(b) = rt.baseline.get_mut(&path) {
                b.offer = false;
                let seen = Seen { path: path.clone(), size: b.size, mtime: b.mtime, id: b.id.clone(), ours: false };
                rt.ids.remove(&seen.produced_id());
                rt.baseline.remove(&path);
                rt.cands.insert(path, Cand::new(seen, now));
            }
        }
        rt.approved = true;
        rt.approved_high = rt.pending();
        rt.dirty = true;
    }

    /// The op row's cancel: stops this file and what's waiting; those files aren't tried again.
    pub fn cancel_rule(&mut self, id: &str) {
        if let Some(j) = self.running.as_ref().filter(|j| j.rule_id == id) {
            j.user_cancelled.store(true, Ordering::SeqCst);
            j.cancel.store(true, Ordering::SeqCst);
        }
        self.resolve(id, false);
        if let Some(rt) = self.rt.get_mut(id) {
            rt.batch.cancelled = true;
        }
    }

    pub fn mark_dirty(&mut self, id: &str) {
        if let Some(rt) = self.rt.get_mut(id) {
            rt.dirty = true;
        }
    }

    /// Rules that want a watcher right now: id, folder, subfolders.
    pub fn watch_set(&self) -> Vec<(String, String, bool)> {
        if self.paused_all {
            return vec![];
        }
        self.rules.iter().filter(|r| r.enabled && self.rt.get(&r.id).is_some_and(|x| x.unavailable.is_none())).map(|r| (r.id.clone(), r.folder.clone(), r.subfolders)).collect()
    }

    pub fn due_scans(&mut self, now: u64) -> Vec<ScanSpec> {
        if !self.synced || self.paused_all || !self.owner || self.quitting {
            return vec![];
        }
        let own: Vec<OwnOutput> = self
            .rules
            .iter()
            .filter(|r| r.flag.is_none())
            .filter_map(|r| {
                let o = &self.recipes.iter().find(|x| x.id == r.recipe_id)?.opts;
                Some(OwnOutput { dir: o.out_dir.clone().unwrap_or_else(|| r.folder.clone()), deep: o.out_dir.is_none() && r.subfolders, format: o.format.clone() })
            })
            .collect();
        let mut out = vec![];
        for r in self.rules.iter().filter(|r| r.enabled) {
            let Some(rt) = self.rt.get_mut(&r.id) else { continue };
            let Some(recipe) = self.recipes.iter().find(|x| x.id == r.recipe_id) else { continue };
            // A held or unreachable rule looks again now and then, and no event hurries it.
            let eager = rt.dirty && rt.hold.is_none() && rt.unavailable.is_none();
            if eager || rt.next_scan <= now {
                rt.dirty = false;
                rt.next_scan = now + SCAN_IDLE_MS;
                let sweep = (!rt.baselined).then(|| recipe.opts.out_dir.clone());
                out.push(ScanSpec { rule_id: r.id.clone(), folder: r.folder.clone(), subfolders: r.subfolders, spec: spec_of(&recipe.opts), own: own.clone(), sweep });
            }
        }
        out
    }

    /// A scan's answer. The first one is the baseline: nothing in it is touched without an opt-in,
    /// known or not, so a record that was lost or trimmed can't start anything.
    pub fn apply_scan(&mut self, id: &str, res: Result<ScanOut, String>, now: u64) {
        let Some(rt) = self.rt.get_mut(id) else { return };
        let out = match res {
            Ok(out) => out,
            Err(code) => {
                rt.unavailable = Some(code);
                rt.next_scan = now + SCAN_IDLE_MS;
                return;
            }
        };
        rt.unavailable = None;
        rt.no_engine = out.no_engine;
        let ledger = &self.ledger;
        let running = self.running.as_ref().filter(|j| j.rule_id == id).map(|j| j.path.clone());
        let known = |s: &Seen| ledger.produced(&s.path) || (!s.id.is_empty() && ledger.produced(&s.produced_id())) || ledger.handled(&s.path_key(id)) || (!s.id.is_empty() && ledger.handled(&s.id_key(id)));
        if !rt.baselined {
            rt.baselined = true;
            for s in &out.files {
                let offer = !known(s) && !s.ours;
                rt.remember(s, offer, true);
            }
            return;
        }
        if !out.partial {
            rt.baseline.values_mut().for_each(|b| b.present = false);
            let here: HashSet<&str> = out.files.iter().map(|s| s.path.as_str()).collect();
            rt.cands.retain(|p, _| here.contains(p.as_str()));
            rt.queue.retain(|q| here.contains(q.seen.path.as_str()));
            // Only churn beyond any folder's size is let go of, and never what's here now.
            if rt.baseline.len() > 2 * SCAN_MAX {
                rt.baseline.retain(|p, _| here.contains(p.as_str()));
            }
            if rt.ids.len() > 4 * SCAN_MAX {
                rt.ids.clear();
                let keep: Vec<(String, bool)> = rt.baseline.values().filter(|b| !b.id.is_empty()).map(|b| (output_id(&b.id, b.size, b.mtime), b.offer)).collect();
                rt.ids.extend(keep);
            }
        }
        let queued: HashMap<String, (u64, i64)> = rt.queue.iter().map(|q| (q.seen.path.clone(), (q.seen.size, q.seen.mtime))).collect();
        let mut stale: HashSet<String> = HashSet::new();
        for s in out.files {
            if let Some(b) = rt.baseline.get_mut(&s.path) {
                if (b.size, b.mtime) == (s.size, s.mtime) {
                    b.present = true;
                    continue;
                }
            }
            if let Some(offer) = (!s.id.is_empty()).then(|| rt.ids.get(&s.produced_id()).copied()).flatten() {
                rt.remember(&s, offer, true); // renamed, same contents
                continue;
            }
            if known(&s) {
                rt.remember(&s, false, true);
                continue;
            }
            if queued.get(&s.path) == Some(&(s.size, s.mtime)) || running.as_deref() == Some(s.path.as_str()) {
                continue;
            }
            if queued.contains_key(&s.path) {
                stale.insert(s.path.clone());
            }
            rt.baseline.remove(&s.path);
            let prev = rt.cands.get(&s.path).map(|c| (c.seen.size, c.seen.mtime, c.since));
            let since = filter::observed_since(prev, s.size, s.mtime, now);
            let held = rt.cands.get(&s.path).filter(|c| c.since == since).map(|c| (c.retry_at, c.incomplete));
            let (retry_at, incomplete) = held.unwrap_or((0, false));
            if held.is_none() {
                rt.incomplete_tries.remove(&s.path);
            }
            let path = s.path.clone();
            rt.cands.insert(path, Cand { seen: s, since, retry_at, incomplete });
        }
        if !stale.is_empty() {
            rt.queue.retain(|q| !stale.contains(&q.seen.path));
        }
        if rt.hold.is_none() && rt.pending() >= rt.approved_high + MASS_ASK {
            rt.hold = Some(Hold::Mass);
        }
        if rt.hold.is_none() {
            let mut ready: Vec<String> = rt.cands.iter().filter(|(p, c)| now >= c.retry_at && filter::is_stable(c.since, filter::quiet_for(p), now)).map(|(p, _)| p.clone()).collect();
            ready.sort();
            for p in ready {
                if let Some(c) = rt.cands.remove(&p) {
                    rt.queue.push_back(Queued { seen: c.seen, attempts: 0, not_before: 0 });
                }
            }
            // Look again when something could be ready, not sooner.
            let soonest = rt.cands.iter().map(|(p, c)| if c.incomplete { c.retry_at } else { c.since + filter::quiet_for(p) }).min();
            if let Some(at) = soonest {
                rt.next_scan = at.clamp(now + SCAN_BUSY_MS, now + SCAN_IDLE_MS);
            }
        }
        self.settle(id, now);
    }

    /// The next conversion, one at a time across all rules. Checked here and not when queued: a job
    /// that just ended has recorded its outputs by now, so they can't slip in.
    pub fn next_job(&mut self, now: u64) -> Option<Job> {
        if self.running.is_some() || self.paused_all || !self.owner || self.quitting {
            return None;
        }
        let n = self.rules.len();
        for i in 0..n {
            let at = (self.last_rule + 1 + i) % n;
            let r = &self.rules[at];
            if !r.enabled {
                continue;
            }
            let id = r.id.clone();
            let Some(recipe) = self.recipes.iter().find(|x| x.id == r.recipe_id) else { continue };
            let opts = recipe.opts.clone();
            let Some(rt) = self.rt.get_mut(&id) else { continue };
            if rt.hold.is_some() || rt.unavailable.is_some() {
                continue;
            }
            rt.queue.retain(|q| !self.ledger.produced(&q.seen.path) && !(!q.seen.id.is_empty() && self.ledger.produced(&q.seen.produced_id())));
            let Some(pos) = rt.queue.iter().position(|q| q.not_before <= now) else { continue };
            while rt.starts.front().is_some_and(|t| now.saturating_sub(*t) >= MINUTE) {
                rt.starts.pop_front();
            }
            if rt.starts.len() >= RATE_PER_MIN {
                if !rt.approved {
                    rt.hold = Some(Hold::Rate);
                }
                continue;
            }
            let q = rt.queue.get(pos)?;
            let seen = q.seen.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            self.running = Some(RunningJob { rule_id: id.clone(), path: seen.path.clone(), cancel: cancel.clone(), user_cancelled: Arc::new(AtomicBool::new(false)) });
            self.last_rule = at;
            return Some(Job { rule_id: id, seen, opts, cancel });
        }
        None
    }

    /// The job passed its checks: from here on it counts, against the cap and in the ledger. Marked
    /// before converting, so a crash mid-file can't turn into a crash on every launch.
    pub fn began(&mut self, job: &Job, now: u64) {
        self.ledger.mark_handled(&job.seen.path_key(&job.rule_id), &job.seen.id_key(&job.rule_id));
        if let Some(rt) = self.rt.get_mut(&job.rule_id) {
            rt.starts.push_back(now);
            if rt.batch.pairs.is_empty() && rt.batch.failed.is_empty() {
                rt.batch.started = now;
            }
        }
    }

    /// What an earlier version of this source produced, if this looks like the same file having
    /// been converted while still being written: recently, and it has grown since.
    pub fn superseded(&mut self, job: &Job, now: u64) -> Vec<Output> {
        let Some(rt) = self.rt.get_mut(&job.rule_id) else { return vec![] };
        rt.recent.retain(|_, r| now.saturating_sub(r.at) < RECENT_MS);
        match rt.recent.remove(&job.seen.path) {
            Some(r) if job.seen.size > r.size => r.outs,
            _ => vec![],
        }
    }

    pub fn finish(&mut self, job: &Job, outcome: Outcome, now: u64) {
        let user_cancelled = self.running.as_ref().is_some_and(|j| j.user_cancelled.load(Ordering::SeqCst));
        self.running = None;
        let id = job.rule_id.as_str();
        let key = job.seen.path_key(id);
        let quitting = self.quitting;
        let path = job.seen.path.clone();
        // Outputs are ours whatever became of the rule meanwhile.
        if let Outcome::Written(outs) = &outcome {
            for o in outs {
                self.ledger.mark_produced(&o.path, &output_id(&o.id, o.size, o.mtime));
            }
        }
        let (Some(rt), Some(rule)) = (self.rt.get_mut(id), self.rules.iter_mut().find(|r| r.id == id)) else { return };
        let take = |rt: &mut Rt| rt.queue.iter().position(|q| q.seen.path == path).and_then(|i| rt.queue.remove(i));
        match outcome {
            Outcome::Written(outs) => {
                take(rt);
                rt.remember(&job.seen, false, true);
                rule.stats.done += 1;
                rule.stats.last_run = Some(now);
                rt.batch.done += 1;
                rt.batch.pairs.extend(outs.iter().map(|o| (path.clone(), o.path.clone())));
                rt.recent.insert(path.clone(), Recent { size: job.seen.size, at: now, outs });
            }
            Outcome::Incomplete => {
                let tries = rt.incomplete_tries.entry(path.clone()).or_default();
                *tries = (*tries + 1).min(7);
                let wait = (filter::STABLE_MS << *tries).min(5 * MINUTE);
                if let Some(q) = take(rt) {
                    rt.cands.insert(path.clone(), Cand { seen: q.seen, since: now, retry_at: now + wait, incomplete: true });
                }
            }
            Outcome::NotReady => {
                if let Some(q) = take(rt) {
                    rt.cands.insert(path.clone(), Cand::new(q.seen, now));
                }
                rt.dirty = true;
            }
            Outcome::NotOpenable => {
                let tries = rt.unopenable.entry(path.clone()).or_default();
                *tries += 1;
                if *tries >= MAX_OPEN_TRIES {
                    rt.unopenable.remove(&path);
                    take(rt);
                    self.ledger.mark_handled(&key, &job.seen.id_key(id));
                    rt.remember(&job.seen, false, true);
                    rule.stats.failed += 1;
                    rule.stats.last_run = Some(now);
                    let error = "CANT_OPEN".to_string();
                    rt.batch.failed.push((path.clone(), error.clone()));
                    rule.stats.failures.insert(0, Failure { path, error, at: now });
                    rule.stats.failures.truncate(rules::FAILURES_KEPT);
                } else if let Some(q) = take(rt) {
                    rt.cands.insert(path.clone(), Cand::new(q.seen, now));
                    rt.dirty = true;
                }
            }
            Outcome::Gone => {
                take(rt);
                self.ledger.forget(&key);
                rt.dirty = true;
            }
            Outcome::OutputGone => {
                rt.unavailable = Some("outputMissing".into());
                rt.next_scan = now + SCAN_IDLE_MS;
            }
            Outcome::Cancelled => {
                take(rt);
                if quitting {
                    self.ledger.forget(&key);
                }
                if user_cancelled {
                    rt.batch.cancelled = true;
                }
            }
            Outcome::Failed(error) => {
                let retry = !permanent(&error) && rt.queue.iter().any(|q| q.seen.path == path && q.attempts + 1 < MAX_ATTEMPTS);
                if retry {
                    if let Some(q) = rt.queue.iter_mut().find(|q| q.seen.path == path) {
                        q.attempts += 1;
                        q.not_before = now + RETRY_MS * q.attempts as u64;
                    }
                } else {
                    take(rt);
                    rt.remember(&job.seen, false, true);
                    rule.stats.failed += 1;
                    rule.stats.last_run = Some(now);
                    rt.batch.failed.push((path.clone(), error.clone()));
                    rule.stats.failures.insert(0, Failure { path, error, at: now });
                    rule.stats.failures.truncate(rules::FAILURES_KEPT);
                }
            }
        }
        self.settle(id, now);
        self.save();
    }

    /// Reports a batch when it's big enough, old enough, or there's nothing left.
    fn settle(&mut self, id: &str, now: u64) {
        let Some(rt) = self.rt.get_mut(id) else { return };
        let some = !rt.batch.pairs.is_empty() || !rt.batch.failed.is_empty() || rt.batch.cancelled;
        let idle = rt.pending() == 0 && !self.running.as_ref().is_some_and(|j| j.rule_id == id);
        if idle {
            rt.approved = false;
            rt.approved_high = 0;
        }
        if some && (idle || rt.batch.pairs.len() + rt.batch.failed.len() >= BATCH_FILES || now.saturating_sub(rt.batch.started) >= BATCH_MS) {
            self.flush(id);
        }
    }

    fn flush(&mut self, id: &str) {
        let Some(rt) = self.rt.get_mut(id) else { return };
        let b = std::mem::take(&mut rt.batch);
        if b.pairs.is_empty() && b.failed.is_empty() && !b.cancelled {
            return;
        }
        let recipe_name = self.rules.iter().find(|r| r.id == id).map(|r| r.recipe_name.clone()).unwrap_or_default();
        self.events.push(BatchEvent { rule_id: id.into(), recipe_name, pairs: b.pairs, failed: b.failed, cancelled: b.cancelled, started: b.started });
    }

    pub fn take_events(&mut self) -> Vec<BatchEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn overview(&self) -> Overview {
        let rules = self.rules.iter().map(|r| {
            let rt = self.rt.get(&r.id);
            let running = self.running.as_ref().filter(|j| j.rule_id == r.id).map(|j| {
                let done = rt.map_or(0, |x| x.batch.done);
                Running { file: j.path.clone(), done, total: done + rt.map_or(1, |x| x.pending().max(1)) as u64 }
            });
            RuleView {
                id: r.id.clone(),
                folder: r.folder.clone(),
                recipe_id: r.recipe_id.clone(),
                recipe_name: r.recipe_name.clone(),
                enabled: r.enabled,
                subfolders: r.subfolders,
                flag: r.flag.clone(),
                next_to_source: self.recipe(&r.recipe_id).is_some_and(|x| x.opts.out_dir.is_none()),
                unavailable: rt.and_then(|x| x.unavailable.clone()),
                no_engine: rt.and_then(|x| x.no_engine.clone()),
                hold: rt.and_then(|x| x.hold),
                pending: rt.map_or(0, Rt::pending),
                incomplete: rt.map_or(0, |x| x.cands.values().filter(|c| c.incomplete).count()),
                backlog: if self.ledger.lossy() { 0 } else { rt.map_or(0, |x| x.baseline.values().filter(|b| b.offer && b.present).count()) },
                last_run: r.stats.last_run,
                done: r.stats.done,
                failed: r.stats.failed,
                failures: r.stats.failures.iter().map(|f| FailureView { path: f.path.clone(), error: f.error.clone(), at: f.at }).collect(),
                running,
            }
        });
        Overview { paused_all: self.paused_all, standby: self.is_standby(), ledger_full: self.ledger.lossy(), rules: rules.collect() }
    }
}

/// Everything the rule may take in its folder. Doesn't follow symlinks or leave the volume.
pub fn scan(root: &Path, subfolders: bool, spec: &Spec, own: &[OwnOutput], via: &ViaFn, tick: &dyn Fn()) -> Result<ScanOut, String> {
    let own: Vec<(PathBuf, bool, String)> = own.iter().map(|o| (std::fs::canonicalize(&o.dir).unwrap_or_else(|_| PathBuf::from(&o.dir)), o.deep, filter::fold_ext(&o.format))).collect();
    let mut out = ScanOut::default();
    let mut dirs = vec![(root.to_path_buf(), 0u32)];
    let root_dev = dev_of(&std::fs::metadata(root).map_err(|_| "missing".to_string())?);
    let mut count = 0usize;
    while let Some((dir, depth)) = dirs.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            if depth == 0 {
                return Err("missing".into());
            }
            out.partial = true;
            continue;
        };
        for entry in rd {
            let Ok(entry) = entry else {
                out.partial = true;
                continue;
            };
            tick();
            count += 1;
            if count > SCAN_MAX {
                return Err("tooMany".into());
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(meta) = entry.path().symlink_metadata() else {
                out.partial = true;
                continue;
            };
            if meta.is_dir() {
                if subfolders && depth < 32 && !filter::ignored_dir(&name) && !crate::entry::is_hidden_attr(&meta) && dev_of(&meta) == root_dev {
                    dirs.push((entry.path(), depth + 1));
                }
                continue;
            }
            if !meta.is_file() || filter::ignored_name(&name) || crate::entry::is_hidden_attr(&meta) {
                continue;
            }
            let path = entry.path();
            if !filter::eligible(&path, spec, via) {
                if out.no_engine.is_none() {
                    let e = path.extension().map(|e| crate::convert::routes::norm(&e.to_string_lossy())).unwrap_or_default();
                    let fmt = crate::convert::routes::norm(&spec.format);
                    let m = crate::convert::routes::missing(&e, &fmt);
                    if m.starts_with("NO_ENGINE") && !(spec.next_to_source && filter::fold_ext(&e) == filter::fold_ext(&fmt)) {
                        out.no_engine = Some(m);
                    }
                }
                continue;
            }
            // An empty file is a placeholder: no version yet. It's seen once it has content.
            if meta.len() == 0 {
                continue;
            }
            let ext = path.extension().map(|e| filter::fold_ext(&e.to_string_lossy())).unwrap_or_default();
            let ours = filter::named_like_output(&path) || own.iter().any(|(d, deep, f)| *f == ext && path.parent().is_some_and(|p| p == d || (*deep && p.starts_with(d))));
            out.files.push(Seen { path: path.to_string_lossy().into_owned(), size: meta.len(), mtime: mtime_of(&meta), id: id_of(&meta), ours });
        }
    }
    Ok(out)
}

fn mtime_of(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64)
}

/// A file's identity across renames. Windows has no stable std API for it: there a rename is a new file.
#[cfg(unix)]
fn id_of(m: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!("{}:{}", m.dev(), m.ino())
}
#[cfg(not(unix))]
fn id_of(_: &std::fs::Metadata) -> String {
    String::new()
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only asks whether the process exists.
    unsafe { libc::kill(pid as i32, 0) == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) }
}
#[cfg(not(unix))]
fn pid_alive(_: u32) -> bool {
    true // can't tell cheaply: the age limit alone would decide, so nothing is swept here
}

/// Removes temp files of ours whose process is gone, in this one folder, by exact name only.
pub fn sweep_temps(dir: &Path) -> usize {
    const MIN_AGE: std::time::Duration = std::time::Duration::from_secs(600);
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut gone = 0;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(pid) = filter::temp_owner_pid(&name) else { continue };
        let Ok(m) = e.path().symlink_metadata() else { continue };
        let old = m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a >= MIN_AGE);
        if m.is_file() && old && pid != std::process::id() && !pid_alive(pid) && std::fs::remove_file(e.path()).is_ok() {
            gone += 1;
        }
    }
    gone
}

/// The source turned out to have been unfinished when `prior` was made from it. Unless someone has
/// touched those outputs since, they go to the Trash, and the good output takes the original name:
/// nothing is overwritten, and a failed step just leaves the suffixed name.
fn replace_damaged(prior: Vec<Output>, outs: Vec<Output>, discard: &dyn Fn(&Path) -> bool) -> Vec<Output> {
    let untouched = |o: &Output| Path::new(&o.path).symlink_metadata().is_ok_and(|m| m.len() == o.size && mtime_of(&m) == o.mtime && id_of(&m) == o.id);
    if prior.is_empty() || !prior.iter().all(untouched) || !prior.iter().all(|o| discard(Path::new(&o.path))) {
        return outs;
    }
    if let ([old], [new]) = (prior.as_slice(), outs.as_slice()) {
        // A hard link fails rather than replace; the same volume, as they share a folder rule.
        if std::fs::hard_link(&new.path, &old.path).is_ok() && std::fs::remove_file(&new.path).is_ok() {
            return vec![Output { path: old.path.clone(), ..new.clone() }];
        }
    }
    outs
}

#[cfg(unix)]
fn dev_of(m: &std::fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::dev(m)
}
#[cfg(not(unix))]
fn dev_of(_: &std::fs::Metadata) -> u64 {
    0
}

/// Runs a job start to finish, outside the core's lock. Sources are only ever read. `discard` moves
/// an output of ours that turned out damaged to the Trash.
pub fn run_job(core: &Mutex<Core>, job: Job, enc: &Encoder, via: &ViaFn, discard: &dyn Fn(&Path) -> bool, now: &dyn Fn() -> u64) {
    let lock = || core.lock().unwrap_or_else(|e| e.into_inner());
    let src = PathBuf::from(&job.seen.path);
    let (expected, out_dir) = ((job.seen.size, job.seen.mtime), job.opts.out_dir.clone());
    let probe_src = src.clone();
    let probed = crate::guard::run_blocking(&job.seen.path, crate::guard::stall_for(&job.seen.path), move |beat| {
        if out_dir.as_deref().is_some_and(|d| !Path::new(d).is_dir()) {
            return Ok(Some(Outcome::OutputGone));
        }
        let Ok(meta) = probe_src.symlink_metadata() else { return Ok(Some(Outcome::Gone)) };
        if (meta.len(), mtime_of(&meta)) != expected || !meta.is_file() {
            return Ok(Some(Outcome::NotReady));
        }
        if std::fs::File::open(&probe_src).is_err() {
            return Ok(Some(Outcome::NotOpenable));
        }
        let ext = probe_src.extension().map(|e| filter::fold_ext(&e.to_string_lossy())).unwrap_or_default();
        let whole = super::complete::check(&probe_src, &ext, &|| beat.tick());
        Ok((whole == super::complete::Verdict::Incomplete).then_some(Outcome::Incomplete))
    });
    let early = match probed {
        Ok(o) => o,
        Err(_) => Some(Outcome::Gone),
    };
    if let Some(o) = early {
        return lock().finish(&job, o, now());
    }
    lock().began(&job, now());
    let ext = src.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let opts = filter::gate(&job.opts, &ext, via(&crate::convert::routes::norm(&ext), &crate::convert::routes::norm(&job.opts.format)));
    let flag = job.cancel.clone();
    let res = enc(&src, &opts, &move || flag.load(Ordering::SeqCst));
    let outcome = match res {
        Ok(outs) => {
            // Compared with what scans report, which is resolved: `/var` and `/private/var` are one folder.
            let outs: Vec<Output> = outs
                .into_iter()
                .map(|o| {
                    let path = std::fs::canonicalize(&o).unwrap_or(o);
                    let m = path.symlink_metadata().ok();
                    Output { path: path.to_string_lossy().into_owned(), id: m.as_ref().map_or(String::new(), id_of), size: m.as_ref().map_or(0, |m| m.len()), mtime: m.as_ref().map_or(0, mtime_of) }
                })
                .collect();
            let at = now();
            let prior = lock().superseded(&job, at);
            Outcome::Written(replace_damaged(prior, outs, discard))
        }
        Err(_) if job.cancel.load(Ordering::SeqCst) => Outcome::Cancelled,
        Err(_) if !src.exists() => Outcome::Gone,
        Err(e) => Outcome::Failed(e),
    };
    lock().finish(&job, outcome, now());
}
