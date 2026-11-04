use super::store::*;
use super::*;
use std::fs;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("morfa-journal-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    // The temp folder itself may sit behind a link (macOS): use the real one.
    fs::canonicalize(&d).unwrap()
}
fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
fn entry(kind: Kind, pairs: Vec<(String, String)>) -> Entry {
    build(RecordInput { kind, label: "op".into(), status: Status::Done, error: None, pairs, started: 0 }, true, &|| {})
}
fn inner(dir: &Path) -> Mutex<Inner> {
    Mutex::new(Inner { store: Some(Store::open(dir.join("j.json"))), running: HashSet::new() })
}
fn push(m: &Mutex<Inner>, e: Entry) -> u64 {
    with(m, |st| Ok(st.push(e))).unwrap()
}
fn trash_to(bin: PathBuf) -> impl Fn(&Path) -> CmdResult<()> {
    move |p: &Path| fs::rename(p, bin.join(p.file_name().unwrap())).map_err(|e| e.to_string())
}
fn no_restore(w: &[crate::fsops::trashcan::Window]) -> undo::Restored {
    w.iter().map(|(p, _, _)| (p.clone(), Err("no".to_string()))).collect()
}
fn no_list(_: &[crate::fsops::trashcan::Window]) -> HashSet<String> {
    HashSet::new()
}
fn env<'a>(trash: &'a dyn Fn(&Path) -> CmdResult<()>) -> undo::Env<'a> {
    undo::Env { trash, restore: &no_restore, in_trash: &no_list, can_restore: false, protected: &|_| false }
}
fn mk(st: &mut Store, id: u64) -> CmdResult<Option<u64>> {
    let e = st.get(id).cloned().unwrap();
    let u = undo::undo_entry(&e, &e.pairs, "undo", true, true, &|| {});
    undo::mark(st, id, &e.pairs, u, true)
}
fn never(_: &Path) -> CmdResult<()> {
    panic!("nothing should go to Trash")
}

