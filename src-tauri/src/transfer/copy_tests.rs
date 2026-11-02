use super::super::{rename_into, run, Ctx};
use super::*;
use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Instant};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("orbit-transfer-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("src")).unwrap();
    fs::create_dir_all(d.join("dst")).unwrap();
    d
}

fn cx() -> Ctx {
    Ctx { app: None, ops: Arc::default(), id: 1, done: 0, total: 0, last: Instant::now() }
}

fn mv(src: &Path, dest: &Path) -> (Vec<(String, String)>, CmdResult<()>) {
    let mut pairs = vec![];
    let r = run(&mut cx(), &[src.to_string_lossy().into()], dest, true, &mut pairs);
    (pairs, r)
}

#[test]
fn move_dir_onto_dir_placeholder() {
    let d = tmp("dir");
    let s = d.join("src/a");
    fs::create_dir(&s).unwrap();
    fs::write(s.join("f"), "x").unwrap();
    let t = reserve(&s, &fs::symlink_metadata(&s).unwrap(), &d.join("dst")).unwrap();
    let (ok, t) = rename_into(&s, &fs::symlink_metadata(&t).unwrap(), &d.join("dst"), t).unwrap();
    assert!(ok && !s.exists());
    assert_eq!(fs::read_to_string(t.join("f")).unwrap(), "x");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn move_file_onto_file_placeholder_and_never_overwrite() {
    let d = tmp("file");
    let s = d.join("src/a.txt");
    fs::write(&s, "new").unwrap();
    fs::write(d.join("dst/a.txt"), "old").unwrap();
    let (pairs, r) = mv(&s, &d.join("dst"));
    r.unwrap();
    assert_eq!(fs::read_to_string(d.join("dst/a.txt")).unwrap(), "old");
    assert_eq!(fs::read_to_string(d.join("dst/a (2).txt")).unwrap(), "new");
    assert_eq!(pairs.len(), 1);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn failed_copy_cleans_up_and_keeps_source() {
    let d = tmp("unreadable");
    let s = d.join("src/a");
    fs::create_dir_all(s.join("locked")).unwrap();
    fs::write(s.join("locked/f"), "x").unwrap();
    fs::set_permissions(s.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    let mut pairs = vec![];
    // Copy (not move), so the fill path runs on the same filesystem.
    let r = run(&mut cx(), &[s.to_string_lossy().into()], &d.join("dst"), false, &mut pairs);
    fs::set_permissions(s.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
    if unsafe { libc::geteuid() } != 0 {
        // (root ignores permissions, so only check as a normal user)
        assert!(r.is_err() && pairs.is_empty(), "unreadable entry must fail the item");
        assert_eq!(fs::read_dir(d.join("dst")).unwrap().count(), 0, "partial target removed");
    }
    assert!(s.join("locked/f").exists());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn fifo_is_refused_without_hanging() {
    let d = tmp("fifo");
    let s = d.join("src/a");
    fs::create_dir(&s).unwrap();
    let c = std::ffi::CString::new(s.join("pipe").to_string_lossy().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
    let mut pairs = vec![];
    let r = run(&mut cx(), &[s.to_string_lossy().into()], &d.join("dst"), false, &mut pairs);
    assert!(r.unwrap_err().contains("special file"));
    assert_eq!(fs::read_dir(d.join("dst")).unwrap().count(), 0);
    assert!(s.join("pipe").exists());
    // A top-level FIFO is refused too.
    let r = run(&mut cx(), &[s.join("pipe").to_string_lossy().into()], &d.join("dst"), false, &mut pairs);
    assert!(r.unwrap_err().contains("special file"));
    fs::remove_dir_all(&d).unwrap();
}
