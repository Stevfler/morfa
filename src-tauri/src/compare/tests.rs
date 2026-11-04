use super::apply::{self, Action, Op, Plan, Status};
use super::Record;
use super::scan::{classify_files, merge_names, scan, Basis, Class, CompareResult, Mode};
use super::*;
use crate::cloud::sync::Fold;
use std::{
    fs::File,
    sync::atomic::AtomicUsize,
    time::{Duration, UNIX_EPOCH},
};

static N: AtomicUsize = AtomicUsize::new(0);

struct Tmp {
    base: PathBuf,
    l: PathBuf,
    r: PathBuf,
    bin: PathBuf,
    last: std::cell::RefCell<Option<Record>>,
}
impl Tmp {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!("morfa-cmp-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&base);
        let (l, r, bin) = (base.join("l"), base.join("r"), base.join("bin"));
        for d in [&l, &r, &bin] {
            fs::create_dir_all(d).unwrap();
        }
        Tmp { l: fs::canonicalize(&l).unwrap(), r: fs::canonicalize(&r).unwrap(), bin, base, last: Default::default() }
    }
    /// Stands in for the system Trash, so tests don't fill the real one.
    fn trash(&self) -> impl Fn(&Path) -> CmdResult<()> + '_ {
        move |p: &Path| {
            let to = self.bin.join(format!("{}-{}", N.fetch_add(1, Ordering::SeqCst), p.file_name().unwrap().to_string_lossy()));
            fs::rename(p, to).map_err(|e| e.to_string())
        }
    }
    fn binned(&self) -> Vec<String> {
        fs::read_dir(&self.bin).unwrap().map(|e| fs::read_to_string(e.unwrap().path()).unwrap_or_default()).collect()
    }
    fn scan(&self, mode: Mode) -> CompareResult {
        let (flag, beat) = (AtomicBool::new(false), || {});
        let mut r = scan(&self.l, &self.r, mode, &Job { cancel: &flag, beat: &beat }, &mut |_, _, _| {}).unwrap();
        r.scan_id = 7;
        *self.last.borrow_mut() = Some(Record::of(&r));
        r
    }
    fn record(&self) -> Record {
        self.last.borrow().clone().unwrap_or_else(|| Record::of(&CompareResult { scan_id: 7, left_root: self.l.to_string_lossy().into(), right_root: self.r.to_string_lossy().into(), mode: Mode::Quick, items: vec![], same_count: 0, same_bytes: 0, same_truncated: false, elapsed_ms: 0 }))
    }
    fn plan(&self, actions: Vec<Action>, confirm_deletes: bool) -> Plan {
        Plan { scan_id: 7, left_root: self.l.to_string_lossy().into(), right_root: self.r.to_string_lossy().into(), actions, confirm_deletes }
    }
    fn apply(&self, actions: Vec<Action>, confirm_deletes: bool) -> apply::SyncReport {
        let plan = self.plan(actions, confirm_deletes);
        let (flag, beat) = (AtomicBool::new(false), || {});
        apply::run(&plan, &self.record(), 1, &Job { cancel: &flag, beat: &beat }, &self.trash(), &mut |_, _, _| {}).unwrap()
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn put(p: &Path, text: &str, secs: u64) {
    if let Some(d) = p.parent() {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(p, text).unwrap();
    File::options().write(true).open(p).unwrap().set_modified(UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
}
fn find<'a>(r: &'a CompareResult, rel: &str) -> &'a scan::Item {
    r.items.iter().find(|i| i.left_rel == rel || i.right_rel == rel).unwrap_or_else(|| panic!("{rel} not in {:?}", r.items))
}
fn action(i: &scan::Item, op: Op) -> Action {
    Action { left_rel: i.left_rel.clone(), right_rel: i.right_rel.clone(), op, left: i.left.clone(), right: i.right.clone(), replace_newer: false }
}
fn file(size: u64, ms: u64) -> Snap {
    Snap { kind: Kind::File, size, mtime: ms, files: 0, errors: 0, target: None }
}

#[test]
fn classification() {
    let at = |s, t| file(s, t);
    assert_eq!(classify_files(&at(5, 1_000_000), &at(5, 1_000_000), None), (Class::Same, Basis::SizeTime));
    assert_eq!(classify_files(&at(5, 9_000_000), &at(5, 1_000_000), None).0, Class::NewerLeft);
    assert_eq!(classify_files(&at(5, 1_000_000), &at(9, 9_000_000), None).0, Class::NewerRight);
    assert_eq!(classify_files(&at(5, 1_000_000), &at(9, 1_000_000), None).0, Class::Differs);
    assert_eq!(classify_files(&at(5, 1), &at(5, 9_000_000), Some(true)), (Class::Same, Basis::Sha256), "content beats time");
    assert_eq!(classify_files(&at(5, 1_000_000), &at(5, 1_000_000), Some(false)), (Class::SameSizeDifferent, Basis::Sha256));
}

#[test]
fn time_tolerance_is_two_seconds() {
    let base = 10_000_000;
    assert_eq!(classify_files(&file(1, base + TOLERANCE_MS), &file(1, base), None).0, Class::Same);
    assert_eq!(classify_files(&file(1, base), &file(1, base + TOLERANCE_MS), None).0, Class::Same);
    assert_eq!(classify_files(&file(1, base + TOLERANCE_MS + 1), &file(1, base), None).0, Class::NewerLeft);
}

#[test]
fn walk_classifies_and_compares_content_only_in_content_mode() {
    let t = Tmp::new();
    put(&t.l.join("only-l"), "x", 1000);
    put(&t.r.join("sub/only-r"), "x", 1000);
    fs::create_dir_all(t.l.join("sub")).unwrap();
    put(&t.l.join("twin"), "aaa", 1000);
    put(&t.r.join("twin"), "aaa", 1000);
    put(&t.l.join("sneaky"), "aaa", 1000);
    put(&t.r.join("sneaky"), "bbb", 1000);
    put(&t.l.join("newer"), "long one", 5000);
    put(&t.r.join("newer"), "short", 1000);
    put(&t.l.join("clash"), "file", 1000);
    fs::create_dir_all(t.r.join("clash")).unwrap();

    let q = t.scan(Mode::Quick);
    assert_eq!(find(&q, "only-l").class, Class::OnlyLeft);
    assert_eq!(find(&q, "sub/only-r").class, Class::OnlyRight);
    assert_eq!(find(&q, "sneaky").class, Class::Same, "quick mode can't see this");
    assert_eq!(find(&q, "newer").class, Class::NewerLeft);
    assert_eq!(find(&q, "clash").class, Class::TypeConflict);
    assert!(q.items.iter().all(|i| i.left_rel != "sub"), "a folder on both sides isn't an item");

    let c = t.scan(Mode::Content);
    assert_eq!((find(&c, "sneaky").class, find(&c, "sneaky").basis), (Class::SameSizeDifferent, Basis::Sha256));
    assert_eq!((find(&c, "twin").class, find(&c, "twin").basis), (Class::Same, Basis::Sha256));
    assert_eq!(find(&c, "newer").basis, Basis::SizeTime, "no hash when sizes differ");
}

#[test]
fn one_sided_folder_is_one_item_with_totals() {
    let t = Tmp::new();
    put(&t.l.join("d/a"), "12", 1000);
    put(&t.l.join("d/e/b"), "345", 9000);
    let r = t.scan(Mode::Quick);
    assert_eq!(r.items.len(), 1);
    let s = r.items[0].left.as_ref().unwrap();
    assert_eq!((s.kind, s.size, s.files), (Kind::Dir, 5, 2));
    assert!(s.mtime >= 9_000_000);
}

#[cfg(unix)]
#[test]
fn symlinks_are_compared_by_target_and_never_followed() {
    use std::os::unix::fs::symlink;
    let t = Tmp::new();
    let outside = t.base.join("outside");
    put(&outside.join("secret"), "x", 1000);
    symlink("t1", t.l.join("same")).unwrap();
    symlink("t1", t.r.join("same")).unwrap();
    symlink("t1", t.l.join("diff")).unwrap();
    symlink("t2", t.r.join("diff")).unwrap();
    symlink(&outside, t.l.join("dirlink")).unwrap();
    fs::create_dir(t.r.join("dirlink")).unwrap();
    symlink(&outside, t.l.join("onelink")).unwrap();
    symlink(t.l.join("."), t.l.join("loop")).unwrap();
    let r = t.scan(Mode::Content);
    assert_eq!((find(&r, "same").class, find(&r, "same").basis), (Class::Same, Basis::Target));
    assert_eq!((find(&r, "diff").class, find(&r, "diff").basis), (Class::Differs, Basis::Target));
    assert_eq!(find(&r, "dirlink").class, Class::TypeConflict);
    let one = find(&r, "onelink").left.as_ref().unwrap();
    assert_eq!((one.kind, one.files), (Kind::Symlink, 0));
    assert!(r.items.iter().all(|i| !i.left_rel.contains("secret")), "not followed");
    assert_eq!(find(&r, "loop").class, Class::OnlyLeft);
}

#[cfg(unix)]
#[test]
fn special_files_are_listed_as_skipped() {
    let t = Tmp::new();
    let fifo = |p: &Path| {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(p.as_os_str().as_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    };
    fifo(&t.l.join("pipe"));
    fs::create_dir(t.l.join("d")).unwrap();
    fifo(&t.l.join("d/inner"));
    let r = t.scan(Mode::Content);
    assert_eq!((find(&r, "pipe").class, find(&r, "pipe").reason), (Class::Skipped, Some("special")));
    assert_eq!((find(&r, "d/inner").class, find(&r, "d/inner").reason), (Class::Skipped, Some("special")));
    assert_eq!(find(&r, "d").class, Class::OnlyLeft);
}

#[test]
fn folding_pairs_spellings_only_when_the_disk_folds() {
    let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let on = Fold { case: true, unicode: true };
    let off = Fold { case: false, unicode: false };
    let (nfc, nfd) = ("caf\u{e9}", "cafe\u{301}");
    assert_eq!(merge_names(v(&["Readme"]), v(&["README"]), on), vec![(v(&["Readme"]), v(&["README"]))]);
    assert_eq!(merge_names(v(&[nfc]), v(&[nfd]), on), vec![(v(&[nfc]), v(&[nfd]))]);
    assert_eq!(merge_names(v(&["Readme"]), v(&["README"]), off).len(), 2);
    assert_eq!(merge_names(v(&[nfc]), v(&[nfd]), off).len(), 2);
    // Two spellings on one side stay together, so the walk can refuse to pair them.
    assert_eq!(merge_names(v(&["a", "A"]), v(&["a"]), on), vec![(v(&["a", "A"]), v(&["a"]))]);
}

/// On this disk whatever it is: the result must fit what the disk says about the names.
#[test]
fn case_only_difference_on_the_real_disk() {
    let t = Tmp::new();
    put(&t.l.join("Readme.TXT"), "same", 1000);
    put(&t.r.join("readme.txt"), "same", 1000);
    let folds = scan::detect_fold(&t.l).case;
    let r = t.scan(Mode::Quick);
    if folds {
        assert_eq!(r.same_count, 1);
        assert_eq!((r.items[0].left_rel.as_str(), r.items[0].right_rel.as_str()), ("Readme.TXT", "readme.txt"));
    } else {
        assert_eq!(r.items.len(), 2);
    }
}

#[test]
fn nested_or_identical_folders_are_refused() {
    let t = Tmp::new();
    let (l, r) = (t.l.to_string_lossy().to_string(), t.r.to_string_lossy().to_string());
    assert!(check_roots(&l, &r).is_ok());
    assert_eq!(check_roots(&l, &l).unwrap_err(), "compare-same");
    let inner = t.l.join("in/ner");
    fs::create_dir_all(&inner).unwrap();
    let inner = inner.to_string_lossy().to_string();
    assert_eq!(check_roots(&l, &inner).unwrap_err(), "compare-nested");
    assert_eq!(check_roots(&inner, &l).unwrap_err(), "compare-nested");
    #[cfg(unix)]
    {
        let link = t.base.join("alias");
        std::os::unix::fs::symlink(&t.l, &link).unwrap();
        assert_eq!(check_roots(&link.to_string_lossy(), &l).unwrap_err(), "compare-same");
        let via = link.join("in").to_string_lossy().to_string();
        assert_eq!(check_roots(&via, &l).unwrap_err(), "compare-nested");
    }
}

#[test]
fn copy_creates_with_time_and_undo_list() {
    let t = Tmp::new();
    put(&t.l.join("d/a"), "hello", 1000);
    put(&t.l.join("f"), "file", 1000);
    let r = t.scan(Mode::Quick);
    let rep = t.apply(vec![action(find(&r, "d"), Op::CopyLeftToRight), action(find(&r, "f"), Op::CopyLeftToRight)], false);
    assert!(rep.outcomes.iter().all(|o| o.status == Status::Done), "{rep:?}");
    assert_eq!(fs::read_to_string(t.r.join("d/a")).unwrap(), "hello");
    assert_eq!(rep.created.len(), 2);
    assert_eq!(rep.bytes, 9);
    let again = t.scan(Mode::Quick);
    assert_eq!((again.same_count, again.items.iter().filter(|i| i.class != Class::Same).count()), (2, 0), "copies keep their times, so nothing differs now");
}

#[test]
fn replaced_file_goes_to_trash_first() {
    let t = Tmp::new();
    put(&t.l.join("f"), "new text", 9000);
    put(&t.r.join("f"), "old", 1000);
    let r = t.scan(Mode::Quick);
    let rep = t.apply(vec![action(find(&r, "f"), Op::CopyLeftToRight)], false);
    assert_eq!(rep.outcomes[0].status, Status::Done);
    assert_eq!(fs::read_to_string(t.r.join("f")).unwrap(), "new text");
    assert_eq!(t.binned(), vec!["old"]);
    assert_eq!(rep.replaced.len(), 1);
}

#[test]
fn a_failed_trash_leaves_the_destination_alone() {
    let t = Tmp::new();
    put(&t.l.join("f"), "new text", 9000);
    put(&t.r.join("f"), "old", 1000);
    let r = t.scan(Mode::Quick);
    let plan = t.plan(vec![action(find(&r, "f"), Op::CopyLeftToRight)], false);
    let (flag, beat) = (AtomicBool::new(false), || {});
    let no_trash = |_: &Path| Err("no trash here".to_string());
    let rep = apply::run(&plan, &t.record(), 1, &Job { cancel: &flag, beat: &beat }, &no_trash, &mut |_, _, _| {}).unwrap();
    assert_eq!(rep.outcomes[0].status, Status::Failed);
    assert_eq!(fs::read_to_string(t.r.join("f")).unwrap(), "old");
    assert_eq!(fs::read_dir(&t.r).unwrap().count(), 1, "no temporary file left");
}

#[test]
fn overwriting_something_newer_takes_an_explicit_choice() {
    let t = Tmp::new();
    put(&t.l.join("f"), "older", 1000);
    put(&t.r.join("f"), "newer text", 9000);
    let r = t.scan(Mode::Quick);
    let a = action(find(&r, "f"), Op::CopyLeftToRight);
    let rep = t.apply(vec![a.clone()], false);
    assert_eq!((rep.outcomes[0].status, rep.outcomes[0].reason), (Status::Skipped, Some("newerDest")));
    assert_eq!(fs::read_to_string(t.r.join("f")).unwrap(), "newer text");
    let rep = t.apply(vec![Action { replace_newer: true, ..a }], false);
    assert_eq!(rep.outcomes[0].status, Status::Done);
    assert_eq!(t.binned(), vec!["newer text"]);
}

#[test]
fn revalidation_skips_what_changed_since_the_scan() {
    let t = Tmp::new();
    put(&t.l.join("src-edit"), "a", 1000);
    put(&t.l.join("dst-appeared"), "b", 1000);
    put(&t.l.join("gone"), "c", 1000);
    put(&t.l.join("del-edit"), "d", 1000);
    put(&t.l.join("deep/x/y"), "e", 1000);
    put(&t.l.join("fine"), "f", 1000);
    let r = t.scan(Mode::Quick);
    let actions = vec![
        action(find(&r, "src-edit"), Op::CopyLeftToRight),
        action(find(&r, "dst-appeared"), Op::CopyLeftToRight),
        action(find(&r, "gone"), Op::CopyLeftToRight),
        action(find(&r, "del-edit"), Op::DeleteLeft),
        action(find(&r, "deep"), Op::DeleteLeft),
        action(find(&r, "fine"), Op::CopyLeftToRight),
    ];
    put(&t.l.join("src-edit"), "edited", 2000);
    put(&t.r.join("dst-appeared"), "theirs", 1000);
    fs::remove_file(t.l.join("gone")).unwrap();
    put(&t.l.join("del-edit"), "edited", 2000);
    put(&t.l.join("deep/x/z"), "new deep inside", 1000);
    let rep = t.apply(actions, true);
    let statuses: Vec<_> = rep.outcomes.iter().map(|o| (o.status, o.reason)).collect();
    let changed = (Status::Skipped, Some("changed"));
    assert_eq!(statuses, vec![changed, changed, changed, changed, changed, (Status::Done, None)]);
    assert_eq!(fs::read_to_string(t.r.join("dst-appeared")).unwrap(), "theirs");
    assert!(t.l.join("deep/x/z").exists() && t.l.join("del-edit").exists());
    assert!(t.binned().is_empty());
}

#[test]
fn deletes_need_the_confirmation_and_go_to_trash() {
    let t = Tmp::new();
    put(&t.r.join("f"), "bye", 1000);
    let r = t.scan(Mode::Quick);
    let a = action(find(&r, "f"), Op::DeleteRight);
    let rep = t.apply(vec![a.clone()], false);
    assert_eq!(rep.outcomes[0].reason, Some("deleteNotConfirmed"));
    assert!(t.r.join("f").exists());
    let rep = t.apply(vec![a], true);
    assert_eq!(rep.outcomes[0].status, Status::Done);
    assert!(!t.r.join("f").exists());
    assert_eq!(t.binned(), vec!["bye"]);
    assert_eq!(rep.trashed.len(), 1);
}

#[cfg(unix)]
#[test]
fn never_writes_through_a_symlinked_parent() {
    let t = Tmp::new();
    put(&t.l.join("sub/f"), "x", 1000);
    fs::create_dir(t.r.join("sub")).unwrap();
    let r = t.scan(Mode::Quick);
    let a = action(find(&r, "sub/f"), Op::CopyLeftToRight);
    let elsewhere = t.base.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    fs::remove_dir(t.r.join("sub")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, t.r.join("sub")).unwrap();
    let rep = t.apply(vec![a], false);
    assert_eq!((rep.outcomes[0].status, rep.outcomes[0].reason), (Status::Skipped, Some("symlinkParent")));
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[test]
fn traversal_in_a_plan_is_refused() {
    let t = Tmp::new();
    put(&t.base.join("victim"), "keep", 1000);
    let a = Action { left_rel: "../victim".into(), right_rel: "../victim".into(), op: Op::DeleteLeft, left: Some(file(4, 1_000_000)), right: None, replace_newer: false };
    let rep = t.apply(vec![a], true);
    assert_eq!((rep.outcomes[0].status, rep.outcomes[0].reason), (Status::Skipped, Some("badPath")));
    assert!(t.base.join("victim").exists());
}

#[cfg(unix)]
#[test]
fn a_root_that_stopped_being_canonical_stops_the_plan() {
    let t = Tmp::new();
    let alias = t.base.join("alias");
    std::os::unix::fs::symlink(&t.l, &alias).unwrap();
    let plan = Plan { scan_id: 7, left_root: alias.to_string_lossy().into(), right_root: t.r.to_string_lossy().into(), actions: vec![], confirm_deletes: false };
    let (flag, beat) = (AtomicBool::new(false), || {});
    assert_eq!(apply::run(&plan, &t.record(), 1, &Job { cancel: &flag, beat: &beat }, &t.trash(), &mut |_, _, _| {}).unwrap_err(), "compare-changed");
}

#[test]
fn cancel_stops_the_scan_and_the_plan() {
    let t = Tmp::new();
    put(&t.l.join("a"), "1", 1000);
    put(&t.l.join("b"), "2", 1000);
    let (on, beat) = (AtomicBool::new(true), || {});
    let e = scan(&t.l, &t.r, Mode::Quick, &Job { cancel: &on, beat: &beat }, &mut |_, _, _| {}).unwrap_err();
    assert_eq!(e, CANCELLED);

    // Cancelled mid-copy: the first report trips the flag; nothing partial stays behind.
    let big = "x".repeat(3 << 20);
    put(&t.l.join("c"), &big, 1000);
    let r = t.scan(Mode::Quick);
    let plan = t.plan(vec![action(find(&r, "c"), Op::CopyLeftToRight), action(find(&r, "a"), Op::CopyLeftToRight)], false);
    let off = AtomicBool::new(false);
    let rep = apply::run(&plan, &t.record(), 1, &Job { cancel: &off, beat: &beat }, &t.trash(), &mut |_, _, _| off.store(true, Ordering::Relaxed)).unwrap();
    assert_eq!(rep.outcomes.iter().map(|o| o.status).collect::<Vec<_>>(), vec![Status::Cancelled, Status::Cancelled]);
    assert_eq!(fs::read_dir(&t.r).unwrap().count(), 0, "temporary file removed");
}

#[test]
fn hashing_cancels_partway() {
    let t = Tmp::new();
    let big = "x".repeat(4 << 20);
    put(&t.l.join("f"), &big, 1000);
    put(&t.r.join("f"), &big, 1000);
    let flag = AtomicBool::new(false);
    let beat = || {};
    let e = scan(&t.l, &t.r, Mode::Content, &Job { cancel: &flag, beat: &beat }, &mut |_, bytes, _| {
        if bytes > 0 {
            flag.store(true, Ordering::Relaxed)
        }
    });
    assert_eq!(e.unwrap_err(), CANCELLED);
}

/// Stands in for a Trash that, like the real one, restores the newest item of a path.
#[derive(Default)]
struct FakeTrash(std::cell::RefCell<Vec<(PathBuf, String)>>);
impl FakeTrash {
    fn put(&self, p: &Path) -> CmdResult<()> {
        self.0.borrow_mut().push((p.to_path_buf(), fs::read_to_string(p).map_err(|e| e.to_string())?));
        fs::remove_file(p).map_err(|e| e.to_string())
    }
    fn restore(&self, p: &str) -> CmdResult<()> {
        let mut v = self.0.borrow_mut();
        let Some(i) = v.iter().rposition(|(o, _)| o == Path::new(p)) else { return Ok(()) };
        let (o, text) = v.remove(i);
        fs::write(o, text).map_err(|e| e.to_string())
    }
}

#[test]
fn undo_of_a_replace_brings_back_the_original_not_the_copy() {
    let t = Tmp::new();
    let f = t.r.join("f");
    let bin = FakeTrash::default();
    fs::write(&f, "original").unwrap();
    bin.put(&f).unwrap();
    fs::write(&f, "new copy").unwrap();
    let s = f.to_string_lossy().to_string();
    apply::undo(&[], std::slice::from_ref(&s), &[], &|p| bin.put(p), &|p| bin.restore(p)).unwrap();
    assert_eq!(fs::read_to_string(&f).unwrap(), "original");
    assert_eq!(fs::read_dir(&t.r).unwrap().count(), 1, "nothing left aside");
    assert_eq!(bin.0.borrow().iter().map(|x| x.1.as_str()).collect::<Vec<_>>(), vec!["new copy"]);
}

#[test]
fn undo_keeps_the_copy_when_nothing_can_be_restored() {
    let t = Tmp::new();
    let f = t.r.join("f");
    fs::write(&f, "new copy").unwrap();
    let s = f.to_string_lossy().to_string();
    let bin = FakeTrash::default();
    // Trash has no such item: the restore "succeeds" with nothing.
    let e = apply::undo(&[], std::slice::from_ref(&s), &[], &|p| bin.put(p), &|p| bin.restore(p)).unwrap_err();
    assert!(e.contains("not found in Trash"), "{e}");
    assert_eq!(fs::read_to_string(&f).unwrap(), "new copy");
    // No restore on this OS: same, and the app's own message gets through.
    let e = apply::undo(&[], std::slice::from_ref(&s), &[], &|p| bin.put(p), &|_| Err("UNSUPPORTED_RESTORE: no".into())).unwrap_err();
    assert!(e.starts_with("UNSUPPORTED_RESTORE:"));
    assert_eq!(fs::read_to_string(&f).unwrap(), "new copy");
    assert_eq!(fs::read_dir(&t.r).unwrap().count(), 1);
}

#[cfg(unix)]
fn chmod(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(unix)]
#[test]
fn parents_swapped_during_the_copy_or_trash_stop_it() {
    use std::os::unix::fs::symlink;
    let t = Tmp::new();
    put(&t.l.join("sub/a"), &"x".repeat(100), 9000);
    put(&t.l.join("sub/b"), "new", 9000);
    put(&t.r.join("sub/b"), "old", 1000);
    let r = t.scan(Mode::Quick);
    let elsewhere = t.base.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    let swap = || {
        let moved = t.base.join("moved");
        if !moved.exists() {
            fs::rename(t.r.join("sub"), &moved).unwrap();
            symlink(&elsewhere, t.r.join("sub")).unwrap();
        }
    };
    // During the copy.
    let plan = t.plan(vec![action(find(&r, "sub/a"), Op::CopyLeftToRight)], false);
    let (flag, beat) = (AtomicBool::new(false), || {});
    let job = Job { cancel: &flag, beat: &beat };
    let rep = apply::run(&plan, &t.record(), 1, &job, &t.trash(), &mut |_, _, _| swap()).unwrap();
    assert_eq!(rep.outcomes[0].reason, Some("symlinkParent"));
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    // During Trash: the old item is gone, the new one isn't put through the link, and the report says so.
    fs::remove_file(t.r.join("sub")).unwrap();
    fs::rename(t.base.join("moved"), t.r.join("sub")).unwrap();
    for e in fs::read_dir(t.r.join("sub")).unwrap() {
        let p = e.unwrap().path();
        if p.file_name().unwrap() != "b" {
            fs::remove_file(p).unwrap();
        }
    }
    let plan = t.plan(vec![action(find(&r, "sub/b"), Op::CopyLeftToRight)], false);
    let trash = |p: &Path| {
        (t.trash())(p)?;
        swap();
        Ok(())
    };
    let rep = apply::run(&plan, &t.record(), 1, &job, &trash, &mut |_, _, _| {}).unwrap();
    assert_eq!((rep.outcomes[0].status, rep.outcomes[0].reason), (Status::Failed, Some("oldInTrash")));
    assert_eq!(rep.bytes, 0);
    assert_eq!(rep.trashed.len(), 1);
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    assert_eq!(rep.leftovers.len(), 1, "the temporary file moved away with its folder: named, not lost");
}

#[test]
fn a_rename_failing_after_trash_says_so_and_counts_nothing() {
    let t = Tmp::new();
    put(&t.l.join("f"), "new text", 9000);
    put(&t.r.join("f"), "old", 1000);
    let r = t.scan(Mode::Quick);
    let plan = t.plan(vec![action(find(&r, "f"), Op::CopyLeftToRight)], false);
    let dst = t.r.join("f");
    let trash = |p: &Path| {
        (t.trash())(p)?;
        fs::write(&dst, "someone else's").map_err(|e| e.to_string())
    };
    let (flag, beat) = (AtomicBool::new(false), || {});
    let rep = apply::run(&plan, &t.record(), 1, &Job { cancel: &flag, beat: &beat }, &trash, &mut |_, _, _| {}).unwrap();
    assert_eq!((rep.outcomes[0].status, rep.outcomes[0].reason), (Status::Failed, Some("oldInTrash")));
    assert_eq!((rep.bytes, rep.trashed.len(), rep.replaced.len()), (0, 1, 0));
    assert_eq!(fs::read_to_string(&dst).unwrap(), "someone else's");
    assert_eq!(fs::read_dir(&t.r).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn a_failed_look_is_not_absence() {
    let t = Tmp::new();
    put(&t.l.join("d/f"), "x", 1000);
    put(&t.r.join("d/f"), "x", 1000);
    put(&t.r.join("d/g"), "x", 1000);
    chmod(&t.l.join("d"), 0o400); // listable, but its items can't be looked at
    let denied = fs::symlink_metadata(t.l.join("d/f")).is_err();
    let r = t.scan(Mode::Quick);
    let (flag, beat) = (AtomicBool::new(false), || {});
    let looked = snap(&t.l.join("d/f"), &Job { cancel: &flag, beat: &beat }, &mut |_, _| {});
    chmod(&t.l.join("d"), 0o700);
    if denied {
        assert_eq!(looked.unwrap_err(), UNREADABLE);
        assert_eq!((find(&r, "d/f").class, find(&r, "d/f").reason), (Class::Skipped, Some("unreadable")));
        assert!(r.items.iter().all(|i| i.class != Class::OnlyRight || i.left_rel != "d/f"));
    }
}

/// A name the disk maps onto an existing entry the listing shows under another spelling.
#[test]
fn copy_onto_another_spelling_is_skipped_with_the_true_reason() {
    let t = Tmp::new();
    put(&t.l.join("abc"), "mine", 1000);
    put(&t.r.join("ABC"), "theirs", 1000);
    let folds = scan::detect_fold(&t.r).case;
    let _ = t.scan(Mode::Quick);
    let a = Action { left_rel: "abc".into(), right_rel: "abc".into(), op: Op::CopyLeftToRight, left: Some(snap(&t.l.join("abc"), &Job { cancel: &AtomicBool::new(false), beat: &|| {} }, &mut |_, _| {}).unwrap().unwrap()), right: None, replace_newer: false };
    if folds {
        // Refused by the record first; the scan paired them, so check the disk rule through a fresh record.
        let mut res = scan(&t.l, &t.r, Mode::Quick, &Job { cancel: &AtomicBool::new(false), beat: &|| {} }, &mut |_, _, _| {}).unwrap();
        res.scan_id = 7;
        res.items.push(scan::Item { left_rel: "abc".into(), right_rel: "abc".into(), class: Class::OnlyLeft, basis: Basis::Presence, reason: None, left: a.left.clone(), right: None });
        let rec = Record::of(&res);
        let plan = t.plan(vec![a], false);
        let (flag, beat) = (AtomicBool::new(false), || {});
        let rep = apply::run(&plan, &rec, 1, &Job { cancel: &flag, beat: &beat }, &t.trash(), &mut |_, _, _| {}).unwrap();
        assert_eq!((rep.outcomes[0].status, rep.outcomes[0].reason), (Status::Skipped, Some("nameClash")));
        assert_eq!(fs::read_to_string(t.r.join("ABC")).unwrap(), "theirs");
    }
}

#[test]
fn sigma_spellings_never_become_a_wrong_action() {
    let t = Tmp::new();
    put(&t.l.join("\u{3a3}\u{391}\u{3a3}"), "x", 1000); // lowercases with a final sigma
    put(&t.r.join("\u{3c3}\u{3b1}\u{3c3}"), "y", 1000);
    let same_on_disk = t.r.join("\u{3a3}\u{391}\u{3a3}").symlink_metadata().is_ok();
    let r = t.scan(Mode::Quick);
    if same_on_disk {
        assert!(r.items.iter().all(|i| i.class == Class::Skipped && i.reason == Some("nameClash")), "{:?}", r.items);
    } else {
        assert_eq!(r.items.iter().filter(|i| matches!(i.class, Class::OnlyLeft | Class::OnlyRight)).count(), 2);
    }
}

#[test]
fn any_newer_destination_or_unknown_time_needs_the_choice() {
    let at = |ms| file(1, ms);
    assert!(apply::needs_choice(&at(5000), &at(5001)), "inside the tolerance still counts");
    assert!(apply::needs_choice(&at(5000), &at(0)));
    assert!(apply::needs_choice(&at(0), &at(5000)));
    assert!(!apply::needs_choice(&at(5000), &at(5000)));
    assert!(!apply::needs_choice(&at(5001), &at(5000)));
}

#[test]
fn a_temporary_name_we_did_not_create_is_left_alone() {
    let t = Tmp::new();
    put(&t.l.join("f"), "x", 1000);
    let r = t.scan(Mode::Quick);
    let foreign = t.r.join(format!(".morfa-sync-{}-1-1.part", std::process::id()));
    fs::write(&foreign, "not ours").unwrap();
    let rep = t.apply(vec![action(find(&r, "f"), Op::CopyLeftToRight)], false);
    assert_eq!(rep.outcomes[0].status, Status::Done);
    assert_eq!(fs::read_to_string(&foreign).unwrap(), "not ours");
}

#[test]
fn own_temporaries_are_not_compared() {
    let t = Tmp::new();
    put(&t.l.join(".morfa-sync-1-2-3.part"), "x", 1000);
    put(&t.l.join("d/.morfa-sync-1-2-3.part"), "x", 1000);
    let r = t.scan(Mode::Quick);
    assert_eq!(r.items.len(), 1);
    assert_eq!(r.items[0].left.as_ref().unwrap().files, 0);
}

#[cfg(unix)]
#[test]
fn read_only_temporaries_are_removed_and_undeletable_ones_reported() {
    let t = Tmp::new();
    let tree = t.r.join("tree");
    put(&tree.join("sub/f"), "x", 1000);
    chmod(&tree.join("sub"), 0o500);
    apply::discard_tree(&tree).unwrap();
    assert!(!tree.exists());

    put(&t.l.join("p/f"), "new", 9000);
    put(&t.r.join("p/f"), "old", 1000);
    let r = t.scan(Mode::Quick);
    let plan = t.plan(vec![action(find(&r, "p/f"), Op::CopyLeftToRight)], false);
    let parent = t.r.join("p");
    let trash = |_: &Path| {
        chmod(&parent, 0o500);
        Err("no trash".to_string())
    };
    let (flag, beat) = (AtomicBool::new(false), || {});
    let rep = apply::run(&plan, &t.record(), 1, &Job { cancel: &flag, beat: &beat }, &trash, &mut |_, _, _| {}).unwrap();
    chmod(&parent, 0o700);
    assert_eq!(rep.leftovers.len(), 1);
    assert_eq!(fs::read_to_string(parent.join("f")).unwrap(), "old");
}

#[test]
fn a_plan_is_bound_to_its_scan() {
    let t = Tmp::new();
    put(&t.l.join("f"), "x", 1000);
    put(&t.l.join("same"), "s", 1000);
    put(&t.r.join("same"), "s", 1000);
    let r = t.scan(Mode::Quick);
    let good = action(find(&r, "f"), Op::CopyLeftToRight);
    let (flag, beat) = (AtomicBool::new(false), || {});
    let job = Job { cancel: &flag, beat: &beat };
    // Another scan's id.
    let plan = Plan { scan_id: 8, ..t.plan(vec![good.clone()], false) };
    assert_eq!(apply::run(&plan, &t.record(), 1, &job, &t.trash(), &mut |_, _, _| {}).unwrap_err(), "compare-stale");
    // Items the scan didn't offer: an identical one, a forged snapshot, a side that isn't there.
    let same = Action { left: find(&r, "same").left.clone(), right: find(&r, "same").right.clone(), ..action(find(&r, "f"), Op::DeleteLeft) };
    let same = Action { left_rel: "same".into(), right_rel: "same".into(), ..same };
    let mut forged = good.clone();
    forged.left.as_mut().unwrap().size = 99;
    let no_side = action(find(&r, "f"), Op::DeleteRight);
    let rep = t.apply(vec![same, forged, no_side], true);
    assert!(rep.outcomes.iter().all(|o| o.reason == Some("notInScan")), "{rep:?}");
    assert!(t.l.join("same").exists() && t.l.join("f").exists());
    assert_eq!(t.apply(vec![good], false).outcomes[0].status, Status::Done);
}
