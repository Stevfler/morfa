use super::{
    apply::apply,
    engine::{fits_watch_budget, Engine, Reason, Request},
    query::Query,
    store,
    tree::{self, Tree},
    walk::{self, Ctl, Rules},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{atomic::AtomicU64, RwLock},
    time::{Duration, Instant},
};

struct Td(PathBuf);
impl Td {
    fn new(tag: &str) -> Td {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!("morfa-findex-{tag}-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Td(std::fs::canonicalize(&d).unwrap())
    }
    fn file(&self, rel: &str) -> PathBuf {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
        p
    }
}
impl Drop for Td {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn rules(root: &Path, user: &[&str], mounts: Vec<PathBuf>) -> Rules {
    let user: Vec<String> = user.iter().map(|s| s.to_string()).collect();
    Rules::new(&user, None, &[root.to_path_buf()], mounts)
}
fn build(root: &Path, r: &Rules) -> Tree {
    let dev = walk::dev_of(&std::fs::metadata(root).unwrap());
    let seen = AtomicU64::new(0);
    walk::build(&[(root.to_path_buf(), dev)], r, &Ctl { stop: &|| false, paused: &|| false, seen: &seen, throttle: false }).unwrap()
}
fn names(t: &Tree, scope: &Path, q: &str, hidden: bool) -> Vec<String> {
    let scope = t.lookup(scope).unwrap();
    let hits = t.search(&Query::parse(q), scope, hidden, &|| false).unwrap();
    tree::rank(hits, 100).iter().map(|h| t.rel_path(h.id, scope).to_string_lossy().replace('\\', "/")).collect()
}
fn update(lock: &RwLock<Option<Tree>>, r: &Rules, paths: &[PathBuf]) {
    apply(lock, r, &paths.iter().cloned().collect::<BTreeSet<_>>(), &BTreeSet::new(), &|| false);
}
fn found(lock: &RwLock<Option<Tree>>, root: &Path, q: &str) -> Vec<String> {
    let g = lock.read().unwrap();
    names(g.as_ref().unwrap(), root, q, true)
}

#[test]
fn build_finds_names_and_scopes_to_the_folder() {
    let d = Td::new("build");
    d.file("a/report.pdf");
    d.file("b/report.txt");
    d.file("b/c/Report-Final.PDF");
    let t = build(&d.0, &rules(&d.0, &[], vec![]));
    assert_eq!(t.live_entries(), 6);
    let mut all = names(&t, &d.0, "report *.pdf", false);
    all.sort();
    assert_eq!(all, ["a/report.pdf", "b/c/Report-Final.PDF"]);
    assert_eq!(names(&t, &d.0.join("b"), "report", false).len(), 2);
    assert_eq!(names(&t, &d.0, "*.txt", false), ["b/report.txt"]);
    assert_eq!(names(&t, &d.0.join("a"), "*.txt", false), Vec::<String>::new());
    assert!(names(&t, &d.0.join("b"), "b", false).is_empty(), "the folder itself isn't a result");
}

#[test]
fn hidden_follows_the_setting_at_query_time() {
    let d = Td::new("hidden");
    d.file(".secret/note.txt");
    d.file("seen/.note.txt");
    d.file("seen/note.txt");
    let t = build(&d.0, &rules(&d.0, &[], vec![]));
    assert_eq!(names(&t, &d.0, "note", false), ["seen/note.txt"]);
    assert_eq!(names(&t, &d.0, "note", true).len(), 3);
    assert_eq!(names(&t, &d.0.join(".secret"), "note", false), ["note.txt"], "searching from inside a hidden folder");
}

#[test]
fn excludes_builtin_and_user() {
    let d = Td::new("excl");
    d.file("node_modules/pkg/keep.js");
    d.file("x/.git/config-keep");
    d.file(".cache/keep");
    d.file("x/keep.txt");
    d.file("private/keep.txt");
    d.file("x/Thumbs/keep.bin");
    let private = d.0.join("private");
    let r = rules(&d.0, &["Thumbs", private.to_str().unwrap()], vec![]);
    let t = build(&d.0, &r);
    assert_eq!(names(&t, &d.0, "keep", true), ["x/keep.txt"]);
    assert!(t.lookup(&d.0.join("node_modules")).is_none(), "excluded folders count as not covered");
    assert!(t.lookup(&private).is_none());
    // Home-relative rules: secrets stay out wherever home is.
    let home = Td::new("excl-home");
    home.file(".ssh/id_keep");
    home.file("doc/keep");
    let r = Rules::new(&[], Some(&home.0), std::slice::from_ref(&home.0), vec![]);
    assert_eq!(names(&build(&home.0, &r), &home.0, "keep", true), ["doc/keep"]);
}

#[test]
fn does_not_cross_mounts_unless_root_is_on_one() {
    let d = Td::new("mount");
    d.file("disk/inner.txt");
    d.file("here.txt");
    let disk = d.0.join("disk");
    let r = rules(&d.0, &[], vec![disk.clone()]);
    let t = build(&d.0, &r);
    assert!(t.lookup(&disk).is_none());
    assert!(r.crosses(&disk, &d.0, 7, 7), "listed in the mount table");
    assert!(r.crosses(&d.0.join("y"), &d.0, 8, 7), "another device");
    assert!(!r.crosses(&d.0.join("y"), &d.0, 7, 7));
    // Events from inside don't get it in either.
    let lock = RwLock::new(Some(t));
    update(&lock, &r, &[disk.join("inner.txt"), disk.clone()]);
    assert!(found(&lock, &d.0, "inner").is_empty());
    // The mount as a root is walked.
    let r2 = rules(&disk, &[], vec![disk.clone()]);
    assert_eq!(names(&build(&disk, &r2), &disk, "inner", true), ["inner.txt"]);
}

#[cfg(unix)]
#[test]
fn symlinks_are_listed_not_followed() {
    let d = Td::new("link");
    let out = Td::new("link-out");
    out.file("beyond.txt");
    std::os::unix::fs::symlink(&out.0, d.0.join("away")).unwrap();
    let t = build(&d.0, &rules(&d.0, &[], vec![]));
    assert_eq!(names(&t, &d.0, "away", true), ["away"]);
    assert!(names(&t, &d.0, "beyond", true).is_empty());
    assert!(!t.is_dir(t.lookup(&d.0.join("away")).unwrap()));
}

#[test]
fn incremental_create_modify_delete() {
    let d = Td::new("incr");
    d.file("a/old.txt");
    let r = rules(&d.0, &[], vec![]);
    let lock = RwLock::new(Some(build(&d.0, &r)));
    // A file, a nested tree whose parents nobody told us about, a file in an excluded place.
    let f = d.file("a/new.txt");
    let deep = d.file("b/c/d/deep.txt");
    let nm = d.file("a/node_modules/skip.txt");
    update(&lock, &r, &[f, deep, nm]);
    assert_eq!(found(&lock, &d.0, "new"), ["a/new.txt"]);
    assert_eq!(found(&lock, &d.0, "deep"), ["b/c/d/deep.txt"]);
    assert!(found(&lock, &d.0, "skip").is_empty());
    // Delete a file, then a whole folder, reporting only the folder.
    std::fs::remove_file(d.0.join("a/old.txt")).unwrap();
    std::fs::remove_dir_all(d.0.join("b")).unwrap();
    update(&lock, &r, &[d.0.join("a/old.txt"), d.0.join("b")]);
    assert!(found(&lock, &d.0, "old").is_empty());
    assert!(found(&lock, &d.0, "deep").is_empty());
    let g = lock.read().unwrap();
    assert!(g.as_ref().unwrap().lookup(&d.0.join("b")).is_none());
}

#[test]
fn rename_file_and_folder_both_ways() {
    let d = Td::new("rename");
    d.file("dir/sub/inside.txt");
    d.file("one.txt");
    let r = rules(&d.0, &[], vec![]);
    let lock = RwLock::new(Some(build(&d.0, &r)));
    std::fs::rename(d.0.join("one.txt"), d.0.join("two.txt")).unwrap();
    std::fs::rename(d.0.join("dir"), d.0.join("moved")).unwrap();
    update(&lock, &r, &[d.0.join("one.txt"), d.0.join("two.txt"), d.0.join("dir"), d.0.join("moved")]);
    assert_eq!(found(&lock, &d.0, "two"), ["two.txt"]);
    assert!(found(&lock, &d.0, "one").is_empty());
    assert_eq!(found(&lock, &d.0, "inside"), ["moved/sub/inside.txt"]);
    // Only the parent reported (what some platforms' coalesced events amount to).
    std::fs::rename(d.0.join("two.txt"), d.0.join("three.txt")).unwrap();
    update(&lock, &r, std::slice::from_ref(&d.0));
    assert_eq!(found(&lock, &d.0, "three"), ["three.txt"]);
    assert!(found(&lock, &d.0, "two").is_empty());
    // A file replaced by a folder of the same name.
    std::fs::remove_file(d.0.join("three.txt")).unwrap();
    d.file("three.txt/kid.txt");
    update(&lock, &r, &[d.0.join("three.txt")]);
    assert_eq!(found(&lock, &d.0, "kid"), ["three.txt/kid.txt"]);
}

#[test]
fn a_crowd_of_changes_in_one_folder_is_listed_once() {
    let d = Td::new("crowd");
    d.file("big/seed.txt");
    let r = rules(&d.0, &[], vec![]);
    let lock = RwLock::new(Some(build(&d.0, &r)));
    let paths: Vec<PathBuf> = (0..500).map(|i| d.file(&format!("big/f{i}.txt"))).collect();
    update(&lock, &r, &paths);
    assert_eq!(found(&lock, &d.0, "*.txt").len(), 100, "capped by the helper");
    assert_eq!(lock.read().unwrap().as_ref().unwrap().live_entries(), 502);
}

#[test]
fn swapped_folders_are_walked_again_below_the_same_names() {
    let d = Td::new("swap");
    d.file("a/sub/from_a.txt");
    d.file("b/sub/from_b.txt");
    let r = rules(&d.0, &[], vec![]);
    let lock = RwLock::new(Some(build(&d.0, &r)));
    std::fs::rename(d.0.join("a"), d.0.join("t")).unwrap();
    std::fs::rename(d.0.join("b"), d.0.join("a")).unwrap();
    std::fs::rename(d.0.join("t"), d.0.join("b")).unwrap();
    let paths: BTreeSet<PathBuf> = ["a", "b", "t"].iter().map(|n| d.0.join(n)).collect();
    apply(&lock, &r, &paths, &paths, &|| false);
    assert_eq!(found(&lock, &d.0, "from_a"), ["b/sub/from_a.txt"]);
    assert_eq!(found(&lock, &d.0, "from_b"), ["a/sub/from_b.txt"]);
    // The same inside a folder crowded enough to be listed rather than looked at one by one.
    let many: Vec<PathBuf> = (0..80).map(|i| d.file(&format!("big/f{i}"))).collect();
    d.file("big/x/sub/one.txt");
    let lock = RwLock::new(Some(build(&d.0, &r)));
    std::fs::remove_file(d.0.join("big/x/sub/one.txt")).unwrap();
    d.file("big/x/sub/two.txt");
    let mut paths: BTreeSet<PathBuf> = many.into_iter().collect();
    paths.insert(d.0.join("big/x"));
    apply(&lock, &r, &paths, &paths.clone(), &|| false);
    assert_eq!(found(&lock, &d.0, "two"), ["big/x/sub/two.txt"]);
    assert!(found(&lock, &d.0, "one.txt").is_empty());
}

#[test]
fn key_folders_are_out_by_name_at_any_depth() {
    let d = Td::new("keys");
    for n in [".ssh", ".gnupg", ".aws", ".kube", ".password-store", "Keychains"] {
        d.file(&format!("backup/old/{n}/keep-out"));
    }
    d.file("backup/fine");
    let t = build(&d.0, &rules(&d.0, &[], vec![]));
    assert!(names(&t, &d.0, "keep-out", true).is_empty());
    assert_eq!(names(&t, &d.0, "fine", true).len(), 1);
    let home = Td::new("keys-home");
    home.file("Library/Containers/app/keep-out");
    home.file("Library/Group Containers/app/keep-out");
    let r = Rules::new(&[], Some(&home.0), std::slice::from_ref(&home.0), vec![]);
    assert!(names(&build(&home.0, &r), &home.0, "keep-out", true).is_empty());
}

#[test]
fn compaction_keeps_what_lives() {
    let d = Td::new("compact");
    d.file("a/keep.txt");
    d.file("gone/x.txt");
    let mut t = build(&d.0, &rules(&d.0, &[], vec![]));
    t.remove(t.lookup(&d.0.join("gone")).unwrap());
    let before = t.slots();
    t.compact();
    assert!(t.slots() < before);
    assert_eq!(names(&t, &d.0, "keep", true), ["a/keep.txt"]);
    assert!(names(&t, &d.0, "x.txt", true).is_empty());
}

#[test]
fn ranking_name_match_then_recency() {
    let d = Td::new("rank");
    let mut t = Tree::new(vec![(d.0.clone(), 0)]);
    t.add(0, b"keynote old.txt", 10, 0);
    t.add(0, b"my-note.txt", 10, 0);
    t.add(0, b"notebook.txt", 10, 0);
    t.add(0, b"note.txt", 10, 0);
    t.add(0, b"keynote new.txt", 99, 0);
    assert_eq!(names(&t, &d.0, "note", true), ["note.txt", "notebook.txt", "my-note.txt", "keynote new.txt", "keynote old.txt"]);
    let hits = t.search(&Query::parse("note"), 0, true, &|| false).unwrap();
    assert_eq!(hits.len(), 5);
    assert_eq!(tree::rank(hits, 2).len(), 2, "capped; the caller still has the full count");
    assert!(t.search(&Query::parse("note"), 0, true, &|| true).is_some_and(|h| h.len() == 5), "short scans finish before the first check");
}

#[test]
fn save_load_round_trip_and_private_file() {
    let d = Td::new("store");
    let cache = Td::new("store-cache");
    d.file("Dir/Файл.TXT");
    d.file("plain.txt");
    let mut t = build(&d.0, &rules(&d.0, &[], vec![]));
    let gone = d.file("late.txt");
    let id = t.add(0, b"late.txt", 1, 0);
    t.remove(id);
    drop(gone);
    let spec = store::spec_hash(std::slice::from_ref(&d.0), &[]);
    let size = store::save(&t, &spec, &cache.0).unwrap();
    assert_eq!(size, std::fs::metadata(store::file_in(&cache.0)).unwrap().len());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(store::file_in(&cache.0)).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&cache.0).unwrap().permissions().mode() & 0o777, 0o700);
    }
    let back = store::load(&cache.0, &spec).unwrap();
    assert_eq!(back.live_entries(), t.live_entries());
    assert_eq!(names(&back, &d.0, "файл", true), ["Dir/Файл.TXT"], "original case kept, folded search");
    assert!(!cache.0.join("names.idx.tmp").exists());
    let other = store::spec_hash(std::slice::from_ref(&d.0), &["x".into()]);
    assert!(store::load(&cache.0, &other).is_err(), "built from other settings");
}

