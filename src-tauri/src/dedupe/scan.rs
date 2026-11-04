//! The pipeline: walk, group by size, merge hard links, hash head and tail, then the whole file.
//! Each stage only reads what the last one couldn't tell apart.
use crate::{
    cleanup::{is_trash_dir, Protection},
    cloud::sync::ignored,
    compare::{is_own_temp, kind_of, mtime_ms, Job, Kind, CANCELLED},
    CmdResult,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File as FsFile},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Instant,
};

/// Bytes hashed at each end of a file in the partial stage.
pub(super) const EDGE: u64 = 64 * 1024;
const MAX_DEPTH: usize = 200;
const CHUNK: usize = 1 << 20;
/// Placeholders named in the result; the count covers them all.
const PLACEHOLDER_LIST: usize = 200;

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Walk,
    Partial,
    Full,
}

/// Deleting here reaches a server too.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Cloud {
    /// A folder synced by Morfa's own cloud accounts.
    Mirror,
    /// A folder of a desktop sync client, recognised by its location.
    Provider,
}

/// What makes a file itself rather than another name for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Id {
    pub dev: u64,
    pub ino: u64,
    pub links: u64,
    /// Inode change time, ns: moves on any write or chmod, whatever the mtime says. 0 on Windows.
    pub ctime: u64,
}
impl Id {
    fn same(&self, o: &Id) -> bool {
        self.dev == o.dev && self.ino == o.ino
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DupFile {
    pub path: String,
    pub size: u64,
    pub mtime: u64,
    /// Names of this file on disk, seen or not. Above 1, removing this name frees nothing.
    pub links: u64,
    /// Other names of it found by the scan. They stay when this one goes.
    pub aliases: Vec<String>,
    pub cloud: Option<Cloud>,
    pub protection: Protection,
    /// The package or container this file is a part of: removing it can damage the whole.
    pub whole: Option<String>,
    #[serde(skip)]
    pub id: Id,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub hash: String,
    pub size: u64,
    pub reclaimable: u64,
    pub files: Vec<DupFile>,
}

#[derive(Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub unreadable: u64,
    pub special: u64,
    pub symlinks: u64,
    pub other_filesystem: u64,
    /// Changed while being read.
    pub changed: u64,
    pub below_minimum: u64,
}

/// How far files got through the pipeline.
#[derive(Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Stages {
    pub walked: u64,
    pub same_size: u64,
    pub partial_hashed: u64,
    pub full_hashed: u64,
    pub bytes_read: u64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub scan_id: u64,
    pub root: String,
    pub min_size: u64,
    pub groups: Vec<Group>,
    pub reclaimable: u64,
    pub skipped: Skipped,
    pub stages: Stages,
    /// Stored online only: reading one would download it, so it's never compared.
    pub placeholders: Vec<String>,
    pub placeholder_count: u64,
    pub elapsed_ms: u64,
}

pub struct Options<'a> {
    pub min_size: u64,
    pub mirrors: &'a [PathBuf],
    pub home: Option<&'a Path>,
    pub protection: &'a (dyn Fn(&Path) -> Protection + Sync),
}

#[derive(Clone, Debug)]
pub(super) struct Found {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: u64,
}

/// Opens without following a link and without blocking on a FIFO that appeared after the walk.
pub(super) fn open_file(p: &Path) -> io::Result<FsFile> {
    let mut o = fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        o.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    o.open(p)
}

#[cfg(unix)]
fn id_of_meta(m: &fs::Metadata) -> Id {
    use std::os::unix::fs::MetadataExt;
    Id { dev: m.dev(), ino: m.ino(), links: m.nlink(), ctime: (m.ctime() as u64).wrapping_mul(1_000_000_000).wrapping_add(m.ctime_nsec() as u64) }
}

/// `identity`, for a caller that already has the metadata (on Windows it isn't in there).
pub(super) fn id_of_stat(_p: &Path, _m: &fs::Metadata) -> io::Result<Id> {
    #[cfg(unix)]
    {
        Ok(id_of_meta(_m))
    }
    #[cfg(windows)]
    {
        identity(_p)
    }
}

#[cfg(unix)]
fn id_of_open(f: &FsFile) -> io::Result<Id> {
    Ok(id_of_meta(&f.metadata()?))
}

#[cfg(windows)]
fn id_of_open(f: &FsFile) -> io::Result<Id> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the handle is open for the duration of the call, and `info` is a plain out-struct.
    if unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Id { dev: info.dwVolumeSerialNumber as u64, ino: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64, links: info.nNumberOfLinks as u64, ctime: 0 })
}

