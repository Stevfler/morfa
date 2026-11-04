use super::apply::{self, check, Env, Plan, Status};
use super::scan::{self, by_size, cloud_kind, firmlink_spelling, hash_partial, identity, merge_links, whole_of, Cloud, Found, Options, Phase, EDGE};
use super::*;
use crate::cleanup::Protection;
use std::{
    fs,
    fs::File,
    path::PathBuf,
    sync::atomic::AtomicUsize,
    time::{Duration, UNIX_EPOCH},
};

static N: AtomicUsize = AtomicUsize::new(0);

struct Tmp {
    base: PathBuf,
    root: PathBuf,
    bin: PathBuf,
}
impl Tmp {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!("morfa-dedupe-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&base);
        let (root, bin) = (base.join("root"), base.join("bin"));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&bin).unwrap();
        Tmp { root: fs::canonicalize(&root).unwrap(), bin, base }
    }
    fn at(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
    fn s(&self, rel: &str) -> String {
        self.at(rel).to_string_lossy().into_owned()
    }
    fn put(&self, rel: &str, data: &[u8]) {
        put(&self.at(rel), data, 1_000_000);
    }
    fn scan_min(&self, min_size: u64) -> ScanResult {
        let (flag, beat) = (AtomicBool::new(false), || {});
        let opts = Options { min_size, mirrors: &[], home: None, protection: &|_| Protection::Safe };
        let mut r = scan::scan(&self.root, &opts, &Job { cancel: &flag, beat: &beat }, &mut |_, _, _, _, _| {}).unwrap();
        r.scan_id = 7;
        r
    }
    fn scan(&self) -> ScanResult {
        self.scan_min(1)
    }
    /// Stands in for the system Trash, so tests don't fill the real one.
    fn trash(&self) -> impl Fn(&Path) -> CmdResult<()> + '_ {
        move |p: &Path| {
            let to = self.bin.join(format!("{}-{}", N.fetch_add(1, Ordering::SeqCst), p.file_name().unwrap().to_string_lossy()));
            fs::rename(p, to).map_err(|e| e.to_string())
        }
    }
    fn binned(&self) -> usize {
        fs::read_dir(&self.bin).unwrap().count()
    }
    fn remove(&self, rec: &mut Record, rels: &[&str]) -> CmdResult<apply::Report> {
        let plan = self.plan(rels);
        let (flag, beat) = (AtomicBool::new(false), || {});
        let trash = self.trash();
        apply::run(&plan, rec, &Job { cancel: &flag, beat: &beat }, &Env { trash: &trash, protection: &|_| Protection::Safe, rehash_limit: apply::REHASH_LIMIT }, &mut |_, _, _| {})
    }
    fn plan(&self, rels: &[&str]) -> Plan {
        Plan { scan_id: 7, paths: rels.iter().map(|r| self.s(r)).collect(), confirmed: true, confirm_cloud: false, confirm_whole: false }
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn put(p: &Path, data: &[u8], secs: u64) {
    if let Some(d) = p.parent() {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(p, data).unwrap();
    File::options().write(true).open(p).unwrap().set_modified(UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
}
fn outcome(r: &apply::Report, p: &str) -> (Status, Option<&'static str>) {
    let o = r.outcomes.iter().find(|o| o.path == p).unwrap();
    (o.status, o.reason)
}
fn job_with<T>(f: impl FnOnce(&Job) -> T) -> T {
    let (flag, beat) = (AtomicBool::new(false), || {});
    f(&Job { cancel: &flag, beat: &beat })
}

#[test]
fn size_stage_keeps_only_repeated_sizes() {
    let f = |p: &str, size| Found { path: p.into(), size, mtime: 0 };
    let g = by_size(vec![f("a", 1), f("b", 2), f("c", 1), f("d", 3)]);
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].iter().map(|x| x.path.to_str().unwrap()).collect::<Vec<_>>(), ["a", "c"]);
}

#[test]
fn minimum_size_is_respected_and_empty_files_never_match() {
    let t = Tmp::new();
    t.put("small1", b"abc");
    t.put("small2", b"abc");
    t.put("empty1", b"");
    t.put("empty2", b"");
    let big = vec![9u8; 2000];
    t.put("big1", &big);
    t.put("big2", &big);
    let r = t.scan_min(1000);
    assert_eq!(r.groups.len(), 1);
    assert_eq!(r.groups[0].size, 2000);
    assert_eq!(r.skipped.below_minimum, 4);
    assert_eq!(t.scan_min(0).groups.len(), 2, "a minimum of 0 still leaves empty files out");
}

#[test]
fn partial_stage_filters_before_any_full_read() {
    let t = Tmp::new();
    let size = (3 * EDGE) as usize;
    let mut a = vec![1u8; size];
    let b = a.clone();
    a[0] = 2; // differs in the head
    t.put("a", &a);
    t.put("b", &b);
    let r = t.scan();
    assert!(r.groups.is_empty());
    assert_eq!(r.stages.partial_hashed, 2);
    assert_eq!(r.stages.full_hashed, 0, "a head that differs saves reading the file");
    assert_eq!(r.stages.bytes_read, 4 * EDGE);
}

#[test]
fn full_stage_catches_a_difference_in_the_middle() {
    let t = Tmp::new();
    let size = (3 * EDGE) as usize;
    let a = vec![1u8; size];
    let mut b = a.clone();
    b[size / 2] = 2;
    t.put("a", &a);
    t.put("b", &b);
    t.put("c", &a);
    let r = t.scan();
    assert_eq!(r.stages.full_hashed, 3);
    assert_eq!(r.groups.len(), 1);
    let paths: Vec<_> = r.groups[0].files.iter().map(|f| f.path.clone()).collect();
    assert_eq!(paths, [t.s("a"), t.s("c")]);
}

#[test]
fn partial_hash_sees_the_tail() {
    let t = Tmp::new();
    let size = (3 * EDGE) as usize;
    let a = vec![1u8; size];
    let mut b = a.clone();
    b[size - 1] = 2;
    t.put("a", &a);
    t.put("b", &b);
    job_with(|j| {
        let h = |n: &str| hash_partial(&t.at(n), &identity(&t.at(n)).unwrap(), size as u64, j).unwrap();
        assert_ne!(h("a"), h("b"));
    });
}

#[test]
fn groups_are_ordered_by_reclaimable_bytes() {
    let t = Tmp::new();
    for n in ["s1", "s2", "s3", "s4"] {
        t.put(n, &[1u8; 100]);
    }
    for n in ["l1", "l2"] {
        t.put(n, &[2u8; 250]);
    }
    let r = t.scan();
    assert_eq!(r.groups.iter().map(|g| g.reclaimable).collect::<Vec<_>>(), [300, 250]);
    assert_eq!(r.reclaimable, 550);
    assert_eq!(r.groups[0].files[0].size, 100);
    assert_eq!(r.groups[0].files[0].mtime, 1_000_000_000);
}

#[test]
fn hard_links_are_one_file() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    fs::hard_link(t.at("a"), t.at("a-link")).unwrap();
    let r = t.scan();
    assert!(r.groups.is_empty(), "two names of one file free nothing");
    assert_eq!(r.stages.partial_hashed, 0, "nor are they read");
}