#[test]
fn save_is_atomic_and_private() {
    let d = tmp("save");
    let path = d.join("j.json");
    fs::write(d.join("j.tmp"), "left by a crash").unwrap();
    let mut st = Store::open(path.clone());
    st.push(entry(Kind::Create, vec![(String::new(), "/x".into())]));
    st.save().unwrap();
    assert!(!d.join("j.tmp").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let again = Store::open(path.clone());
    assert_eq!(again.entries.len(), 1);
    // A failing write leaves the old file whole, and no temp file.
    let before = fs::read(&path).unwrap();
    let blocked = Store::open(d.join("sub/j.json"));
    fs::write(d.join("sub"), "a file where the folder should be").unwrap();
    assert!(blocked.save().is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn tightens_loose_permissions_on_open() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp("perm");
        let path = d.join("j.json");
        Store::open(path.clone()).save().unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        Store::open(path.clone());
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        fs::remove_dir_all(&d).unwrap();
    }
}

#[test]
fn corrupt_file_is_set_aside() {
    let d = tmp("corrupt");
    let path = d.join("j.json");
    fs::write(&path, "{ not json").unwrap();
    let mut st = Store::open(path.clone());
    assert!(st.entries.is_empty());
    let aside = st.set_aside.clone().unwrap();
    assert_eq!(fs::read_to_string(&aside).unwrap(), "{ not json");
    st.push(entry(Kind::Create, vec![(String::new(), "/x".into())]));
    st.save().unwrap();
    assert_eq!(Store::open(path.clone()).entries.len(), 1);
    st.clear().unwrap();
    assert!(!aside.exists());
    assert!(Store::open(path).entries.is_empty());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn newer_format_is_set_aside_not_misread() {
    let d = tmp("version");
    let path = d.join("j.json");
    fs::write(&path, r#"{"version":99,"nextId":5,"entries":[]}"#).unwrap();
    assert!(Store::open(path).set_aside.is_some());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn bounds_hold() {
    let d = tmp("bounds");
    let mut st = Store::open(d.join("j.json"));
    for i in 0..MAX_OPS + 5 {
        st.push(entry(Kind::Create, vec![(String::new(), format!("/nope/{i}"))]));
    }
    assert_eq!(st.entries.len(), MAX_OPS);
    assert_eq!(st.entries[0].pairs[0].to, "/nope/5");

    let many: Vec<(String, String)> = (0..MAX_PAIRS + 7).map(|i| (format!("/a/{i}"), format!("/b/{i}"))).collect();
    let id = st.push(entry(Kind::Copy, many));
    let e = st.get(id).unwrap();
    assert_eq!((e.pairs.len(), e.total), (MAX_PAIRS, (MAX_PAIRS + 7) as u64));

    for _ in 0..MAX_TOTAL_PAIRS / MAX_PAIRS + 2 {
        st.push(entry(Kind::Copy, (0..MAX_PAIRS).map(|i| (format!("/a/{i}"), format!("/b/{i}"))).collect()));
    }
    assert!(st.entries.iter().map(|e| e.pairs.len()).sum::<usize>() <= MAX_TOTAL_PAIRS);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn ids_are_not_reused_after_clear_or_restart() {
    let d = tmp("ids");
    let path = d.join("j.json");
    let mut st = Store::open(path.clone());
    let a = st.push(entry(Kind::Create, vec![]));
    st.clear().unwrap();
    assert!(Store::open(path).push(entry(Kind::Create, vec![])) > a);
    fs::remove_dir_all(&d).unwrap();
}

fn moved(from: &str, to: &str) -> Entry {
    let mut e = entry(Kind::Move, vec![(from.into(), to.into())]);
    e.pairs[0].done = true;
    e
}

fn book(list: Vec<Entry>) -> Vec<Entry> {
    list.into_iter().enumerate().map(|(i, mut e)| { e.id = i as u64 + 1; e.time = e.id; e.started = e.id; e }).collect()
}

#[test]
fn chain_follows_moves_and_renames_and_folders() {
    let mut ren = moved("/b/a.txt", "/b/c.txt");
    ren.kind = Kind::Rename;
    let list = book(vec![moved("/a/a.txt", "/b/a.txt"), ren, moved("/b", "/d/b")]);
    let c = chain::find(&list, "/a/a.txt");
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].latest, "/d/b/c.txt");
    assert_eq!(c[0].hops.len(), 3);
    assert_eq!(c[0].end, chain::End::Located);
    // From a stop along the way, by name, and from inside a moved folder.
    assert_eq!(chain::find(&list, "/b/a.txt")[0].latest, "/d/b/c.txt");
    assert_eq!(chain::find(&list, "A.TXT")[0].latest, "/d/b/c.txt");
    let inside = chain::find(&book(vec![moved("/x", "/y")]), "/x/deep/f");
    assert_eq!(inside[0].latest, "/y/deep/f");
    // No false match on a shared name prefix.
    assert!(chain::find(&book(vec![moved("/x", "/y")]), "/xy/f").is_empty());
}

#[test]
fn chain_of_reused_path_is_two_chains_and_ends_in_trash() {
    let mut tr = entry(Kind::Trash, vec![("/z".into(), String::new())]);
    tr.pairs[0].done = true;
    let list = book(vec![moved("/a", "/y"), moved("/a", "/z"), tr]);
    let c = chain::find(&list, "/a");
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].latest, "/z");
    assert_eq!(c[0].end, chain::End::Trash);
    assert_eq!(c[1].latest, "/y");
}

#[test]
fn undoing_a_move_shows_in_the_chain() {
    let d = tmp("chainundo");
    let (a, b) = (d.join("a"), d.join("b"));
    fs::write(&a, "1").unwrap();
    fs::rename(&a, &b).unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Move, vec![(s(&a), s(&b))]));
    let t = never;
    apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).unwrap();
    let c = with(&m, |st| Ok(chain::find(&st.entries, &s(&a)))).unwrap();
    assert_eq!(c[0].latest, s(&a));
    assert_eq!(c[0].hops.len(), 2);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn move_undo_goes_back_once_only() {
    let d = tmp("double");
    let (a, b) = (d.join("a"), d.join("b"));
    fs::write(&a, "1").unwrap();
    fs::rename(&a, &b).unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Move, vec![(s(&a), s(&b))]));
    let t = never;
    let r = apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).unwrap();
    assert!(r.applied && r.undo_id.is_some());
    assert!(a.exists() && !b.exists());
    // The undo record itself can't be undone, nor the original twice.
    assert_eq!(apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).unwrap_err(), "journal-already-undone");
    assert!(apply(&m, r.undo_id.unwrap(), &HashSet::new(), "undo", true, &env(&t)).unwrap_err().starts_with("journal-blocked"));
    assert!(a.exists() && !b.exists());
    // And it survives a restart.
    assert!(Store::open(d.join("j.json")).get(id).unwrap().undone_by.is_some());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn ctrl_z_mark_blocks_journal_undo() {
    let d = tmp("ctrlz");
    let (a, b) = (d.join("a"), d.join("b"));
    fs::write(&b, "1").unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Move, vec![(s(&a), s(&b))]));
    // Ctrl+Z moved it back, then reported it.
    fs::rename(&b, &a).unwrap();
    with(&m, |st| mk(st, id)).unwrap();
    let t = never;
    assert_eq!(apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).unwrap_err(), "journal-already-undone");
    assert!(a.exists());
    assert!(with(&m, |st| mk(st, id)).is_err());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn running_undo_refuses_a_second() {
    let d = tmp("running");
    let (a, b) = (d.join("a"), d.join("b"));
    fs::write(&b, "1").unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Move, vec![(s(&a), s(&b))]));
    m.lock().unwrap().running.insert(id);
    let t = never;
    assert!(apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).is_err());
    assert!(b.exists());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn occupied_source_is_never_overwritten() {
    let d = tmp("occupied");
    let (a, b) = (d.join("a"), d.join("b"));
    fs::write(&b, "moved").unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Move, vec![(s(&a), s(&b))]));
    fs::write(&a, "newcomer").unwrap();
    let t = never;
    // Naming it doesn't help: only changed files can be named.
    let named: HashSet<String> = [s(&a), s(&b)].into_iter().collect();
    let r = apply(&m, id, &named, "undo", true, &env(&t)).unwrap();
    assert!(!r.applied);
    assert_eq!(r.outcomes[0].reason, Some(undo::Reason::Occupied));
    assert_eq!(fs::read_to_string(&a).unwrap(), "newcomer");
    assert_eq!(fs::read_to_string(&b).unwrap(), "moved");
    // Nothing happened, so it stays undoable once the place is free.
    fs::remove_file(&a).unwrap();
    assert!(apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).unwrap().applied);
    assert_eq!(fs::read_to_string(&a).unwrap(), "moved");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn edited_copy_is_skipped_unless_named() {
    let d = tmp("edited");
    let bin = d.join("bin");
    fs::create_dir(&bin).unwrap();
    let (src, copy) = (d.join("src"), d.join("copy"));
    fs::write(&src, "same").unwrap();
    fs::write(&copy, "same").unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Copy, vec![(s(&src), s(&copy))]));
    fs::write(&copy, "edited by hand").unwrap();
    let t = trash_to(bin.clone());

    let p = with(&m, |st| Ok(undo::preview(&st.entries, st.get(id).unwrap(), &env(&t), &|| {}))).unwrap();
    assert_eq!(p.items[0].reason, Some(undo::Reason::Changed));

    let r = apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).unwrap();
    assert!(!r.applied);
    assert_eq!(r.outcomes[0].status, "skipped");
    assert_eq!(fs::read_to_string(&copy).unwrap(), "edited by hand");

    let named: HashSet<String> = [s(&copy)].into_iter().collect();
    assert!(apply(&m, id, &named, "undo", true, &env(&t)).unwrap().applied);
    assert!(!copy.exists() && bin.join("copy").exists() && src.exists());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn edit_deep_inside_a_copied_folder_counts() {
    let d = tmp("deep");
    let (src, copy) = (d.join("src"), d.join("copy"));
    fs::create_dir_all(copy.join("sub")).unwrap();
    fs::write(copy.join("sub/f"), "1").unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Copy, vec![(s(&src), s(&copy))]));
    fs::write(copy.join("sub/f"), "longer").unwrap();
    let t = never;
    let r = apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).unwrap();
    assert_eq!(r.outcomes[0].reason, Some(undo::Reason::Changed));
    fs::remove_dir_all(&d).unwrap();
}

