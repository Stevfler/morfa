//! "Office module": an on-demand, pruned, headless-only LibreOffice in Orbit's data dir
//! (`%LOCALAPPDATA%\Orbit\office`, `~/Library/Application Support/Orbit/office`,
//! `~/.local/share/Orbit/office`). Never registered with the OS: no shortcuts, file
//! associations, Add/Remove entry or Launchpad item. Offered on Windows and macOS.
mod download;
mod extract;
mod glob;
mod install;
pub mod manifest;
pub mod prune;
#[cfg(test)]
mod tests;

use crate::CmdResult;
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
pub struct OfficeState {
    busy: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

#[derive(Serialize, Clone)]
pub struct Progress {
    phase: &'static str, // "download" | "extract" | "prune" | "done"
    done: u64,
    total: u64,
}

impl Progress {
    fn new(phase: &'static str, done: u64, total: u64) -> Self {
        Progress { phase, done, total }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    installed: bool,
    system_office: bool,
    path: Option<String>,
    size_on_disk: u64,
    download_size: u64,
    version: String,
    supported: bool,
}

/// `<data dir>/Orbit/office`.
pub fn dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("Orbit").join("office"))
}

/// The soffice binary inside a module root, if present.
pub fn soffice_in(root: &Path) -> Option<PathBuf> {
    let cands: &[&str] = if cfg!(windows) {
        &["program/soffice.com", "program/soffice.exe"] // .com = console variant, waits for exit
    } else if cfg!(target_os = "macos") {
        &["LibreOffice.app/Contents/MacOS/soffice"]
    } else {
        &["program/soffice"]
    };
    cands.iter().map(|c| root.join(c)).find(|p| p.is_file())
}

/// The installed module's soffice (wins over PATH in convert::doc).
pub fn soffice() -> Option<PathBuf> {
    dir().as_deref().and_then(soffice_in)
}

fn dir_size(p: &Path) -> u64 {
    let Ok(md) = std::fs::symlink_metadata(p) else { return 0 };
    if !md.is_dir() {
        return md.len();
    }
    std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| dir_size(&e.path())).sum()).unwrap_or(0)
}

#[tauri::command]
pub async fn office_status() -> Status {
    tauri::async_runtime::spawn_blocking(|| {
        let root = dir();
        let installed = root.as_deref().and_then(soffice_in).is_some();
        let version = root
            .as_deref()
            .filter(|_| installed)
            .and_then(|r| std::fs::read_to_string(r.join("orbit-office-version")).ok())
            .unwrap_or_else(|| manifest::VERSION.into());
        let pkg = manifest::package();
        Status {
            installed,
            system_office: crate::convert::doc::system_office().is_some(),
            path: crate::convert::doc::office().map(|p| p.to_string_lossy().into_owned()),
            size_on_disk: if installed { root.as_deref().map(dir_size).unwrap_or(0) } else { 0 },
            download_size: manifest::DOWNLOAD_ESTIMATE,
            version: version.trim().into(),
            supported: pkg.is_some(),
        }
    })
    .await
    .expect("office_status task")
}

/// Download and install the module. Emits `office-progress`; cancel with `office_cancel`.
#[tauri::command]
pub async fn office_install(app: AppHandle, state: State<'_, OfficeState>) -> CmdResult<()> {
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("already installing".into());
    }
    state.cancel.store(false, Ordering::SeqCst);
    let (busy, cancel) = (state.busy.clone(), state.cancel.clone());
    let res = tauri::async_runtime::spawn_blocking(move || {
        let paths = install::Paths::new(dir().ok_or("no data directory")?);
        let emit = |p: Progress| {
            let _ = app.emit("office-progress", p);
        };
        let r = install::run(&paths, &emit, &|| cancel.load(Ordering::SeqCst));
        paths.cleanup();
        crate::convert::doc::invalidate();
        if r.is_ok() {
            emit(Progress::new("done", 1, 1));
        }
        r
    })
    .await
    .map_err(|e| e.to_string());
    busy.store(false, Ordering::SeqCst);
    res?
}

#[tauri::command]
pub fn office_cancel(state: State<'_, OfficeState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub async fn office_remove(state: State<'_, OfficeState>) -> CmdResult<()> {
    if state.busy.load(Ordering::SeqCst) {
        return Err("installation in progress".into());
    }
    tauri::async_runtime::spawn_blocking(|| {
        let root = dir().ok_or("no data directory")?;
        install::Paths::new(root.clone()).cleanup();
        let r = match std::fs::remove_dir_all(&root) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
        crate::convert::doc::invalidate();
        r
    })
    .await
    .map_err(|e| e.to_string())?
}
