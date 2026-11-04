//! Is a path on a network filesystem? The mount table is read without touching any mount (never
//! `statfs` per mount: that's the call that hangs), so `network_mount_of` is safe on any thread.
//! `is_network_at` resolves symlinks too, by asking the filesystem: run it through `guard`.
use std::path::Path;

pub struct Mount {
    pub path: String,
    pub fs: String,
}

/// Filesystem types whose calls can wait on a remote server. FUSE counts unless it's block-backed:
/// sshfs, rclone, s3fs and friends all look alike. Misjudging a local one costs it auto-preview and
/// live updates (no watcher), which is the safe side of the mistake.
pub fn is_network_fs(fs: &str) -> bool {
    let fs = fs.to_ascii_lowercase();
    const NET: [&str; 19] = ["nfs", "nfs4", "cifs", "smb", "smb2", "smb3", "smbfs", "afpfs", "afs", "webdav", "ncpfs", "ncp", "9p", "ceph", "coda", "lustre", "glusterfs", "ftp", "autofs"];
    NET.contains(&fs.as_str()) || fs.starts_with("fuse.") || matches!(fs.as_str(), "fuse" | "fusefs" | "osxfuse" | "macfuse" | "sshfs") || fs.starts_with("macfuse") || fs.starts_with("osxfuse")
}

/// Kernel bookkeeping mounts sysinfo also leaves out; only matters when it's unavailable.
pub fn is_pseudo_fs(fs: &str) -> bool {
    const PSEUDO: [&str; 18] = ["tmpfs", "devtmpfs", "devpts", "devfs", "proc", "sysfs", "securityfs", "pstore", "bpf", "debugfs", "tracefs", "configfs", "fusectl", "hugetlbfs", "mqueue", "efivarfs", "binfmt_misc", "overlay"];
    PSEUDO.contains(&fs) || fs.starts_with("cgroup")
}

/// Longest mount point containing `path`, compared by name only.
fn containing<'a>(mounts: &'a [Mount], path: &Path) -> Option<&'a Mount> {
    mounts.iter().filter(|m| path.starts_with(&m.path)).max_by_key(|m| m.path.len())
}

/// The network mount `path` sits on by name, if any. Doesn't follow symlinks.
pub fn network_mount_of(path: &Path) -> Option<String> {
    if let Some(root) = unc_root(path) {
        return Some(root);
    }
    let mounts = table();
    containing(&mounts, path).filter(|m| is_network_fs(&m.fs)).map(|m| m.path.clone())
}

/// `\\server\share` of a UNC path, the network mount it names. `\\?\` and `\\.\` are local namespaces.
fn unc_root(path: &Path) -> Option<String> {
    let s = path.to_str()?;
    if !cfg!(windows) {
        return None;
    }
    let rest = s.strip_prefix(r"\\").or_else(|| s.strip_prefix("//"))?;
    if rest.starts_with('?') || rest.starts_with('.') {
        return None;
    }
    let mut parts = rest.split(['\\', '/']);
    Some(format!(r"\\{}\{}", parts.next()?, parts.next().unwrap_or_default()))
}

#[cfg(target_os = "linux")]
pub fn table() -> Vec<Mount> {
    // A kernel view: reading it doesn't reach the filesystems listed.
    parse_proc_mounts(&std::fs::read_to_string("/proc/self/mounts").unwrap_or_default())
}

#[cfg(any(target_os = "linux", test))]
fn parse_proc_mounts(s: &str) -> Vec<Mount> {
    let unesc = |s: &str| s.replace("\\040", " ").replace("\\011", "\t").replace("\\043", "#").replace("\\012", "\n").replace("\\134", "\\");
    s.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            (f.len() > 2).then(|| Mount { path: unesc(f[1]), fs: f[2].to_string() })
        })
        .collect()
}

#[cfg(target_os = "macos")]
pub fn table() -> Vec<Mount> {
    use std::ffi::CStr;
    // MNT_NOWAIT returns what the kernel has cached, without asking any filesystem.
    // SAFETY: a null buffer asks for the count; then the buffer holds `n` entries of the stated size.
    unsafe {
        let n = libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT);
        if n <= 0 {
            return vec![];
        }
        let mut buf: Vec<libc::statfs> = Vec::with_capacity(n as usize + 4);
        let size = (buf.capacity() * std::mem::size_of::<libc::statfs>()) as libc::c_int;
        let n = libc::getfsstat(buf.as_mut_ptr(), size, libc::MNT_NOWAIT);
        if n <= 0 {
            return vec![];
        }
        buf.set_len(n as usize);
        buf.iter()
            .map(|s| Mount {
                path: CStr::from_ptr(s.f_mntonname.as_ptr()).to_string_lossy().into_owned(),
                fs: CStr::from_ptr(s.f_fstypename.as_ptr()).to_string_lossy().into_owned(),
            })
            .collect()
    }
}

/// Mapped network drives. GetDriveTypeW answers from the drive's registration, not the server.
#[cfg(windows)]
pub fn table() -> Vec<Mount> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    const DRIVE_REMOTE: u32 = 4;
    // SAFETY: no arguments.
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| {
            let path = format!("{}:\\", (b'A' + i) as char);
            let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
            // SAFETY: a valid NUL-terminated string.
            (unsafe { GetDriveTypeW(wide.as_ptr()) } == DRIVE_REMOTE).then(|| Mount { path, fs: "smb".into() })
        })
        .collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn table() -> Vec<Mount> {
    vec![]
}