#[cfg(unix)]
#[test]
fn symlinked_parent_is_never_followed() {
    let d = tmp("link");
    let (real, other) = (d.join("real"), d.join("other"));
    fs::create_dir_all(&real).unwrap();
    fs::create_dir_all(&other).unwrap();
    let (out, a) = (real.join("out"), d.join("a"));
    fs::write(&out, "1").unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Move, vec![(s(&a), s(&out))]));
    // The folder is swapped for a link to a lookalike.
    fs::rename(&real, d.join("real2")).unwrap();
    fs::write(other.join("out"), "1").unwrap();
    std::os::unix::fs::symlink(&other, &real).unwrap();
    let t = never;
    let r = apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).unwrap();
    assert_eq!(r.outcomes[0].reason, Some(undo::Reason::SymlinkParent));
    assert!(other.join("out").exists() && !a.exists());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn partial_failure_is_reported_pair_by_pair() {
    let d = tmp("partial");
    let (a1, a2, b1, b2) = (d.join("a1"), d.join("a2"), d.join("b1"), d.join("b2"));
    fs::write(&b1, "1").unwrap();
    fs::write(&b2, "2").unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Move, vec![(s(&a1), s(&b1)), (s(&a2), s(&b2))]));
    fs::remove_file(&b1).unwrap();
    let t = never;
    let r = apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).unwrap();
    assert!(r.applied);
    assert_eq!(r.outcomes.iter().filter(|o| o.status == "done").count(), 1);
    assert_eq!(r.outcomes.iter().filter(|o| o.reason == Some(undo::Reason::Gone)).count(), 1);
    assert!(a2.exists());
    let undo_entry = with(&m, |st| Ok(st.get(r.undo_id.unwrap()).cloned().unwrap())).unwrap();
    assert_eq!(undo_entry.status, Status::Partial);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn newer_entries_warn_on_shared_paths_only() {
    let d = tmp("newer");
    let (a, b, c) = (d.join("a"), d.join("b"), d.join("c"));
    fs::write(&b, "1").unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Move, vec![(s(&a), s(&b))]));
    push(&m, entry(Kind::Create, vec![(String::new(), s(&d.join("elsewhere")))]));
    let t = never;
    let peek = || with(&m, |st| Ok(undo::preview(&st.entries, st.get(id).unwrap(), &env(&t), &|| {}))).unwrap();
    assert!(peek().newer.is_empty());
    let later = push(&m, entry(Kind::Copy, vec![(s(&b), s(&c))]));
    let p = peek();
    assert_eq!(p.newer.len(), 1);
    assert_eq!(p.newer[0].id, later);
    assert!(p.items[0].newer);
    // Undone, it no longer counts.
    with(&m, |st| mk(st, later)).unwrap();
    assert!(peek().newer.is_empty());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn delete_and_macos_style_restore_say_so() {
    let d = tmp("forever");
    let gone = d.join("gone");
    let m = inner(&d);
    let del = push(&m, entry(Kind::Delete, vec![(s(&gone), String::new())]));
    let tr = push(&m, entry(Kind::Trash, vec![(s(&gone), String::new())]));
    let t = never;
    assert!(apply(&m, del, &HashSet::new(), "undo", true, &env(&t)).unwrap_err().contains("forever"));
    // No restore on this platform: reported, not marked undone, so it isn't falsely closed.
    let r = apply(&m, tr, &HashSet::new(), "undo", true, &env(&t)).unwrap();
    assert!(!r.applied);
    assert_eq!(r.outcomes[0].reason, Some(undo::Reason::Unsupported));
    assert!(with(&m, |st| Ok(st.get(tr).unwrap().undone_by.is_none())).unwrap());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn restore_checks_the_place_and_records() {
    let d = tmp("restore");
    let (a, b) = (d.join("a"), d.join("b"));
    let m = inner(&d);
    let id = push(&m, entry(Kind::Trash, vec![(s(&a), String::new()), (s(&b), String::new())]));
    fs::write(&b, "someone new").unwrap();
    let a2 = a.clone();
    let restore = move |w: &[crate::fsops::trashcan::Window]| -> undo::Restored {
        fs::write(&a2, "back").unwrap();
        w.iter().map(|(p, _, _)| (p.clone(), Ok(()))).collect()
    };
    let in_trash = |w: &[crate::fsops::trashcan::Window]| -> HashSet<String> { w.iter().map(|x| x.0.clone()).collect() };
    let t = never;
    let e = undo::Env { trash: &t, restore: &restore, in_trash: &in_trash, can_restore: true, protected: &|_| false };
    let r = apply(&m, id, &HashSet::new(), "undo", true, &e).unwrap();
    assert!(r.applied);
    assert_eq!(fs::read_to_string(&a).unwrap(), "back");
    assert!(r.outcomes.iter().any(|o| o.to == s(&b) && o.reason == Some(undo::Reason::Occupied)));
    let u = with(&m, |st| Ok(st.get(r.undo_id.unwrap()).cloned().unwrap())).unwrap();
    assert_eq!(u.kind, Kind::Restore);
    // The chain sees it come back.
    let c = with(&m, |st| Ok(chain::find(&st.entries, &s(&a)))).unwrap();
    assert_eq!(c[0].end, chain::End::Located);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn failed_operation_records_nothing_as_done() {
    let d = tmp("failed");
    let (a, b) = (d.join("a"), d.join("b"));
    fs::write(&b, "looks moved").unwrap();
    let e = build(RecordInput { kind: Kind::Rename, label: "x".into(), status: Status::Failed, error: Some("boom".into()), pairs: vec![(s(&a), s(&b))], started: 0 }, true, &|| {});
    assert_eq!(e.status, Status::Failed);
    assert!(!e.pairs[0].done && e.pairs[0].fp.is_none());
    assert_eq!(undo::blocked(&e), Some("nothing"));
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn stale_rename_failure_never_touches_the_unrelated_file() {
    let d = tmp("stale");
    let (a, b) = (d.join("a"), d.join("b"));
    fs::write(&b, "unrelated").unwrap(); // `a` was deleted behind our back
    let pairs = vec![(s(&a), s(&b))];
    let r = crate::fsops::move_tracked(&pairs);
    assert!(r.error.is_some() && r.completed.is_empty());
    let m = inner(&d);
    let id = push(&m, build(RecordInput { kind: Kind::Rename, label: "x".into(), status: Status::Failed, error: r.error, pairs: vec![(s(&a), String::new())], started: 0 }, true, &|| {}));
    let t = never;
    assert!(apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).is_err());
    assert_eq!(fs::read_to_string(&b).unwrap(), "unrelated");
    assert!(!a.exists());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn failed_batch_rolls_back_all_its_steps() {
    let d = tmp("batch");
    let (a, b, ta, tb) = (d.join("a"), d.join("b"), d.join("a.tmp"), d.join("b.tmp"));
    fs::write(&a, "A").unwrap();
    fs::write(&b, "B").unwrap();
    fs::write(&tb, "in the way").unwrap();
    let steps = vec![(s(&a), s(&ta)), (s(&b), s(&tb)), (s(&ta), s(&b)), (s(&tb), s(&a))];
    let r = crate::fsops::move_tracked(&steps);
    assert!(r.error.is_some());
    assert!(r.completed.is_empty(), "{:?}", r.completed);
    assert_eq!(fs::read_to_string(&a).unwrap(), "A");
    assert_eq!(fs::read_to_string(&b).unwrap(), "B");
    assert!(!ta.exists());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn completed_steps_compose_through_temp_names() {
    let d = tmp("net");
    let (a, t, b) = (d.join("a"), d.join("t"), d.join("b"));
    fs::write(&a, "A").unwrap();
    let r = crate::fsops::move_tracked(&[(s(&a), s(&t)), (s(&t), s(&b))]);
    assert_eq!(r.completed, vec![(s(&a), s(&b))]);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn folder_digest_sees_edits_that_keep_the_totals() {
    let d = tmp("digest");
    let root = d.join("t");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("x"), "1").unwrap();
    fs::write(root.join("y"), "22").unwrap();
    let keep = fs::metadata(root.join("y")).unwrap().modified().unwrap();
    let f0 = fingerprint(&root, &|| {}).unwrap();
    // Same count, same total, same newest time: sizes trade places.
    fs::write(root.join("x"), "11").unwrap();
    fs::write(root.join("y"), "2").unwrap();
    for n in ["x", "y"] {
        fs::File::options().write(true).open(root.join(n)).unwrap().set_modified(keep).unwrap();
    }
    let f1 = fingerprint(&root, &|| {}).unwrap();
    assert_eq!((f0.size, f0.count), (f1.size, f1.count));
    assert_ne!(f0.digest, f1.digest);
    assert_eq!(f1, fingerprint(&root, &|| {}).unwrap());
    fs::remove_dir_all(&d).unwrap();
}

#[cfg(unix)]
#[test]
fn file_replaced_with_same_size_and_time_is_changed() {
    let d = tmp("ino");
    let p = d.join("f");
    fs::write(&p, "one").unwrap();
    let keep = fs::metadata(&p).unwrap().modified().unwrap();
    let f0 = fingerprint(&p, &|| {}).unwrap();
    fs::remove_file(&p).unwrap();
    fs::write(d.join("other"), "tmp").unwrap(); // keeps the old inode from being reused at once
    fs::write(&p, "two").unwrap();
    fs::File::options().write(true).open(&p).unwrap().set_modified(keep).unwrap();
    let f1 = fingerprint(&p, &|| {}).unwrap();
    assert_eq!((f0.size, f0.mtime), (f1.size, f1.mtime));
    assert_ne!(f0, f1);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn trash_window_stops_before_the_next_trash_of_that_path() {
    let mk_tr = |id: u64, started: u64, time: u64, p: &str| {
        let mut e = entry(Kind::Trash, vec![(p.into(), String::new())]);
        e.id = id;
        e.started = started;
        e.time = time;
        e
    };
    let first = mk_tr(1, 100_000, 105_000, "/a");
    let later = mk_tr(2, 200_000, 201_000, "/a");
    let other = mk_tr(3, 150_000, 151_000, "/b");
    assert_eq!(undo::window(&[first.clone(), other.clone()], &first, "/a"), ("/a".to_string(), 100, 107));
    let close = mk_tr(4, 103_000, 104_000, "/a");
    assert_eq!(undo::window(&[first.clone(), close, later], &first, "/a"), ("/a".to_string(), 100, 102));
}

#[test]
fn protected_paths_are_refused_whatever_the_file_says() {
    let d = tmp("prot");
    let (a, b) = (d.join("a"), d.join("b"));
    fs::write(&b, "1").unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Move, vec![(s(&a), s(&b))]));
    let t = never;
    let e = undo::Env { trash: &t, restore: &no_restore, in_trash: &no_list, can_restore: false, protected: &|_| true };
    let named: HashSet<String> = [s(&b)].into_iter().collect();
    let r = apply(&m, id, &named, "undo", true, &e).unwrap();
    assert_eq!(r.outcomes[0].reason, Some(undo::Reason::Protected));
    assert!(b.exists() && !a.exists());
    // A relative or `..` path in a tampered file is no better.
    let mut bad = entry(Kind::Copy, vec![]);
    bad.pairs = vec![Pair { from: String::new(), to: format!("{}/../b", d.display()), done: true, ..Default::default() }];
    let id = push(&m, bad);
    let r = apply(&m, id, &named, "undo", true, &env(&t)).unwrap();
    assert_eq!(r.outcomes[0].reason, Some(undo::Reason::Protected));
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn partial_undo_leaves_the_rest_retryable() {
    let d = tmp("retry");
    let (a1, a2, b1, b2) = (d.join("a1"), d.join("a2"), d.join("b1"), d.join("b2"));
    fs::write(&b1, "1").unwrap();
    fs::write(&b2, "2").unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Move, vec![(s(&a1), s(&b1)), (s(&a2), s(&b2))]));
    fs::write(&a1, "in the way").unwrap();
    let t = never;
    let r = apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).unwrap();
    assert!(r.applied);
    assert!(a2.exists() && b1.exists());
    assert!(with(&m, |st| Ok(st.get(id).unwrap().undone_by.is_none())).unwrap());
    // The preview lists only what's left.
    let left = with(&m, |st| Ok(undo::preview(&st.entries, st.get(id).unwrap(), &env(&t), &|| {}))).unwrap();
    assert_eq!(left.items.len(), 1);
    fs::remove_file(&a1).unwrap();
    let r = apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).unwrap();
    assert_eq!(r.outcomes.len(), 1);
    assert_eq!(fs::read_to_string(&a1).unwrap(), "1");
    assert_eq!(apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).unwrap_err(), "journal-already-undone");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn swap_in_one_entry_moves_each_file_once() {
    let mut swap = entry(Kind::BatchRename, vec![("/a".into(), "/b".into()), ("/b".into(), "/a".into())]);
    swap.pairs.iter_mut().for_each(|p| p.done = true);
    let list = book(vec![swap]);
    assert_eq!(chain::follow(list.iter(), "/a").0, "/b");
    assert_eq!(chain::follow(list.iter(), "/b").0, "/a");
    let mut seed = moved("/z", "/a");
    seed.id = 0;
    let list = book(vec![seed, list[0].clone()]);
    let c = chain::find(&list, "/z");
    assert_eq!(c[0].latest, "/b");
    assert_eq!(c[0].hops.len(), 2);
}