#[test]
fn corrupt_files_are_refused_without_panicking() {
    let d = Td::new("corrupt");
    let cache = Td::new("corrupt-cache");
    d.file("a/b.txt");
    let t = build(&d.0, &rules(&d.0, &[], vec![]));
    let spec = store::spec_hash(std::slice::from_ref(&d.0), &[]);
    store::save(&t, &spec, &cache.0).unwrap();
    let good = std::fs::read(store::file_in(&cache.0)).unwrap();
    assert!(store::parse(&good, &spec).is_ok());
    for len in 0..good.len() {
        assert!(store::parse(&good[..len], &spec).is_err(), "truncated at {len}");
    }
    for i in 0..good.len() {
        let mut bad = good.clone();
        bad[i] ^= 0x41;
        assert!(store::parse(&bad, &spec).is_err(), "flip at {i}");
    }
    assert!(store::parse(b"garbage", &spec).is_err());
    // Dangerous names with a valid checksum: Tree::add takes them, the reader must not.
    for bad in [&b"."[..], b"..", b"a/b"] {
        let mut t = Tree::new(vec![(d.0.clone(), 0)]);
        t.add(0, bad, 1, 0);
        store::save(&t, &spec, &cache.0).unwrap();
        assert!(store::load(&cache.0, &spec).is_err(), "{bad:?}");
    }
    // With a checksum that's valid over bad contents too: structure is checked on its own.
    let mut forged = good[..good.len() - 32].to_vec();
    let at = forged.len() - 1;
    forged[at] = 0;
    use sha2::{Digest, Sha256};
    let sum = Sha256::digest(&forged);
    forged.extend_from_slice(&sum);
    assert!(store::parse(&forged, &spec).is_err());
}

