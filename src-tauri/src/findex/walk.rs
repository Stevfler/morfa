//! What gets indexed, and the single-threaded walk that finds it. Doesn't follow symlinks, doesn't
//! leave the root's filesystem, and goes easy on the machine: background priority where the OS has
//! one, and short sleeps between folders.
use super::tree::{os_bytes, Tree, DIR, HIDDEN};
use std::{
    collections::HashSet,
    fs::Metadata,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, UNIX_EPOCH},
};

const CASE_BLIND: bool = cfg!(any(target_os = "macos", windows));
/// Any folder or file with one of these names, at any depth. The last few are folders whose file
/// names are themselves secrets.
const NAMES: &[&str] = &["node_modules", ".git", ".hg", ".svn", "__pycache__", ".cache", ".Trash", ".Trashes", "$RECYCLE.BIN", "System Volume Information", ".Spotlight-V100", ".fseventsd", ".DocumentRevisions-V100", ".TemporaryItems", ".ssh", ".gnupg", ".aws", ".kube", ".password-store", ".pki", "Keychains"];
/// Relative to home: caches, app sandboxes, online-only cloud folders (listing them can start
/// downloads) and trash.
const HOME: &[&str] = &["Library/Caches", "Library/CloudStorage", "Library/Mobile Documents", "Library/Containers", "Library/Group Containers", "Library/Logs", ".local/share/Trash", "AppData/Local/Temp", "AppData/Local/Microsoft/Windows/INetCache"];
#[cfg(target_os = "macos")]
const SYSTEM: &[&str] = &["/System", "/Library", "/private", "/usr", "/bin", "/sbin", "/etc", "/var", "/tmp", "/cores", "/dev", "/Volumes"];
#[cfg(not(any(target_os = "macos", windows)))]
const SYSTEM: &[&str] = &["/proc", "/sys", "/dev", "/run", "/boot", "/efi", "/tmp", "/usr", "/etc", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32", "/var", "/root", "/lost+found", "/snap", "/nix"];
#[cfg(windows)]
const SYSTEM: &[&str] = &["Windows", "Program Files", "Program Files (x86)", "ProgramData", "Recovery", "PerfLogs"];

pub struct Rules {
    names: HashSet<Vec<u8>>,
    prefixes: Vec<PathBuf>,
    mounts: HashSet<PathBuf>,
}

fn norm(b: &[u8]) -> Vec<u8> {
    if CASE_BLIND {
        super::query::fold_bytes(b)
    } else {
        b.to_vec()
    }
}
fn norm_path(p: &Path) -> PathBuf {
    if CASE_BLIND {
        super::tree::bytes_os(&norm(&os_bytes(p.as_os_str()))).into()
    } else {
        p.to_path_buf()
    }
}

impl Rules {
    /// `user`: a bare name excludes that name anywhere; `~/x` or an absolute path excludes that path.
    pub fn new(user: &[String], home: Option<&Path>, roots: &[PathBuf], mounts: impl IntoIterator<Item = PathBuf>) -> Rules {
        let mut names: HashSet<Vec<u8>> = NAMES.iter().map(|n| norm(n.as_bytes())).collect();
        let mut prefixes = vec![];
        if let Some(h) = home {
            prefixes.extend(HOME.iter().map(|r| h.join(r)));
        }
        #[cfg(windows)]
        for r in roots {
            if let Some(std::path::Component::Prefix(d)) = r.components().next() {
                let drive = Path::new(d.as_os_str()).join("\\");
                prefixes.extend(SYSTEM.iter().map(|s| drive.join(s)));
            }
        }
        #[cfg(not(windows))]
        {
            // A root the user picked inside one (an external disk under /Volumes) was asked for.
            prefixes.extend(SYSTEM.iter().map(PathBuf::from).filter(|s| !roots.iter().any(|r| r.starts_with(s))));
        }
        for u in user.iter().map(|u| u.trim().trim_end_matches(['/', '\\'])).filter(|u| !u.is_empty()) {
            if let (Some(rest), Some(h)) = (u.strip_prefix("~/").or_else(|| u.strip_prefix("~\\")), home) {
                prefixes.push(h.join(rest));
            } else if u.contains(['/', '\\']) {
                prefixes.push(PathBuf::from(u));
            } else {
                names.insert(norm(u.as_bytes()));
            }
        }
        Rules { names, prefixes: prefixes.iter().map(|p| norm_path(p)).collect(), mounts: mounts.into_iter().collect() }
    }

    /// One more location, always left out (the app's own cache).
    pub fn exclude_path(&mut self, p: &Path) {
        self.prefixes.push(norm_path(p));
    }

    /// Excluded by name or location. Mounts are `crosses`'s business.
    pub fn excluded(&self, path: &Path) -> bool {
        let name_hit = path.file_name().is_some_and(|n| self.names.contains(&norm(&os_bytes(n))));
        name_hit || {
            let p = norm_path(path);
            self.prefixes.iter().any(|x| p.starts_with(x))
        }
    }

    /// Is a folder inside `root` on another mount? `dev` is 0 where there are no device numbers.
    pub fn crosses(&self, path: &Path, root: &Path, dev: u64, root_dev: u64) -> bool {
        path != root && (dev != root_dev || self.mounts.contains(path))
    }

    /// Every component below `root` passes: for paths from events, which arrive from anywhere.
    pub fn allows(&self, path: &Path, root: &Path) -> bool {
        let mut cur = root.to_path_buf();
        let Ok(rest) = path.strip_prefix(root) else { return false };
        for c in rest.components() {
            cur.push(c);
            if self.excluded(&cur) || self.mounts.contains(&cur) {
                return false;
            }
        }
        true
    }
}

pub fn mtime_of(m: &Metadata) -> u32 {
    m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs().min(u32::MAX as u64) as u32)
}