#[test]
fn hard_link_beside_a_real_copy_counts_as_one_and_frees_nothing() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    fs::hard_link(t.at("a"), t.at("a-link")).unwrap();
    t.put("b", &[1u8; 100]);
    let r = t.scan();
    assert_eq!(r.groups.len(), 1);
    let g = &r.groups[0];
    assert_eq!(g.files.len(), 2);
    assert_eq!(g.files[0].path, t.s("a"));
    assert_eq!(g.files[0].aliases, [t.s("a-link")]);
    assert_eq!(g.files[0].links, 2);
    assert_eq!(g.reclaimable, 100, "only the unlinked copy frees anything");
    let mut rec = Record::of(&r);
    let rep = t.remove(&mut rec, &["a"]).unwrap();
    assert_eq!(rep.bytes, 100);
    assert_eq!(rep.freed, 0, "the other name keeps it on disk");
    assert!(t.at("a-link").exists());
}

#[test]
fn merge_links_has_one_entry_per_inode() {
    let id = |ino| scan::Id { dev: 1, ino, links: 2, ctime: 0 };
    let f = |p: &str| Found { path: p.into(), size: 1, mtime: 0 };
    let m = merge_links(vec![(f("z"), id(1)), (f("b"), id(1)), (f("c"), id(2))]);
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].file.path, Path::new("b"));
    assert_eq!(m[0].aliases, ["z"]);
}

