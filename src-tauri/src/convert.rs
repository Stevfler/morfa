//! Image conversion by shelling out to libvips (preferred) or ImageMagick.
mod engine;
mod job;
mod naming;
pub mod preview;
#[cfg(test)]
mod tests;

use crate::CmdResult;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{atomic::{AtomicUsize, Ordering}, Arc, Mutex},
};
use tauri::{AppHandle, Emitter, State};

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Resize {
    pub mode: String, // "percent" | "max"
    pub value: u32,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConvertOpts {
    pub format: String,
    pub quality: u8,
    pub resize: Option<Resize>,
    pub strip_metadata: bool,
    pub out_dir: Option<String>,
    pub name_template: String,
}

#[derive(Serialize)]
pub struct Caps {
    engine: Option<String>,
    formats: Vec<String>,
}

#[derive(Serialize)]
pub struct ConvertResult {
    pairs: Vec<(String, String)>,
    failed: Vec<(String, String)>,
    cancelled: bool,
}

#[derive(Serialize)]
pub struct Estimate {
    before: u64,
    after: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    id: u64,
    done: u64,
    total: u64,
    current: String,
    finished: bool,
}

#[derive(Default)]
pub struct ConvertState(Arc<Mutex<HashSet<u64>>>); // cancelled op ids

fn ready(o: &mut ConvertOpts) -> CmdResult<&'static engine::Engine> {
    let e = engine::engine().ok_or(engine::NO_ENGINE)?;
    o.format = o.format.trim_start_matches('.').to_lowercase();
    if o.format == "jpeg" {
        o.format = "jpg".into();
    }
    if !e.formats.contains(&o.format) {
        return Err(format!("Can't save as {} with {}", o.format, e.name()));
    }
    Ok(e)
}

#[tauri::command]
pub async fn convert_caps() -> Caps {
    tauri::async_runtime::spawn_blocking(|| match engine::engine() {
        Some(e) => Caps { engine: Some(e.name().into()), formats: e.formats.clone() },
        None => Caps { engine: None, formats: vec![] },
    })
    .await
    .unwrap_or(Caps { engine: None, formats: vec![] })
}

#[tauri::command]
pub async fn convert_images(app: AppHandle, state: State<'_, ConvertState>, id: u64, sources: Vec<String>, opts: ConvertOpts) -> CmdResult<ConvertResult> {
    let cancels = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut opts = opts;
        let e = ready(&mut opts)?;
        let emit = |done: u64, current: &str, finished: bool| {
            let _ = app.emit("op-progress", Progress { id, done, total: sources.len() as u64, current: current.into(), finished });
        };
        let r = run_all(e, &sources, &opts, &|| cancels.lock().unwrap_or_else(|x| x.into_inner()).contains(&id), &emit);
        cancels.lock().unwrap_or_else(|x| x.into_inner()).remove(&id);
        Ok(r)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Converts with a small worker pool; one failure doesn't stop the others.
fn run_all(e: &engine::Engine, sources: &[String], o: &ConvertOpts, cancelled: &(dyn Fn() -> bool + Sync), emit: &(dyn Fn(u64, &str, bool) + Sync)) -> ConvertResult {
    let next = AtomicUsize::new(0);
    let out = Mutex::new(ConvertResult { pairs: vec![], failed: vec![], cancelled: false });
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(2, 4).min(sources.len().max(1));
    emit(0, sources.first().map(String::as_str).unwrap_or(""), false);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= sources.len() || cancelled() {
                    break;
                }
                let src = &sources[i];
                let res = job::convert_file(e, &PathBuf::from(src), o, cancelled);
                let mut g = out.lock().unwrap_or_else(|x| x.into_inner());
                match res {
                    Ok(d) => g.pairs.push((src.clone(), d.to_string_lossy().into())),
                    Err(_) if cancelled() => {}
                    Err(err) => g.failed.push((src.clone(), err)),
                }
                let done = (g.pairs.len() + g.failed.len()) as u64;
                drop(g);
                emit(done, src, false);
            });
        }
    });
    let mut r = out.into_inner().unwrap_or_else(|x| x.into_inner());
    r.cancelled = cancelled();
    emit((r.pairs.len() + r.failed.len()) as u64, "", true);
    r
}

#[tauri::command]
pub fn convert_cancel(state: State<'_, ConvertState>, id: u64) {
    state.0.lock().unwrap_or_else(|x| x.into_inner()).insert(id);
}

#[tauri::command]
pub async fn convert_estimate(source: String, opts: ConvertOpts) -> CmdResult<Estimate> {
    tauri::async_runtime::spawn_blocking(move || job::estimate(&source, opts)).await.map_err(|e| e.to_string())?
}
