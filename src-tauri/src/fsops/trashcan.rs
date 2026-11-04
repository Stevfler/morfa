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
    tauri::async_runtime::spawn_blocking(move || restore_blocking(&paths)).await.map_err(|e| e.to_string())?
}

/// `restore_trashed` on the current thread.
pub(crate) fn restore_blocking(paths: &[String]) -> CmdResult<()> {
    #[cfg(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))))]
    {
        use std::{collections::HashSet, path::PathBuf};
        let wanted: HashSet<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let mut items: Vec<_> = trash::os_limited::list().map_err(|e| e.to_string())?.into_iter().filter(|i| wanted.contains(&i.original_path())).collect();
        // If an item was trashed more than once, restore only the newest copy.
        items.sort_by_key(|i| std::cmp::Reverse(i.time_deleted));
        let mut seen = HashSet::new();
        items.retain(|i| seen.insert(i.original_path()));
        trash::os_limited::restore_all(items).map_err(|e| e.to_string())
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android")))))]
    {
        let _ = paths;
        Err("UNSUPPORTED_RESTORE: restoring from Trash isn't supported on this OS — use Show in Trash".into())
    }
}

/// Whether this OS lets an app put items back from Trash.
pub(crate) const CAN_RESTORE: bool = cfg!(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))));

/// A path and the seconds, both inclusive, its copy in Trash must have been deleted within.
pub(crate) type Window = (String, i64, i64);

/// Of these, the paths with a matching copy in Trash.
pub(crate) fn in_trash(wanted: &[Window]) -> std::collections::HashSet<String> {
    #[cfg(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))))]
    {
        use std::{collections::HashMap, path::PathBuf};
        let by: HashMap<PathBuf, (i64, i64)> = wanted.iter().map(|(p, lo, hi)| (PathBuf::from(p), (*lo, *hi))).collect();
        let items = trash::os_limited::list().unwrap_or_default();
        items
            .into_iter()
            .filter(|i| by.get(&i.original_path()).is_some_and(|(lo, hi)| i.time_deleted >= *lo && i.time_deleted <= *hi))
            .map(|i| i.original_path().to_string_lossy().into_owned())
            .collect()
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android")))))]
    {
        let _ = wanted;
        Default::default()
    }
}

/// Puts each path back from its newest copy within its window, reporting per path. An item that isn't
/// there is an error rather than silently nothing, unlike `restore_blocking`.
pub(crate) fn restore_since(wanted: &[Window]) -> Vec<(String, Result<(), String>)> {
    #[cfg(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))))]
    {
        use std::{collections::HashMap, path::PathBuf};
        let by: HashMap<PathBuf, (i64, i64)> = wanted.iter().map(|(p, lo, hi)| (PathBuf::from(p), (*lo, *hi))).collect();
        let mut newest: HashMap<PathBuf, trash::TrashItem> = HashMap::new();
        for i in trash::os_limited::list().unwrap_or_default() {
            let path = i.original_path();
            let fits = by.get(&path).is_some_and(|(lo, hi)| i.time_deleted >= *lo && i.time_deleted <= *hi);
            if fits && newest.get(&path).is_none_or(|o| o.time_deleted < i.time_deleted) {
                newest.insert(path, i);
            }
        }
        wanted
            .iter()
            .map(|(p, _, _)| {
                let r = match newest.remove(&PathBuf::from(p)) {
                    Some(item) => trash::os_limited::restore_all(vec![item]).map_err(|e| e.to_string()),
                    None => Err("not in Trash".to_string()),
                };
                (p.clone(), r)
            })
            .collect()
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android")))))]
    {
        wanted.iter().map(|(p, _, _)| (p.clone(), Err("UNSUPPORTED_RESTORE".to_string()))).collect()
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