#[cfg(unix)]
#[test]
fn symlinks_are_never_followed() {
    use std::os::unix::fs::symlink;
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    symlink(t.at("a"), t.at("link-to-file")).unwrap();
    let out = t.base.join("outside");
    fs::create_dir_all(&out).unwrap();
    put(&out.join("copy"), &[1u8; 100], 1);
    symlink(&out, t.at("link-to-dir")).unwrap();
    let r = t.scan();
    assert!(r.groups.is_empty(), "{:?}", r.groups);
    assert_eq!(r.skipped.symlinks, 2);
}

#[cfg(unix)]
#[test]
fn special_files_are_skipped_without_blocking() {
    let t = Tmp::new();
    let c = std::ffi::CString::new(t.s("fifo")).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert_eq!(t.scan().skipped.special, 1);
}

#[test]
fn trash_folders_and_our_temporaries_are_left_out() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put(".Trash/a", &[1u8; 100]);
    t.put(".morfa-sync-1-2-3.part", &[1u8; 100]);
    assert!(t.scan().groups.is_empty());
}

#[test]
fn placeholders_are_never_read_and_are_named() {
    let t = Tmp::new();
    t.put(".movie.mov.icloud", &[1u8; 100]);
    t.put("real.mov", &[1u8; 100]);
    let r = t.scan();
    assert!(r.groups.is_empty());
    assert_eq!(r.placeholder_count, 1);
    assert_eq!(r.placeholders, [t.s(".movie.mov.icloud")]);
    assert_eq!(r.stages.bytes_read, 0);
}

#[test]
fn cloud_folders_are_recognised() {
    let home = Path::new("/h");
    let mirrors = [PathBuf::from("/data/cloud")];
    let k = |p: &str| cloud_kind(Path::new(p), &mirrors, Some(home));
    assert_eq!(k("/data/cloud/x/y"), Some(Cloud::Mirror));
    assert_eq!(k("/data/clouds/x"), None);
    assert_eq!(k("/h/Library/CloudStorage/OneDrive-Me/a"), Some(Cloud::Provider));
    assert_eq!(k("/h/Library/Mobile Documents/com~apple~CloudDocs/a"), Some(Cloud::Provider));
    assert_eq!(k("/h/Dropbox/a"), Some(Cloud::Provider));
    assert_eq!(k("/h/OneDrive - Acme/a"), Some(Cloud::Provider));
    assert_eq!(k("/h/Dropboxes/a"), None);
    assert_eq!(k("/h/Documents/a"), None);
}

#[test]
fn flagged_scan_results_carry_cloud_and_protection() {
    let t = Tmp::new();
    t.put("cloud/a", &[1u8; 100]);
    t.put("sys/b", &[1u8; 100]);
    let (flag, beat) = (AtomicBool::new(false), || {});
    let mirrors = [t.at("cloud")];
    let sys = t.at("sys");
    let protect = move |p: &Path| if p.starts_with(&sys) { Protection::Protected } else { Protection::Safe };
    let opts = Options { min_size: 1, mirrors: &mirrors, home: None, protection: &protect };
    let r = scan::scan(&t.root, &opts, &Job { cancel: &flag, beat: &beat }, &mut |_, _, _, _, _| {}).unwrap();
    let f = &r.groups[0].files;
    assert_eq!(f[0].cloud, Some(Cloud::Mirror));
    assert_eq!(f[1].protection, Protection::Protected);
}

