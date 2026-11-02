//! freedesktop `user.xdg.tags`: comma-separated UTF-8. Linux only; elsewhere a no-op.

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    const NAME: &[u8] = b"user.xdg.tags\0";

    fn cpath(p: &Path) -> Option<CString> {
        CString::new(p.as_os_str().as_bytes()).ok()
    }

    pub fn read(p: &Path) -> Option<Vec<String>> {
        let c = cpath(p)?;
        // SAFETY: valid NUL-terminated strings; a null buffer of length 0 asks for the size.
        let size = unsafe { libc::getxattr(c.as_ptr(), NAME.as_ptr().cast(), std::ptr::null_mut(), 0) };
        if size < 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        // SAFETY: as above, with a buffer of the stated length.
        let n = unsafe { libc::getxattr(c.as_ptr(), NAME.as_ptr().cast(), buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            return None;
        }
        buf.truncate(n as usize);
        let s = String::from_utf8_lossy(&buf);
        Some(s.split(',').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect())
    }

    pub fn write(p: &Path, tags: &[String]) -> Result<(), ()> {
        let c = cpath(p).ok_or(())?;
        // SAFETY: as above; value pointer/length come from a live String.
        let r = unsafe {
            if tags.is_empty() {
                libc::removexattr(c.as_ptr(), NAME.as_ptr().cast())
            } else {
                let v = tags.join(",");
                libc::setxattr(c.as_ptr(), NAME.as_ptr().cast(), v.as_ptr().cast(), v.len(), 0)
            }
        };
        // Removing an absent attribute is fine.
        if r == 0 || (tags.is_empty() && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENODATA)) {
            Ok(())
        } else {
            Err(())
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use std::path::Path;
    pub fn read(_: &Path) -> Option<Vec<String>> {
        None
    }
    pub fn write(_: &Path, _: &[String]) -> Result<(), ()> {
        Err(())
    }
}

pub use imp::{read, write};
