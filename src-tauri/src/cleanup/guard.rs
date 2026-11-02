//! Runtime delete guard: canonicalised protection check + "is this from the last scan".
use super::{categories, index::Index, protect, Protection};
use std::path::{Component, Path};

/// Static rules plus "can this user actually remove it". The parent is canonicalised so
/// symlinked ancestors (`~/sys -> /`) and 8.3 names can't dodge the rules; the item itself
/// is not resolved (removing a symlink removes only the link).
pub fn protection(p: &Path) -> Protection {
    let real = match (p.parent().map(std::fs::canonicalize), p.file_name()) {
        (Some(Ok(dir)), Some(name)) => dir.join(name),
        _ => return Protection::Protected,
    };
    let home = dirs::home_dir().and_then(|h| std::fs::canonicalize(&h).ok().or(Some(h))).unwrap_or_default();
    let c = protect::classify(&real.to_string_lossy(), protect::Os::current(), &home.to_string_lossy());
    if c == Protection::Protected || !removable(&real) {
        return Protection::Protected;
    }
    c
}

/// Parent (and the dir itself) writable, and not a mount point.
#[cfg(unix)]
fn removable(p: &Path) -> bool {
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
    let ok = |q: &Path| std::ffi::CString::new(q.as_os_str().as_bytes()).is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0);
    let Some(parent) = p.parent() else { return false };
    let (Ok(m), Ok(pm)) = (p.symlink_metadata(), parent.metadata()) else { return false };
    m.dev() == pm.dev() && ok(parent) && (!m.is_dir() || ok(p))
}

/// Not read-only / system / hidden-system, and not a volume root.
#[cfg(windows)]
fn removable(p: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const READONLY: u32 = 0x1;
    const SYSTEM: u32 = 0x4;
    p.parent().is_some_and(|q| q.parent().is_some() || q.file_name().is_some()) && p.symlink_metadata().is_ok_and(|m| m.file_attributes() & (READONLY | SYSTEM) == 0)
}

#[cfg(not(any(unix, windows)))]
fn removable(_p: &Path) -> bool {
    false
}

/// Only items from the last scan, strictly inside its root, that pass `protection`.
pub fn refuse(p: &Path, ix: Option<&Index>) -> Option<&'static str> {
    let Some(ix) = ix else { return Some("no scan to check against") };
    if !p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Some("not a plain absolute path");
    }
    if p == ix.root || !p.starts_with(&ix.root) {
        return Some("not inside the scanned folder");
    }
    if !ix.knows(p) {
        return Some("not found in the scan");
    }
    if categories::is_trash_dir(p) {
        return Some("use Empty Trash");
    }
    (protection(p) == Protection::Protected).then_some("System file — can't be removed")
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup::scan;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn refuse_rules() {
        let d = std::env::temp_dir().join(format!("orbit-cleanup-refuse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::write(d.join("a/f"), vec![0u8; 8 << 20]).unwrap();
        std::fs::write(d.join("a/small"), b"x").unwrap();
        let prog = scan::Progress { files: AtomicU64::new(0), bytes: AtomicU64::new(0) };
        let ix = scan::scan(&d, &prog, &|| false).unwrap();
        assert_eq!(refuse(&d.join("a"), None), Some("no scan to check against"));
        assert_eq!(refuse(&d, Some(&ix)), Some("not inside the scanned folder"));
        assert_eq!(refuse(Path::new("/usr"), Some(&ix)), Some("not inside the scanned folder"));
        assert_eq!(refuse(&d.join("a/../a"), Some(&ix)), Some("not a plain absolute path"));
        assert_eq!(refuse(&d.join("a/small"), Some(&ix)), Some("not found in the scan"));
        assert_eq!(refuse(&d.join("nope"), Some(&ix)), Some("not found in the scan"));
        assert_eq!(refuse(&d.join("a"), Some(&ix)), None);
        assert_eq!(refuse(&d.join("a/f"), Some(&ix)), None);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/", d.join("sys")).unwrap();
            assert_eq!(protection(&d.join("sys/usr")), Protection::Protected);
            assert_eq!(protection(&d.join("sys/etc/passwd")), Protection::Protected);
            assert_eq!(protection(&d.join("sys")), Protection::Safe, "the link itself is removable");
        }
        std::fs::remove_dir_all(&d).unwrap();
    }
}