#[test]
fn wholes_are_named() {
    for (p, name) in [
        ("/x/Foo.app/Contents/lib", "Foo.app"),
        ("/x/.git/objects/ab", ".git"),
        ("/x/Disk.sparsebundle/bands/1f", "Disk.sparsebundle"),
        ("/x/My.photoslibrary/originals/a.jpg", "My.photoslibrary"),
        ("/x/Doc.pages/Data/p.jpg", "Doc.pages"),
        ("/x/P.xcodeproj/project.pbxproj", "P.xcodeproj"),
        ("/x/C.lrlibrary/a", "C.lrlibrary"),
        ("/x/L.fcpbundle/a", "L.fcpbundle"),
        ("/x/S.logicx/a", "S.logicx"),
        ("/x/I.pkg/a", "I.pkg"),
        ("/x/A.plugin/a", "A.plugin"),
        ("/x/B.dSYM/Contents/a", "B.dSYM"),
        ("/x/W.vmwarevm/d.vmdk", "W.vmwarevm"),
        ("/x/W.pvm/d.hdd", "W.pvm"),
        ("/x/proj/node_modules/a/i.js", "node_modules"),
        ("/x/VirtualBox VMs/w/d.vdi", "VirtualBox VMs"),
    ] {
        assert_eq!(whole_of(Path::new(p)).as_deref(), Some(name), "{p}");
    }
    for p in ["/x/Foo.app", "/x/y/z.bin", "/x/my.pages.old/a", "/x/.app/a"] {
        assert_eq!(whole_of(Path::new(p)), None, "{p}");
    }
}

#[test]
fn more_wholes_are_named() {
    for (p, name) in [("/x/M.backupbundle/a", "M.backupbundle"), ("/x/Backups.backupdb/m/a", "Backups.backupdb"), ("/x/D.xcdatamodeld/a", "D.xcdatamodeld"), ("/x/O.imovieproject/a", "O.imovieproject"), ("/x/R.tmbundle/a", "R.tmbundle")] {
        assert_eq!(whole_of(Path::new(p)).as_deref(), Some(name), "{p}");
    }
}

#[test]
fn a_sparse_image_is_recognised_by_its_layout() {
    for marker in ["Info.plist", "token"] {
        let t = Tmp::new();
        t.put("renamed/bands/1", &[1u8; 10]);
        assert_eq!(whole_of(&t.at("renamed/bands/1")), None, "bands alone isn't enough");
        t.put(&format!("renamed/{marker}"), b"x");
        assert_eq!(whole_of(&t.at("renamed/bands/1")).as_deref(), Some("renamed"), "{marker}");
    }
}

#[test]
fn cancel_arriving_during_the_checks_stops_the_trash() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    let plan = t.plan(&["a"]);
    let flag = AtomicBool::new(false);
    let beat = || {};
    let trash = |_: &Path| -> CmdResult<()> { panic!("trashed after cancel") };
    let protection = |_: &Path| {
        flag.store(true, Ordering::Relaxed);
        Protection::Safe
    };
    let rep = apply::run(&plan, &mut rec, &Job { cancel: &flag, beat: &beat }, &Env { trash: &trash, protection: &protection, rehash_limit: apply::REHASH_LIMIT }, &mut |_, _, _| {}).unwrap();
    assert_eq!(outcome(&rep, &t.s("a")), (Status::Cancelled, Some("cancelled")));
    assert!(t.at("a").exists());
}

#[test]
fn a_part_of_a_whole_needs_its_own_confirmation() {
    let t = Tmp::new();
    t.put("Disk.sparsebundle/bands/1", &[1u8; 100]);
    t.put("Disk.sparsebundle/bands/2", &[1u8; 100]);
    let r = t.scan();
    assert_eq!(r.groups[0].files[0].whole.as_deref(), Some("Disk.sparsebundle"));
    let rec = Record::of(&r);
    let mut p = t.plan(&["Disk.sparsebundle/bands/1"]);
    assert_eq!(check(&p, &rec).unwrap_err(), "dedupe-whole-unconfirmed");
    p.confirm_whole = true;
    assert!(check(&p, &rec).is_ok());
}

#[test]
fn the_short_spelling_wins_when_it_is_the_same_item() {
    let yes = |_: &Path, _: &Path| true;
    let no = |_: &Path, _: &Path| false;
    let long = Path::new("/System/Volumes/Data/Users/u/a");
    assert_eq!(firmlink_spelling(long, &yes).unwrap(), Path::new("/Users/u/a"));
    assert_eq!(firmlink_spelling(long, &no), None);
    assert_eq!(firmlink_spelling(Path::new("/Users/u/a"), &yes), None);
    assert_eq!(firmlink_spelling(Path::new("/System/Volumes/Data"), &yes), None);
    let id = |ino| scan::Id { dev: 1, ino, links: 2, ctime: 0 };
    let f = |p: &str| Found { path: p.into(), size: 1, mtime: 0 };
    let m = merge_links(vec![(f("/System/Volumes/Data/Users/u/a"), id(1)), (f("/Users/u/a"), id(1))]);
    assert_eq!(m[0].file.path, Path::new("/Users/u/a"));
}

