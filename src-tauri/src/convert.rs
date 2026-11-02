//! File conversion by shelling out: images via libvips/ImageMagick, documents via LibreOffice/pandoc.
mod dispatch;
pub(crate) mod doc;
mod engine;
mod job;
mod naming;
mod pdf;
pub mod preview;
pub(crate) mod proc;
mod routes;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_doc;

use crate::CmdResult;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
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
    office: bool,
    pandoc: bool,
}

#[derive(Serialize)]
pub struct ConvertResult {
    // one (src, dest) per written file; a PDF -> images source contributes one per page
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
    let none = || Caps { engine: None, formats: vec![], office: false, pandoc: false };
    tauri::async_runtime::spawn_blocking(move || {
        let (office, pandoc) = (doc::office().is_some(), doc::pandoc());
        match engine::engine() {
            Some(e) => Caps { engine: Some(e.name().into()), formats: e.formats.clone(), office, pandoc },
            None => Caps { office, pandoc, ..none() },
        }
    })
    .await
    .unwrap_or_else(|_| none())
}

/// Output formats valid for all the given input extensions, most useful first.
#[tauri::command]
pub async fn convert_targets(exts: Vec<String>) -> Vec<String> {
    tauri::async_runtime::spawn_blocking(move || routes::detected(|a| routes::targets(&exts, a))).await.unwrap_or_default()
}

#[tauri::command]
pub async fn convert_images(app: AppHandle, state: State<'_, ConvertState>, id: u64, sources: Vec<String>, opts: ConvertOpts) -> CmdResult<ConvertResult> {
    let cancels = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut opts = opts;
        opts.format = routes::norm(&opts.format);
        let emit = |done: u64, current: &str, finished: bool| {
            let _ = app.emit("op-progress", Progress { id, done, total: sources.len() as u64, current: current.into(), finished });
        };
        let r = dispatch::run_all(&sources, &opts, &|| cancels.lock().unwrap_or_else(|x| x.into_inner()).contains(&id), &emit);
        cancels.lock().unwrap_or_else(|x| x.into_inner()).remove(&id);
        Ok(r)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Alias of `convert_images` (accepts any mix of inputs).
#[tauri::command]
pub async fn convert_files(app: AppHandle, state: State<'_, ConvertState>, id: u64, sources: Vec<String>, opts: ConvertOpts) -> CmdResult<ConvertResult> {
    convert_images(app, state, id, sources, opts).await
}

#[tauri::command]
pub fn convert_cancel(state: State<'_, ConvertState>, id: u64) {
    state.0.lock().unwrap_or_else(|x| x.into_inner()).insert(id);
}

#[tauri::command]
pub async fn convert_estimate(source: String, opts: ConvertOpts) -> CmdResult<Estimate> {
    let ext = routes::norm(&std::path::Path::new(&source).extension().unwrap_or_default().to_string_lossy());
    if !routes::is_image(&ext) {
        return Err("UNSUPPORTED".into());
    }
    tauri::async_runtime::spawn_blocking(move || job::estimate(&source, opts)).await.map_err(|e| e.to_string())?
}