/// The identity of what's at `p`, itself and never what it links to.
pub(super) fn identity(p: &Path) -> io::Result<Id> {
    #[cfg(unix)]
    {
        Ok(id_of_meta(&fs::symlink_metadata(p)?))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        // No data access: asks nothing of a sync provider.
        id_of_open(&fs::OpenOptions::new().access_mode(0).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(p)?)
    }
}

/// A file whose content is only on a server. On Linux nothing marks one in a way we could trust.
pub(super) fn is_placeholder(p: &Path, m: &fs::Metadata) -> bool {
    let stub = p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.') && n.ends_with(".icloud"));
    stub || offline_flag(m)
}

#[cfg(target_os = "macos")]
fn offline_flag(m: &fs::Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    const SF_DATALESS: u32 = 0x4000_0000;
    m.st_flags() & SF_DATALESS != 0
}
#[cfg(windows)]
fn offline_flag(m: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const OFFLINE: u32 = 0x1000;
    const RECALL_ON_OPEN: u32 = 0x0004_0000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    m.file_attributes() & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0
}
#[cfg(not(any(target_os = "macos", windows)))]
fn offline_flag(_m: &fs::Metadata) -> bool {
    false
}

pub(super) fn cloud_kind(p: &Path, mirrors: &[PathBuf], home: Option<&Path>) -> Option<Cloud> {
    if mirrors.iter().any(|m| p.starts_with(m)) {
        return Some(Cloud::Mirror);
    }
    let rest = home.and_then(|h| p.strip_prefix(h).ok())?;
    if rest.starts_with("Library/CloudStorage") || rest.starts_with("Library/Mobile Documents") {
        return Some(Cloud::Provider);
    }
    let first = rest.components().next()?.as_os_str().to_str()?.to_lowercase();
    const CLIENTS: &[&str] = &["onedrive", "dropbox", "google drive", "googledrive", "icloud drive", "iclouddrive", "nextcloud", "owncloud", "yandex.disk", "pcloud", "box"];
    // "OneDrive - Company", "Dropbox (Team)".
    CLIENTS.iter().any(|c| first == *c || first.strip_prefix(c).is_some_and(|r| r.starts_with(" -") || r.starts_with(" ("))).then_some(Cloud::Provider)
}

/// Folders macOS shows as one item, and their kin elsewhere: one file out of one breaks all of it.
const PACKAGES: &[&str] = &[
    "app", "framework", "bundle", "plugin", "kext", "xpc", "appex", "prefpane", "qlgenerator", "saver", "mdimporter", "wdgt", "dsym", "pkg", "mpkg",
    "sparsebundle", "backupbundle", "xcdatamodeld", "imovieproject", "tmbundle", "photoslibrary", "photolibrary", "migratedphotolibrary", "aplibrary", "lrlibrary", "lrdata", "fcpbundle", "imovielibrary", "tvlibrary", "musiclibrary", "theater",
    "logicx", "band", "pages", "numbers", "key", "rtfd", "scptd", "xcodeproj", "xcworkspace", "xcarchive", "xcassets", "playground", "docarchive", "vmwarevm", "pvm", "utm", "vbox",
];
/// Containers by exact name, on every OS.
const CONTAINERS: &[&str] = &[
    "backups.backupdb", ".git", ".hg", ".svn", ".bzr", "node_modules", "bower_components", ".venv", "venv", "site-packages", "__pycache__", ".tox", "pods", ".gradle", ".m2", ".cargo", ".rustup",
    "virtualbox vms", "virtual machines", "virtual machines.localized", "parallels", ".docker", ".vagrant", "steamapps", "windowsapps",
];

/// The named folder that `p` is a part of, outermost first, if any.
pub(super) fn whole_of(p: &Path) -> Option<String> {
    let mut cur = PathBuf::new();
    for c in p.parent()?.components() {
        cur.push(c);
        let Some(name) = c.as_os_str().to_str() else { continue };
        let low = name.to_lowercase();
        let package = low.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && PACKAGES.contains(&ext));
        // A package we have no name for still has the usual layout.
        let laid_out = cfg!(target_os = "macos") && low.contains('.') && cur.join("Contents/Info.plist").is_file();
        // A sparse image under any name.
        let bands = cur.join("bands").is_dir() && (cur.join("Info.plist").is_file() || cur.join("token").is_file());
        if package || laid_out || bands || CONTAINERS.contains(&low.as_str()) {
            return Some(name.to_string());
        }
    }
    None
}

