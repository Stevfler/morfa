use crate::CmdResult;
use serde::Serialize;
use std::path::PathBuf;

mod disks;
use disks::{disk_name, filter_disks, Os, RawDisk};

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
    let raw: Vec<RawDisk> = disks
        .iter()
        .flat_map(|d| d.iter())
        .map(|d| RawDisk {
            label: d.name().to_string_lossy().into(),
            mount: d.mount_point().to_string_lossy().into(),
            fs: d.file_system().to_string_lossy().into(),
            removable: d.is_removable(),
            total: d.total_space(),
            free: d.available_space(),
        })
        .collect();
    for d in filter_disks(Os::current(), raw) {
        out.push(Place {
            name: disk_name(Os::current(), &d),
            kind: if d.removable { "removable".into() } else { "disk".into() },
            path: d.mount,
            total: d.total,
            free: d.free,
        });
    }
    Ok(out)
}
