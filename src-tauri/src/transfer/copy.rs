//! Low-level copy helpers: exclusive target reservation, recursive copy, symlink recreation.
use super::Ctx;
use crate::CmdResult;
use std::{
    fs::{self, Metadata, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const CANCELLED: &str = "Cancelled";

/// Total byte size of `p` (symlinks not followed). `None` once the op is cancelled.
pub(super) fn size_of(p: &Path, cx: &Ctx) -> Option<u64> {
    if cx.cancelled() {
        return None;
    }
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => {
            let mut sum = 0;
            // Unreadable entries count as 0 here; the copy itself reports them.
            for e in fs::read_dir(p).into_iter().flatten().flatten() {
                sum += size_of(&e.path(), cx)?;
            }
            Some(sum)
        }
        Ok(m) => Some(m.len()),
        Err(_) => Some(0),
    }
}

/// Recreate the symlink `src` at `dst`; fails with AlreadyExists if `dst` is taken.
pub(crate) fn link(src: &Path, dst: &Path) -> io::Result<()> {
    let target = fs::read_link(src)?;
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, dst);
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        let dir = fs::symlink_metadata(src)?.file_type().is_symlink_dir();
        let r = if dir { std::os::windows::fs::symlink_dir(&target, dst) } else { std::os::windows::fs::symlink_file(&target, dst) };
        return r.map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                e
            } else {
                io::Error::new(e.kind(), format!("can't create symbolic link \"{}\" (needs Developer Mode or admin rights): {e}", dst.display()))
            }
        });
    }
    #[allow(unreachable_code)]
    Err(io::Error::new(io::ErrorKind::Unsupported, "symbolic links are not supported here"))
}

/// Create `dst` exclusively in the shape of `meta` (empty dir, empty file, or the symlink itself).
fn create_exclusive(src: &Path, meta: &Metadata, dst: &Path) -> io::Result<()> {
    if meta.file_type().is_symlink() {
        link(src, dst)
    } else if meta.is_dir() {
        fs::create_dir(dst)
    } else if !meta.is_file() {
        // FIFOs, sockets, devices: opening could block forever or stream endlessly.
        Err(io::Error::new(io::ErrorKind::Unsupported, format!("\"{}\" is a special file (pipe, socket or device) and can't be copied", src.display())))
    } else {
        OpenOptions::new().write(true).create_new(true).open(dst).map(|_| ())
    }
}

/// Reserve a free name for `src` inside `dest` ("name", "name (2)", ...) by creating it
/// exclusively, so an existing item is never overwritten even under races.
pub(super) fn reserve(src: &Path, meta: &Metadata, dest: &Path) -> CmdResult<PathBuf> {
    let name = src.file_name().ok_or("bad path")?.to_string_lossy().to_string();
    let (stem, ext) = match (meta.is_dir(), name.rfind('.')) {
        (false, Some(i)) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.clone(), String::new()),
    };
    for n in 1..10_000 {
        let cand = dest.join(if n == 1 { name.clone() } else { format!("{stem} ({n}){ext}") });
        match create_exclusive(src, meta, &cand) {
            Ok(()) => return Ok(cand),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) if e.kind() == io::ErrorKind::Unsupported => return Err(e.to_string()),
            Err(e) => return Err(format!("{}: {e}", cand.display())),
        }
    }
    Err(format!("No free name for \"{name}\""))
}

/// Fill the already-reserved `dst` with the contents of `src`.
pub(super) fn fill(src: &Path, dst: &Path, cx: &mut Ctx) -> CmdResult<()> {
    if cx.cancelled() {
        return Err(CANCELLED.into());
    }
    let meta = fs::symlink_metadata(src).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Ok(()); // the link itself was created by `reserve`/`copy_new`
    }
    if meta.is_dir() {
        // Every entry error aborts: a silently skipped entry would be lost when a move removes the source.
        for e in fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
            let e = e.map_err(|e| format!("{}: {e}", src.display()))?;
            copy_new(&e.path(), &dst.join(e.file_name()), cx)?;
        }
        return Ok(());
    }
    if !meta.is_file() {
        return Err(format!("\"{}\" is a special file (pipe, socket or device) and can't be copied", src.display()));
    }
    let mut r = fs::File::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let mut w = OpenOptions::new().write(true).truncate(true).open(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        cx.done += n as u64;
        cx.tick(src, false);
        if cx.cancelled() {
            return Err(CANCELLED.into());
        }
    }
    let _ = w.set_permissions(meta.permissions());
    if let Ok(m) = meta.modified() {
        let _ = w.set_modified(m);
    }
    Ok(())
}

/// Copy `src` to a not-yet-existing `dst` (inside a directory we just created).
fn copy_new(src: &Path, dst: &Path, cx: &mut Ctx) -> CmdResult<()> {
    let meta = fs::symlink_metadata(src).map_err(|e| e.to_string())?;
    create_exclusive(src, &meta, dst).map_err(|e| if e.kind() == io::ErrorKind::Unsupported { e.to_string() } else { format!("{}: {e}", dst.display()) })?;
    fill(src, dst, cx)
}

#[cfg(all(test, unix))]
#[path = "copy_tests.rs"]
mod tests;