const DATA_VOLUME: &str = "/System/Volumes/Data";

/// The short spelling of a path on the macOS data volume, if `same` says it's the same item.
pub(super) fn firmlink_spelling(p: &Path, same: &dyn Fn(&Path, &Path) -> bool) -> Option<PathBuf> {
    let rest = p.strip_prefix(DATA_VOLUME).ok()?;
    let short = Path::new("/").join(rest);
    (!rest.as_os_str().is_empty() && same(p, &short)).then_some(short)
}
fn same_item(a: &Path, b: &Path) -> bool {
    crate::fsops::rename::same_file(a, b)
}
fn preferred(p: PathBuf) -> PathBuf {
    if cfg!(target_os = "macos") {
        firmlink_spelling(&p, &same_item).unwrap_or(p)
    } else {
        p
    }
}
/// `/` on macOS is the sealed system volume: the user's files are on the data volume, reached through
/// firmlinks. Returns where to walk, and the root paths will be reported under.
pub(super) fn roots_for(root: &Path) -> (PathBuf, PathBuf) {
    if !cfg!(target_os = "macos") {
        return (root.to_path_buf(), root.to_path_buf());
    }
    if root == Path::new("/") || root == Path::new(DATA_VOLUME) {
        return (PathBuf::from(DATA_VOLUME), PathBuf::from("/"));
    }
    (root.to_path_buf(), preferred(root.to_path_buf()))
}

fn is_trash_name(n: &str) -> bool {
    n == ".Trash" || n == ".Trashes" || n.eq_ignore_ascii_case("$RECYCLE.BIN") || n.starts_with(".Trash-")
}

fn same_filesystem(m: &fs::Metadata, root: Option<u64>) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        root.is_none_or(|r| m.dev() == r)
    }
    #[cfg(not(unix))]
    {
        let _ = (m, root);
        true
    }
}
fn device_of(_m: &fs::Metadata) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(_m.dev())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

pub(super) struct Walked {
    pub found: Vec<Found>,
    pub skipped: Skipped,
    pub walked: u64,
    pub placeholders: Vec<String>,
    pub placeholder_count: u64,
}

/// One filesystem, like the clean-up scan. Trash is left out: a copy there isn't a kept copy.
pub(super) fn walk(root: &Path, min_size: u64, job: &Job, report: &mut dyn FnMut(Phase, u64, u64, u64, &Path)) -> CmdResult<Walked> {
    let mut w = Walked { found: vec![], skipped: Skipped::default(), walked: 0, placeholders: vec![], placeholder_count: 0 };
    let dev = device_of(&fs::symlink_metadata(root).map_err(|e| format!("{}: {e}", root.display()))?);
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut bytes = 0;
    while let Some((dir, depth)) = stack.pop() {
        if job.stopped() {
            return Err(CANCELLED.into());
        }
        let list = if depth >= MAX_DEPTH { None } else { fs::read_dir(&dir).ok() };
        let Some(list) = list else {
            w.skipped.unreadable += 1;
            continue;
        };
        for e in list {
            if job.stopped() {
                return Err(CANCELLED.into());
            }
            let Ok(e) = e else {
                w.skipped.unreadable += 1;
                continue;
            };
            let name = e.file_name();
            let path = e.path();
            let Ok(m) = fs::symlink_metadata(&path) else {
                w.skipped.unreadable += 1;
                continue;
            };
            match kind_of(&m) {
                Kind::Symlink => w.skipped.symlinks += 1,
                Kind::Special => w.skipped.special += 1,
                Kind::Dir => {
                    if name.to_str().is_some_and(is_trash_name) || is_trash_dir(&path) {
                        continue;
                    }
                    if same_filesystem(&m, dev) {
                        stack.push((path, depth + 1));
                    } else {
                        w.skipped.other_filesystem += 1;
                    }
                }
                Kind::File => {
                    if is_own_temp(&name) || name.to_str().is_some_and(ignored) {
                        continue;
                    }
                    w.walked += 1;
                    // A plan names paths as text, so a name that doesn't survive that can't be in one.
                    if path.to_str().is_none() {
                        w.skipped.unreadable += 1;
                    } else if m.len() < min_size.max(1) {
                        w.skipped.below_minimum += 1;
                    } else if is_placeholder(&path, &m) {
                        w.placeholder_count += 1;
                        if w.placeholders.len() < PLACEHOLDER_LIST {
                            w.placeholders.push(path.to_string_lossy().into_owned());
                        }
                    } else {
                        bytes += m.len();
                        w.found.push(Found { path: path.clone(), size: m.len(), mtime: mtime_ms(&m) });
                    }
                    report(Phase::Walk, w.walked, 0, bytes, &path);
                }
            }
        }
    }
    Ok(w)
}

