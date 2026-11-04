//! The worker that owns the index: starts walks, listens for changes and saves.
//!
//! A walk always builds a fresh tree on its own thread; searches keep using the old one until the
//! swap. Changes that arrive during a walk are remembered and replayed after, so none falls between.
//! Each start also re-walks, to catch what changed while the app was closed. After that:
//! - macOS: `notify` recursive (FSEvents), cheap whatever the tree size, excludes filtered here.
//! - Windows: `notify` recursive, plus a re-walk every few hours, as its change buffer can overflow.
//! - Linux: inotify needs a watch per folder and `notify`'s recursive mode would also watch
//!   `node_modules` and other mounts, so each indexed folder is watched individually. Past half of
//!   `max_user_watches`, or on any watch error, it stops watching and re-walks periodically instead.
use super::{apply, query::Query, store, tree::{self, Tree}, walk};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError, Sender},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant},
};

const TICK: Duration = Duration::from_millis(250);
/// Changes settle this long before being applied.
const SETTLE: Duration = Duration::from_millis(1500);
const SAVE_EVERY: Duration = Duration::from_secs(300);
const POLL_LIMITED: Duration = Duration::from_secs(30 * 60);
const POLL_WINDOWS: Duration = Duration::from_secs(6 * 3600);
/// More pending changes than this: a re-walk is cheaper.
const MAX_DIRTY: usize = 100_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reason {
    Off,
    Building,
    NotCovered,
    Gitignore,
}
impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Off => "off",
            Reason::Building => "building",
            Reason::NotCovered => "notCovered",
            Reason::Gitignore => "gitignore",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub roots: Vec<String>,
    pub excludes: Vec<String>,
}

#[derive(Serialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// off | building | ready | cancelled
    pub state: &'static str,
    /// A walk is running behind a usable index.
    pub refreshing: bool,
    pub paused: bool,
    pub entries: u64,
    /// Entries seen by the running walk.
    pub seen: u64,
    pub disk_bytes: u64,
    pub memory_bytes: u64,
    pub built_ms: u64,
    pub roots: Vec<String>,
    /// Asked for but not usable: network, excluded or missing.
    pub skipped: Vec<String>,
    /// live | periodic: whether changes arrive as they happen.
    pub watch: &'static str,
    pub truncated: bool,
}

pub struct Found {
    /// (as shown under the pane's path, as indexed).
    pub paths: Vec<(PathBuf, PathBuf)>,
    pub total: u64,
    pub scanned: u64,
}

enum Msg {
    Configure(Option<Request>),
    Rebuild,
    Cancel,
    Delete(Sender<()>),
    Resolved { epoch: u64, rules: Arc<walk::Rules>, roots: Vec<(PathBuf, u64)>, skipped: Vec<String>, spec: [u8; 32] },
    Loaded { epoch: u64, tree: Tree, disk: u64 },
    Built { epoch: u64, tree: Option<Tree>, ms: u64 },
    /// Changed paths; `deep` when created or renamed, so what's below them can't be trusted either.
    Fs { paths: Vec<PathBuf>, deep: bool },
    Rescan,
    WatchFailed,
    #[cfg_attr(not(test), allow(dead_code))]
    Quit,
}

