use crate::CmdResult;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    name: String,
    path: String,
    kind: String,
    total: u64,
    free: u64,
}

#[tauri::command]
pub async fn places() -> CmdResult<Vec<Place>> {
    let mut out = vec![];
    let mut push = |name: &str, p: Option<PathBuf>, kind: &str| {
        if let Some(p) = p.filter(|p| p.exists()) {
            out.push(Place { name: name.into(), path: p.to_string_lossy().into(), kind: kind.into(), total: 0, free: 0 });
        }
    };
    push("Home", dirs::home_dir(), "home");
    push("Desktop", dirs::desktop_dir(), "desktop");
    push("Downloads", dirs::download_dir(), "downloads");
    push("Documents", dirs::document_dir(), "documents");
    push("Pictures", dirs::picture_dir(), "pictures");
    push("Music", dirs::audio_dir(), "music");
    push("Videos", dirs::video_dir(), "videos");

    let disks = sysinfo::Disks::new_with_refreshed_list().ok();
    let mut seen = std::collections::HashSet::new();
    for d in disks.iter().flat_map(|d| d.iter()) {
        let mount = d.mount_point().to_string_lossy().to_string();
        // Hide pseudo/system mounts that only add noise.
        if mount.starts_with("/boot") || mount.starts_with("/snap") || mount.starts_with("/var") || mount.starts_with("/sys") || mount.starts_with("/proc") {
            continue;
        }
        if mount != "/" && !seen.insert(d.name().to_string_lossy().to_string()) {
            continue;
        }
        let label = Path::new(&mount).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| mount.clone());
        out.push(Place {
            name: if mount == "/" { "System".into() } else { label },
            path: mount,
            kind: if d.is_removable() { "removable".into() } else { "disk".into() },
            total: d.total_space(),
            free: d.available_space(),
        });
    }
    Ok(out)
}