#[cfg(unix)]
#[test]
fn unknown_real_folder_is_walked_and_naming_does_not_pass_a_link() {
    let d = tmp("unknown");
    let (real, link) = (d.join("real"), d.join("link"));
    fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    fs::write(real.join("f"), "1").unwrap();
    // A record made with the disk not answering.
    let via_link = link.join("f");
    let plain = build(RecordInput { kind: Kind::Copy, label: "x".into(), status: Status::Done, error: None, pairs: vec![("/src".into(), s(&via_link))], started: 0 }, false, &|| {});
    let m = inner(&d);
    let id = push(&m, plain);
    let t = never;
    let named: HashSet<String> = [s(&via_link)].into_iter().collect();
    let r = apply(&m, id, &named, "undo", true, &env(&t)).unwrap();
    assert_eq!(r.outcomes[0].reason, Some(undo::Reason::SymlinkParent));
    assert!(real.join("f").exists());
    // Through real folders it's merely unverified, which naming may pass.
    let bin = d.join("bin");
    fs::create_dir(&bin).unwrap();
    let tr = trash_to(bin.clone());
    let direct = real.join("f");
    let plain = build(RecordInput { kind: Kind::Copy, label: "x".into(), status: Status::Done, error: None, pairs: vec![("/src".into(), s(&direct))], started: 0 }, false, &|| {});
    let id = push(&m, plain);
    assert_eq!(apply(&m, id, &HashSet::new(), "undo", true, &env(&tr)).unwrap().outcomes[0].reason, Some(undo::Reason::Unverified));
    let named: HashSet<String> = [s(&direct)].into_iter().collect();
    assert!(apply(&m, id, &named, "undo", true, &env(&tr)).unwrap().applied);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn entries_order_by_start_not_by_finish() {
    let d = tmp("order");
    let mut st = Store::open(d.join("j.json"));
    let now = now_ms();
    let mut slow = moved("/a", "/b");
    slow.started = now - 5000;
    let mut quick = moved("/b", "/c");
    quick.started = now - 1000;
    st.push(quick);
    st.push(slow);
    assert_eq!(st.entries[0].pairs[0].from, "/a");
    assert_eq!(chain::find(&st.entries, "/a")[0].latest, "/c");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn newer_warning_counts_partly_undone_and_folded_spelling() {
    assert_eq!(undo::fold_key("/A/\u{0435}\u{0308}", true), undo::fold_key("/a/\u{0451}", true));
    assert_ne!(undo::fold_key("/A", false), undo::fold_key("/a", false));
    let d = tmp("newer2");
    let (a, b, c1, c2) = (d.join("a"), d.join("b"), d.join("c1"), d.join("c2"));
    fs::write(&b, "1").unwrap();
    fs::write(&c1, "1").unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Move, vec![(s(&a), s(&b))]));
    let later = push(&m, entry(Kind::Copy, vec![(s(&b), s(&c1)), (s(&b), s(&c2))]));
    with(&m, |st| {
        let e = st.get(later).cloned().unwrap();
        let u = undo::undo_entry(&e, &e.pairs[..1], "undo", false, false, &|| {});
        undo::mark(st, later, &e.pairs[..1], u, true)
    })
    .unwrap();
    let t = never;
    let p = with(&m, |st| Ok(undo::preview(&st.entries, st.get(id).unwrap(), &env(&t), &|| {}))).unwrap();
    assert!(p.newer.iter().any(|n| n.id == later));
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn set_aside_is_unique_keeps_a_few_and_spares_transient_errors() {
    let d = tmp("aside");
    let path = d.join("j.json");
    let mut seen = HashSet::new();
    for i in 0..5 {
        fs::write(&path, format!("broken {i}")).unwrap();
        seen.insert(Store::open(path.clone()).set_aside.unwrap());
    }
    assert_eq!(seen.len(), 5);
    let left = fs::read_dir(&d).unwrap().flatten().filter(|x| x.file_name().to_string_lossy().starts_with("j.corrupt-")).count();
    assert_eq!(left, 3);
    // Can't be read (here: a folder in its place) is not corrupt: stays, nothing writes over it.
    let dpath = d.join("k.json");
    fs::create_dir(&dpath).unwrap();
    let st = Store::open(dpath.clone());
    assert!(st.blocked && st.set_aside.is_none() && dpath.is_dir());
    assert!(st.save().is_err());
    let m = Mutex::new(Inner { store: Some(st), running: HashSet::new() });
    assert_eq!(with(&m, |_| Ok(())).unwrap_err(), "journal-unavailable");
    fs::remove_dir(&dpath).unwrap();
    assert!(with(&m, |_| Ok(())).is_ok()); // tried again
    fs::remove_dir_all(&d).unwrap();
}

/// Real rule, real folder: temp folders classify as protected, so this has to live under home.
struct HomeDir(PathBuf);
impl HomeDir {
    fn new(tag: &str) -> Self {
        let d = dirs::home_dir().unwrap().join(format!(".morfa-journal-test-{tag}-{}-{}", std::process::id(), now_ms()));
        fs::create_dir_all(&d).unwrap();
        HomeDir(fs::canonicalize(&d).unwrap())
    }
}
impl Drop for HomeDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn real_env<'a>(trash: &'a dyn Fn(&Path) -> CmdResult<()>) -> undo::Env<'a> {
    undo::Env { trash, restore: &no_restore, in_trash: &no_list, can_restore: false, protected: &is_protected }
}

