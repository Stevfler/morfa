use crate::{
    entry::{entry_from, Entry},
    CmdResult,
};
use serde::Serialize;
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
}

/// Every whitespace-separated token must appear in the name (case-insensitive).
/// Tokens like `*.rs` / `ext:rs` filter by extension.
fn matcher(query: &str) -> impl Fn(&str) -> bool {
    let q = query.to_lowercase();
    let mut words = vec![];
    let mut exts = vec![];
    for t in q.split_whitespace() {
        if let Some(e) = t.strip_prefix("*.").or_else(|| t.strip_prefix("ext:")) {
            exts.push(format!(".{e}"));
        } else {
            words.push(t.to_string());
        }
    }
    move |name: &str| {
        let n = name.to_lowercase();
        words.iter().all(|w| n.contains(w.as_str())) && (exts.is_empty() || exts.iter().any(|e| n.ends_with(e.as_str())))
    }
}

#[tauri::command]
pub async fn search(app: AppHandle, state: State<'_, SearchState>, root: String, query: String, hidden: bool, gitignore: Option<bool>, id: u64) -> CmdResult<u64> {
    if id == 0 {
        return Err("search id 0 is reserved".into());
    }
    // The id comes from the frontend so it can claim batches before this returns.
    // Storing it cancels any older search; 0 is reserved for "cancelled".
    let gen = state.0.clone();
    gen.store(id, Ordering::SeqCst);
    std::thread::spawn(move || {
        let is_match = matcher(&query);
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
                let _ = app.emit("search", SearchBatch { id, entries: std::mem::take(&mut buf), done: false, scanned: scanned.load(Ordering::Relaxed) });
                last = std::time::Instant::now();
            }
        }
        let _ = walk.join();
        let _ = app.emit("search", SearchBatch { id, entries: buf, done: true, scanned: scanned.load(Ordering::Relaxed) });
    });
    Ok(id)
}

#[tauri::command]
pub fn cancel_search(state: State<'_, SearchState>) {
    state.0.store(0, Ordering::SeqCst);
}

pub fn num_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(12)
}
