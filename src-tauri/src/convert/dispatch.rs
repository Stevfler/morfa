//! Picks the engine for one source by its extension and runs it.
use super::{doc, engine, job, pdf, routes::{self, Via}, ConvertOpts, ConvertResult};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::{AtomicUsize, Ordering}, Mutex},
};

/// Convert one source; returns every file written (several for PDF -> images).
pub fn convert_one(src: &Path, o: &ConvertOpts, worker: usize, cancelled: &dyn Fn() -> bool) -> Result<Vec<PathBuf>, String> {
    let ext = routes::norm(&src.extension().unwrap_or_default().to_string_lossy());
    let fmt = o.format.as_str();
    // Same-format image re-encodes (resize/strip/quality) stay allowed, as before.
    let via = routes::detected(|a| routes::route(&ext, fmt, a).or_else(|| (ext == fmt && a.image.iter().any(|f| f == fmt)).then_some(Via::Image)));
    match via {
        Some(Via::Image) => job::convert_file(engine::engine().ok_or(engine::NO_ENGINE)?, src, o, cancelled).map(|p| vec![p]),
        Some(Via::PdfPages) => pdf::convert(src, o, cancelled),
        Some(Via::Office) => doc::office_convert(src, o, worker, cancelled).map(|p| vec![p]),
        Some(Via::Pandoc) => doc::pandoc_convert(src, o, cancelled).map(|p| vec![p]),
        None => Err(routes::missing(&ext, fmt)),
    }
}

/// Converts with a small worker pool; one failure doesn't stop the others.
pub fn run_all(sources: &[String], o: &ConvertOpts, cancelled: &(dyn Fn() -> bool + Sync), emit: &(dyn Fn(u64, &str, bool) + Sync)) -> ConvertResult {
    let (next, finished) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let out = Mutex::new(ConvertResult { pairs: vec![], failed: vec![], cancelled: false });
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(2, 4).min(sources.len().max(1));
    emit(0, sources.first().map(String::as_str).unwrap_or(""), false);
    std::thread::scope(|s| {
        for w in 0..workers {
            let (next, out, finished) = (&next, &out, &finished);
            s.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= sources.len() || cancelled() {
                    break;
                }
                let src = &sources[i];
                let res = convert_one(&PathBuf::from(src), o, w, cancelled);
                let mut g = out.lock().unwrap_or_else(|x| x.into_inner());
                match res {
                    Ok(ds) => g.pairs.extend(ds.iter().map(|d| (src.clone(), d.to_string_lossy().into()))),
                    Err(_) if cancelled() => {}
                    Err(err) => g.failed.push((src.clone(), err)),
                }
                drop(g);
                let done = finished.fetch_add(1, Ordering::SeqCst) as u64 + 1;
                emit(done, src, false);
            });
        }
    });
    let mut r = out.into_inner().unwrap_or_else(|x| x.into_inner());
    r.cancelled = cancelled();
    emit(finished.into_inner() as u64, "", true);
    r
}
