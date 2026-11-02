//! Displayable stand-ins for images the webview can't decode (AVIF/HEIC/TIFF/JXL on some WebKitGTK builds).
use super::{job, ConvertOpts, Resize};
use crate::CmdResult;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

const MAX_SIDE: u32 = 2560;

/// Cache path keyed by source path + size + mtime, so an edited file gets a fresh preview.
fn cache_path(src: &Path) -> CmdResult<PathBuf> {
    let m = std::fs::metadata(src).map_err(|e| e.to_string())?;
    let mut h = DefaultHasher::new();
    (src, m.len(), m.modified().ok()).hash(&mut h);
    let dir = std::env::temp_dir().join("orbit-preview");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(format!("{:016x}.png", h.finish())))
}

/// PNG copy of `path` (downscaled to fit `MAX_SIDE`) for Quick Look; returns its path.
#[tauri::command]
pub async fn convert_preview(path: String) -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let src = PathBuf::from(&path);
        let out = cache_path(&src)?;
        if !out.is_file() {
            let mut o = ConvertOpts { format: "png".into(), quality: 90, resize: Some(Resize { mode: "max".into(), value: MAX_SIDE }), strip_metadata: true, out_dir: None, name_template: "{name}".into() };
            let e = super::ready(&mut o)?;
            let tmp = super::naming::temp_for(&out);
            job::convert_to(e, &src, &tmp, &o, &|| false)?;
            std::fs::rename(&tmp, &out).map_err(|e| e.to_string())?;
        }
        Ok(out.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Raw bytes of an image for the webview (a blob URL), sidestepping the asset protocol.
#[tauri::command]
pub async fn image_bytes(path: String) -> CmdResult<tauri::ipc::Response> {
    tauri::async_runtime::spawn_blocking(move || {
        let len = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
        if len > 256 << 20 {
            return Err("image too large to preview".into());
        }
        std::fs::read(&path).map(tauri::ipc::Response::new).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
