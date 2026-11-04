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