pub struct Engine {
    tree: Arc<RwLock<Option<Tree>>>,
    status: Mutex<Status>,
    tx: Mutex<Sender<Msg>>,
    paused: Arc<AtomicBool>,
    /// A walk stops as soon as this isn't the value it started with.
    epoch: Arc<AtomicU64>,
    seen: Arc<AtomicU64>,
    dir: PathBuf,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Engine {
    pub fn start(dir: PathBuf, private: Vec<PathBuf>, emit: Box<dyn Fn(&Status) + Send>) -> Arc<Engine> {
        let (tx, rx) = mpsc::channel();
        let e = Arc::new(Engine {
            tree: Arc::default(),
            status: Mutex::new(Status { state: "off", watch: "live", ..Default::default() }),
            tx: Mutex::new(tx.clone()),
            paused: Arc::default(),
            epoch: Arc::default(),
            seen: Arc::default(),
            dir: dir.clone(),
        });
        let worker = Worker { e: e.clone(), rx, tx, dir, private, cancelled: false, deep: BTreeSet::new(), emit, req: None, spec: None, rules: None, roots: vec![], watcher: None, dirty: BTreeSet::new(), last_fs: None, overflow: false, walking: false, changed: false, saved_at: Instant::now(), next_poll: None, caught_up: false, started: Instant::now(), emitted: 0 };
        std::thread::spawn(move || worker.run());
        e
    }

    fn send(&self, m: Msg) {
        let _ = lock(&self.tx).send(m);
    }
    pub fn configure(&self, r: Option<Request>) {
        if r.is_none() {
            self.erase();
        }
        self.send(Msg::Configure(r));
    }
    /// What must happen even if the worker is stuck on a dying disk: stop the walk, stop answering,
    /// remove the file. The worker repeats it when it gets there, in case it was mid-save.
    fn erase(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        store::delete(&self.dir);
        if let Ok(mut t) = self.tree.try_write() {
            *t = None;
        }
        lock(&self.status).state = "off";
    }
    pub fn rebuild(&self) {
        self.send(Msg::Rebuild);
    }
    pub fn cancel(&self) {
        // Right away: a walk must not need the worker's attention to stop.
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.send(Msg::Cancel);
    }
    pub fn pause(&self, p: bool) {
        self.paused.store(p, Ordering::SeqCst);
        lock(&self.status).paused = p;
    }
    pub fn delete_now(&self) {
        self.erase();
        let (tx, rx) = mpsc::channel();
        self.send(Msg::Delete(tx));
        let _ = rx.recv_timeout(Duration::from_secs(10));
    }
    /// Results that turned out not to exist any more.
    pub fn stale(&self, paths: Vec<PathBuf>) {
        if !paths.is_empty() {
            self.send(Msg::Fs { paths, deep: false });
        }
    }
    pub fn status(&self) -> Status {
        let mut s = lock(&self.status).clone();
        s.seen = self.seen.load(Ordering::Relaxed);
        s
    }
    #[cfg(test)]
    pub fn quit(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.send(Msg::Quit);
    }

    pub fn ready(&self) -> Result<(), Reason> {
        match lock(&self.status).state {
            "ready" => Ok(()),
            "building" => Err(Reason::Building),
            _ => Err(Reason::Off),
        }
    }

    /// Is this real path held as a folder in the index? Excluded folders and other mounts aren't.
    pub fn covers(&self, real: &Path) -> Result<(), Reason> {
        let g = self.tree.read().unwrap_or_else(|e| e.into_inner());
        let t = g.as_ref().ok_or(Reason::Building)?;
        if t.truncated || !t.lookup(real).is_some_and(|id| t.is_dir(id)) {
            return Err(Reason::NotCovered);
        }
        Ok(())
    }

    /// `Ok(None)` when cancelled. Paths are built under `pane` so rows match what the pane shows,
    /// even when the pane got here through a symlink.
    pub fn query(&self, real: &Path, pane: &Path, text: &str, hidden: bool, cap: usize, cancelled: &dyn Fn() -> bool) -> Result<Option<Found>, Reason> {
        self.ready()?;
        let q = Query::parse(text);
        let g = self.tree.read().unwrap_or_else(|e| e.into_inner());
        let t = g.as_ref().ok_or(Reason::Building)?;
        let scope = t.lookup(real).filter(|&id| t.is_dir(id)).ok_or(Reason::NotCovered)?;
        let Some(hits) = t.search(&q, scope, hidden, cancelled) else { return Ok(None) };
        let total = hits.len() as u64;
        let paths = tree::rank(hits, cap)
            .into_iter()
            .map(|h| {
                let rel = t.rel_path(h.id, scope);
                (pane.join(&rel), real.join(&rel))
            })
            .collect();
        Ok(Some(Found { paths, total, scanned: t.live_entries() as u64 }))
    }
}

struct Worker {
    e: Arc<Engine>,
    rx: mpsc::Receiver<Msg>,
    tx: Sender<Msg>,
    dir: PathBuf,
    /// The app's own cache folders: never indexed, whatever the roots.
    private: Vec<PathBuf>,
    /// The user stopped the walk: nothing starts another until they ask.
    cancelled: bool,
    /// The part of `dirty` that was created or renamed.
    deep: BTreeSet<PathBuf>,
    emit: Box<dyn Fn(&Status) + Send>,
    req: Option<Request>,
    spec: Option<[u8; 32]>,
    rules: Option<Arc<walk::Rules>>,
    roots: Vec<(PathBuf, u64)>,
    watcher: Option<RecommendedWatcher>,
    dirty: BTreeSet<PathBuf>,
    last_fs: Option<Instant>,
    overflow: bool,
    walking: bool,
    changed: bool,
    saved_at: Instant,
    next_poll: Option<Instant>,
    caught_up: bool,
    started: Instant,
    emitted: u64,
}

fn resolve(req: &Request, home: Option<PathBuf>) -> (Vec<PathBuf>, Vec<String>) {
    let asked: Vec<String> = if req.roots.is_empty() { home.iter().map(|h| h.to_string_lossy().into_owned()).collect() } else { req.roots.clone() };
    let (mut roots, mut skipped) = (Vec::<PathBuf>::new(), vec![]);
    for a in asked {
        let p = a.clone();
        // A dead mount mustn't stall this; a network one isn't indexed at all.
        let real = if crate::netfs::network_mount_of(Path::new(&a)).is_some() {
            None
        } else {
            crate::guard::run_blocking(&a, crate::guard::quick_for(&a), move |_| {
                let path = Path::new(&p);
                if crate::netfs::is_network_at(path) || !path.is_dir() {
                    return Ok(None);
                }
                Ok(super::real_path(path).ok())
            })
            .ok()
            .flatten()
        };
        match real {
            Some(r) => roots.push(r),
            None => skipped.push(a),
        }
    }
    roots.sort();
    roots.dedup();
    // Inside another root: already covered, and walking it twice would double every result.
    let all = roots.clone();
    roots.retain(|r| !all.iter().any(|o| o != r && r.starts_with(o)));
    (roots, skipped)
}

impl Worker {
    fn run(mut self) {
        walk::lower_thread_priority();
        loop {
            // With the index off nothing is due: sleep until told otherwise.
            let got = if self.req.is_none() && !self.walking { self.rx.recv().map_err(|_| RecvTimeoutError::Disconnected) } else { self.rx.recv_timeout(TICK) };
            match got {
                Ok(Msg::Quit) => return,
                Ok(m) => self.on(m),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.housekeeping();
        }
    }

    fn set(&self, f: impl FnOnce(&mut Status)) {
        f(&mut lock(&self.e.status));
    }
    fn publish(&mut self) {
        let mut s = self.e.status();
        if let Some(t) = self.e.tree.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
            s.entries = t.live_entries() as u64;
            s.memory_bytes = t.heap_bytes() as u64;
            s.truncated = t.truncated;
        } else {
            s.entries = 0;
            s.memory_bytes = 0;
        }
        lock(&self.e.status).entries = s.entries;
        lock(&self.e.status).memory_bytes = s.memory_bytes;
        lock(&self.e.status).truncated = s.truncated;
        self.emitted = s.seen;
        (self.emit)(&s);
    }