#[cfg(unix)]
#[test]
fn non_utf8_names_survive_and_stat() {
    use std::os::unix::ffi::OsStrExt;
    let d = Td::new("bytes");
    let name = std::ffi::OsStr::from_bytes(b"caf\xE9-note.txt");
    if std::fs::write(d.0.join(name), b"x").is_err() {
        return; // APFS refuses invalid UTF-8; the rest is exercised on Linux
    }
    let t = build(&d.0, &rules(&d.0, &[], vec![]));
    let id = t.lookup(&d.0.join(name)).unwrap();
    assert!(d.0.join(t.rel_path(id, 0)).exists());
    assert_eq!(names(&t, &d.0, "note", true).len(), 1);
}

#[test]
fn raw_bytes_round_trip_without_the_filesystem() {
    let d = Td::new("rawbytes");
    let cache = Td::new("rawbytes-cache");
    let mut t = Tree::new(vec![(d.0.clone(), 0)]);
    if cfg!(unix) {
        t.add(0, b"caf\xE9-Note.txt", 5, 0);
        let spec = store::spec_hash(std::slice::from_ref(&d.0), &[]);
        store::save(&t, &spec, &cache.0).unwrap();
        let back = store::load(&cache.0, &spec).unwrap();
        assert_eq!(back.raw_name(1), b"caf\xE9-Note.txt");
        assert_eq!(back.search(&Query::parse("note"), 0, true, &|| false).unwrap().len(), 1);
    }
}

