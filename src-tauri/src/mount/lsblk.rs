//! Pure `lsblk -J` parsing: flattens the device tree and picks the
//! unmounted filesystems a desktop file manager would offer to mount.
use super::Drive;
use serde_json::Value;

const MOUNTABLE: [&str; 15] = [
    "ext2", "ext3", "ext4", "btrfs", "xfs", "ntfs", "ntfs3", "exfat", "vfat", "f2fs", "hfsplus", "apfs", "iso9660", "udf", "jfs",
];
const SYSTEM_LABELS: [&str; 8] = ["efi", "esp", "system", "boot", "recovery", "system reserved", "winre", "msr"];
const GIB: u64 = 1 << 30;

#[derive(Clone, Debug, Default)]
pub struct Dev {
    pub path: String,
    pub label: String,
    pub fstype: String,
    pub kind: String,
    pub size: u64,
    pub mounts: Vec<String>,
    pub removable: bool,
    pub parent: Option<usize>,
    pub has_children: bool,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn flag(v: &Value, k: &str) -> bool {
    // Older lsblk prints booleans as "0"/"1" strings.
    v.get(k).is_some_and(|x| x.as_bool().unwrap_or(false) || x.as_str() == Some("1"))
}

fn walk(v: &Value, parent: Option<usize>, inherit_rm: bool, out: &mut Vec<Dev>) {
    let kids = v.get("children").and_then(Value::as_array);
    let mut mounts: Vec<String> = match v.get("mountpoints").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
        None => v.get("mountpoint").and_then(Value::as_str).map(String::from).into_iter().collect(),
    };
    mounts.retain(|m| !m.is_empty());
    let name = s(v, "name");
    let path = Some(s(v, "path")).filter(|p| !p.is_empty()).unwrap_or_else(|| format!("/dev/{name}"));
    let removable = inherit_rm || flag(v, "rm") || flag(v, "hotplug");
    let size = v.get("size").and_then(|x| x.as_u64().or_else(|| x.as_str()?.parse().ok())).unwrap_or(0);
    let idx = out.len();
    out.push(Dev {
        path,
        label: s(v, "label"),
        fstype: s(v, "fstype"),
        kind: s(v, "type"),
        size,
        mounts,
        removable,
        parent,
        has_children: kids.is_some_and(|k| !k.is_empty()),
    });
    for c in kids.into_iter().flatten() {
        walk(c, Some(idx), removable, out);
    }
}

pub fn parse(json: &str) -> Result<Vec<Dev>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut out = vec![];
    for d in v.get("blockdevices").and_then(Value::as_array).into_iter().flatten() {
        walk(d, None, false, &mut out);
    }
    Ok(out)
}

/// Topmost ancestor (the whole disk).
pub fn root_of(devs: &[Dev], mut i: usize) -> usize {
    while let Some(p) = devs[i].parent {
        i = p;
    }
    i
}

pub fn find_mounted(devs: &[Dev], mount: &str) -> Option<usize> {
    devs.iter().position(|d| d.mounts.iter().any(|m| m == mount))
}

fn is_system(d: &Dev) -> bool {
    let label = d.label.to_lowercase();
    if SYSTEM_LABELS.contains(&label.as_str()) {
        return true;
    }
    // Small unlabeled vfat = EFI system partition; small unlabeled ntfs = Windows recovery.
    let small_unlabeled = d.label.is_empty() && d.size <= GIB;
    (d.fstype == "vfat" && d.size <= GIB && (small_unlabeled || label.contains("efi"))) || (d.fstype == "ntfs" && small_unlabeled)
}

/// Unmounted volumes worth showing. A LUKS container that is already unlocked
/// is represented by its cleartext child instead.
pub fn candidates(devs: &[Dev]) -> Vec<Drive> {
    let mut out = vec![];
    for d in devs {
        let encrypted = d.fstype == "crypto_LUKS";
        if !matches!(d.kind.as_str(), "part" | "disk" | "crypt" | "rom" | "lvm" | "md")
            || !(encrypted || MOUNTABLE.contains(&d.fstype.as_str()))
            || !d.mounts.is_empty()
            || (encrypted && d.has_children)
            || is_system(d)
        {
            continue;
        }
        out.push(Drive {
            name: d.label.clone(),
            device: d.path.clone(),
            fstype: d.fstype.clone(),
            size: d.size,
            encrypted,
            removable: d.removable,
        });
    }
    out
}

/// `udisksctl mount` → "Mounted /dev/sdb1 at /run/media/u/X" (older versions append '.').
pub fn parse_mounted(stdout: &str) -> Option<String> {
    let rest = stdout.lines().find_map(|l| l.trim().strip_prefix("Mounted "))?;
    Some(rest.split_once(" at ")?.1.trim().to_string())
}

/// "Unlocked /dev/sda2 as /dev/dm-3." or error "... is already unlocked as /dev/dm-3".
pub fn parse_unlocked(out: &str) -> Option<String> {
    let rest = out.lines().find_map(|l| l.split_once("nlocked ").map(|x| x.1))?;
    Some(rest.rsplit_once("as ")?.1.trim().trim_end_matches(['.', '\'', '`']).to_string())
}

/// "... is already mounted at `/run/media/u/X'."
pub fn parse_already_mounted(err: &str) -> Option<String> {
    let rest = err.split_once("already mounted at ")?.1;
    let rest = rest.trim().trim_start_matches(['`', '\'']);
    Some(rest.split(['\'', '`']).next()?.trim_end_matches('.').to_string())
}

pub fn friendly_error(err: &str) -> String {
    let e = err.to_lowercase();
    let msg = if e.contains("notauthorized") || e.contains("not authorized") || e.contains("dismissed") {
        "Authorization was cancelled or denied"
    } else if e.contains("passphrase") || e.contains("no key available") || e.contains("operation not permitted") {
        "Wrong passphrase"
    } else if e.contains("busy") {
        "The drive is busy. Close any files or terminals using it and try again"
    } else if e.contains("not mounted") {
        "The drive is not mounted"
    } else {
        return err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("Unknown error").trim().into();
    };
    msg.into()
}

#[cfg(test)]
#[path = "lsblk_tests.rs"]
mod tests;