    fn set_tree(&self, t: Option<Tree>) {
        *self.e.tree.write().unwrap_or_else(|e| e.into_inner()) = t;
    }

    fn start_run(&mut self, use_saved: bool) {
        let Some(req) = self.req.clone() else { return };
        let epoch = self.e.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.walking = true;
        self.cancelled = false;
        self.started = Instant::now();
        self.e.seen.store(0, Ordering::Relaxed);
        self.e.paused.store(false, Ordering::SeqCst);
        let has_tree = self.e.tree.read().unwrap_or_else(|e| e.into_inner()).is_some();
        self.set(|s| {
            s.paused = false;
            s.refreshing = has_tree;
            s.state = if has_tree { "ready" } else { "building" };
        });
        self.publish();
        let (tx, flag, paused, seen, dir, private) = (self.tx.clone(), self.e.epoch.clone(), self.e.paused.clone(), self.e.seen.clone(), self.dir.clone(), self.private.clone());
        std::thread::spawn(move || {
            walk::lower_thread_priority();
            let home = dirs::home_dir().and_then(|h| super::real_path(&h).ok());
            let (real, skipped) = resolve(&req, home.clone());
            let mut mounts = vec![];
            let table = crate::netfs::table();
            for m in table {
                mounts.push(PathBuf::from(m.path));
            }
            let mut rules = walk::Rules::new(&req.excludes, home.as_deref(), &real, mounts);
            for p in private.iter().chain([&dir]) {
                rules.exclude_path(p);
                if let Ok(r) = super::real_path(p) {
                    rules.exclude_path(&r);
                }
            }
            let (mut ok, mut skipped) = (Vec::new(), skipped);
            for r in real {
                match std::fs::metadata(&r) {
                    Ok(m) if !rules.excluded(&r) => ok.push((r, walk::dev_of(&m))),
                    _ => skipped.push(r.to_string_lossy().into_owned()),
                }
            }
            let names: Vec<PathBuf> = ok.iter().map(|(r, _)| r.clone()).collect();
            let spec = store::spec_hash(&names, &req.excludes);
            let rules = Arc::new(rules);
            let _ = tx.send(Msg::Resolved { epoch, rules: rules.clone(), roots: ok.clone(), skipped, spec });
            if use_saved {
                match store::load(&dir, &spec) {
                    Ok(tree) => {
                        let disk = std::fs::metadata(store::file_in(&dir)).map_or(0, |m| m.len());
                        let _ = tx.send(Msg::Loaded { epoch, tree, disk });
                    }
                    // Corrupt, or names kept under settings that have since changed: not to linger.
                    Err(_) if flag.load(Ordering::SeqCst) == epoch => store::delete(&dir),
                    Err(_) => {}
                }
            }
            let t = Instant::now();
            let stop = || flag.load(Ordering::SeqCst) != epoch;
            let is_paused = || paused.load(Ordering::SeqCst);
            let tree = walk::build(&ok, &rules, &walk::Ctl { stop: &stop, paused: &is_paused, seen: &seen, throttle: true });
            let _ = tx.send(Msg::Built { epoch, tree, ms: t.elapsed().as_millis() as u64 });
        });
    }