#[test]
fn watch_budget_leaves_room() {
    assert!(fits_watch_budget(10, None));
    assert!(fits_watch_budget(4096, Some(8192)));
    assert!(!fits_watch_budget(4097, Some(8192)));
}

fn engine(cache: &Path) -> std::sync::Arc<Engine> {
    Engine::start(cache.to_path_buf(), vec![], Box::new(|_| {}))
}
fn wait(what: &str, secs: u64, f: impl Fn() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(secs), "timed out: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn request(root: &Path) -> Option<Request> {
    Some(Request { roots: vec![root.to_string_lossy().into_owned()], excludes: vec![] })
}
fn settled(e: &Engine) -> bool {
    let s = e.status();
    s.state == "ready" && !s.refreshing
}

#[test]
fn fallback_reasons_and_lifecycle() {
    let d = Td::new("life");
    let cache = Td::new("life-cache");
    let store_dir = cache.0.join("idx");
    d.file("a/hello.txt");
    d.file("node_modules/x.js");
    let e = engine(&store_dir);
    assert_eq!(e.ready(), Err(Reason::Off), "off by default");
    assert!(!store_dir.exists(), "nothing written while off");
    e.configure(request(&d.0));
    wait("first build", 20, || settled(&e));
    assert!(store::file_in(&store_dir).exists());
    e.covers(&d.0).unwrap();
    e.covers(&d.0.join("a")).unwrap();
    assert_eq!(e.covers(&d.0.join("node_modules")), Err(Reason::NotCovered), "excluded");
    assert_eq!(e.covers(Path::new("/definitely/elsewhere")), Err(Reason::NotCovered));
    let f = e.query(&d.0, Path::new("/shown/here"), "hello", false, 10, &|| false).unwrap().unwrap();
    assert_eq!(f.total, 1);
    assert_eq!(f.paths[0].0, Path::new("/shown/here/a/hello.txt"), "rows sit under the pane's own path");
    assert_eq!(f.paths[0].1, d.0.join("a/hello.txt"));
    // Disabling deletes.
    e.configure(None);
    wait("disable", 10, || e.status().state == "off");
    assert_eq!(e.ready(), Err(Reason::Off));
    assert!(!store::file_in(&store_dir).exists());
    assert!(e.query(&d.0, &d.0, "hello", false, 10, &|| false).is_err());
    // On again, then the delete button.
    e.configure(request(&d.0));
    wait("second build", 20, || settled(&e));
    e.delete_now();
    assert_eq!(e.ready(), Err(Reason::Off));
    assert!(!store_dir.exists());
    e.quit();
}

