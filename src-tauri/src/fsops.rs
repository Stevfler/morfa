use crate::CmdResult;
use std::{fs, io, path::Path};

mod names;
pub mod trashcan;
pub(crate) mod rename;
#[cfg(test)]
use rename::{move_by_copy, same_file};
pub use names::validate_name;

fn free(p: &Path) -> bool {
    p.symlink_metadata().is_err()
}

#[tauri::command]
pub async fn create_dir(parent: String, name: String) -> CmdResult<String> {
    validate_name(&name)?;
    let p = names::create_unique(&Path::new(&parent).join(name), true)?;
    Ok(p.to_string_lossy().into())
}

#[tauri::command]
pub async fn create_file(parent: String, name: String) -> CmdResult<String> {
    validate_name(&name)?;
    let p = names::create_unique(&Path::new(&parent).join(name), false)?;
    Ok(p.to_string_lossy().into())
}

pub(crate) fn remove_any(p: &Path) -> std::io::Result<()> {
    if fs::symlink_metadata(p)?.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}

/// Rename/move each `from` to exactly `to`. Refuses to overwrite. Used for rename, batch rename and undo.
#[tauri::command]
pub async fn move_pairs(pairs: Vec<(String, String)>) -> CmdResult<()> {
    for (from, to) in &pairs {
        if from == to {
            continue;
        }
        let (f, t) = (Path::new(from), Path::new(to));
        // A case-only rename on a case-insensitive FS sees `t` as existing; allow it only when
        // `t` really is the same file as `f`, never when it's a different file.
        if !free(t) && !rename::same_file(f, t) {
            return Err(format!("\"{}\" already exists", t.file_name().unwrap_or_default().to_string_lossy()));
        }
        if let Some(p) = t.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        // Only a cross-device move may fall back to copy+delete; any other rename error
        // (permissions, busy/locked file, non-empty dir…) is reported as-is.
        // `rename_noreplace` closes the window between the check above and the rename: a file that
        // appears at `t` meanwhile is never silently replaced.
        match rename::rename_noreplace(f, t) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(format!("\"{}\" already exists", t.file_name().unwrap_or_default().to_string_lossy()))
            }
            Err(e) if rename::is_cross_device(&e) => rename::move_by_copy(f, t).map_err(|e| format!("{from}: {e}"))?,
            Err(e) => return Err(format!("{from}: {e}")),
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_items(paths: Vec<String>) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || delete_all(&paths)).await.map_err(|e| e.to_string())?
}

/// Attempts every path; on any failure reports which ones failed and how many were deleted.
fn delete_all(paths: &[String]) -> CmdResult<()> {
    let failed: Vec<String> = paths.iter().filter_map(|p| remove_any(Path::new(p)).err().map(|e| format!("{p}: {e}"))).collect();
    if failed.is_empty() {
        return Ok(());
    }
    let ok = paths.len() - failed.len();
    Err(format!("Couldn't delete {} of {} item(s) ({ok} deleted):\n{}", failed.len(), paths.len(), failed.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("morfa-fsops-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn names() {
        for ok in ["a", "photo.jpg", ".hidden", "COM0", "LPT10", "CONIN", "conout.txt$", "my file (2).txt", "consul"] {
            assert!(validate_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "  ", ".", "..", "a/b", "a\\b", "a\0b", "a<b", "a>b", "a:b", "a\"b", "a|b", "a?b", "a*b", "con", "CON", "nul.txt", "Com1", "lpt9.tar.gz", "aux .txt", "end.", "end ", "COM¹", "com².txt", "lpt³", "LPT¹.x", "CONIN$", "conout$.log", "Conin$"] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn create_unique_never_overwrites() {
        let d = tmp("uniq");
        let a = names::create_unique(&d.join("x.txt"), false).unwrap();
        let b = names::create_unique(&d.join("x.txt"), false).unwrap();
        let c = names::create_unique(&d.join("x.txt"), true).unwrap();
        assert_eq!(a, d.join("x.txt"));
        assert_eq!(b, d.join("x (2).txt"));
        assert_eq!(c, d.join("x.txt (2)"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn case_rename_rule() {
        let d = tmp("case");
        let (lo, up) = (d.join("a"), d.join("A"));
        fs::write(&lo, "lower").unwrap();
        assert!(same_file(&lo, &lo));
        if !same_file(&lo, &up) {
            // Case-sensitive FS: an existing different "A" must not be overwritten.
            fs::write(&up, "upper").unwrap();
            let r = tauri::async_runtime::block_on(move_pairs(vec![(lo.to_string_lossy().into(), up.to_string_lossy().into())]));
            assert!(r.unwrap_err().contains("already exists"));
            assert_eq!(fs::read_to_string(&up).unwrap(), "upper");
            assert_eq!(fs::read_to_string(&lo).unwrap(), "lower");
        }
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn copy_fallback_no_overwrite_and_symlinks() {
        let d = tmp("copy");
        let src = d.join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("f"), "data").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("f", src.join("link")).unwrap();
        let taken = d.join("taken");
        fs::write(&taken, "keep").unwrap();
        assert!(move_by_copy(&src, &taken).is_err());
        assert_eq!(fs::read_to_string(&taken).unwrap(), "keep");
        assert!(src.join("f").exists());
        let dst = d.join("dst");
        move_by_copy(&src, &dst).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(dst.join("f")).unwrap(), "data");
        #[cfg(unix)]
        assert!(fs::symlink_metadata(dst.join("link")).unwrap().file_type().is_symlink());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn delete_continues_past_failures() {
        let d = tmp("del");
        let (a, b) = (d.join("a"), d.join("b"));
        fs::write(&a, "").unwrap();
        fs::create_dir(&b).unwrap();
        fs::write(b.join("x"), "").unwrap();
        let missing = d.join("missing").to_string_lossy().to_string();
        let paths = vec![a.to_string_lossy().into(), missing.clone(), b.to_string_lossy().into()];
        let err = delete_all(&paths).unwrap_err();
        assert!(err.contains("1 of 3") && err.contains("2 deleted") && err.contains(&missing), "{err}");
        assert!(!a.exists() && !b.exists());
        fs::remove_dir_all(&d).unwrap();
    }
}