    fn current(&self, epoch: u64) -> bool {
        self.e.epoch.load(Ordering::SeqCst) == epoch
    }

    fn drop_everything(&mut self) {
        self.watcher = None;
        self.set_tree(None);
        self.rules = None;
        self.spec = None;
        self.roots.clear();
        self.dirty.clear();
        self.deep.clear();
        self.last_fs = None;
        self.overflow = false;
        self.walking = false;
        self.cancelled = false;
        self.changed = false;
        self.next_poll = None;
        store::delete(&self.dir);
        self.e.seen.store(0, Ordering::Relaxed);
        self.e.paused.store(false, Ordering::SeqCst);
        self.set(|s| *s = Status { state: "off", watch: "live", ..Default::default() });
    }

    fn on(&mut self, m: Msg) {
        match m {
            Msg::Configure(None) => {
                self.e.epoch.fetch_add(1, Ordering::SeqCst);
                self.req = None;
                self.drop_everything();
                self.publish();
            }
            Msg::Configure(Some(r)) => {
                if self.req.as_ref() != Some(&r) {
                    if self.req.is_some() {
                        // Other roots or excludes: names now left out must not stay on disk.
                        self.e.epoch.fetch_add(1, Ordering::SeqCst);
                        store::delete(&self.dir);
                    }
                    self.watcher = None;
                    self.set_tree(None);
                    self.dirty.clear();
                    self.deep.clear();
                    self.last_fs = None;
                    self.caught_up = false;
                    self.req = Some(r);
                    self.start_run(true);
                }
            }
            Msg::Rebuild => self.start_run(false),
            Msg::Cancel => {
                self.e.epoch.fetch_add(1, Ordering::SeqCst);
                self.walking = false;
                self.cancelled = true;
                self.overflow = false;
                self.next_poll = None;
                let has = self.e.tree.read().unwrap_or_else(|e| e.into_inner()).is_some();
                self.set(|s| {
                    s.refreshing = false;
                    s.paused = false;
                    s.state = if has { "ready" } else { "cancelled" };
                });
                self.e.paused.store(false, Ordering::SeqCst);
                self.publish();
            }
            Msg::Delete(done) => {
                self.e.epoch.fetch_add(1, Ordering::SeqCst);
                self.req = None;
                self.drop_everything();
                self.publish();
                let _ = done.send(());
            }
            Msg::Resolved { epoch, rules, roots, skipped, spec } if self.current(epoch) => {
                self.rules = Some(rules);
                self.spec = Some(spec);
                self.roots = roots;
                let shown = self.roots.iter().map(|(r, _)| r.to_string_lossy().into_owned()).collect();
                self.set(|s| {
                    s.roots = shown;
                    s.skipped = skipped;
                });
                if !cfg!(target_os = "linux") && self.watcher.is_none() {
                    self.watch_recursive();
                }
                self.publish();
            }
            Msg::Loaded { epoch, tree, disk } if self.current(epoch) => {
                self.set_tree(Some(tree));
                self.set(|s| {
                    s.state = "ready";
                    s.refreshing = true;
                    s.disk_bytes = disk;
                });
                self.publish();
            }
            Msg::Built { epoch, tree, ms } if self.current(epoch) => {
                self.walking = false;
                let Some(tree) = tree else { return };
                self.set_tree(Some(tree));
                self.flush();
                self.save();
                if cfg!(target_os = "linux") {
                    self.watch_each_dir();
                }
                self.next_poll = self.poll_interval().map(|d| Instant::now() + d);
                self.caught_up = true;
                self.publish(); // counts first: whoever sees "ready" sees them too
                self.set(|s| {
                    s.state = "ready";
                    s.refreshing = false;
                    s.built_ms = ms;
                });
                self.publish();
            }
            Msg::Fs { paths, deep } => {
                // Excluded churn (browser caches, build folders) ends here, whatever the watcher sends.
                let rules = self.rules.clone();
                let before = self.dirty.len();
                let roots = &self.roots;
                let kept: Vec<PathBuf> = paths.into_iter().filter(|p| rules.as_ref().is_none_or(|r| roots.iter().any(|(root, _)| r.allows(p, root)))).collect();
                if deep {
                    self.deep.extend(kept.iter().cloned());
                }
                self.dirty.extend(kept);
                // From the first change, not the last: a folder that never goes quiet must still flush.
                if self.dirty.len() > before && self.last_fs.is_none() {
                    self.last_fs = Some(Instant::now());
                }
                if self.dirty.len() > MAX_DIRTY {
                    self.dirty.clear();
                    self.deep.clear();
                    self.last_fs = None;
                    self.overflow = !self.cancelled;
                }
            }
            Msg::Rescan => self.overflow = !self.cancelled,
            Msg::WatchFailed => {
                self.watcher = None;
                self.set(|s| s.watch = "periodic");
                self.next_poll = Some(Instant::now() + POLL_LIMITED);
                self.publish();
            }
            _ => {} // from a walk that's been superseded
        }
    }