#[test]
fn corrupt_file_on_start_rebuilds_silently() {
    let d = Td::new("restart");
    let cache = Td::new("restart-cache");
    d.file("hello.txt");
    std::fs::write(store::file_in(&cache.0), b"not an index").unwrap();
    let e = engine(&cache.0);
    e.configure(request(&d.0));
    wait("rebuild", 20, || settled(&e));
    assert_eq!(e.query(&d.0, &d.0, "hello", false, 10, &|| false).unwrap().unwrap().total, 1);
    assert!(std::fs::read(store::file_in(&cache.0)).unwrap().starts_with(b"MFIX"));
    e.quit();
}

#[test]
fn cancel_leaves_no_index_and_search_falls_back() {
    let d = Td::new("cancel");
    let cache = Td::new("cancel-cache");
    for i in 0..3000 {
        d.file(&format!("d{}/f{i}", i % 300));
    }
    let e = engine(&cache.0);
    e.configure(request(&d.0));
    wait("walk started", 10, || e.status().state == "building");
    e.pause(true);
    assert!(e.status().paused);
    e.cancel();
    wait("cancelled", 10, || e.status().state == "cancelled");
    assert_eq!(e.ready(), Err(Reason::Off));
    assert!(!store::file_in(&cache.0).exists());
    e.rebuild();
    wait("rebuilt", 30, || settled(&e));
    assert!(e.status().entries >= 3300);
    e.quit();
}

