use super::*;
use std::path::{Path, PathBuf};

fn opts(format: &str, resize: Option<Resize>) -> ConvertOpts {
    ConvertOpts { format: format.into(), quality: 80, resize, strip_metadata: true, out_dir: None, name_template: "{name}".into() }
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("morfa-convert-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn naming_avoids_collisions() {
    let d = tmpdir("naming");
    let src = d.join("a.png");
    std::fs::write(&src, b"x").unwrap();
    // Same name and format as the source -> never the source itself.
    assert_eq!(naming::dest_for(&src, None, "{name}", "png"), d.join("a (converted).png"));
    assert_eq!(naming::dest_for(&src, None, "{name}", "webp"), d.join("a.webp"));
    std::fs::write(d.join("a.webp"), b"x").unwrap();
    std::fs::write(d.join("a (converted).webp"), b"x").unwrap();
    assert_eq!(naming::dest_for(&src, None, "{name}", "webp"), d.join("a (converted 2).webp"));
    assert_eq!(naming::dest_for(&src, Some("/x"), "small-{name}", "jpg"), Path::new("/x/small-a.jpg"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn converts_png_to_webp() {
    let Some(e) = engine::engine() else { return eprintln!("no engine; skipping") };
    assert!(e.formats.contains(&"webp".to_string()), "formats: {:?}", e.formats);
    let d = tmpdir("run");
    let src = d.join("pic.png");
    let made = match e.kind {
        engine::Kind::Vips => std::process::Command::new("vips").args(["black"]).arg(&src).args(["300", "200"]).status(),
        _ => std::process::Command::new("magick").args(["-size", "300x200", "xc:red"]).arg(&src).status(),
    };
    assert!(made.unwrap().success());
    std::fs::write(d.join("pic.webp"), b"occupied").unwrap();
    let s = vec![src.to_string_lossy().into_owned(), d.join("missing.png").to_string_lossy().into_owned()];
    let o = opts("webp", Some(Resize { mode: "max".into(), value: 100 }));
    let r = dispatch::run_all(&s, &o, &|| false, &|_, _, _| {});
    assert_eq!(r.pairs.len(), 1, "failed: {:?}", r.failed);
    assert_eq!(r.failed.len(), 1);
    assert!(!r.cancelled);
    let out = PathBuf::from(&r.pairs[0].1);
    assert_eq!(out, d.join("pic (converted).webp"));
    assert_eq!(std::fs::read(d.join("pic.webp")).unwrap(), b"occupied");
    let names: Vec<_> = std::fs::read_dir(&d).unwrap().map(|x| x.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(names.iter().all(|n| !n.starts_with(".morfa-convert")), "temp left: {names:?}");
    if e.kind == engine::Kind::Vips {
        let o = std::process::Command::new("vipsheader").args(["-f", "width"]).arg(&out).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "100");
    }
    let est = job::estimate(&src.to_string_lossy(), opts("jpg", Some(Resize { mode: "percent".into(), value: 50 }))).unwrap();
    assert!(est.before > 0 && est.after > 0);
    // Cancelled before start: nothing written.
    let r = dispatch::run_all(&s, &o, &|| true, &|_, _, _| {});
    assert!(r.cancelled && r.pairs.is_empty());
    let _ = std::fs::remove_dir_all(&d);
}
