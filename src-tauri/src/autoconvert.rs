//! Watched folders: apply a saved recipe to new files, only while the app is open. Rules exist and
//! run only if the user made them. The watcher here is separate from the panes'. See `core`.
mod complete;
mod core;
mod filter;
mod ledger;
mod rules;
#[cfg(test)]
mod tests;

use self::core::{scan, Core, Overview, ScanOut, ViaFn};
use crate::{convert, guard, CmdResult};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rules::{PathEnv, Recipe, RealEnv};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Emitter, State};

const TICK: Duration = Duration::from_millis(500);
/// A private profile, so a watched office conversion doesn't collide with the dialog's workers.
const OFFICE_WORKER: usize = 64;

pub struct AutoState {
    core: Arc<Mutex<Core>>,
    dirty: Arc<Mutex<HashSet<String>>>,
}

impl Default for AutoState {
    fn default() -> Self {
        let dir = dirs::config_dir().map(|d| d.join("morfa"));
        Self { core: Arc::new(Mutex::new(Core::load(dir))), dirty: Default::default() }
    }
}

fn lock(core: &Mutex<Core>) -> std::sync::MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn real_via() -> Arc<ViaFn> {
    Arc::new(|e: &str, f: &str| convert::routes::detected(|a| convert::routes::route_or_same(e, f, a)))
}

/// The folder as it stands now, on a guarded thread: a rule's folder can turn into a mount, or die.
fn scan_rule(s: &self::core::ScanSpec, via: Arc<ViaFn>) -> Result<ScanOut, String> {
    let env = RealEnv;
    let i = env.inspect(Path::new(&s.folder));
    if i.network {
        return Err("network".into());
    }
    if i.dead {
        return Err("unavailable".into());
    }
    let Some(real) = i.real.filter(|_| i.is_dir) else { return Err("missing".into()) };
    if env.protected(&real) {
        return Err("protected".into());
    }
    let (spec, sub, own, sweep) = (s.spec.clone(), s.subfolders, s.own.clone(), s.sweep.clone());
    guard::run_blocking(&s.folder, guard::stall_for(&s.folder), move |beat| {
        if let Some(out_dir) = sweep {
            self::core::sweep_temps(&real);
            if let Some(d) = out_dir {
                self::core::sweep_temps(Path::new(&d));
            }
        }
        scan(&real, sub, &spec, &own, &*via, &|| beat.tick())
    }).map_err(|e| if e == "tooMany" || e == "missing" { e } else { "unavailable".to_string() })
}

/// Keeps one watcher per live rule. A failed watch is fine: the periodic rescan still finds files.
fn reconcile(core: &Mutex<Core>, dirty: &Arc<Mutex<HashSet<String>>>, watchers: &mut HashMap<String, (String, bool, RecommendedWatcher)>) {
    let want = lock(core).watch_set();
    watchers.retain(|id, (folder, sub, _)| want.iter().any(|(i, f, s)| i == id && f == folder && s == sub));
    for (id, folder, sub) in want {
        if watchers.contains_key(&id) {
            continue;
        }
        let (d, rid) = (dirty.clone(), id.clone());
        let made = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok_and(|ev| !matches!(ev.kind, notify::EventKind::Access(_))) {
                d.lock().unwrap_or_else(|e| e.into_inner()).insert(rid.clone());
            }
        });
        let Ok(mut w) = made else { continue };
        let mode = if sub { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
        if w.watch(Path::new(&folder), mode).is_ok() {
            watchers.insert(id, (folder, sub, w));
        }
    }
}

/// Runs only while there are rules: without any, no thread wakes up at all.
fn ensure_driver(app: &AppHandle, state: &AutoState) {
    let mut c = lock(&state.core);
    if c.driver_on || !c.wants_driver() {
        return;
    }
    c.driver_on = true;
    start_driver(app.clone(), state.core.clone(), state.dirty.clone());
}

