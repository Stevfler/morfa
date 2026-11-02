//! Non-replacing rename plus the cross-device copy fallback.
use super::{free, remove_any};
use std::{fs, io, path::Path};

/// Renames `f` to `t`, failing with `AlreadyExists` if `t` exists — atomically where the OS allows,
/// so a file appearing at `t` after the caller's check is never clobbered. A case-only rename of the
/// same file (case-insensitive FS) uses a plain rename, since an exclusive rename may see `t` as taken.
pub(crate) fn rename_noreplace(f: &Path, t: &Path) -> io::Result<()> {
    if same_file(f, t) {
        return fs::rename(f, t);
    }
    match os_rename_excl(f, t) {
        Some(r) => r,
        // Exclusive rename unavailable here (old kernel / FS without support): check, then rename.
        None if !free(t) => Err(io::Error::new(io::ErrorKind::AlreadyExists, "destination already exists")),
        None => fs::rename(f, t),
    }
}

#[cfg(unix)]
fn cstr(p: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
}

/// `None` means "not supported for this path, fall back".
#[cfg(any(target_os = "linux", target_os = "android"))]
fn os_rename_excl(f: &Path, t: &Path) -> Option<io::Result<()>> {
    const RENAME_NOREPLACE: libc::c_uint = 1;
    let (a, b) = match (cstr(f), cstr(t)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Some(Err(e)),
    };
    // Raw syscall: works regardless of the libc wrapper's availability (glibc < 2.28, musl).
    let r = unsafe { libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), RENAME_NOREPLACE) };
    if r == 0 {
        return Some(Ok(()));
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP) => None,
        _ => Some(Err(e)),
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn os_rename_excl(f: &Path, t: &Path) -> Option<io::Result<()>> {
    let (a, b) = match (cstr(f), cstr(t)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Some(Err(e)),
    };
    if unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_EXCL) } == 0 {
        return Some(Ok(()));
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::EINVAL | libc::ENOTSUP) => None,
        _ => Some(Err(e)),
    }
}

/// std's `fs::rename` on Windows *replaces* existing files (MOVEFILE_REPLACE_EXISTING / POSIX rename
/// semantics), so call MoveFileExW without that flag: it fails with ERROR_ALREADY_EXISTS instead.
#[cfg(windows)]
fn os_rename_excl(f: &Path, t: &Path) -> Option<io::Result<()>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let wide = |p: &Path| p.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (a, b) = (wide(f), wide(t));
    if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 0) } != 0 {
        return Some(Ok(()));
    }
    let e = io::Error::last_os_error();
    // ERROR_FILE_EXISTS (80) / ERROR_ALREADY_EXISTS (183) -> AlreadyExists for the caller.
    if matches!(e.raw_os_error(), Some(80 | 183)) {
        return Some(Err(io::Error::new(io::ErrorKind::AlreadyExists, e)));
    }
    Some(Err(e))
}

#[cfg(not(any(unix, windows)))]
fn os_rename_excl(_f: &Path, _t: &Path) -> Option<io::Result<()>> {
    None
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "ios"))))]
fn os_rename_excl(_f: &Path, _t: &Path) -> Option<io::Result<()>> {
    None
}

pub(super) fn is_cross_device(e: &io::Error) -> bool {
    // EXDEV = 18 on Linux/macOS; ERROR_NOT_SAME_DEVICE = 17 on Windows.
    const RAW: i32 = if cfg!(windows) { 17 } else { 18 };
    e.kind() == io::ErrorKind::CrossesDevices || e.raw_os_error() == Some(RAW)
}

/// True if both paths name the same filesystem object (links not followed).
pub(super) fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (a.symlink_metadata(), b.symlink_metadata()) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        match (fs::canonicalize(a), fs::canonicalize(b)) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
    }
}

/// Cross-device fallback: copy (never following symlinks, never overwriting), then delete the source.
/// A partially written destination is removed on failure; the source is only deleted after success.
pub(super) fn move_by_copy(src: &Path, dst: &Path) -> io::Result<()> {
    if !free(dst) {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "destination already exists"));
    }
    let mut created = false;
    if let Err(e) = copy_node(src, dst, &mut created) {
        if created {
            let _ = remove_any(dst);
        }
        return Err(e);
    }
    remove_any(src).map_err(|e| {
        io::Error::new(e.kind(), format!("copied completely to \"{}\", but the original couldn't be fully removed: {e}", dst.display()))
    })
}

#[cfg(unix)]
fn make_symlink(target: &Path, dst: &Path, _ft: fs::FileType) -> io::Result<()> {
    std::os::unix::fs::symlink(target, dst)
}

#[cfg(windows)]
fn make_symlink(target: &Path, dst: &Path, ft: fs::FileType) -> io::Result<()> {
    // Windows symlinks are typed; ask the source link which kind it is.
    use std::os::windows::fs::{symlink_dir, symlink_file, FileTypeExt};
    if ft.is_symlink_dir() { symlink_dir(target, dst) } else { symlink_file(target, dst) }
}

#[cfg(not(any(unix, windows)))]
fn make_symlink(_target: &Path, _dst: &Path, _ft: fs::FileType) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "can't copy symlinks on this platform"))
}

/// Creates `dst` exclusively (fails if it exists). Sets `created` once `dst` itself exists.
fn copy_node(src: &Path, dst: &Path, created: &mut bool) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        make_symlink(&fs::read_link(src)?, dst, ft)?;
        *created = true;
    } else if ft.is_dir() {
        fs::create_dir(dst)?;
        *created = true;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_node(&e.path(), &dst.join(e.file_name()), &mut false)?;
        }
        let _ = fs::set_permissions(dst, meta.permissions());
    } else if !ft.is_file() {
        // FIFO/socket/device: opening would block forever or stream endlessly. Fail before creating
        // anything, so the caller never removes the source.
        return Err(io::Error::new(io::ErrorKind::Unsupported, format!("\"{}\" is a special file (pipe, socket or device) and can't be moved across drives", src.display())));
    } else {
        let mut out = fs::OpenOptions::new().write(true).create_new(true).open(dst)?;
        *created = true;
        io::copy(&mut fs::File::open(src)?, &mut out)?;
        out.sync_all()?;
        let _ = fs::set_permissions(dst, meta.permissions());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noreplace_refuses_existing_target() {
        let d = std::env::temp_dir().join(format!("orbit-rename-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let (a, b, c) = (d.join("a"), d.join("b"), d.join("c"));
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();
        assert_eq!(os_rename_excl(&a, &b).map(|r| r.unwrap_err().kind()).unwrap_or(io::ErrorKind::AlreadyExists), io::ErrorKind::AlreadyExists);
        assert_eq!(rename_noreplace(&a, &b).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&b).unwrap(), "b");
        rename_noreplace(&a, &c).unwrap();
        assert_eq!(fs::read_to_string(&c).unwrap(), "a");
        fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn copy_refuses_fifo_and_keeps_source() {
        let d = std::env::temp_dir().join(format!("orbit-fifo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("src")).unwrap();
        fs::write(d.join("src/f"), "data").unwrap();
        let fifo = cstr(&d.join("src/pipe")).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
        let err = move_by_copy(&d.join("src"), &d.join("dst")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        assert!(d.join("src/pipe").exists() && d.join("src/f").exists() && !d.join("dst").exists());
        fs::remove_dir_all(&d).unwrap();
    }
}