/// A real scan through the data volume's spelling, in a small folder of the home directory.
#[cfg(target_os = "macos")]
#[test]
fn a_home_folder_scanned_by_its_long_spelling_has_plain_paths_and_is_not_protected() {
    let home = fs::canonicalize(dirs::home_dir().unwrap()).unwrap();
    let dir = home.join(format!(".morfa-dedupe-test-{}", std::process::id()));
    put(&dir.join("a"), &[1u8; 100], 1_000_000);
    put(&dir.join("b"), &[1u8; 100], 1_000_000);
    let long = Path::new("/System/Volumes/Data").join(dir.strip_prefix("/").unwrap());
    let (flag, beat) = (AtomicBool::new(false), || {});
    let opts = Options { min_size: 1, mirrors: &[], home: Some(&home), protection: &protection_of };
    let r = scan::scan(&long, &opts, &Job { cancel: &flag, beat: &beat }, &mut |_, _, _, _, _| {});
    let cleaned = fs::remove_dir_all(&dir);
    let r = r.unwrap();
    cleaned.unwrap();
    assert_eq!(r.root, dir.to_string_lossy());
    let f = &r.groups[0].files;
    assert_eq!(f[0].path, dir.join("a").to_string_lossy());
    assert_eq!(f[0].protection, Protection::Safe);
}

fn edit_keeping_mtime(p: &Path, data: &[u8]) {
    std::thread::sleep(Duration::from_millis(20));
    put(p, data, 1_000_000);
}

fn remove_limited(t: &Tmp, rec: &mut Record, rels: &[&str], rehash_limit: u64) -> apply::Report {
    let plan = t.plan(rels);
    let trash = t.trash();
    job_with(|j| apply::run(&plan, rec, j, &Env { trash: &trash, protection: &|_| Protection::Safe, rehash_limit }, &mut |_, _, _| {}).unwrap())
}

#[cfg(unix)]
#[test]
fn an_edit_that_restores_the_mtime_is_caught_on_the_target() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    edit_keeping_mtime(&t.at("a"), &[7u8; 100]);
    assert_eq!(outcome(&t.remove(&mut rec, &["a"]).unwrap(), &t.s("a")), (Status::Skipped, Some("changed")));
}

#[cfg(unix)]
#[test]
fn a_large_kept_copy_is_trusted_only_while_its_ctime_stands() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let r = t.scan();
    let mut untouched = Record::of(&r);
    edit_keeping_mtime(&t.at("b"), &[2u8; 100]);
    assert_eq!(outcome(&remove_limited(&t, &mut untouched, &["a"], 0), &t.s("a")).1, Some("noKeptCopy"), "changed ctime means read it");

    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    assert_eq!(remove_limited(&t, &mut rec, &["a"], 0).trashed.len(), 1, "unchanged and above the limit: not read");
}

