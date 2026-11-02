//! macOS: best-effort parsing of `diskutil info -all` text output, plus
//! mount/unlock/unmount/eject via diskutil.
use super::Drive;

const HIDDEN: [&str; 6] = ["EFI", "Recovery", "Preboot", "VM", "Update", "xART"];

fn field<'a>(block: &'a str, key: &str) -> Option<&'a str> {
    block.lines().find_map(|l| l.trim().strip_prefix(key)?.strip_prefix(':').map(str::trim))
}

/// Each device is a block separated by a line of asterisks.
pub fn candidates(info_all: &str) -> Vec<Drive> {
    let mut out = vec![];
    for b in info_all.split("**********") {
        let yes = |k: &str| field(b, k) == Some("Yes");
        let (Some(id), Some(fs)) = (field(b, "Device Identifier"), field(b, "File System Personality")) else { continue };
        let name = field(b, "Volume Name").unwrap_or_default();
        let role_hidden = field(b, "APFS Volume Role").is_some_and(|r| r.contains("System") || (r.contains("Data") && name.ends_with(" - Data")));
        if field(b, "Mounted") != Some("No") || HIDDEN.iter().any(|h| name.starts_with(h)) || role_hidden {
            continue;
        }
        // "Disk Size: 64.0 GB (64023257088 Bytes) (exactly ...)"
        let size = field(b, "Disk Size")
            .or_else(|| field(b, "Volume Total Space"))
            .and_then(|s| s.split_once('(')?.1.split_whitespace().next()?.parse().ok())
            .unwrap_or(0);
        out.push(Drive {
            name: name.to_string(),
            device: id.to_string(),
            fstype: fs.to_string(),
            size,
            encrypted: yes("Locked") || (yes("FileVault") && yes("Encrypted")),
            removable: matches!(field(b, "Removable Media"), Some("Yes" | "Removable")) || yes("Ejectable") || field(b, "Device Location") == Some("External"),
        });
    }
    out
}

#[cfg(target_os = "macos")]
pub mod exec {
    use super::super::{run, Drive};

    fn friendly(e: String) -> String {
        let l = e.to_lowercase();
        if l.contains("passphrase") || l.contains("incorrect") {
            "Wrong passphrase".into()
        } else if l.contains("busy") || l.contains("in use") {
            "The drive is busy. Close any files using it and try again".into()
        } else {
            e.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("Unknown error").trim().into()
        }
    }

    pub fn list() -> Result<Vec<Drive>, String> {
        Ok(super::candidates(&run("diskutil", &["info", "-all"], None)?))
    }

    pub fn mount(device: &str, passphrase: Option<String>) -> Result<String, String> {
        if !list()?.iter().any(|d| d.device == device) {
            return Err("Unknown or already mounted drive".into());
        }
        if let Some(p) = passphrase {
            let mut key = p.into_bytes();
            let res = run("diskutil", &["apfs", "unlockVolume", device, "-stdinpassphrase"], Some(&key));
            key.iter_mut().for_each(|b| *b = 0);
            std::hint::black_box(&key);
            res.map_err(friendly)?;
        } else {
            run("diskutil", &["mount", device], None).map_err(friendly)?;
        }
        let info = run("diskutil", &["info", device], None)?;
        super::field(&info, "Mount Point").filter(|m| !m.is_empty()).map(String::from).ok_or("Mounted, but the mount point is unknown".into())
    }

    pub fn unmount(mount: &str, eject: bool) -> Result<(), String> {
        run("diskutil", &[if eject { "eject" } else { "unmount" }, mount], None).map(|_| ()).map_err(friendly)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_info_all() {
        let s = "   Device Identifier:         disk4s1\n   Volume Name:               STICK\n   Mounted:                   No\n   File System Personality:   ExFAT\n   Disk Size:                 64.0 GB (64023257088 Bytes) (exactly 125045424 512-Byte-Units)\n   Removable Media:           Removable\n   Device Location:           External\n**********\n   Device Identifier:         disk0s1\n   Volume Name:               EFI\n   Mounted:                   No\n   File System Personality:   MS-DOS FAT32\n**********\n   Device Identifier:         disk3s5\n   Volume Name:               Vault\n   Mounted:                   No\n   File System Personality:   APFS\n   FileVault:                 Yes\n   Locked:                    Yes\n**********\n   Device Identifier:         disk3s1\n   Volume Name:               Macintosh HD\n   Mounted:                   Yes\n   File System Personality:   APFS\n";
        let c = super::candidates(s);
        assert_eq!(c.len(), 2);
        assert!(c[0].removable && c[0].size == 64023257088 && c[0].device == "disk4s1");
        assert!(c[1].encrypted && c[1].name == "Vault");
    }
}
