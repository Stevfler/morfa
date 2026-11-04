use serde::Serialize;
use std::{path::Path, time::UNIX_EPOCH};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    name: String,
    path: String,
    is_dir: bool,
    is_symlink: bool,
    size: u64,
    modified: u64,
    ext: String,
    hidden: bool,
}

pub fn entry_from(path: &Path) -> Option<Entry> {
    let sym = std::fs::symlink_metadata(path).ok()?;
    // One round trip per entry on a share: only a symlink needs a second, for its target.
    let meta = if sym.file_type().is_symlink() { std::fs::metadata(path).unwrap_or_else(|_| sym.clone()) } else { sym.clone() };
    let name = path.file_name()?.to_string_lossy().to_string();
    let is_dir = meta.is_dir();
    Some(Entry {
        hidden: name.starts_with('.') || is_hidden_attr(&meta),
        ext: if is_dir {
            String::new()
        } else {
            path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
        },
        name,
        path: path.to_string_lossy().to_string(),
        is_dir,
        is_symlink: sym.file_type().is_symlink(),
        size: if is_dir { 0 } else { meta.len() },
        modified: meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    })
}

/// A row for a symlink or mount point whose target doesn't answer. Shown as a folder, since opening
/// one gives the pane's "not responding" error rather than handing the path to another app.
pub fn entry_unresponsive(path: &Path, is_symlink: bool) -> Option<Entry> {
    let name = path.file_name()?.to_string_lossy().to_string();
    Some(Entry { hidden: name.starts_with('.'), ext: String::new(), name, path: path.to_string_lossy().to_string(), is_dir: true, is_symlink, size: 0, modified: 0 })
}

#[cfg(windows)]
pub(crate) fn is_hidden_attr(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x2 != 0
}
#[cfg(target_os = "macos")]
pub(crate) fn is_hidden_attr(meta: &std::fs::Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    meta.st_flags() & libc::UF_HIDDEN != 0
}
#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn is_hidden_attr(_: &std::fs::Metadata) -> bool {
    false
}
