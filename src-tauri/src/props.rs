//! Detailed properties for the Properties dialog.
use crate::{guard, CmdResult};
use serde::Serialize;
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Props {
    path: String,
    is_dir: bool,
    is_symlink: bool,
    link_target: Option<String>,
    size: u64,
    /// Direct children of a folder.
    children: Option<u64>,
    created: u64,
    modified: u64,
    accessed: u64,
    readonly: bool,
    /// Unix permission bits (`st_mode & 0o7777`).
    mode: Option<u32>,
    owner: Option<String>,
    group: Option<String>,
    mount: Option<String>,
    fs_type: Option<String>,
    device: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "avif", "heic", "heif", "jxl", "svg", "ico", "tga", "psd"];

fn ms(t: std::io::Result<SystemTime>) -> u64 {
    t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[tauri::command]
pub async fn properties(path: String) -> CmdResult<Props> {
    let p = path.clone();
    guard::guarded(&path, guard::stall_for(&path), move |_| read(Path::new(&p))).await
}

fn read(p: &Path) -> Result<Props, String> {
    let sym = std::fs::symlink_metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let meta = std::fs::metadata(p).unwrap_or_else(|_| sym.clone());
    let is_dir = meta.is_dir();
    let mut r = Props {
        path: p.to_string_lossy().to_string(),
        is_dir,
        is_symlink: sym.file_type().is_symlink(),
        link_target: std::fs::read_link(p).ok().map(|t| t.to_string_lossy().to_string()),
        size: if is_dir { 0 } else { meta.len() },
        children: is_dir.then(|| std::fs::read_dir(p).map(|d| d.count() as u64).unwrap_or(0)),
        created: ms(meta.created()),
        modified: ms(meta.modified()),
        accessed: ms(meta.accessed()),
        readonly: meta.permissions().readonly(),
        ..Default::default()
    };
    unix_bits(&meta, &mut r);
    if let Some((mount, fs, dev)) = mount_of(p) {
        (r.mount, r.fs_type, r.device) = (Some(mount), Some(fs), Some(dev));
    }
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !is_dir && IMAGE_EXTS.contains(&ext.as_str()) {
        if let Some((w, h)) = dimensions(p) {
            (r.width, r.height) = (Some(w), Some(h));
        }
    }
    Ok(r)
}

/// Width × height via `vipsheader`, falling back to ImageMagick `identify`.
fn dimensions(p: &Path) -> Option<(u32, u32)> {
    let nums = |cmd: &str, args: &[&str]| -> Vec<u32> {
        crate::sidecar::command(cmd)
            .args(args)
            .arg(p)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().filter_map(|n| n.parse().ok()).collect())
            .unwrap_or_default()
    };
    let vips = (nums("vipsheader", &["-f", "width"]).first().copied()).zip(nums("vipsheader", &["-f", "height"]).first().copied());
    vips.or_else(|| match nums("identify", &["-format", "%w %h\n"])[..] {
        [w, h, ..] => Some((w, h)),
        _ => None,
    })
}

#[cfg(unix)]
fn unix_bits(m: &std::fs::Metadata, r: &mut Props) {
    use std::os::unix::fs::MetadataExt;
    r.mode = Some(m.mode() & 0o7777);
    r.owner = Some(name_of("/etc/passwd", m.uid()));
    r.group = Some(name_of("/etc/group", m.gid()));
}
#[cfg(not(unix))]
fn unix_bits(_: &std::fs::Metadata, _: &mut Props) {}

/// Looks up an id in a passwd/group-style file; falls back to the number.
#[cfg(unix)]
fn name_of(file: &str, id: u32) -> String {
    let id_s = id.to_string();
    std::fs::read_to_string(file)
        .ok()
        .and_then(|s| s.lines().find_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            (f.len() > 2 && f[2] == id_s).then(|| f[0].to_string())
        }))
        .unwrap_or(id_s)
}

/// (mount point, fs type, device) from /proc/self/mounts — longest matching prefix.
#[cfg(target_os = "linux")]
fn mount_of(p: &Path) -> Option<(String, String, String)> {
    let real = std::fs::canonicalize(p).ok()?;
    let mounts = std::fs::read_to_string("/proc/self/mounts").ok()?;
    let unesc = |s: &str| s.replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\");
    mounts
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            (f.len() > 2).then(|| (unesc(f[1]), f[2].to_string(), unesc(f[0])))
        })
        .filter(|(m, _, _)| real.starts_with(m))
        .max_by_key(|(m, _, _)| m.len())
}
#[cfg(not(target_os = "linux"))]
fn mount_of(_: &Path) -> Option<(String, String, String)> {
    None
}
