//! Listing and mounting drives that are not mounted yet (sidebar "Drives").
//! Linux: lsblk + udisksctl; macOS: diskutil; Windows: nothing to do.
use crate::CmdResult;
use serde::Serialize;

#[cfg(any(target_os = "macos", test))]
mod diskutil;
#[cfg(any(target_os = "linux", test))]
mod lsblk;
#[cfg(target_os = "linux")]
mod udisks;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Drive {
    /// Volume label; empty when unlabeled (the UI names it by size).
    pub name: String,
    pub device: String,
    pub fstype: String,
    pub size: u64,
    pub encrypted: bool,
    pub removable: bool,
}

/// Run a command, optionally feeding `stdin`; Ok(stdout) or Err(stderr).
/// Killed after 120 s (e.g. a polkit prompt nobody answers / no polkit agent).
#[cfg(unix)]
fn run(cmd: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<String, String> {
    run_timeout(cmd, args, stdin, std::time::Duration::from_secs(120))
}

#[cfg(unix)]
fn run_timeout(cmd: &str, args: &[&str], stdin: Option<&[u8]>, limit: std::time::Duration) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run {cmd}: {e}"))?;
    let reader = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let out_t = reader(child.stdout.take().map(|r| Box::new(r) as _));
    let err_t = reader(child.stderr.take().map(|r| Box::new(r) as _));
    // A write error (EPIPE when the tool exits early) is only reported if the tool itself succeeded.
    let write_err = match (stdin, child.stdin.take()) {
        (Some(data), Some(mut pipe)) => pipe.write_all(data).err(),
        _ => None,
    };
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(st) => break st,
            None if start.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Authentication timed out. Is a polkit agent running?".into());
            }
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    };
    let (out, err) = (out_t.join().unwrap_or_default(), err_t.join().unwrap_or_default());
    match (status.success(), write_err) {
        (true, None) => Ok(out),
        (true, Some(e)) => Err(e.to_string()),
        (false, _) => Err(if err.trim().is_empty() { out } else { err }),
    }
}

#[cfg(all(test, unix))]
#[test]
fn run_times_out_and_captures_stderr() {
    let t = std::time::Instant::now();
    assert!(run_timeout("sleep", &["5"], None, std::time::Duration::from_secs(1)).unwrap_err().contains("timed out"));
    assert!(t.elapsed().as_secs() < 3);
    assert_eq!(run("sh", &["-c", "echo bad >&2; exit 1"], Some(b"x")).unwrap_err().trim(), "bad");
    assert_eq!(run("cat", &[], Some(b"hi")).unwrap(), "hi");
}

#[cfg(target_os = "linux")]
use udisks as backend;
#[cfg(target_os = "macos")]
use diskutil::exec as backend;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod backend {
    pub fn list() -> Result<Vec<super::Drive>, String> {
        Ok(vec![])
    }
    pub fn mount(_: &str, _: Option<String>) -> Result<String, String> {
        Err("Mounting drives is not supported on this platform".into())
    }
    pub fn unmount(_: &str, _: bool) -> Result<(), String> {
        Err("Ejecting drives is not supported on this platform".into())
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

/// Unmounted volumes the user could mount.
#[tauri::command]
pub async fn list_drives() -> CmdResult<Vec<Drive>> {
    crate::guard::guarded("list-drives", std::time::Duration::from_secs(10), |_| backend::list()).await
}

/// Mount (unlocking first when `passphrase` is given / required); returns the mount path.
#[tauri::command]
pub async fn mount_drive(device: String, passphrase: Option<String>) -> CmdResult<String> {
    blocking(move || backend::mount(&device, passphrase)).await
}

/// Unmount the drive mounted at `path`; with `eject`, also power off removable media.
#[tauri::command]
pub async fn unmount_drive(path: String, eject: bool) -> CmdResult<()> {
    blocking(move || backend::unmount(&path, eject)).await
}