#[cfg(unix)]
pub fn dev_of(m: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    m.dev()
}
#[cfg(not(unix))]
pub fn dev_of(_: &Metadata) -> u64 {
    0 // Junctions and mount points read as symlinks here, which the walk never follows.
}

/// A folder whose listing would have to come from the cloud.
#[cfg(target_os = "macos")]
pub fn online_only(m: &Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    const SF_DATALESS: u32 = 0x4000_0000;
    m.st_flags() & SF_DATALESS != 0
}
#[cfg(windows)]
pub fn online_only(m: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const OFFLINE: u32 = 0x1000;
    const RECALL_ON_OPEN: u32 = 0x4_0000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
    m.file_attributes() & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn online_only(_: &Metadata) -> bool {
    false
}

/// What the live walk's `hidden` filter skips: dotfiles, and on Windows the attribute. macOS's
/// `UF_HIDDEN` isn't in it (the walk does enter `~/Library`).
pub fn flags_of(name: &[u8], m: &Metadata) -> u8 {
    let mut f = if m.file_type().is_dir() { DIR } else { 0 };
    if name.first() == Some(&b'.') || hidden_attr(m) {
        f |= HIDDEN;
    }
    f
}
#[cfg(windows)]
fn hidden_attr(m: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & 0x2 != 0
}
#[cfg(not(windows))]
fn hidden_attr(_: &Metadata) -> bool {
    false
}

pub struct Ctl<'a> {
    pub stop: &'a (dyn Fn() -> bool + Sync),
    pub paused: &'a (dyn Fn() -> bool + Sync),
    pub seen: &'a AtomicU64,
    pub throttle: bool,
}

/// Add everything under the directory node `top`. False if stopped.
pub fn walk_into(tree: &mut Tree, top: u32, path: &Path, root: &Path, root_dev: u64, rules: &Rules, ctl: &Ctl) -> bool {
    let mut stack = vec![(top, path.to_path_buf())];
    let mut dirs = 0u32;
    while let Some((node, dir)) = stack.pop() {
        while (ctl.paused)() && !(ctl.stop)() {
            std::thread::sleep(Duration::from_millis(100));
        }
        if (ctl.stop)() {
            return false;
        }
        dirs += 1;
        if ctl.throttle && dirs.is_multiple_of(64) {
            std::thread::sleep(Duration::from_millis(1));
        }
        // Unreadable folders stay in the index, empty; the next reconciliation tries again.
        let Ok(list) = std::fs::read_dir(&dir) else { continue };
        for e in list.flatten() {
            let name = e.file_name();
            let p = dir.join(&name);
            if rules.excluded(&p) {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            let raw = os_bytes(&name);
            let is_dir = m.file_type().is_dir();
            if is_dir && (rules.crosses(&p, root, dev_of(&m), root_dev) || online_only(&m)) {
                continue;
            }
            if tree.is_full() {
                tree.truncated = true;
                return true;
            }
            let id = tree.add(node, &raw, mtime_of(&m), flags_of(&raw, &m));
            ctl.seen.fetch_add(1, Ordering::Relaxed);
            if is_dir {
                stack.push((id, p));
            }
        }
    }
    true
}

/// A full walk of every root. `None` if stopped.
pub fn build(roots: &[(PathBuf, u64)], rules: &Rules, ctl: &Ctl) -> Option<Tree> {
    let mut tree = Tree::new(roots.to_vec());
    for (i, (r, dev)) in roots.iter().enumerate() {
        if !walk_into(&mut tree, i as u32, r, r, *dev, rules, ctl) {
            return None;
        }
    }
    tree.shrink();
    Some(tree)
}

/// Background priority for this thread, CPU and disk both where the OS separates them.
pub fn lower_thread_priority() {
    // SAFETY: plain calls on the current thread with constant arguments; failure just leaves the
    // priority as it was.
    #[cfg(target_os = "macos")]
    unsafe {
        libc::setpriority(libc::PRIO_DARWIN_THREAD, 0, libc::PRIO_DARWIN_BG);
    }
    #[cfg(target_os = "linux")]
    unsafe {
        // Per-thread on Linux, despite the name. Class 3 is "idle" in ioprio_set(2).
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 19);
        libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13);
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
        SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}