#[cfg(unix)]
#[test]
fn a_kept_copy_that_cannot_be_read_does_not_count() {
    use std::os::unix::fs::PermissionsExt;
    for limit in [0, apply::REHASH_LIMIT] {
        let t = Tmp::new();
        t.put("a", &[1u8; 100]);
        t.put("b", &[1u8; 100]);
        let mut rec = Record::of(&t.scan());
        fs::set_permissions(t.at("b"), fs::Permissions::from_mode(0o000)).unwrap();
        let rep = remove_limited(&t, &mut rec, &["a"], limit);
        fs::set_permissions(t.at("b"), fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(outcome(&rep, &t.s("a")).1, Some("noKeptCopy"), "limit {limit}");
    }
}

#[cfg(unix)]
#[test]
fn a_kept_copy_edited_during_the_run_stops_the_last_removal() {
    let t = Tmp::new();
    for n in ["a", "b", "c"] {
        t.put(n, &[1u8; 100]);
    }
    let mut rec = Record::of(&t.scan());
    let plan = t.plan(&["a", "b"]);
    let inner = t.trash();
    let kept = t.at("c");
    let trash = |p: &Path| {
        edit_keeping_mtime(&kept, &[2u8; 100]);
        inner(p)
    };
    let rep = job_with(|j| apply::run(&plan, &mut rec, j, &Env { trash: &trash, protection: &|_| Protection::Safe, rehash_limit: apply::REHASH_LIMIT }, &mut |_, _, _| {}).unwrap());
    assert_eq!(rep.trashed, [t.s("a")]);
    assert_eq!(outcome(&rep, &t.s("b")).1, Some("noKeptCopy"));
}

#[test]
fn real_protection_refuses_system_paths() {
    assert_eq!(protection_of(Path::new("/usr/bin/env")), Protection::Protected);
}

#[test]
fn cancel_stops_each_stage() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let opts = Options { min_size: 1, mirrors: &[], home: None, protection: &|_| Protection::Safe };
    let flag = AtomicBool::new(true);
    let beat = || {};
    let e = scan::scan(&t.root, &opts, &Job { cancel: &flag, beat: &beat }, &mut |_, _, _, _, _| {}).unwrap_err();
    assert_eq!(e, "cancelled");
    // Cancelled once hashing starts.
    for at in [Phase::Partial, Phase::Full] {
        let flag = AtomicBool::new(false);
        let mut report = |p: Phase, _: u64, _: u64, _: u64, _: &Path| {
            if p == at {
                flag.store(true, Ordering::Relaxed);
            }
        };
        let e = scan::scan(&t.root, &opts, &Job { cancel: &flag, beat: &beat }, &mut report).unwrap_err();
        assert_eq!(e, "cancelled", "{at:?}");
    }
}

#[test]
fn removes_the_chosen_copy_and_keeps_the_rest() {
    let t = Tmp::new();
    t.put("keep", &[1u8; 100]);
    t.put("extra", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    let rep = t.remove(&mut rec, &["extra"]).unwrap();
    assert_eq!(rep.trashed, [t.s("extra")]);
    assert_eq!((rep.bytes, rep.freed), (100, 100));
    assert!(t.at("keep").exists() && !t.at("extra").exists());
    assert_eq!(t.binned(), 1);
}

#[test]
fn a_plan_naming_every_copy_is_refused_whole() {
    let t = Tmp::new();
    t.put("a1", &[1u8; 100]);
    t.put("a2", &[1u8; 100]);
    t.put("b1", &[2u8; 100]);
    t.put("b2", &[2u8; 100]);
    let mut rec = Record::of(&t.scan());
    assert_eq!(t.remove(&mut rec, &["a1", "a2", "b1"]).unwrap_err(), "dedupe-every-copy");
    assert_eq!(t.binned(), 0, "not even the other group's file");
}

#[test]
fn repeating_a_path_does_not_hide_or_double_anything() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    t.put("c", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    let rep = t.remove(&mut rec, &["a", "a"]).unwrap();
    assert_eq!(rep.outcomes.len(), 1);
    assert_eq!(rep.bytes, 100);
}

#[test]
fn plans_in_turn_cannot_remove_every_copy() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    t.remove(&mut rec, &["a"]).unwrap();
    assert_eq!(t.remove(&mut rec, &["b"]).unwrap_err(), "dedupe-every-copy");
    assert!(t.at("b").exists());
}

#[test]
fn tampered_plans_are_refused() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("a-link-free", &[1u8; 100]);
    t.put("other", &[9u8; 50]);
    let outside = t.base.join("secret");
    put(&outside, &[1u8; 100], 1);
    let r = t.scan();
    let rec = Record::of(&r);
    let mut p = t.plan(&["a"]);
    p.paths.push(outside.to_string_lossy().into());
    assert_eq!(check(&p, &rec).unwrap_err(), "dedupe-not-in-scan");
    let mut p = t.plan(&["a"]);
    p.paths.push(format!("{}/../root/a", t.root.display()));
    assert_eq!(check(&p, &rec).unwrap_err(), "dedupe-not-in-scan");
    assert_eq!(check(&t.plan(&["other"]), &rec).unwrap_err(), "dedupe-not-in-scan", "a file of no group");
    let mut p = t.plan(&["a"]);
    p.scan_id = 8;
    assert_eq!(check(&p, &rec).unwrap_err(), "dedupe-stale");
    let mut p = t.plan(&["a"]);
    p.confirmed = false;
    assert_eq!(check(&p, &rec).unwrap_err(), "dedupe-unconfirmed");
    assert!(check(&t.plan(&["a"]), &rec).is_ok());
    assert!(outside.exists());
}