/// Asks the filesystem, so it sees through symlinks; may block. `None` if it can't tell.
#[cfg(target_os = "linux")]
fn statfs_network(path: &Path) -> Option<bool> {
    use std::os::unix::ffi::OsStrExt;
    // The table tells fuseblk (local NTFS, exFAT) from FUSE, which share one magic number.
    if let Ok(real) = std::fs::canonicalize(path) {
        if let Some(m) = containing(&table(), &real) {
            return Some(is_network_fs(&m.fs));
        }
    }
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: a valid path and an out-pointer of the right type, read only on success.
    if unsafe { libc::statfs(c.as_ptr(), s.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statfs succeeded.
    Some(is_network_magic(unsafe { s.assume_init() }.f_type as u32 as u64))
}

/// Only when the mount table has no answer. linux/magic.h, plus the CIFS, SMB2, Ceph and AFS values defined in their own headers.
#[cfg(any(target_os = "linux", test))]
fn is_network_magic(t: u64) -> bool {
    const NFS: u64 = 0x6969;
    const SMB: u64 = 0x517B;
    const SMB2: u64 = 0xFE53_4D42;
    const CIFS: u64 = 0xFF53_4D42;
    const FUSE: u64 = 0x6573_5546;
    const CEPH: u64 = 0x00C3_6400;
    const AFS: u64 = 0x5346_414F;
    const V9FS: u64 = 0x0102_1997;
    const CODA: u64 = 0x7375_7245;
    const NCP: u64 = 0x564C;
    [NFS, SMB, SMB2, CIFS, FUSE, CEPH, AFS, V9FS, CODA, NCP].contains(&t)
}

#[cfg(target_os = "macos")]
fn statfs_network(path: &Path) -> Option<bool> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: a valid path and an out-pointer of the right type, read only on success.
    if unsafe { libc::statfs(c.as_ptr(), s.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statfs succeeded, and f_fstypename is NUL-terminated.
    let name = unsafe { std::ffi::CStr::from_ptr(s.assume_init_ref().f_fstypename.as_ptr()) };
    Some(is_network_fs(&name.to_string_lossy()))
}

/// Symlinks to shares don't get a drive type of their own; the name check covers drives and UNC.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn statfs_network(_: &Path) -> Option<bool> {
    None
}

/// True for network filesystems, following symlinks. May block: call it through `guard`.
pub fn is_network_at(path: &Path) -> bool {
    statfs_network(path).unwrap_or(false)
}

/// (total, free) bytes. May block: call it through `guard`.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)] // field widths differ between Linux and macOS
pub fn space(path: &str) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut s = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: a valid path and an out-pointer of the right type, read only on success.
    if unsafe { libc::statvfs(c.as_ptr(), s.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statvfs succeeded.
    let s = unsafe { s.assume_init() };
    let unit = s.f_frsize as u64;
    Some((s.f_blocks as u64 * unit, s.f_bavail as u64 * unit))
}

#[cfg(windows)]
pub fn space(path: &str) -> Option<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: a valid NUL-terminated string and out-pointers; the third is optional.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) };
    (ok != 0).then_some((total, free))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_filesystem_names() {
        for fs in ["nfs", "nfs4", "cifs", "smb3", "smbfs", "fuse.sshfs", "fuse.rclone", "macfuse", "webdav", "afpfs"] {
            assert!(is_network_fs(fs), "{fs}");
        }
        assert!(!is_network_fs("fuseblk"));
        for fs in ["ext4", "btrfs", "apfs", "hfs", "tmpfs", "fuseblk", "NTFS", "exfat", "msdos"] {
            assert!(!is_network_fs(fs), "{fs}");
        }
    }

    #[test]
    fn classifies_linux_magic() {
        assert!(is_network_magic(0x6969));
        assert!(is_network_magic(0xFE53_4D42));
        assert!(is_network_magic(0x6573_5546));
        assert!(!is_network_magic(0xEF53)); // ext4
    }

    #[test]
    fn parses_proc_mounts_and_picks_the_longest_prefix() {
        let t = parse_proc_mounts("/dev/sda2 / ext4 rw 0 0\nsrv:/x /mnt/my\\040nas nfs4 rw 0 0\nx /mnt/a\\043b ext4 rw 0 0\nuser@h: /mnt/ssh fuse.sshfs rw 0 0\n");
        assert_eq!(t[1].path, "/mnt/my nas");
        assert_eq!(t[2].path, "/mnt/a#b");
        assert_eq!(containing(&t, Path::new("/mnt/my nas/a/b")).unwrap().fs, "nfs4");
        assert_eq!(containing(&t, Path::new("/mnt/ssh")).unwrap().fs, "fuse.sshfs");
        assert_eq!(containing(&t, Path::new("/mnt/sshfoo")).unwrap().fs, "ext4"); // whole components only
    }

    #[test]
    fn table_lookup_returns_promptly_for_a_local_path() {
        // Whatever this machine mounts, its temp dir isn't expected to be remote in CI or dev.
        let _ = network_mount_of(&std::env::temp_dir());
    }
}
