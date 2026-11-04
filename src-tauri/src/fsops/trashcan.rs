use crate::CmdResult;

#[tauri::command]
pub async fn trash_items(paths: Vec<String>) -> CmdResult<()> {
    if let Some(first) = paths.first() {
        crate::guard::probe(first).await?;
    }
    tauri::async_runtime::spawn_blocking(move || trash_blocking(&paths)).await.map_err(|e| e.to_string())?
}

/// Move `paths` to the system Trash on the current thread.
pub(crate) fn trash_blocking(paths: &[String]) -> CmdResult<()> {
    #[allow(unused_mut)]
    let mut ctx = trash::TrashContext::default();
    // Finder (the default) triggers an Automation permission prompt and is slow.
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        ctx.set_delete_method(DeleteMethod::NsFileManager);
    }
    ctx.delete_all(paths).map_err(|e| e.to_string())
}

/// Put trashed items back where they came from (Linux & Windows).
/// On macOS: `Err` starting with `UNSUPPORTED_RESTORE:` (stable; the frontend matches it to
/// offer "Show in Trash" via `reveal_trash`).
#[tauri::command]
pub async fn restore_trashed(paths: Vec<String>) -> CmdResult<()> {
    #[cfg(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))))]
    {
        tauri::async_runtime::spawn_blocking(move || {
            use std::{collections::HashSet, path::PathBuf};
            let wanted: HashSet<PathBuf> = paths.iter().map(PathBuf::from).collect();
            let mut items: Vec<_> = trash::os_limited::list().map_err(|e| e.to_string())?.into_iter().filter(|i| wanted.contains(&i.original_path())).collect();
            // If an item was trashed more than once, restore only the newest copy.
            items.sort_by_key(|i| std::cmp::Reverse(i.time_deleted));
            let mut seen = HashSet::new();
            items.retain(|i| seen.insert(i.original_path()));
            trash::os_limited::restore_all(items).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android")))))]
    {
        let _ = paths;
        Err("UNSUPPORTED_RESTORE: restoring from Trash isn't supported on this OS — use Show in Trash".into())
    }
}

/// Opens the system Trash in the native file manager (Finder / default FM / Explorer).
/// Resolves once the opener has been launched; never waits for the window.
#[tauri::command]
pub fn reveal_trash() -> CmdResult<()> {
    let mut cmd = trash_opener()?;
    let mut child = cmd.spawn().map_err(|e| format!("couldn't open Trash: {e}"))?;
    // Reap the opener so it doesn't linger as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(target_os = "macos")]
fn trash_opener() -> CmdResult<std::process::Command> {
    let home = dirs::home_dir().ok_or("no home directory")?;
    let mut c = std::process::Command::new("open");
    c.arg(home.join(".Trash"));
    Ok(c)
}

#[cfg(windows)]
fn trash_opener() -> CmdResult<std::process::Command> {
    let mut c = std::process::Command::new("explorer.exe");
    c.arg("shell:RecycleBinFolder");
    Ok(c)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn trash_opener() -> CmdResult<std::process::Command> {
    // `trash:///` is understood by gio-based file managers; fall back to the XDG files dir.
    let data = dirs::data_dir().ok_or("no data directory")?;
    let files = data.join("Trash").join("files");
    let mut c = std::process::Command::new("xdg-open");
    c.arg(if files.is_dir() { files.into_os_string() } else { "trash:///".into() });
    Ok(c)
}