#[test]
fn a_hard_link_alias_cannot_be_planned() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    fs::hard_link(t.at("a"), t.at("b-link")).unwrap();
    t.put("c", &[1u8; 100]);
    let rec = Record::of(&t.scan());
    assert_eq!(check(&t.plan(&["b-link"]), &rec).unwrap_err(), "dedupe-not-in-scan");
}

#[test]
fn cloud_copies_need_their_own_confirmation() {
    let t = Tmp::new();
    t.put("cloud/a", &[1u8; 100]);
    t.put("local", &[1u8; 100]);
    let (flag, beat) = (AtomicBool::new(false), || {});
    let mirrors = [t.at("cloud")];
    let opts = Options { min_size: 1, mirrors: &mirrors, home: None, protection: &|_| Protection::Safe };
    let mut r = scan::scan(&t.root, &opts, &Job { cancel: &flag, beat: &beat }, &mut |_, _, _, _, _| {}).unwrap();
    r.scan_id = 7;
    let rec = Record::of(&r);
    assert_eq!(check(&t.plan(&["cloud/a"]), &rec).unwrap_err(), "dedupe-cloud-unconfirmed");
    let mut p = t.plan(&["cloud/a"]);
    p.confirm_cloud = true;
    assert!(check(&p, &rec).is_ok());
    assert!(check(&t.plan(&["local"]), &rec).is_ok());
}

#[test]
fn protected_paths_are_refused_at_removal() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    let plan = t.plan(&["a"]);
    let trash = t.trash();
    let (flag, beat) = (AtomicBool::new(false), || {});
    let rep = apply::run(&plan, &mut rec, &Job { cancel: &flag, beat: &beat }, &Env { trash: &trash, protection: &|_| Protection::Protected, rehash_limit: apply::REHASH_LIMIT }, &mut |_, _, _| {}).unwrap();
    assert_eq!(outcome(&rep, &t.s("a")), (Status::Skipped, Some("protected")));
    assert!(t.at("a").exists());
}

#[test]
fn a_changed_file_is_skipped() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    t.put("c", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    // Same size, another time.
    put(&t.at("a"), &[7u8; 100], 2_000_000);
    // Another size, same time.
    put(&t.at("b"), &[1u8; 90], 1_000_000);
    let rep = t.remove(&mut rec, &["a", "b"]).unwrap();
    assert_eq!(outcome(&rep, &t.s("a")), (Status::Skipped, Some("changed")));
    assert_eq!(outcome(&rep, &t.s("b")), (Status::Skipped, Some("changed")));
    assert_eq!(t.binned(), 0);
}

#[test]
fn a_file_swapped_for_one_with_the_same_stats_is_skipped() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    let new = t.at("new");
    put(&new, &[7u8; 100], 1_000_000);
    fs::rename(&new, t.at("a")).unwrap();
    let rep = t.remove(&mut rec, &["a"]).unwrap();
    assert_eq!(outcome(&rep, &t.s("a")), (Status::Skipped, Some("changed")));
}

#[test]
fn a_file_already_gone_is_reported() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    fs::remove_file(t.at("a")).unwrap();
    let rep = t.remove(&mut rec, &["a"]).unwrap();
    assert_eq!(outcome(&rep, &t.s("a")), (Status::Skipped, Some("gone")));
}

#[test]
fn no_removal_without_a_surviving_copy() {
    // The kept copy vanished.
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    fs::remove_file(t.at("b")).unwrap();
    let rep = t.remove(&mut rec, &["a"]).unwrap();
    assert_eq!(outcome(&rep, &t.s("a")), (Status::Skipped, Some("noKeptCopy")));
    assert!(t.at("a").exists());

    // Other size.
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    put(&t.at("b"), &[1u8; 50], 1_000_000);
    assert_eq!(outcome(&t.remove(&mut rec, &["a"]).unwrap(), &t.s("a")).1, Some("noKeptCopy"));

    // Same size, same time, other content: only reading it shows.
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    put(&t.at("b"), &[2u8; 100], 1_000_000);
    assert_eq!(outcome(&t.remove(&mut rec, &["a"]).unwrap(), &t.s("a")).1, Some("noKeptCopy"));
    assert!(t.at("a").exists());
}

