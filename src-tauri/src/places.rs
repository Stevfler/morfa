use crate::{guard, netfs, CmdResult};
use serde::Serialize;
use std::{sync::Mutex, time::Duration};

mod disks;
use disks::{disk_name, filter_disks, Os, RawDisk};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    name: String,
    path: String,
    kind: String,
    total: u64,
    free: u64,
    /// Filesystem type ("ext4", "ntfs", ...); empty for folders.
    fs: String,
    removable: bool,
    /// On a network filesystem.
    network: bool,
    /// Didn't answer in time: total and free are unknown.
    unavailable: bool,
}

/// Per folder or mount; these run side by side, so this is about how long `places` can take.
const CHECK: Duration = Duration::from_millis(1500);
/// Long enough for a healthy sysinfo; on a timeout, the last good answer plus the mount table.
const DISKS: Duration = Duration::from_secs(2);
/// sysinfo stats every local mount in one go: a thread or two stuck in it is plenty.
const DISK_LIMITS: guard::Limits = guard::Limits { abandoned: 2, running: 4, group: 2, ..guard::LIMITS };

static LAST_DISKS: Mutex<Vec<RawDisk>> = Mutex::new(Vec::new());

/// sysinfo stats local mounts and asks macOS about every volume, either of which can hang. It
/// leaves out network mounts by design, except the FUSE ones on Linux.
fn sysinfo_disks() -> CmdResult<Vec<RawDisk>> {
    guard::run_keyed("sysinfo-disks", "disks", DISKS, DISK_LIMITS, |_| {
        let disks = sysinfo::Disks::new_with_refreshed_list().map_err(|e| e.to_string())?;
        Ok(disks
            .iter()
            .map(|d| RawDisk {
                label: d.name().to_string_lossy().into(),
                mount: d.mount_point().to_string_lossy().into(),
                fs: d.file_system().to_string_lossy().into(),
                removable: d.is_removable(),
                total: d.total_space(),
                free: d.available_space(),
            })
            .collect())
    })
}

#[tauri::command]
pub async fn places() -> CmdResult<Vec<Place>> {
    tauri::async_runtime::spawn_blocking(collect).await.map_err(|e| e.to_string())
}

/// Runs every check on its own guarded thread at once, keeping `items`' order.
fn each<T: Send, R: Send>(items: Vec<T>, f: impl Fn(T) -> R + Sync) -> Vec<R> {
    std::thread::scope(|s| {
        let f = &f;
        let hs: Vec<_> = items.into_iter().map(|i| s.spawn(move || f(i))).collect();
        hs.into_iter().filter_map(|h| h.join().ok()).collect()
    })
}

fn collect() -> Vec<Place> {
    let folders = [
        ("Home", dirs::home_dir(), "home"),
        ("Desktop", dirs::desktop_dir(), "desktop"),
        ("Downloads", dirs::download_dir(), "downloads"),
        ("Documents", dirs::document_dir(), "documents"),
        ("Pictures", dirs::picture_dir(), "pictures"),
        ("Music", dirs::audio_dir(), "music"),
        ("Videos", dirs::video_dir(), "videos"),
    ];
    let folders = folders.into_iter().filter_map(|(n, p, k)| Some((n, p?.to_string_lossy().to_string(), k))).collect();
    let mut out: Vec<Place> = each(folders, |(name, path, kind): (&str, String, &str)| {
        let p = path.clone();
        // A folder on a share that doesn't answer stays listed; opening it explains the rest.
        let exists = guard::run_blocking(&path, CHECK, move |_| Ok(std::path::Path::new(&p).exists())).unwrap_or(true);
        exists.then(|| Place { name: name.into(), path, kind: kind.into(), total: 0, free: 0, fs: String::new(), removable: false, network: false, unavailable: false })
    })
    .into_iter()
    .flatten()
    .collect();

    // Without sysinfo's answer, the last good one still knows names and removability.
    let (mut raw, fresh) = match sysinfo_disks() {
        Ok(v) => {
            *LAST_DISKS.lock().unwrap_or_else(|e| e.into_inner()) = v.clone();
            (v, true)
        }
        Err(_) => (LAST_DISKS.lock().unwrap_or_else(|e| e.into_inner()).clone(), false),
    };
    let table = netfs::table();
    // What sysinfo didn't report comes from the mount table: network mounts always, and every
    // mount when sysinfo hung. Sizes for those are measured below, each under its own limit.
    let known: std::collections::HashSet<String> = raw.iter().map(|d| d.mount.clone()).collect();
    let extra: Vec<String> = table.iter().filter(|m| !known.contains(&m.path) && (if fresh { netfs::is_network_fs(&m.fs) } else { !netfs::is_pseudo_fs(&m.fs) }) && m.fs != "autofs").map(|m| m.path.clone()).collect();
    let fs_of = |m: &str| table.iter().rev().find(|t| t.path == m).map(|t| t.fs.clone()).unwrap_or_default();
    raw.extend(extra.into_iter().map(|mount| RawDisk { label: String::new(), fs: fs_of(&mount), mount, removable: false, total: 0, free: 0 }));
    let disks = filter_disks(Os::current(), raw);
    // A sysinfo answer has sizes, unless a previous answer is standing in: measure those again.
    out.extend(each(disks, |mut d: RawDisk| {
        let network = netfs::is_network_fs(&d.fs);
        let mut unavailable = false;
        if network || !fresh || d.total == 0 {
            let m = d.mount.clone();
            match guard::run_blocking(&d.mount, CHECK, move |_| netfs::space(&m).ok_or_else(|| "no size".to_string())) {
                Ok((total, free)) => (d.total, d.free) = (total, free),
                Err(e) => {
                    unavailable = e.starts_with(guard::NOT_RESPONDING);
                    (d.total, d.free) = (0, 0);
                }
            }
        }
        Place {
            name: disk_name(Os::current(), &d),
            kind: if d.removable { "removable".into() } else { "disk".into() },
            path: d.mount,
            total: d.total,
            free: d.free,
            fs: d.fs,
            removable: d.removable,
            network,
            unavailable,
        }
    }));
    out
}