#[test]
fn real_rule_lets_a_move_go_back_to_a_free_place() {
    let h = HomeDir::new("move");
    let (a, b) = (h.0.join("gone/deep/a"), h.0.join("b"));
    fs::write(&b, "1").unwrap();
    let m = inner(&h.0);
    let id = push(&m, entry(Kind::Move, vec![(s(&a), s(&b))]));
    let t = never;
    assert!(!is_protected(&a), "a free place under home");
    let p = with(&m, |st| Ok(undo::preview(&st.entries, st.get(id).unwrap(), &real_env(&t), &|| {}))).unwrap();
    assert_eq!(p.items[0].reason, None);
    assert!(apply(&m, id, &HashSet::new(), "undo", true, &real_env(&t)).unwrap().applied);
    assert_eq!(fs::read_to_string(&a).unwrap(), "1");
}

#[test]
fn real_rule_lets_a_restore_go_back_to_a_free_place() {
    let h = HomeDir::new("restore");
    let a = h.0.join("a");
    let m = inner(&h.0);
    let id = push(&m, entry(Kind::Trash, vec![(s(&a), String::new())]));
    let a2 = a.clone();
    let restore = move |w: &[crate::fsops::trashcan::Window]| -> undo::Restored {
        fs::write(&a2, "back").unwrap();
        w.iter().map(|x| (x.0.clone(), Ok(()))).collect()
    };
    let in_trash = |w: &[crate::fsops::trashcan::Window]| -> HashSet<String> { w.iter().map(|x| x.0.clone()).collect() };
    let t = never;
    let e = undo::Env { trash: &t, restore: &restore, in_trash: &in_trash, can_restore: true, protected: &is_protected };
    assert!(apply(&m, id, &HashSet::new(), "undo", true, &e).unwrap().applied);
    assert!(a.exists());
}