/// Sizes that occur more than once, in a stable order.
pub(super) fn by_size(found: Vec<Found>) -> Vec<Vec<Found>> {
    let mut m: BTreeMap<u64, Vec<Found>> = BTreeMap::new();
    for f in found {
        m.entry(f.size).or_default().push(f);
    }
    m.into_values().filter(|g| g.len() > 1).collect()
}

pub(super) struct Linked {
    pub file: Found,
    pub id: Id,
    pub aliases: Vec<String>,
}

/// One entry per file on disk: names of the same file free nothing, so they aren't duplicates.
pub(super) fn merge_links(group: Vec<(Found, Id)>) -> Vec<Linked> {
    let mut by: HashMap<(u64, u64), Linked> = HashMap::new();
    let mut sorted = group;
    // The name that stands for the file is the plain one, never the data volume's long spelling.
    sorted.sort_by(|a, b| a.0.path.starts_with(DATA_VOLUME).cmp(&b.0.path.starts_with(DATA_VOLUME)).then_with(|| a.0.path.cmp(&b.0.path)));
    for (f, id) in sorted {
        match by.get_mut(&(id.dev, id.ino)) {
            Some(l) => l.aliases.push(f.path.to_string_lossy().into_owned()),
            None => {
                by.insert((id.dev, id.ino), Linked { file: f, id, aliases: vec![] });
            }
        }
    }
    let mut out: Vec<Linked> = by.into_values().collect();
    out.sort_by(|a, b| a.file.path.cmp(&b.file.path));
    out
}