#[test]
fn own_cache_is_never_indexed_even_inside_a_root() {
    let d = Td::new("own");
    d.file("cache/file-index/own-secret");
    d.file("doc/mine");
    let e = Engine::start(d.0.join("cache/file-index"), vec![d.0.join("cache")], Box::new(|_| {}));
    e.configure(request(&d.0));
    wait("build", 20, || settled(&e));
    assert_eq!(e.query(&d.0, &d.0, "mine", false, 10, &|| false).unwrap().unwrap().total, 1);
    assert_eq!(e.query(&d.0, &d.0, "own-secret", false, 10, &|| false).unwrap().unwrap().total, 0);
    assert_eq!(e.covers(&d.0.join("cache")), Err(Reason::NotCovered));
    e.quit();
}

#[test]
fn changing_excludes_deletes_the_old_file_first() {
    let d = Td::new("chg");
    let cache = Td::new("chg-cache");
    d.file("secretdir/x");
    let has = |n: &[u8]| std::fs::read(store::file_in(&cache.0)).is_ok_and(|b| b.windows(n.len()).any(|w| w == n));
    let e = engine(&cache.0);
    e.configure(request(&d.0));
    wait("build", 20, || settled(&e));
    assert!(has(b"secretdir"));
    e.configure(Some(Request { roots: vec![d.0.to_string_lossy().into_owned()], excludes: vec!["secretdir".into()] }));
    wait("old names gone from disk", 20, || !has(b"secretdir"));
    wait("rebuilt", 20, || settled(&e) && e.covers(&d.0.join("secretdir")).is_err());
    assert!(!has(b"secretdir"));
    e.quit();
}

#[test]
fn delete_and_disable_work_while_the_worker_is_busy_elsewhere() {
    let d = Td::new("erase");
    let cache = Td::new("erase-cache");
    d.file("x");
    let e = engine(&cache.0);
    e.configure(request(&d.0));
    wait("build", 20, || settled(&e));
    e.configure(None);
    // Synchronously: no waiting for the worker.
    assert!(!store::file_in(&cache.0).exists());
    assert_eq!(e.ready(), Err(Reason::Off));
    e.quit();
}