#[test]
fn real_rule_still_refuses_system_places() {
    assert!(is_protected(Path::new("/usr/morfa-nope/x")));
    assert!(is_protected(Path::new("/etc/morfa-nope")));
    assert!(is_protected(&dirs::home_dir().unwrap()));
}

#[test]
fn undoing_in_reverse_order_is_not_called_changed() {
    let d = tmp("reverse");
    let (a, b, c) = (d.join("a"), d.join("b"), d.join("c"));
    fs::write(&b, "1").unwrap();
    let m = inner(&d);
    let first = push(&m, entry(Kind::Move, vec![(s(&a), s(&b))]));
    std::thread::sleep(std::time::Duration::from_millis(5));
    fs::rename(&b, &c).unwrap();
    let second = push(&m, entry(Kind::Rename, vec![(s(&b), s(&c))]));
    let t = never;
    assert!(apply(&m, second, &HashSet::new(), "undo", true, &env(&t)).unwrap().applied);
    std::thread::sleep(std::time::Duration::from_millis(5));
    let r = apply(&m, first, &HashSet::new(), "undo", true, &env(&t)).unwrap();
    assert!(r.applied, "{:?}", r.outcomes);
    assert_eq!(fs::read_to_string(&a).unwrap(), "1");
    fs::remove_dir_all(&d).unwrap();
}

