use crate::CmdResult;

#[tauri::command]
pub fn open_terminal(path: String) -> CmdResult<()> {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").args(["-a", "Terminal", &path]).spawn();
    #[cfg(windows)]
    let r = std::process::Command::new("cmd").args(["/C", "start", "wt", "-d", &path]).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = {
        let candidates = [std::env::var("TERMINAL").unwrap_or_default(), "xdg-terminal-exec".into(), "ghostty".into(), "kitty".into(), "alacritty".into(), "foot".into(), "wezterm".into(), "gnome-terminal".into(), "konsole".into(), "xterm".into()];
        let mut res = Err(std::io::Error::other("no terminal found"));
        for c in candidates.iter().filter(|c| !c.is_empty()) {
            res = std::process::Command::new(c).current_dir(&path).spawn();
            if res.is_ok() {
                break;
            }
        }
        res
    };
    r.map(|_| ()).map_err(|e| e.to_string())
}