/// Open `p`, provided it's still the regular file `id` names.
fn open_checked(p: &Path, id: &Id) -> Result<FsFile, String> {
    let f = open_file(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let now = id_of_open(&f).map_err(|e| format!("{}: {e}", p.display()))?;
    let is_file = f.metadata().map_err(|e| e.to_string())?.is_file();
    if !is_file || !now.same(id) {
        return Err(format!("{}: changed", p.display()));
    }
    Ok(f)
}

/// Head and tail. For a file up to twice `EDGE` that's all of it.
pub(super) fn hash_partial(p: &Path, id: &Id, size: u64, job: &Job) -> Result<String, String> {
    if job.stopped() {
        return Err(CANCELLED.into());
    }
    let mut f = open_checked(p, id)?;
    let mut h = Sha256::new();
    let mut read = |f: &mut FsFile, n: u64| -> io::Result<()> {
        let mut buf = vec![0u8; n as usize];
        f.read_exact(&mut buf)?;
        h.update(&buf);
        Ok(())
    };
    let err = |e: io::Error| format!("{}: {e}", p.display());
    if size <= 2 * EDGE {
        read(&mut f, size).map_err(err)?;
    } else {
        read(&mut f, EDGE).map_err(err)?;
        f.seek(SeekFrom::Start(size - EDGE)).map_err(err)?;
        read(&mut f, EDGE).map_err(err)?;
    }
    Ok(hex(&h.finalize()))
}

/// SHA-256 of the whole file, streamed; `on_bytes` hears of every chunk. An error other than
/// `CANCELLED` means this file couldn't be read to the expected end.
pub(super) fn hash_full(p: &Path, id: &Id, size: u64, job: &Job, on_bytes: &mut dyn FnMut(u64)) -> Result<String, String> {
    let mut f = open_checked(p, id)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        if job.stopped() {
            return Err(CANCELLED.into());
        }
        let n = f.read(&mut buf).map_err(|e| format!("{}: {e}", p.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        total += n as u64;
        on_bytes(n as u64);
    }
    if total != size {
        return Err(format!("{}: changed", p.display()));
    }
    Ok(hex(&h.finalize()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Bytes removing all but one could free: names with other links free nothing.
pub(super) fn reclaimable_of(g: &Group) -> u64 {
    let freeable = g.files.iter().filter(|f| f.links <= 1).count() as u64;
    g.size * freeable.min(g.files.len() as u64 - 1)
}

/// The same size, time and identity as when last looked at?
fn unchanged(l: &Linked) -> bool {
    fs::symlink_metadata(&l.file.path).is_ok_and(|m| m.is_file() && m.len() == l.file.size && mtime_ms(&m) == l.file.mtime && id_of_stat(&l.file.path, &m).is_ok_and(|i| i.same(&l.id) && i.ctime == l.id.ctime))
}

pub fn scan(root: &Path, o: &Options, job: &Job, report: &mut dyn FnMut(Phase, u64, u64, u64, &Path)) -> CmdResult<ScanResult> {
    let started = Instant::now();
    let (walk_root, shown_root) = roots_for(root);
    let w = walk(&walk_root, o.min_size, job, report)?;
    let mut skipped = w.skipped;
    let mut stages = Stages { walked: w.walked, ..Stages::default() };

    let mut candidates: Vec<Vec<Linked>> = vec![];
    for bucket in by_size(w.found) {
        let mut ids = Vec::with_capacity(bucket.len());
        for mut f in bucket {
            if job.stopped() {
                return Err(CANCELLED.into());
            }
            f.path = preferred(f.path);
            match identity(&f.path) {
                Ok(id) => ids.push((f, id)),
                Err(_) => skipped.unreadable += 1,
            }
        }
        let linked = merge_links(ids);
        if linked.len() > 1 {
            stages.same_size += linked.len() as u64;
            candidates.push(linked);
        }
    }

    let total = stages.same_size;
    let mut done = 0;
    let mut partial: Vec<Vec<Linked>> = vec![];
    for bucket in candidates {
        let mut by: BTreeMap<String, Vec<Linked>> = BTreeMap::new();
        for l in bucket {
            done += 1;
            report(Phase::Partial, done, total, stages.bytes_read, &l.file.path);
            match hash_partial(&l.file.path, &l.id, l.file.size, job) {
                Ok(h) => {
                    stages.partial_hashed += 1;
                    stages.bytes_read += (2 * EDGE).min(l.file.size);
                    by.entry(h).or_default().push(l);
                }
                Err(e) if e == CANCELLED => return Err(e),
                Err(_) => skipped.unreadable += 1,
            }
        }
        partial.extend(by.into_values().filter(|g| g.len() > 1));
    }

    let total: u64 = partial.iter().flatten().map(|l| l.file.size).sum();
    let mut hashed = 0u64;
    let mut groups = vec![];
    for bucket in partial {
        let mut by: BTreeMap<String, Vec<Linked>> = BTreeMap::new();
        for l in bucket {
            report(Phase::Full, hashed, total, stages.bytes_read, &l.file.path);
            let mut read = 0;
            let h = hash_full(&l.file.path, &l.id, l.file.size, job, &mut |n| {
                read += n;
                report(Phase::Full, hashed + read, total, 0, &l.file.path);
            });
            hashed += l.file.size;
            stages.bytes_read += read;
            match h {
                // Written to while being read: the hash says nothing about what's there now.
                Ok(h) if unchanged(&l) => {
                    stages.full_hashed += 1;
                    by.entry(h).or_default().push(l);
                }
                Ok(_) => skipped.changed += 1,
                Err(e) if e == CANCELLED => return Err(e),
                Err(_) => skipped.unreadable += 1,
            }
        }
        for (hash, ls) in by {
            if ls.len() < 2 {
                continue;
            }
            let size = ls[0].file.size;
            let files = ls
                .into_iter()
                .map(|l| DupFile {
                    cloud: cloud_kind(&l.file.path, o.mirrors, o.home),
                    protection: (o.protection)(&l.file.path),
                    whole: whole_of(&l.file.path),
                    path: l.file.path.to_string_lossy().into_owned(),
                    size,
                    mtime: l.file.mtime,
                    links: l.id.links,
                    aliases: l.aliases,
                    id: l.id,
                })
                .collect();
            let mut g = Group { hash, size, reclaimable: 0, files };
            g.reclaimable = reclaimable_of(&g);
            groups.push(g);
        }
    }
    groups.sort_by(|a, b| b.reclaimable.cmp(&a.reclaimable).then(b.size.cmp(&a.size)).then_with(|| a.hash.cmp(&b.hash)));
    Ok(ScanResult {
        scan_id: 0,
        root: shown_root.to_string_lossy().into_owned(),
        min_size: o.min_size,
        reclaimable: groups.iter().map(|g| g.reclaimable).sum(),
        groups,
        skipped,
        stages,
        placeholders: w.placeholders,
        placeholder_count: w.placeholder_count,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}
