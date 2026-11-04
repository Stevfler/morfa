//! What each row of a batch would run into: other rows, files on disk, the disk's own rules.
use crate::{
    cloud::sync::{fold_key, Fold},
    fsops::{name_issue, rename::same_file},
    CmdResult,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Deserialize, Debug, Clone)]
pub struct Row {
    pub path: String,
    /// The new name; the current one when this row stays as it is.
    pub next: String,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "status", content = "detail")]
pub enum RowCheck {
    Ok,
    Same,
    Duplicate,
    Exists,
    /// The file being renamed is no longer there.
    Gone,
    Invalid(&'static str),
}

/// A name's length as the limit counts it: UTF-16 units on Windows, bytes elsewhere.
pub fn name_len(name: &str, utf16: bool) -> usize {
    if utf16 { name.encode_utf16().count() } else { name.len() }
}

/// Longest name this folder's disk takes (see `name_len`). 255 where the system won't say.
pub fn name_max(dir: &Path) -> usize {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        if let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) {
            let n = unsafe { libc::pathconf(c.as_ptr(), libc::_PC_NAME_MAX) };
            if n > 0 {
                return (n as usize).min(255);
            }
        }
    }
    let _ = dir;
    255
}

pub fn check_rows(rows: &[Row], fold_of: &dyn Fn(&Path) -> Fold, max_of: &dyn Fn(&Path) -> usize, cancel: &AtomicBool, tick: &dyn Fn()) -> CmdResult<Vec<RowCheck>> {
    let mut dirs: HashMap<PathBuf, (Fold, usize)> = HashMap::new();
    let split = |r: &Row| -> (PathBuf, String) {
        let p = Path::new(&r.path);
        (p.parent().unwrap_or(Path::new("")).to_path_buf(), p.file_name().unwrap_or_default().to_string_lossy().into_owned())
    };
    let mut parsed = Vec::with_capacity(rows.len());
    // Names this batch frees (a file moving away) and takes, as the disk sees them.
    let mut leaving: HashSet<(PathBuf, String)> = HashSet::new();
    let mut taking: HashMap<(PathBuf, String), usize> = HashMap::new();
    for r in rows {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let (dir, base) = split(r);
        let (fold, _) = *dirs.entry(dir.clone()).or_insert_with(|| (fold_of(&dir), max_of(&dir)));
        tick();
        if r.next != base {
            leaving.insert((dir.clone(), fold_key(&base, fold)));
            *taking.entry((dir.clone(), fold_key(&r.next, fold))).or_default() += 1;
        }
        parsed.push((dir, base, fold));
    }
    let mut out = Vec::with_capacity(rows.len());
    for (r, (dir, base, fold)) in rows.iter().zip(parsed) {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        tick();
        if r.path.starts_with("morfa://") {
            out.push(RowCheck::Invalid("pseudo"));
            continue;
        }
        if r.next == base {
            out.push(RowCheck::Same);
            continue;
        }
        if let Some(issue) = name_issue(&r.next) {
            out.push(RowCheck::Invalid(issue.code()));
            continue;
        }
        if name_len(&r.next, cfg!(windows)) > dirs[&dir].1 {
            out.push(RowCheck::Invalid("long"));
            continue;
        }
        let key = fold_key(&r.next, fold);
        if taking[&(dir.clone(), key.clone())] > 1 {
            out.push(RowCheck::Duplicate);
            continue;
        }
        let src = Path::new(&r.path);
        if src.symlink_metadata().is_err() {
            out.push(RowCheck::Gone);
            continue;
        }
        let target = dir.join(&r.next);
        out.push(match target.symlink_metadata() {
            Err(e) if e.kind() == ErrorKind::NotFound => RowCheck::Ok,
            Err(_) => RowCheck::Invalid("unreadable"),
            // A case-only rename finds itself; a hard link to the same data under another name is a different name.
            Ok(_) if key == fold_key(&base, fold) && same_file(src, &target) => RowCheck::Ok,
            Ok(_) if leaving.contains(&(dir.clone(), key)) => RowCheck::Ok,
            Ok(_) => RowCheck::Exists,
        });
    }
    Ok(out)
}