    fn poll_interval(&self) -> Option<Duration> {
        if lock(&self.e.status).watch == "periodic" {
            Some(POLL_LIMITED)
        } else if cfg!(windows) {
            Some(POLL_WINDOWS)
        } else if cfg!(target_os = "linux") {
            // Linux's first walk runs before its watches exist: one catch-up for that window.
            (!self.caught_up).then_some(POLL_LIMITED)
        } else {
            None
        }
    }

    fn sender(&self) -> impl Fn(notify::Result<notify::Event>) + Send + 'static {
        let tx = self.tx.clone();
        move |res: notify::Result<notify::Event>| match res {
            Ok(ev) if ev.need_rescan() => {
                let _ = tx.send(Msg::Rescan);
            }
            Ok(ev) if !matches!(ev.kind, notify::EventKind::Access(_)) => {
                use notify::{event::ModifyKind, EventKind};
                // Content and metadata changes leave what's below a path alone. So does Modify(Any), all Windows
                // says of a folder when a child changes; its renames are reported as such.
                let deep = !matches!(ev.kind, EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_) | ModifyKind::Any));
                let _ = tx.send(Msg::Fs { paths: ev.paths, deep });
            }
            Ok(_) => {}
            Err(_) => {
                let _ = tx.send(Msg::WatchFailed);
            }
        }
    }

    fn watch_recursive(&mut self) {
        let Ok(mut w) = notify::recommended_watcher(self.sender()) else {
            return self.on(Msg::WatchFailed);
        };
        for (r, _) in &self.roots {
            if w.watch(r, RecursiveMode::Recursive).is_err() {
                return self.on(Msg::WatchFailed);
            }
        }
        self.watcher = Some(w);
        self.set(|s| s.watch = "live");
    }

    fn watch_each_dir(&mut self) {
        self.watcher = None; // the re-walk may have found new folders, or the limit been raised
        let dirs: Vec<PathBuf> = self.e.tree.read().unwrap_or_else(|e| e.into_inner()).as_ref().map_or(vec![], |t| t.dirs().collect());
        if !fits_watch_budget(dirs.len(), inotify_limit()) {
            return self.on(Msg::WatchFailed);
        }
        let Ok(mut w) = notify::recommended_watcher(self.sender()) else {
            return self.on(Msg::WatchFailed);
        };
        for d in &dirs {
            if w.watch(d, RecursiveMode::NonRecursive).is_err() && d.is_dir() {
                return self.on(Msg::WatchFailed); // ENOSPC, most likely; another app took the rest
            }
        }
        self.watcher = Some(w);
        self.set(|s| s.watch = "live");
    }

    /// Apply what's pending. New folders need watches of their own on Linux.
    fn flush(&mut self) {
        let Some(rules) = self.rules.clone() else { return };
        if self.dirty.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.dirty);
        let deep = std::mem::take(&mut self.deep);
        let (flag, epoch) = (self.e.epoch.clone(), self.e.epoch.load(Ordering::SeqCst));
        apply::apply(&self.e.tree, &rules, &batch, &deep, &move || flag.load(Ordering::SeqCst) != epoch);
        if let (true, Some(w)) = (cfg!(target_os = "linux"), self.watcher.as_mut()) {
            // A tree moved in brings folders no event named.
            let new_dirs: Vec<PathBuf> = {
                let g = self.e.tree.read().unwrap_or_else(|e| e.into_inner());
                g.as_ref().map_or(vec![], |t| batch.iter().flat_map(|p| t.dirs_under(p)).collect())
            };
            let mut failed = false;
            for d in &new_dirs {
                failed |= w.watch(d, RecursiveMode::NonRecursive).is_err();
            }
            if failed {
                self.on(Msg::WatchFailed);
            }
        }
        self.changed = true;
        self.last_fs = None;
    }

    fn save(&mut self) {
        let Some(spec) = self.spec else { return };
        let size = {
            let g = self.e.tree.read().unwrap_or_else(|e| e.into_inner());
            g.as_ref().and_then(|t| store::save(t, &spec, &self.dir).ok())
        };
        if let Some(size) = size {
            self.set(|s| s.disk_bytes = size);
        }
        self.saved_at = Instant::now();
        self.changed = false;
    }

    fn housekeeping(&mut self) {
        let seen = self.e.seen.load(Ordering::Relaxed);
        if self.walking && seen != self.emitted {
            self.publish();
        }
        if self.walking || self.req.is_none() {
            return;
        }
        if self.overflow {
            self.overflow = false;
            return self.start_run(false);
        }
        if self.next_poll.is_some_and(|t| Instant::now() >= t) {
            self.next_poll = None;
            return self.start_run(false);
        }
        if self.last_fs.is_some_and(|t| t.elapsed() >= SETTLE) {
            self.flush();
            self.publish();
        }
        if self.changed && self.saved_at.elapsed() >= SAVE_EVERY {
            self.save();
            self.publish();
        }
    }
}

/// Half the limit at most: the rest belongs to the pane watcher and every other app.
pub fn fits_watch_budget(dirs: usize, limit: Option<usize>) -> bool {
    limit.is_none_or(|l| dirs <= l / 2)
}

fn inotify_limit() -> Option<usize> {
    std::fs::read_to_string("/proc/sys/fs/inotify/max_user_watches").ok()?.trim().parse().ok()
}
