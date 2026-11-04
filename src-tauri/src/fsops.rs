use crate::CmdResult;
use std::{fs, io, path::Path};

mod names;
pub mod trashcan;
pub(crate) mod rename;
#[cfg(test)]
use rename::{move_by_copy, same_file};
pub use names::{name_issue, validate_name};

fn free(p: &Path) -> bool {
    p.symlink_metadata().is_err()
}

#[tauri::command]
pub async fn create_dir(parent: String, name: String) -> CmdResult<String> {
    validate_name(&name)?;
    crate::guard::probe(&parent).await?;
    // Past the probe it runs to the end: abandoning it would report failure for an item it creates.
    tauri::async_runtime::spawn_blocking(move || Ok(names::create_unique(&Path::new(&parent).join(name), true)?.to_string_lossy().into())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn create_file(parent: String, name: String) -> CmdResult<String> {
    validate_name(&name)?;
    crate::guard::probe(&parent).await?;
    // Past the probe it runs to the end: abandoning it would report failure for an item it creates.
    tauri::async_runtime::spawn_blocking(move || Ok(names::create_unique(&Path::new(&parent).join(name), false)?.to_string_lossy().into())).await.map_err(|e| e.to_string())?
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
    if let Some((from, _)) = pairs.first() {
        crate::guard::probe(from).await?;
    }
    tauri::async_runtime::spawn_blocking(move || move_all(&pairs)).await.map_err(|e| e.to_string())?
}

/// What a move call really did. Empty `completed` with an error: nothing changed.
#[derive(serde::Serialize, Debug, Default)]
pub struct MoveReport {
    /// Net moves that stand, detours through temporary names composed. Survivors of a failed rollback
    /// keep their temporary names, so the journal knows where those files are.
    pub completed: Vec<(String, String)>,
    pub error: Option<String>,
}

/// Moves in order. On the first failure, undoes what it did, newest first. A step that can't be
/// undone stays in the report, and the rest are still tried: one stuck file mustn't strand a batch.
pub(crate) fn move_tracked(pairs: &[(String, String)]) -> MoveReport {
    move_tracked_with(pairs, &mut |p| move_all(std::slice::from_ref(p)))
}

/// `mv` performs one step; tests use it to make a rollback fail.
fn move_tracked_with(pairs: &[(String, String)], mv: &mut dyn FnMut(&(String, String)) -> CmdResult<()>) -> MoveReport {
    let mut done: Vec<(String, String)> = vec![];
    // Folders each step had to create, deepest first.
    let mut made: Vec<Vec<std::path::PathBuf>> = vec![];
    let mut error = None;
    for p in pairs {
        let missing: Vec<std::path::PathBuf> = Path::new(&p.1).ancestors().skip(1).take_while(|a| !a.as_os_str().is_empty() && free(a)).map(Path::to_path_buf).collect();
        match mv(p) {
            Ok(()) if p.0 == p.1 => {}
            Ok(()) => {
                done.push(p.clone());
                made.push(missing);
            }
            Err(e) => {
                error = Some(explain_collision(&done, p).unwrap_or(e));
                break;
            }
        }
    }
    if error.is_some() {
        let mut stuck = vec![];
        while let Some((f, t)) = done.pop() {
            let dirs = made.pop().unwrap_or_default();
            if mv(&(t.clone(), f.clone())).is_err() {
                stuck.push((f, t));
                continue;
            }
            // `remove_dir` only takes empty ones, so nothing that arrived meanwhile is lost.
            for dir in dirs {
                if fs::remove_dir(&dir).is_err() {
                    break;
                }
            }
        }
        stuck.reverse();
        done = stuck;
    }
    let mut net: Vec<(String, String)> = vec![];
    for (f, t) in done {
        match net.iter().position(|(_, to)| *to == f) {
            Some(i) => net[i].1 = t,
            None => net.push((f, t)),
        }
    }
    net.retain(|(f, t)| f != t);
    MoveReport { completed: net, error }
}

/// A step refused because an earlier step of this same call took the name: the disk treats the two
/// spellings as one (like "ß" and "ss" on some disks), which "already exists" wouldn't explain.
fn explain_collision(done: &[(String, String)], step: &(String, String)) -> Option<String> {
    let t = Path::new(&step.1);
    let (_, other) = done.iter().find(|(_, to)| *to != step.1 && rename::same_file(Path::new(to), t))?;
    let name = |p: &Path| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
    Some(format!("\"{}\" and \"{}\" are the same name on this disk, so both can't be used", name(t), name(Path::new(other))))
}

/// `move_pairs` for a new user operation: reports exactly what stands instead of only failing.
#[tauri::command]
pub async fn move_pairs_report(pairs: Vec<(String, String)>) -> CmdResult<MoveReport> {
    if let Some((from, _)) = pairs.first() {
        crate::guard::probe(from).await?;
    }
    tauri::async_runtime::spawn_blocking(move || move_tracked(&pairs)).await.map_err(|e| e.to_string())
}

pub(crate) fn move_all(pairs: &[(String, String)]) -> CmdResult<()> {
    for (from, to) in pairs {
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
    if let Some(first) = paths.first() {
        crate::guard::probe(first).await?;
    }
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

    fn p(d: &Path, n: &str) -> String {
        d.join(n).to_string_lossy().into_owned()
    }

    /// Batch steps through temporaries, as the dialog builds them.
    fn detour(d: &Path, moves: &[(String, String)]) -> Vec<(String, String)> {
        let tmp: Vec<_> = moves.iter().enumerate().map(|(i, (f, _))| (f.clone(), p(d, &format!("t{i}.morfa-tmp")))).collect();
        let back: Vec<_> = tmp.iter().zip(moves).map(|((_, t), (_, to))| (t.clone(), to.clone())).collect();
        tmp.into_iter().chain(back).collect()
    }

    #[test]
    fn undoing_a_large_batch_over_a_new_file_changes_nothing() {
        let d = tmp("undo-large");
        // Renamed files now named x<i>; undo wants o<i> back, but a new o150 took a name.
        let undo: Vec<_> = (0..300).map(|i| (p(&d, &format!("x{i}")), p(&d, &format!("o{i}")))).collect();
        for (i, (f, _)) in undo.iter().enumerate() {
            fs::write(f, i.to_string()).unwrap();
        }
        fs::write(d.join("o150"), "new").unwrap();
        let r = move_tracked(&detour(&d, &undo));
        assert!(r.error.unwrap().contains("o150"));
        assert!(r.completed.is_empty());
        for (i, (f, _)) in undo.iter().enumerate() {
            assert_eq!(fs::read_to_string(f).unwrap(), i.to_string());
        }
        assert_eq!(fs::read_to_string(d.join("o150")).unwrap(), "new");
        assert_eq!(fs::read_dir(&d).unwrap().count(), 301, "no temporaries left");
        // With the blocker gone the same undo works: the item is worth keeping.
        fs::remove_file(d.join("o150")).unwrap();
        assert_eq!(move_tracked(&detour(&d, &undo)).error, None);
        assert_eq!(fs::read_to_string(d.join("o299")).unwrap(), "299");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_stuck_rollback_reports_where_the_file_is_and_frees_the_rest() {
        let d = tmp("stuck");
        let moves: Vec<_> = (0..3).map(|i| (p(&d, &format!("a{i}")), p(&d, &format!("b{i}")))).collect();
        for (f, _) in &moves {
            fs::write(f, "x").unwrap();
        }
        fs::write(d.join("b2"), "blocker").unwrap();
        let t1 = p(&d, "t1.morfa-tmp");
        let a1 = p(&d, "a1");
        // Only the way back for a1 is broken.
        let r = move_tracked_with(&detour(&d, &moves), &mut |s| if s.0 == t1 && s.1 == a1 { Err("stuck".into()) } else { move_all(std::slice::from_ref(s)) });
        assert!(r.error.is_some());
        assert_eq!(r.completed, [(a1.clone(), t1.clone())]);
        assert!(Path::new(&t1).exists() && !Path::new(&a1).exists());
        assert!(d.join("a0").exists() && d.join("a2").exists() && !d.join("b0").exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn two_spellings_of_one_name_are_explained() {
        let d = tmp("fold");
        fs::write(d.join("a"), "1").unwrap();
        fs::write(d.join("b"), "2").unwrap();
        fs::write(d.join("Probe"), "").unwrap();
        let folds = d.join("probe").exists();
        fs::remove_file(d.join("Probe")).unwrap();
        let r = move_tracked(&[(p(&d, "a"), p(&d, "Same")), (p(&d, "b"), p(&d, "same"))]);
        if folds {
            assert!(r.error.unwrap().contains("same name on this disk"));
            assert!(r.completed.is_empty() && d.join("a").exists() && d.join("b").exists());
        } else {
            assert_eq!(r.error, None);
        }
        fs::remove_dir_all(&d).unwrap();
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
