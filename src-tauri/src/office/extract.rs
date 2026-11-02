//! Unpacks the installer into a staging dir without installing anything system-wide:
//! Windows uses an MSI *administrative* install (a plain file copy: no registry, shortcuts,
//! file associations or Add/Remove entry); macOS copies the .app out of the read-only dmg.
use crate::convert::proc;
use std::{path::Path, process::Command, time::Duration};

const TIMEOUT: Duration = Duration::from_secs(20 * 60);

#[cfg_attr(not(windows), allow(dead_code))]
fn run(cmd: Command, what: &str, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    #[cfg(windows)]
    let mut cmd = cmd;
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // CREATE_NO_WINDOW
    proc::run(cmd, cancelled, Some(TIMEOUT)).map_err(|e| format!("{what}: {e}"))
}

/// Windows: `msiexec /a` into `tmp`, then move `tmp/**/LibreOffice` (the folder holding
/// `program/`) to `staging`.
#[cfg(windows)]
pub fn unpack(pkg: &Path, tmp: &Path, staging: &Path, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let mut cmd = Command::new("msiexec");
    // /qn = no UI, no reboot prompts. TARGETDIR must be absolute.
    // msiexec parses its own command line: quote the value, not the whole `KEY=value` arg.
    use std::os::windows::process::CommandExt;
    cmd.arg("/a").arg(pkg).arg("/qn").raw_arg(format!("TARGETDIR=\"{}\"", tmp.display()));
    run(cmd, "msiexec", cancelled)?;
    let root = find_root(tmp, 4).ok_or("unexpected MSI layout (no program/soffice.exe)")?;
    // (the admin image's copy of the .msi stays at TARGETDIR's top level and goes with `tmp`)
    std::fs::rename(&root, staging).map_err(|e| e.to_string())
}

#[cfg(windows)]
fn find_root(dir: &Path, depth: u32) -> Option<std::path::PathBuf> {
    if dir.join("program").join("soffice.exe").is_file() {
        return Some(dir.to_path_buf());
    }
    if depth == 0 {
        return None;
    }
    std::fs::read_dir(dir).ok()?.flatten().filter(|e| e.path().is_dir()).find_map(|e| find_root(&e.path(), depth - 1))
}

/// macOS: mount read-only and hidden from Finder, `ditto` LibreOffice.app into `staging`.
#[cfg(target_os = "macos")]
pub fn unpack(pkg: &Path, tmp: &Path, staging: &Path, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let mnt = tmp.join("mnt");
    std::fs::create_dir_all(&mnt).map_err(|e| e.to_string())?;
    let mut attach = Command::new("hdiutil");
    attach.args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"]).arg(&mnt).arg(pkg);
    run(attach, "hdiutil attach", cancelled)?;
    std::fs::create_dir_all(staging).map_err(|e| e.to_string())?;
    let mut ditto = Command::new("ditto");
    ditto.arg(mnt.join("LibreOffice.app")).arg(staging.join("LibreOffice.app"));
    let res = run(ditto, "ditto", cancelled);
    let mut detach = Command::new("hdiutil");
    detach.args(["detach", "-force"]).arg(&mnt);
    let _ = run(detach, "hdiutil detach", &|| false);
    res
}

/// macOS: pruning breaks the bundle's code seal; re-sign ad hoc and drop quarantine so
/// Gatekeeper doesn't block the (never user-launched) binary.
#[cfg(target_os = "macos")]
pub fn finalize(staging: &Path, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let app = staging.join("LibreOffice.app");
    let mut x = Command::new("xattr");
    x.args(["-dr", "com.apple.quarantine"]).arg(&app);
    let _ = run(x, "xattr", cancelled); // fails harmlessly when the attribute is absent
    let mut cs = Command::new("codesign");
    cs.args(["--force", "--deep", "--sign", "-"]).arg(&app);
    run(cs, "codesign", cancelled)
}

#[cfg(not(target_os = "macos"))]
pub fn finalize(_staging: &Path, _cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn unpack(_pkg: &Path, _tmp: &Path, _staging: &Path, _cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    Err("The Office module isn't offered on this OS".into())
}