#[test]
fn live_changes_arrive_through_the_watcher() {
    let d = Td::new("live");
    let cache = Td::new("live-cache");
    d.file("seed.txt");
    let e = engine(&cache.0);
    e.configure(request(&d.0));
    wait("build", 20, || settled(&e));
    // FSEvents can take a moment to start delivering.
    std::thread::sleep(Duration::from_millis(500));
    let total = |q: &str| e.query(&d.0, &d.0, q, false, 10, &|| false).unwrap().unwrap().total;
    d.file("arrived.txt");
    wait("create seen", 30, || total("arrived") == 1);
    std::fs::rename(d.0.join("arrived.txt"), d.0.join("renamed.txt")).unwrap();
    wait("rename seen", 30, || total("renamed") == 1 && total("arrived") == 0);
    std::fs::remove_file(d.0.join("renamed.txt")).unwrap();
    wait("delete seen", 30, || total("renamed") == 0);
    e.quit();
}

/// Read-only measurement of a real tree; nothing outside `MEASURE_DIR` (default temp) is written, and
/// the file is removed. `cargo test --release findex_measure -- --ignored --nocapture`
#[cfg(unix)]
#[test]
#[ignore]
fn findex_measure() {
    let home = dirs::home_dir().unwrap();
    let real = std::fs::canonicalize(&home).unwrap();
    let dir = Td::new("measure");
    let mounts = crate::netfs::table().into_iter().map(|m| PathBuf::from(m.path));
    let r = Rules::new(&[], Some(&real), std::slice::from_ref(&real), mounts);
    let rss = || {
        let mut u = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: an out-pointer of the right type, read only after the call fills it.
        unsafe {
            libc::getrusage(libc::RUSAGE_SELF, u.as_mut_ptr());
            u.assume_init().ru_maxrss as u64
        }
    };
    let unit = if cfg!(target_os = "macos") { 1 << 20 } else { 1 << 10 };
    let throttle = std::env::var("MEASURE_THROTTLE").is_ok();
    if throttle {
        walk::lower_thread_priority();
    }
    let seen = AtomicU64::new(0);
    let before = rss();
    let t = Instant::now();
    let dev = walk::dev_of(&std::fs::metadata(&real).unwrap());
    let tree = walk::build(&[(real.clone(), dev)], &r, &Ctl { stop: &|| false, paused: &|| false, seen: &seen, throttle }).unwrap();
    println!("walk: {} entries, {:?} (throttle+bg priority: {throttle})", tree.live_entries(), t.elapsed());
    println!("index heap: {} MB; peak process RSS {} MB (before {} MB)", tree.heap_bytes() >> 20, rss() / unit, before / unit);
    let spec = store::spec_hash(std::slice::from_ref(&real), &[]);
    let t = Instant::now();
    let size = store::save(&tree, &spec, &dir.0).unwrap();
    println!("save: {} MB on disk, {:?}", size >> 20, t.elapsed());
    let t = Instant::now();
    let back = store::load(&dir.0, &spec).unwrap();
    println!("load: {} entries, {:?}", back.live_entries(), t.elapsed());
    drop(back);
    for q in ["a", "report", "*.pdf", "invoice 2024", "zzzzqqq", "e *.txt"] {
        let t = Instant::now();
        let parsed = Query::parse(q);
        let hits = tree.search(&parsed, 0, false, &|| false).unwrap();
        let n = hits.len();
        let m = t.elapsed();
        let top = tree::rank(hits, 5000);
        println!("query {q:?}: {n} hits, match {m:?}, +rank {:?}", t.elapsed());
        drop(top);
    }
    // Reference: the current parallel live walk, single query, no excludes.
    let t = Instant::now();
    let c = AtomicU64::new(0);
    ignore::WalkBuilder::new(&real).hidden(true).git_ignore(false).parents(false).threads(crate::search::num_threads()).build_parallel().run(|| {
        let c = &c;
        Box::new(move |_| {
            c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            ignore::WalkState::Continue
        })
    });
    println!("live walk for comparison: {} entries in {:?}", c.load(std::sync::atomic::Ordering::Relaxed), t.elapsed());
    store::delete(&dir.0);
}
