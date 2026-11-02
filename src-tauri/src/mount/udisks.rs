//! Linux: list via lsblk, mount/unlock/unmount/power-off via udisksctl
//! (polkit shows its own auth dialog when needed).
use super::lsblk::{self, Dev};
use super::{run, Drive};
use std::path::Path;

const LSBLK_ARGS: [&str; 4] = ["-J", "-b", "-o", "NAME,PATH,LABEL,SIZE,FSTYPE,MOUNTPOINTS,TYPE,HOTPLUG,RM,PKNAME"];

fn devices() -> Result<Vec<Dev>, String> {
    let out = run("lsblk", &LSBLK_ARGS, None)?;
    lsblk::parse(&out)
}

pub fn list() -> Result<Vec<Drive>, String> {
    Ok(lsblk::candidates(&devices()?))
}

fn udisks(args: &[&str], stdin: Option<&[u8]>) -> Result<String, String> {
    run("udisksctl", args, stdin)
}

fn unlock(device: &str, passphrase: Option<String>) -> Result<String, String> {
    let mut key = passphrase.ok_or("Passphrase required")?.into_bytes();
    let res = udisks(&["unlock", "-b", device, "--key-file", "/dev/stdin"], Some(&key));
    key.iter_mut().for_each(|b| *b = 0);
    std::hint::black_box(&key);
    let text = match res {
        Ok(out) => out,
        // Already unlocked (e.g. by another app): mount the existing cleartext device.
        Err(e) if e.contains("already unlocked") => e,
        Err(e) => return Err(lsblk::friendly_error(&e)),
    };
    lsblk::parse_unlocked(&text).ok_or_else(|| format!("Unexpected udisksctl output: {}", text.trim()))
}

pub fn mount(device: &str, passphrase: Option<String>) -> Result<String, String> {
    let devs = devices()?;
    let dev = lsblk::candidates(&devs).into_iter().find(|d| d.device == device).ok_or("Unknown or already mounted drive")?;
    let encrypted = dev.encrypted;
    let target = if encrypted { unlock(device, passphrase)? } else { device.to_string() };
    let text = match udisks(&["mount", "-b", &target], None) {
        Ok(out) => out,
        Err(e) => return lsblk::parse_already_mounted(&e).ok_or_else(|| lsblk::friendly_error(&e)),
    };
    let path = lsblk::parse_mounted(&text).ok_or_else(|| format!("Unexpected udisksctl output: {}", text.trim()))?;
    // Older udisks append a '.' to the path.
    match path.strip_suffix('.') {
        Some(p) if !Path::new(&path).exists() => Ok(p.to_string()),
        _ => Ok(path),
    }
}

pub fn unmount(mount: &str, eject: bool) -> Result<(), String> {
    let devs = devices()?;
    let i = lsblk::find_mounted(&devs, mount).ok_or("This location is not a mounted drive")?;
    let fail = |e: String| lsblk::friendly_error(&e);
    udisks(&["unmount", "-b", &devs[i].path], None).map_err(fail)?;
    if devs[i].kind == "crypt" {
        if let Some(p) = devs[i].parent {
            udisks(&["lock", "-b", &devs[p].path], None).map_err(fail)?;
        }
    }
    let disk = &devs[lsblk::root_of(&devs, i)];
    if eject && disk.removable {
        udisks(&["power-off", "-b", &disk.path], None).map_err(fail)?;
    }
    Ok(())
}
