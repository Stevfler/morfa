use crate::CmdResult;
use std::{
    path::Path,
    process::{Child, Command},
};

mod split;
#[cfg(all(unix, not(target_os = "macos")))]
use split::split_command;

/// Reap the child on a detached thread so it never lingers as a zombie.
fn reap(child: Child) {
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
}

#[tauri::command]
pub async fn open_terminal(path: String) -> CmdResult<()> {
    let p = path.clone();
    crate::guard::guarded(&path, crate::guard::quick_for(&path), move |_| {
        let dir = Path::new(&p);
        if !dir.is_dir() {
            return Err(format!("Not a folder: {p}"));
        }
        spawn_terminal(dir).map(reap).map_err(|e| format!("Could not open a terminal: {e}"))
    })
    .await
}

#[cfg(target_os = "macos")]
fn spawn_terminal(dir: &Path) -> std::io::Result<Child> {
    // `open -a Terminal <dir>` opens a new Terminal.app window at that directory.
    Command::new("open").arg("-a").arg("Terminal").arg(dir).spawn()
}

#[cfg(windows)]
fn spawn_terminal(dir: &Path) -> std::io::Result<Child> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    // Windows Terminal is a GUI app that opens its own window; no cmd wrapper, so
    // paths with `&`/`^` are passed verbatim.
    match Command::new("wt.exe").arg("-d").arg(dir).spawn() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        r => return r,
    }
    match Command::new("powershell.exe").arg("-NoExit").current_dir(dir).creation_flags(CREATE_NEW_CONSOLE).spawn() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        r => return r,
    }
    Command::new("cmd.exe").arg("/K").current_dir(dir).creation_flags(CREATE_NEW_CONSOLE).spawn()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn spawn_terminal(dir: &Path) -> std::io::Result<Child> {
    let mut candidates: Vec<Vec<String>> = vec![];
    if let Ok(t) = std::env::var("TERMINAL") {
        let parts = split_command(&t);
        if !parts.is_empty() {
            candidates.push(parts);
        }
    }
    for c in ["xdg-terminal-exec", "ghostty", "kitty", "alacritty", "foot", "wezterm", "gnome-terminal", "konsole", "xterm"] {
        candidates.push(vec![c.into()]);
    }
    let mut last = std::io::Error::other("no terminal found");
    for c in &candidates {
        match Command::new(&c[0]).args(&c[1..]).current_dir(dir).spawn() {
            Ok(child) => return Ok(child),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// How this install gets updates: "auto" — the built-in updater can replace it (Windows
/// installer, AppImage); "manual" — a package manager owns it (deb/rpm/Arch) or there's no
/// signed build for the platform (macOS), so the app only points to the download page.
#[tauri::command]
pub fn update_mode() -> &'static str {
    if cfg!(windows) || (cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some()) {
        "auto"
    } else {
        "manual"
    }
}