#[cfg(unix)]
#[test]
fn only_metadata_differing_gets_its_own_reason() {
    use std::os::unix::fs::PermissionsExt;
    let d = tmp("meta");
    let (src, copy) = (d.join("s"), d.join("c"));
    fs::write(&copy, "1").unwrap();
    let m = inner(&d);
    let id = push(&m, entry(Kind::Copy, vec![(s(&src), s(&copy))]));
    std::thread::sleep(std::time::Duration::from_millis(5));
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o640)).unwrap();
    let t = never;
    let r = apply(&m, id, &HashSet::new(), "undo", true, &env(&t)).unwrap();
    assert_eq!(r.outcomes[0].reason, Some(undo::Reason::Metadata));
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn clear_works_while_the_file_is_unavailable_and_takes_old_leftovers() {
    let d = tmp("clear");
    let path = d.join("j.json");
    fs::write(d.join("j.corrupt.json"), "old name").unwrap();
    let mut st = Store::open(path.clone());
    st.clear().unwrap();
    assert!(!d.join("j.corrupt.json").exists());
    fs::remove_file(&path).unwrap();
    // Unreadable right now; a folder stands for that, which can't be removed as a file.
    fs::create_dir(&path).unwrap();
    let m = Mutex::new(Inner { store: Some(Store::open(path.clone())), running: HashSet::new() });
    assert!(clear(&m).is_err());
    assert_eq!(with(&m, |_| Ok(())).unwrap_err(), "journal-unavailable");
    fs::remove_dir_all(&d).unwrap();
}