#[cfg(unix)]
#[test]
fn a_kept_copy_replaced_by_a_link_does_not_count() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    fs::remove_file(t.at("b")).unwrap();
    std::os::unix::fs::symlink(t.at("a"), t.at("b")).unwrap();
    assert_eq!(outcome(&t.remove(&mut rec, &["a"]).unwrap(), &t.s("a")).1, Some("noKeptCopy"));
    assert!(t.at("a").exists());
}

#[cfg(unix)]
#[test]
fn a_kept_copy_replaced_by_a_hard_link_of_the_target_does_not_count() {
    let t = Tmp::new();
    t.put("a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    fs::remove_file(t.at("b")).unwrap();
    fs::hard_link(t.at("a"), t.at("b")).unwrap();
    // The new link moves the target's ctime, so it's caught before the kept copy is even looked at.
    assert_eq!(outcome(&t.remove(&mut rec, &["a"]).unwrap(), &t.s("a")).1, Some("changed"));
    assert!(t.at("a").exists());
}

#[test]
fn another_copy_saves_the_removal_when_the_first_kept_one_changed() {
    let t = Tmp::new();
    for n in ["a", "b", "c"] {
        t.put(n, &[1u8; 100]);
    }
    let mut rec = Record::of(&t.scan());
    put(&t.at("b"), &[2u8; 100], 1_000_000);
    let rep = t.remove(&mut rec, &["a"]).unwrap();
    assert_eq!(rep.trashed, [t.s("a")]);
}

#[cfg(unix)]
#[test]
fn a_parent_swapped_for_a_link_is_refused() {
    let t = Tmp::new();
    t.put("d/a", &[1u8; 100]);
    t.put("b", &[1u8; 100]);
    let mut rec = Record::of(&t.scan());
    let moved = t.base.join("moved");
    fs::rename(t.at("d"), &moved).unwrap();
    std::os::unix::fs::symlink(&moved, t.at("d")).unwrap();
    assert_eq!(outcome(&t.remove(&mut rec, &["d/a"]).unwrap(), &t.s("d/a")).1, Some("symlinkParent"));
    assert!(moved.join("a").exists());
}

#[test]
fn trash_failures_are_reported_and_the_rest_goes_on() {
    let t = Tmp::new();
    for n in ["a", "b", "c"] {
        t.put(n, &[1u8; 100]);
    }
    let mut rec = Record::of(&t.scan());
    let plan = t.plan(&["a", "b"]);
    let inner = t.trash();
    let bad = t.at("a");
    let trash = |p: &Path| if p == bad { Err("no permission".to_string()) } else { inner(p) };
    let (flag, beat) = (AtomicBool::new(false), || {});
    let rep = apply::run(&plan, &mut rec, &Job { cancel: &flag, beat: &beat }, &Env { trash: &trash, protection: &|_| Protection::Safe, rehash_limit: apply::REHASH_LIMIT }, &mut |_, _, _| {}).unwrap();
    assert_eq!(outcome(&rep, &t.s("a")), (Status::Failed, Some("error")));
    assert_eq!(rep.outcomes[0].detail.as_deref(), Some("no permission"));
    assert_eq!(rep.trashed, [t.s("b")]);
    assert_eq!(rep.bytes, 100);
}

#[test]
fn cancel_leaves_the_rest_in_place_and_says_so() {
    let t = Tmp::new();
    for n in ["a", "b", "c"] {
        t.put(n, &[1u8; 100]);
    }
    let mut rec = Record::of(&t.scan());
    let plan = t.plan(&["a", "b"]);
    let inner = t.trash();
    let flag = AtomicBool::new(false);
    let trash = |p: &Path| {
        flag.store(true, Ordering::Relaxed);
        inner(p)
    };
    let beat = || {};
    let rep = apply::run(&plan, &mut rec, &Job { cancel: &flag, beat: &beat }, &Env { trash: &trash, protection: &|_| Protection::Safe, rehash_limit: apply::REHASH_LIMIT }, &mut |_, _, _| {}).unwrap();
    assert_eq!(rep.trashed.len(), 1);
    assert_eq!(outcome(&rep, &t.s("b")), (Status::Cancelled, Some("cancelled")));
    assert!(t.at("b").exists());
}
