use super::*;
use read::clean_rel;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("morfa-arc-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}
fn job(cancelled: Arc<Mutex<HashSet<u64>>>) -> Job {
    Job { app: None, cancelled, id: 1, done: 0, total: 0, last: Instant::now() }
}
fn index(p: &Path) -> Index {
    let mut i = Index::default();
    walk(p, &mut i).unwrap();
    i
}
fn sample(dir: &Path) -> PathBuf {
    let src = dir.join("proj");
    fs::create_dir_all(src.join("sub/deep")).unwrap();
    fs::write(src.join("a.txt"), "alpha").unwrap();
    fs::write(src.join("sub/deep/b.txt"), "beta").unwrap();
    src
}

#[test]
fn clean_rel_blocks_escapes() {
    assert_eq!(clean_rel("../evil"), None);
    assert_eq!(clean_rel("a/../../evil"), None);
    assert_eq!(clean_rel("..\\evil"), None);
    assert_eq!(clean_rel("/etc/passwd").as_deref(), Some("etc/passwd"));
    assert_eq!(clean_rel("./a//b/").as_deref(), Some("a/b"));
    assert_eq!(clean_rel("./"), None);
}

#[test]
fn round_trip_every_writable_format() {
    for fmt in ["zip", "tar.gz"] {
        let d = tmp(&format!("rt-{}", fmt.replace('.', "")));
        let src = sample(&d);
        let out = d.join("out");
        fs::create_dir(&out).unwrap();
        let mut j = job(Default::default());
        let arc = pack::pack(&mut j, &[src.to_string_lossy().to_string()], &out, fmt).unwrap();
        assert!(arc.ends_with(&format!("proj.{fmt}")));
        // No staging leftovers.
        assert_eq!(fs::read_dir(&out).unwrap().count(), 1);
        let arc = PathBuf::from(arc);
        let idx = index(&arc);
        assert!(idx.dirs[""].iter().any(|i| i.is_dir && i.rel == "proj"));
        assert!(idx.dirs["proj/sub/deep"].iter().any(|i| i.name() == "b.txt" && i.size == 4));
        // Whole archive with a single top folder: no extra wrapper.
        let r = extract(&mut job(Default::default()), &idx, &arc, &[], "", &out, true).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(fs::read_to_string(out.join("proj/sub/deep/b.txt")).unwrap(), "beta");
        // Again: never overwrites.
        let r = extract(&mut job(Default::default()), &idx, &arc, &[], "", &out, true).unwrap();
        assert!(r[0].ends_with("proj (2)"), "{r:?}");
        // One nested file, relative to its folder.
        let pick = extract(&mut job(Default::default()), &idx, &arc, &["proj/sub/deep/b.txt".to_string()], "proj/sub/deep", &out, false).unwrap();
        assert_eq!(fs::read_to_string(&pick[0]).unwrap(), "beta");
        assert!(pick[0].ends_with("b.txt"));
        fs::remove_dir_all(&d).unwrap();
    }
}

#[test]
fn several_top_items_get_a_folder_named_after_the_archive() {
    let d = tmp("wrap");
    fs::write(d.join("x.txt"), "x").unwrap();
    fs::write(d.join("y.txt"), "y").unwrap();
    let sources = ["x.txt", "y.txt"].map(|n| d.join(n).to_string_lossy().to_string());
    let out = d.join("out");
    fs::create_dir(&out).unwrap();
    let arc = PathBuf::from(pack::pack(&mut job(Default::default()), &sources, &out, "tar.gz").unwrap());
    let r = extract(&mut job(Default::default()), &index(&arc), &arc, &[], "", &out, true).unwrap();
    assert_eq!(r, vec![out.join("out").to_string_lossy().to_string()]);
    assert!(out.join("out/x.txt").is_file());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn malicious_zip_stays_inside_and_links_are_skipped() {
    use zip::write::SimpleFileOptions as O;
    let d = tmp("slip");
    let arc = d.join("evil.zip");
    let mut z = zip::ZipWriter::new(fs::File::create(&arc).unwrap());
    z.start_file("../escaped.txt", O::default()).unwrap();
    z.write_all(b"no").unwrap();
    z.start_file("ok.txt", O::default()).unwrap();
    z.write_all(b"yes").unwrap();
    z.add_symlink("link", "/etc/passwd", O::default()).unwrap();
    z.finish().unwrap();
    let out = d.join("out");
    fs::create_dir(&out).unwrap();
    let r = extract(&mut job(Default::default()), &index(&arc), &arc, &[], "", &out, true).unwrap();
    assert_eq!(r.len(), 1);
    assert!(!d.join("escaped.txt").exists());
    assert!(out.join("ok.txt").is_file());
    assert!(fs::symlink_metadata(out.join("link")).is_err());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn cancelled_extract_leaves_nothing() {
    let d = tmp("cancel");
    let src = sample(&d);
    let out = d.join("out");
    fs::create_dir(&out).unwrap();
    let arc = PathBuf::from(pack::pack(&mut job(Default::default()), &[src.to_string_lossy().to_string()], &d, "zip").unwrap());
    let flags: Arc<Mutex<HashSet<u64>>> = Default::default();
    flags.lock().unwrap().insert(1);
    let idx = index(&arc);
    assert!(extract(&mut job(flags), &idx, &arc, &[], "", &out, true).unwrap_err().contains(CANCELLED));
    assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn tar_variants_can_be_read() {
    let d = tmp("tars");
    let mut plain = tar::Builder::new(Vec::new());
    let mut h = tar::Header::new_gnu();
    h.set_size(5);
    h.set_mode(0o644);
    h.set_cksum();
    plain.append_data(&mut h, "dir/f.txt", &b"hello"[..]).unwrap();
    let plain = plain.into_inner().unwrap();
    fs::write(d.join("a.tar"), &plain).unwrap();
    let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    bz.write_all(&plain).unwrap();
    fs::write(d.join("a.tar.bz2"), bz.finish().unwrap()).unwrap();
    let mut xz = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::default()).unwrap();
    xz.write_all(&plain).unwrap();
    fs::write(d.join("a.tar.xz"), xz.finish().unwrap()).unwrap();
    fs::write(d.join("a.tar.zst"), ruzstd::encoding::compress_to_vec(&plain[..], ruzstd::encoding::CompressionLevel::Fastest)).unwrap();
    for n in ["a.tar", "a.tar.bz2", "a.tar.xz", "a.tar.zst"] {
        let i = index(&d.join(n));
        assert!(i.dirs["dir"].iter().any(|x| x.name() == "f.txt" && x.size == 5), "{n}");
    }
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn sevenz_lists_and_extracts_one_file() {
    let d = tmp("7z");
    let arc = d.join("a.7z");
    let mut w = sevenz_rust2::ArchiveWriter::create(&arc).unwrap();
    for (name, body) in [("dir/one.txt", "first"), ("dir/two.txt", "second")] {
        w.push_archive_entry(sevenz_rust2::ArchiveEntry::new_file(name), Some(body.as_bytes())).unwrap();
    }
    w.finish().unwrap();
    let idx = index(&arc);
    assert_eq!(idx.dirs["dir"].len(), 2);
    let out = d.join("out");
    fs::create_dir(&out).unwrap();
    let r = extract(&mut job(Default::default()), &idx, &arc, &["dir/two.txt".to_string()], "dir", &out, false).unwrap();
    assert_eq!(fs::read_to_string(&r[0]).unwrap(), "second");
    assert_eq!(fs::read_dir(&out).unwrap().count(), 1);
    fs::remove_dir_all(&d).unwrap();
}

/// A tar.gz with `first.txt`, then a big incompressible entry, cut off well before its end.
fn tar_gz_with_cut_tail(d: &Path) -> (PathBuf, Index) {
    let noise: Vec<u8> = (0..4_000_000u32).scan(1u32, |s, _| { *s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223); Some((*s >> 24) as u8) }).collect();
    let mut gz = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
    for (name, data) in [("first.txt", &b"hello"[..]), ("noise.bin", &noise[..])] {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        gz.append_data(&mut h, name, data).unwrap();
    }
    let bytes = gz.into_inner().unwrap().finish().unwrap();
    let arc = d.join("a.tar.gz");
    fs::write(&arc, &bytes).unwrap();
    let idx = index(&arc);
    fs::write(&arc, &bytes[..bytes.len() / 2]).unwrap();
    (arc, idx)
}

#[test]
fn exact_file_selection_stops_before_a_corrupt_tail() {
    let d = tmp("early");
    let (arc, idx) = tar_gz_with_cut_tail(&d);
    let out = d.join("out");
    fs::create_dir(&out).unwrap();
    let r = extract(&mut job(Default::default()), &idx, &arc, &["first.txt".to_string()], "", &out, false).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(fs::read_to_string(out.join("first.txt")).unwrap(), "hello");
    // Everything has to scan on, so it does reach the damage.
    assert!(extract(&mut job(Default::default()), &idx, &arc, &[], "", &out, true).is_err());
    assert!(fs::read_dir(&out).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".morfa-extract")));
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn repeated_entry_and_folder_selections_scan_everything() {
    let d = tmp("repeat");
    let mut t = tar::Builder::new(Vec::new());
    for data in [&b"old"[..], &b"new"[..]] {
        let mut h = tar::Header::new_gnu();
        h.set_size(3);
        h.set_mode(0o644);
        h.set_cksum();
        t.append_data(&mut h, "dir/f.txt", data).unwrap();
    }
    let arc = d.join("a.tar");
    fs::write(&arc, t.into_inner().unwrap()).unwrap();
    let idx = index(&arc);
    assert!(idx.exact_files(&["dir/f.txt".to_string()]).is_none());
    assert!(idx.exact_files(&["dir".to_string()]).is_none());
    let root = d.join("cache");
    let p = open_copy(&mut job(Default::default()), &idx, &arc, "dir/f.txt", &root, None).unwrap();
    assert_eq!(fs::read_to_string(p).unwrap(), "new");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn cancelled_open_cleans_up_and_a_later_open_works() {
    let d = tmp("open-cancel");
    let (arc, idx) = tar_gz_with_cut_tail(&d);
    let root = d.join("cache");
    let jobs = Jobs::default();
    jobs.cancelled.lock().unwrap().insert(1);
    let e = with_job(None, &jobs, 1, |j| open_copy(j, &idx, &arc, "first.txt", &root, None)).unwrap_err();
    assert!(e.contains(CANCELLED), "{e}");
    assert_eq!(tree_size(&root), 0);
    for dir in fs::read_dir(&root).unwrap().flatten() {
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0, "staging left behind");
    }
    // The flag is gone with the job, and the same entry opens fine afterwards.
    let p = with_job(None, &jobs, 1, |j| open_copy(j, &idx, &arc, "first.txt", &root, None)).unwrap();
    assert_eq!(fs::read_to_string(p).unwrap(), "hello");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn open_of_a_link_is_a_clear_error() {
    let d = tmp("open-link");
    let mut t = tar::Builder::new(Vec::new());
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(tar::EntryType::Symlink);
    h.set_size(0);
    h.set_mode(0o777);
    t.append_link(&mut h, "l.txt", "/etc/hosts").unwrap();
    let arc = d.join("a.tar");
    fs::write(&arc, t.into_inner().unwrap()).unwrap();
    let e = open_copy(&mut job(Default::default()), &index(&arc), &arc, "l.txt", &d.join("cache"), None).unwrap_err();
    assert!(e.contains("link"), "{e}");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn trim_cache_drops_oldest_but_never_the_kept_one() {
    let d = tmp("trim");
    for (name, age) in [("old", 300), ("mid", 200), ("new", 100)] {
        fs::create_dir(d.join(name)).unwrap();
        fs::write(d.join(name).join("f"), [0u8; 100]).unwrap();
        let t = SystemTime::now() - Duration::from_secs(age);
        fs::File::open(d.join(name)).unwrap().set_modified(t).unwrap();
    }
    trim_cache(&d, &d.join("old"), 150);
    assert!(d.join("old").exists(), "kept even though oldest and over the cap");
    assert!(!d.join("mid").exists());
    assert!(!d.join("new").exists());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn preview_trim_leaves_the_open_root_alone() {
    let d = tmp("roots");
    let (arc, idx) = tar_gz_with_cut_tail(&d);
    let (opened, previews) = (d.join("archive-open"), d.join("archive-preview"));
    let kept = open_copy(&mut job(Default::default()), &idx, &arc, "first.txt", &opened, None).unwrap();
    fs::create_dir_all(previews.join("stale")).unwrap();
    fs::write(previews.join("stale/f"), [0u8; 100]).unwrap();
    open_copy(&mut job(Default::default()), &idx, &arc, "first.txt", &previews, Some(0)).unwrap();
    assert!(!previews.join("stale").exists());
    assert_eq!(fs::read_to_string(kept).unwrap(), "hello");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn trim_and_sweep_spare_in_flight_folders_but_not_crash_leftovers() {
    let d = tmp("inflight");
    let day_old = SystemTime::now() - Duration::from_secs(90_000);
    let live = format!(".morfa-extract-{}-7", std::process::id());
    for (name, staging) in [("live", live.as_str()), ("crashed", ".morfa-extract-0-7")] {
        fs::create_dir_all(d.join(name).join(staging)).unwrap();
        fs::write(d.join(name).join(staging).join("part"), [0u8; 100]).unwrap();
    }
    fs::create_dir(d.join("keep")).unwrap();
    trim_cache(&d, &d.join("keep"), 0);
    assert!(d.join("live").join(&live).exists());
    assert!(!d.join("crashed").exists());
    fs::File::open(d.join("live")).unwrap().set_modified(day_old).unwrap();
    sweep(&d);
    assert!(d.join("live").join(&live).exists());
    fs::remove_dir_all(d.join("live").join(&live)).unwrap();
    fs::File::open(d.join("live")).unwrap().set_modified(day_old).unwrap();
    sweep(&d);
    assert!(!d.join("live").exists());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn reuse_refreshes_the_folder_so_eviction_is_lru() {
    let d = tmp("lru");
    let (arc, idx) = tar_gz_with_cut_tail(&d);
    let root = d.join("cache");
    let p = PathBuf::from(open_copy(&mut job(Default::default()), &idx, &arc, "first.txt", &root, None).unwrap());
    let dir = p.parent().unwrap();
    let old = SystemTime::now() - Duration::from_secs(5000);
    fs::File::open(dir).unwrap().set_modified(old).unwrap();
    open_copy(&mut job(Default::default()), &idx, &arc, "first.txt", &root, None).unwrap();
    assert!(fs::metadata(dir).unwrap().modified().unwrap() > old + Duration::from_secs(4000));
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn cancel_is_accepted_only_while_running_and_never_lingers() {
    let jobs = Jobs::default();
    assert!(!jobs.cancel(5));
    assert!(jobs.cancelled.lock().unwrap().is_empty());
    let r = with_job(None, &jobs, 5, |j| {
        assert!(jobs.cancel(5));
        assert!(j.is_cancelled());
        Ok(())
    });
    assert!(r.is_ok(), "a cancel after the work finished doesn't hide its result");
    assert!(jobs.cancelled.lock().unwrap().is_empty() && jobs.running.lock().unwrap().is_empty());
}
