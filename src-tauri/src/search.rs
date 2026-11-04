use crate::{
    entry::{entry_from, Entry},
    CmdResult,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter, State};

const MAX_RESULTS: usize = 5000;

#[derive(Default)]
pub struct SearchState(Arc<AtomicU64>);

#[derive(Serialize, Clone)]
struct SearchBatch {
    id: u64,
    entries: Vec<Entry>,
    done: bool,
    scanned: u64,
    /// "index", or "live" with why the index wasn't used.
    source: &'static str,
    reason: Option<&'static str>,
    /// Matches in all, when known: the index can count past the cap.
    total: Option<u64>,
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn search(app: AppHandle, state: State<'_, SearchState>, index: State<'_, crate::findex::IndexState>, root: String, query: String, hidden: bool, gitignore: Option<bool>, id: u64) -> CmdResult<u64> {
    if id == 0 {
        return Err("search id 0 is reserved".into());
    }
    // The id comes from the frontend so it can claim batches before this returns.
    // Storing it cancels any older search; 0 is reserved for "cancelled".
    let gen = state.0.clone();
    let parsed = crate::findex::Query::parse(&query);
    gen.store(id, Ordering::SeqCst);
    // A root that doesn't answer gets an error now. A mount dying mid-walk leaves the threads
    // stuck, but the emitting thread below still sees a cancel and the UI moves on.
    crate::guard::probe(&root).await?;
    let reason = match index.engine() {
        Some(engine) => match crate::findex::choose(&engine, &root, gitignore.unwrap_or(false)).await {
            Ok(real) => {
                run_indexed(app, gen, engine, root, real, query, hidden, id);
                return Ok(id);
            }
            Err(why) => why,
        },
        None => crate::findex::Reason::Off,
    }
    .as_str();
    std::thread::spawn(move || {
        let is_match = move |name: &str| parsed.matches(name);
        let walker = ignore::WalkBuilder::new(&root).hidden(!hidden).git_ignore(gitignore.unwrap_or(false)).parents(false).threads(num_threads()).build_parallel();
        let (tx, rx) = std::sync::mpsc::channel::<Entry>();
        let scanned = Arc::new(AtomicU64::new(0));
        let sc = scanned.clone();
        let g2 = gen.clone();
        let walk = std::thread::spawn(move || {
            walker.run(|| {
                let tx = tx.clone();
                let is_match = &is_match;
                let g = g2.clone();
                let sc = sc.clone();
                Box::new(move |res| {
                    if g.load(Ordering::Relaxed) != id {
                        return ignore::WalkState::Quit;
                    }
                    if let Ok(e) = res {
                        sc.fetch_add(1, Ordering::Relaxed);
                        if e.depth() > 0 && is_match(&e.file_name().to_string_lossy()) {
                            if let Some(en) = entry_from(e.path()) {
                                let _ = tx.send(en);
                            }
                        }
                    }
                    ignore::WalkState::Continue
                })
            });
        });
        // Stream results in small batches so the first hits show up instantly.
        let mut buf = vec![];
        let mut last = std::time::Instant::now();
        let mut total = 0usize;
        loop {
            match rx.recv_timeout(std::time::Duration::from_millis(60)) {
                Ok(e) => {
                    total += 1;
                    buf.push(e);
                    if total >= MAX_RESULTS {
                        // Cap reached: stop the walker (it checks `gen` on every entry).
                        let _ = gen.compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(_) => {}
            }
            if gen.load(Ordering::Relaxed) != id {
                return;
            }
            if last.elapsed().as_millis() > 80 {
                let _ = app.emit("search", SearchBatch { id, entries: std::mem::take(&mut buf), done: false, scanned: scanned.load(Ordering::Relaxed), source: "live", reason: Some(reason), total: None });
                last = std::time::Instant::now();
            }
        }
        let _ = walk.join();
        let _ = app.emit("search", SearchBatch { id, entries: buf, done: true, scanned: scanned.load(Ordering::Relaxed), source: "live", reason: Some(reason), total: None });
    });
    Ok(id)
}

/// Answer from the index: match and rank under the read lock, then check each result still exists
/// (without the lock) in order, so the best ones appear first even if a symlink's target hangs.
#[allow(clippy::too_many_arguments)]
fn run_indexed(app: AppHandle, gen: Arc<AtomicU64>, engine: Arc<crate::findex::Engine>, pane: String, real: PathBuf, query: String, hidden: bool, id: u64) {
    std::thread::spawn(move || {
        let cancelled = || gen.load(Ordering::Relaxed) != id;
        let send = |entries: Vec<Entry>, done: bool, scanned: u64, total: u64| {
            let _ = app.emit("search", SearchBatch { id, entries, done, scanned, source: "index", reason: None, total: Some(total) });
        };
        let found = match engine.query(&real, std::path::Path::new(&pane), &query, hidden, MAX_RESULTS, &cancelled) {
            Ok(Some(f)) => f,
            Ok(None) => return, // superseded
            Err(_) => {
                // The index changed between the choice and the query: an empty answer would be a lie.
                let _ = app.emit("search-rerun", id);
                return;
            }
        };
        let mut total = found.total;
        let stall = crate::guard::stall_for(&pane);
        let (gone, failed) = verify(found.paths, &pane, stall, &entry_from, &cancelled, &mut |entries, dropped, done| {
            total -= dropped;
            send(entries, done, found.scanned, total);
        });
        engine.stale(gone);
        if let Some(error) = failed.filter(|_| !cancelled()) {
            let _ = app.emit("search-failed", SearchFailed { id, error });
        }
    });
}

enum Checked<T> {
    Row(T),
    Gone(PathBuf),
    End(Option<String>),
}

#[derive(Serialize, Clone)]
struct SearchFailed {
    id: u64,
    error: String,
}

/// Stat results in rank order under `guard`, off this thread: a symlink to a dead mount gives up after
/// `stall` rather than holding what was found. Rows are flushed by time as well as count, so those
/// found before a hang are on screen during it. `report(rows, dropped, done)`. Returns stale paths and
/// the guard's error, if any; with that and no rows nothing is reported, as "nothing found" would be false.
fn verify<T: Send + 'static>(
    paths: Vec<(PathBuf, PathBuf)>,
    key: &str,
    stall: std::time::Duration,
    stat: &'static (dyn Fn(&std::path::Path) -> Option<T> + Sync),
    cancelled: &dyn Fn() -> bool,
    report: &mut dyn FnMut(Vec<T>, u64, bool),
) -> (Vec<PathBuf>, Option<String>) {
    use std::sync::{atomic::AtomicBool, mpsc};
    let (tx, rx) = mpsc::channel::<Checked<T>>();
    let stop = Arc::new(AtomicBool::new(false));
    let (key, stop2) = (key.to_string(), stop.clone());
    std::thread::spawn(move || {
        let (inner, stop3) = (tx.clone(), stop2);
        let res = crate::guard::run_blocking(&key, stall, move |beat| {
            for (shown, indexed) in paths {
                if stop3.load(Ordering::Relaxed) {
                    break;
                }
                let _ = inner.send(match stat(&shown) {
                    Some(e) => Checked::Row(e),
                    None => Checked::Gone(indexed),
                });
                beat.tick();
            }
            Ok(())
        });
        let _ = tx.send(Checked::End(res.err()));
    });
    let (mut buf, mut gone, mut dropped) = (Vec::new(), Vec::new(), 0u64);
    let mut last = std::time::Instant::now();
    let (mut failed, mut reported) = (None, false);
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(40)) {
            Ok(Checked::Row(e)) => buf.push(e),
            Ok(Checked::Gone(p)) => {
                dropped += 1;
                gone.push(p);
            }
            Ok(Checked::End(e)) => {
                failed = e;
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
        if cancelled() {
            stop.store(true, Ordering::Relaxed);
            return (gone, None);
        }
        if !buf.is_empty() && (buf.len() >= 256 || last.elapsed().as_millis() >= 60) {
            report(std::mem::take(&mut buf), std::mem::take(&mut dropped), false);
            reported = true;
            last = std::time::Instant::now();
        }
    }
    if !cancelled() && (failed.is_none() || reported || !buf.is_empty()) {
        report(buf, dropped, true);
    }
    (gone, failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::Path, time::Duration};

    #[test]
    fn verify_shows_found_rows_during_a_hang_then_finishes() {
        fn stat(p: &Path) -> Option<String> {
            match p.to_str().unwrap() {
                "/hang" => {
                    std::thread::sleep(Duration::from_secs(2));
                    None
                }
                "/gone" => None,
                s => Some(s.to_string()),
            }
        }
        let item = |s: &str| (PathBuf::from(s), PathBuf::from(s));
        let started = std::time::Instant::now();
        let mut seen: Vec<(Vec<String>, bool, u128)> = vec![];
        let (gone, failed) = verify(vec![item("/a"), item("/gone"), item("/hang"), item("/never")], "/verify-test", Duration::from_millis(400), &stat, &|| false, &mut |rows, _, done| {
            seen.push((rows, done, started.elapsed().as_millis()));
        });
        assert_eq!(gone, [PathBuf::from("/gone")]);
        assert!(failed.is_some(), "the hang is reported too");
        let (first_rows, first_done, at) = &seen[0];
        assert_eq!(first_rows, &["/a"]);
        assert!(!first_done && *at < 350, "shown before the stall gave up, at {at} ms");
        assert!(seen.last().unwrap().1, "the search ends");
        assert!(started.elapsed() < Duration::from_millis(1500), "didn't wait out the hung call");
        assert!(!seen.iter().any(|(r, ..)| r.iter().any(|x| x == "/never")));
    }

    #[test]
    fn a_stalled_or_refused_check_is_an_error_not_an_empty_result() {
        fn stat(_: &Path) -> Option<String> {
            std::thread::sleep(Duration::from_secs(2));
            None
        }
        // Enough on one key to pass the abandoned-thread limit, so the last ones are refused outright.
        for _ in 0..crate::guard::LIMITS.abandoned + 2 {
            let mut reports = 0;
            let (_, failed) = verify(vec![(PathBuf::from("/x"), PathBuf::from("/x"))], "/verify-refused", Duration::from_millis(60), &stat, &|| false, &mut |_, _, _| reports += 1);
            assert!(failed.is_some());
            assert_eq!(reports, 0, "no empty \"done\"");
        }
    }
}

#[tauri::command]
pub fn cancel_search(state: State<'_, SearchState>) {
    state.0.store(0, Ordering::SeqCst);
}

pub fn num_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(12)
}