/// Called as the app exits: the engine's child doesn't outlive us if we get the chance.
pub fn shutdown(state: &AutoState) {
    if !lock(&state.core).quit() {
        return;
    }
    for _ in 0..30 {
        if !lock(&state.core).is_running() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn discard(p: &Path) -> bool {
    trash::delete(p).is_ok()
}

/// Entries of files that are gone, on a guarded thread; if it can't finish, nothing is dropped.
fn sweep_ledger(core: &Arc<Mutex<Core>>, now: u64) {
    let Some(paths) = lock(core).ledger_paths_if_tight(now) else { return };
    let gone = guard::run_keyed("autoconvert-ledger", "ledger", Duration::from_secs(60), guard::LIMITS, move |beat| {
        Ok(paths
            .into_iter()
            .filter(|p| {
                beat.tick();
                matches!(Path::new(p).symlink_metadata(), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
            })
            .collect::<HashSet<String>>())
    });
    if let Ok(gone) = gone {
        lock(core).ledger_drop_gone(&|p| !gone.contains(p));
    }
}

fn start_driver(app: AppHandle, core: Arc<Mutex<Core>>, dirty: Arc<Mutex<HashSet<String>>>) {
    let via = real_via();
    let enc: Arc<self::core::Encoder> = Arc::new(|src, o, cancelled| convert::dispatch::convert_one(src, o, OFFICE_WORKER, cancelled, &|_| {}).map_err(|e| e.to_string()));
    std::thread::spawn(move || {
        let mut watchers = HashMap::new();
        let mut last: Option<Overview> = None;
        let mut last_try = 0u64;
        loop {
            std::thread::sleep(TICK);
            let now = now_ms();
            {
                let mut c = lock(&core);
                if !c.wants_driver() && !c.is_running() {
                    c.driver_on = false;
                    let (events, view) = (c.take_events(), c.overview());
                    drop(c);
                    events.into_iter().for_each(|e| drop(app.emit("autoconvert-batch", e)));
                    let _ = app.emit("autoconvert-state", &view);
                    return;
                }
                if c.is_standby() && now.saturating_sub(last_try) >= 5000 {
                    last_try = now;
                    c.try_take_over(&RealEnv);
                }
            }
            sweep_ledger(&core, now);
            let marked: Vec<String> = dirty.lock().unwrap_or_else(|e| e.into_inner()).drain().collect();
            let scans = {
                let mut c = lock(&core);
                marked.iter().for_each(|id| c.mark_dirty(id));
                c.due_scans(now)
            };
            for s in &scans {
                let res = scan_rule(s, via.clone());
                lock(&core).apply_scan(&s.rule_id, res, now_ms());
            }
            reconcile(&core, &dirty, &mut watchers);
            let job = lock(&core).next_job(now_ms());
            if let Some(job) = job {
                let (core, enc, via) = (core.clone(), enc.clone(), via.clone());
                std::thread::spawn(move || self::core::run_job(&core, job, &*enc, &*via, &discard, &now_ms));
            }
            let (events, view) = {
                let mut c = lock(&core);
                (c.take_events(), c.overview())
            };
            for e in events {
                let _ = app.emit("autoconvert-batch", e);
            }
            if last.as_ref() != Some(&view) {
                let _ = app.emit("autoconvert-state", &view);
                last = Some(view);
            }
        }
    });
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())
}

/// Sent at launch and whenever recipes change; the first call starts the engine.
#[tauri::command]
pub async fn autoconvert_sync(app: AppHandle, state: State<'_, AutoState>, recipes: Vec<Recipe>) -> CmdResult<Overview> {
    let core = state.core.clone();
    let view = blocking(move || {
        let mut c = lock(&core);
        c.sync_recipes(recipes, &RealEnv);
        c.overview()
    })
    .await?;
    ensure_driver(&app, &state);
    Ok(view)
}

#[tauri::command]
pub async fn autoconvert_add(app: AppHandle, state: State<'_, AutoState>, folder: String, recipe_id: String, subfolders: bool) -> CmdResult<Overview> {
    let core = state.core.clone();
    let (res, view) = blocking(move || {
        let mut c = lock(&core);
        let id = format!("{:x}-{:x}", now_ms(), std::process::id());
        let res = c.add(folder, recipe_id, subfolders, id, &RealEnv);
        (res, c.overview())
    })
    .await?;
    // Even a refusal may have taken over rules another window left: they must run, and show.
    ensure_driver(&app, &state);
    let _ = app.emit("autoconvert-state", &view);
    res.map(|_| view)
}

#[tauri::command]
pub async fn autoconvert_update(state: State<'_, AutoState>, id: String, recipe_id: Option<String>, subfolders: Option<bool>) -> CmdResult<Overview> {
    let core = state.core.clone();
    blocking(move || {
        let mut c = lock(&core);
        c.update(&id, recipe_id, subfolders, &RealEnv)?;
        Ok(c.overview())
    })
    .await?
}

#[tauri::command]
pub async fn autoconvert_enable(state: State<'_, AutoState>, id: String, enabled: bool) -> CmdResult<Overview> {
    let core = state.core.clone();
    blocking(move || {
        let mut c = lock(&core);
        c.set_enabled(&id, enabled, &RealEnv)?;
        Ok(c.overview())
    })
    .await?
}

/// Off the main thread: the lock may be waiting on a check of a folder that doesn't answer.
async fn quick(state: &AutoState, f: impl FnOnce(&mut Core) + Send + 'static) -> CmdResult<Overview> {
    let core = state.core.clone();
    blocking(move || {
        let mut c = lock(&core);
        f(&mut c);
        c.overview()
    })
    .await
}

#[tauri::command]
pub async fn autoconvert_delete(state: State<'_, AutoState>, id: String) -> CmdResult<Overview> {
    quick(&state, move |c| c.delete(&id)).await
}

#[tauri::command]
pub async fn autoconvert_pause_all(state: State<'_, AutoState>, paused: bool) -> CmdResult<Overview> {
    quick(&state, move |c| c.set_paused(paused)).await
}

/// The answer to a rule paused for too many files.
#[tauri::command]
pub async fn autoconvert_resolve(state: State<'_, AutoState>, id: String, proceed: bool) -> CmdResult<Overview> {
    quick(&state, move |c| c.resolve(&id, proceed)).await
}

#[tauri::command]
pub async fn autoconvert_process_existing(state: State<'_, AutoState>, id: String) -> CmdResult<Overview> {
    quick(&state, move |c| c.process_existing(&id, now_ms())).await
}

#[tauri::command]
pub async fn autoconvert_cancel(state: State<'_, AutoState>, id: String) -> CmdResult<Overview> {
    quick(&state, move |c| c.cancel_rule(&id)).await
}

#[tauri::command]
pub async fn autoconvert_clear_failures(state: State<'_, AutoState>, id: String) -> CmdResult<Overview> {
    quick(&state, move |c| c.clear_failures(&id)).await
}
