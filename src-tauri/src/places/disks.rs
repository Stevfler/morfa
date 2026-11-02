//! Pure disk filtering/naming so it can be unit-tested with plain data.
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Os {
    Linux,
    Mac,
    Windows,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Linux
        }
    }
}

#[derive(Clone, Debug)]
pub struct RawDisk {
    pub label: String,
    pub mount: String,
    pub fs: String,
    pub removable: bool,
    pub total: u64,
    pub free: u64,
}

fn under(mount: &str, root: &str) -> bool {
    mount.strip_prefix(root).is_some_and(|r| r.starts_with('/'))
}

/// Mount points a user cares about. On Linux, btrfs subvolumes share one
/// device and show up as many mounts (/var/log, /var/cache, /.snapshots, ...);
/// for btrfs we only keep `/`, `/home` and user-mount areas.
pub fn keep(os: Os, d: &RawDisk) -> bool {
    let m = d.mount.as_str();
    match os {
        Os::Windows => m.len() <= 3 && m.as_bytes().get(1) == Some(&b':'),
        Os::Mac => m == "/" || under(m, "/Volumes"),
        Os::Linux => {
            const HIDE: [&str; 6] = ["/boot", "/snap", "/var", "/sys", "/proc", "/efi"];
            if HIDE.iter().any(|h| m == *h || under(m, h)) || m.contains("/.snapshots") {
                return false;
            }
            let user_area = ["/mnt", "/media", "/run/media"].iter().any(|r| m == *r || under(m, r));
            if d.fs == "btrfs" {
                return m == "/" || m == "/home" || user_area;
            }
            m == "/" || !m.starts_with("/run") || user_area
        }
    }
}

/// Filter and dedupe by mount point (not device name: btrfs subvolumes share
/// a device, and unlabeled Windows drives share an empty name).
pub fn filter_disks(os: Os, raw: Vec<RawDisk>) -> Vec<RawDisk> {
    let mut seen = HashSet::new();
    raw.into_iter().filter(|d| keep(os, d) && seen.insert(d.mount.clone())).collect()
}

pub fn disk_name(os: Os, d: &RawDisk) -> String {
    match os {
        Os::Windows => {
            let letter = d.mount.trim_end_matches('\\');
            if d.label.is_empty() {
                let kind = if d.removable { "Removable Disk" } else { "Local Disk" };
                format!("{kind} ({letter})")
            } else {
                format!("{} ({letter})", d.label)
            }
        }
        _ if d.mount == "/" => "System".into(),
        _ => d.mount.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(&d.mount).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(mount: &str, fs: &str, label: &str) -> RawDisk {
        RawDisk { label: label.into(), mount: mount.into(), fs: fs.into(), removable: false, total: 1, free: 1 }
    }
    fn mounts(os: Os, v: Vec<RawDisk>) -> Vec<String> {
        filter_disks(os, v).into_iter().map(|d| d.mount).collect()
    }

    #[test]
    fn linux_btrfs() {
        let v = vec![
            d("/", "btrfs", "/dev/nvme0n1p2"),
            d("/home", "btrfs", "/dev/nvme0n1p2"),
            d("/var/log", "btrfs", "/dev/nvme0n1p2"),
            d("/.snapshots", "btrfs", "/dev/nvme0n1p2"),
            d("/srv", "btrfs", "/dev/nvme0n1p2"),
            d("/boot", "vfat", "/dev/nvme0n1p1"),
            d("/run/media/u/USB", "btrfs", "/dev/sda1"),
            d("/run/user/1000/doc", "fuse", "portal"),
            d("/data", "ext4", "/dev/sdb1"),
            d("/data", "ext4", "/dev/sdb1"),
        ];
        assert_eq!(mounts(Os::Linux, v), ["/", "/home", "/run/media/u/USB", "/data"]);
    }

    #[test]
    fn mac() {
        let v = vec![d("/", "apfs", "Macintosh HD"), d("/System/Volumes/Data", "apfs", "Data"), d("/private/var/vm", "apfs", "VM"), d("/Volumes/USB", "msdos", "USB")];
        assert_eq!(mounts(Os::Mac, v), ["/", "/Volumes/USB"]);
    }

    #[test]
    fn windows() {
        let v = vec![d("C:\\", "NTFS", ""), d("D:\\", "NTFS", ""), d("E:\\", "FAT32", "Stick")];
        let out = filter_disks(Os::Windows, v);
        let names: Vec<_> = out.iter().map(|x| disk_name(Os::Windows, x)).collect();
        assert_eq!(names, ["Local Disk (C:)", "Local Disk (D:)", "Stick (E:)"]);
    }
}
