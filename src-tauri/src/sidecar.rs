//! Finds bundled helper binaries (vips, vipsheader, magick, ffmpeg) before falling back to PATH.
//! Tauri's `externalBin` puts sidecars next to the executable; resources go in `<resource_dir>/bin`.
//! On macOS libvips ships as a relocatable tree in `<resource_dir>/vips` (bin/, lib/, lib/vips-modules-*).
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::OnceLock,
};

static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Called once from setup with tauri's resource dir.
pub fn init(resource_dir: Option<PathBuf>) {
    if let Some(d) = resource_dir {
        let _ = RESOURCE_DIR.set(d);
    }
}

fn dirs() -> Vec<PathBuf> {
    let mut v = vec![];
    if let Some(exe) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        v.push(exe.clone());
        v.push(exe.join("bin"));
    }
    if let Some(r) = RESOURCE_DIR.get() {
        v.push(r.join("bin"));
        v.push(r.join("vips").join("bin"));
    }
    // Apps launched from Finder don't get Homebrew's PATH.
    if cfg!(target_os = "macos") {
        v.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    }
    v
}

/// Full path of a bundled `name` if one exists, else `name` (resolved via PATH).
pub fn tool(name: &str) -> OsString {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    dirs().into_iter().map(|d| d.join(&file)).find(|p| p.is_file()).map(OsString::from).unwrap_or_else(|| name.into())
}

/// `Command` for a bundled tool; on Windows it runs without flashing a console window.
pub fn command(name: &str) -> std::process::Command {
    let path = PathBuf::from(tool(name));
    let mut c = std::process::Command::new(&path);
    // Bundled vips tree: point libvips at its own prefix so it finds lib/vips-modules-* (heif, jxl, pdf).
    if let Some(home) = path.parent().and_then(Path::parent).filter(|h| h.file_name() == Some("vips".as_ref())) {
        c.env("VIPSHOME", home);
    }
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0800_0000); // CREATE_NO_WINDOW
    c
}