#[cfg(unix)]
#[test]
fn clear_removes_an_unreadable_file() {
    use std::os::unix::fs::PermissionsExt;
    let d = tmp("clear2");
    let path = d.join("j.json");
    fs::write(&path, "secret").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let st = Store::open(path.clone());
    assert!(st.blocked);
    let m = Mutex::new(Inner { store: Some(st), running: HashSet::new() });
    clear(&m).unwrap();
    assert!(with(&m, |st| Ok(st.entries.is_empty())).unwrap());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn rollback_removes_only_the_empty_folders_it_made() {
    let d = tmp("parents");
    let (a, b, deep) = (d.join("a"), d.join("b"), d.join("new/deep/a"));
    fs::write(&a, "A").unwrap();
    fs::write(&b, "B").unwrap();
    let r = crate::fsops::move_tracked(&[(s(&a), s(&deep)), (s(&d.join("missing")), s(&d.join("z")))]);
    assert!(r.error.is_some() && r.completed.is_empty());
    assert!(a.exists() && !d.join("new").exists());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn edit_before_a_later_undone_rename_still_counts_as_changed() {
    let d = tmp("r1");
    let (src, c, c2) = (d.join("s"), d.join("c"), d.join("c2"));
    fs::write(&c, "copy").unwrap();
    let m = inner(&d);
    let copy = push(&m, entry(Kind::Copy, vec![(s(&src), s(&c))]));
    fs::write(&c, "edited, and longer").unwrap();
    fs::rename(&c, &c2).unwrap();
    let ren = push(&m, entry(Kind::Rename, vec![(s(&c), s(&c2))]));
    let t = never;
    assert!(apply(&m, ren, &HashSet::new(), "undo", true, &env(&t)).unwrap().applied);
    let p = with(&m, |st| Ok(undo::preview(&st.entries, st.get(copy).unwrap(), &env(&t), &|| {}))).unwrap();
    assert_eq!(p.items[0].reason, Some(undo::Reason::Changed));
    let r = apply(&m, copy, &HashSet::new(), "undo", true, &env(&t)).unwrap();
    assert!(!r.applied);
    assert_eq!(fs::read_to_string(&c).unwrap(), "edited, and longer");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn foreign_file_brought_back_by_an_undo_counts_as_changed() {
    let d = tmp("r1b");
    let (src, c, c2) = (d.join("s"), d.join("c"), d.join("c2"));
    fs::write(&c, "copy").unwrap();
    let m = inner(&d);
    let copy = push(&m, entry(Kind::Copy, vec![(s(&src), s(&c))]));
    fs::rename(&c, &c2).unwrap();
    let ren = push(&m, entry(Kind::Rename, vec![(s(&c), s(&c2))]));
    // Same size, but another file takes its place, and the user names it to undo the rename anyway.
    fs::remove_file(&c2).unwrap();
    fs::write(d.join("keeps the inode busy"), "x").unwrap();
    fs::write(&c2, "COPY").unwrap();
    let t = never;
    let named: HashSet<String> = [s(&c2)].into_iter().collect();
    assert!(apply(&m, ren, &named, "undo", true, &env(&t)).unwrap().applied);
    let r = apply(&m, copy, &HashSet::new(), "undo", true, &env(&t)).unwrap();
    assert!(!r.applied);
    assert_eq!(r.outcomes[0].reason, Some(undo::Reason::Changed));
    assert_eq!(fs::read_to_string(&c).unwrap(), "COPY");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn list_searches_paths_and_filters_kind() {
    let d = tmp("list");
    let mut st = Store::open(d.join("j.json"));
    st.push(entry(Kind::Move, vec![("/Photos/Cat.jpg".into(), "/b/Cat.jpg".into())]));
    st.push(entry(Kind::Copy, vec![("/docs/x".into(), "/b/x".into())]));
    let q = |search: &str, kind| ListQuery { search: search.into(), kind, offset: 0, limit: 10 };
    assert_eq!(list(&st, &q("cat.j", None)).matched, 1);
    assert_eq!(list(&st, &q("/b/", None)).matched, 2);
    assert_eq!(list(&st, &q("/b/", Some(Kind::Copy))).rows[0].kind, Kind::Copy);
    assert_eq!(list(&st, &q("", None)).rows[0].kind, Kind::Copy);
    fs::remove_dir_all(&d).unwrap();
}
