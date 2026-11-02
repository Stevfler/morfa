use super::*;
use routes::{targets, Avail};
use std::{path::PathBuf, time::Instant};

fn opts(format: &str) -> ConvertOpts {
    ConvertOpts { format: format.into(), quality: 80, resize: None, strip_metadata: false, out_dir: None, name_template: "{name}".into() }
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("orbit-convert-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn names(d: &std::path::Path) -> Vec<String> {
    let mut v: Vec<_> = std::fs::read_dir(d).unwrap().map(|x| x.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn targets_intersect() {
    let img = s(&["jpg", "png", "webp", "avif"]);
    let all = Avail { image: &img, pdf: true, office: true, pandoc: false };
    assert_eq!(targets(&s(&["PNG"]), &all), s(&["webp", "jpg", "avif"]));
    assert_eq!(targets(&s(&["docx"]), &all), s(&["pdf", "odt", "rtf", "txt", "html", "epub"]));
    assert_eq!(targets(&s(&["docx", "xlsx"]), &all), s(&["pdf", "html"]));
    assert_eq!(targets(&s(&["pptx", "ods"]), &all), s(&["pdf"]));
    assert_eq!(targets(&s(&["pdf"]), &all), s(&["png", "jpg", "webp"]));
    assert_eq!(targets(&s(&["pdf", "jpeg"]), &all), s(&["png", "webp"]));
    assert!(targets(&s(&["md"]), &all).is_empty());
    let pd = Avail { pandoc: true, office: false, ..all };
    assert_eq!(targets(&s(&["markdown"]), &pd), s(&["docx", "odt", "html", "epub", "rst"]));
    assert!(targets(&s(&["xlsx"]), &pd).is_empty());
    assert!(targets(&[], &all).is_empty());
    assert_eq!(routes::missing("xlsx", "pdf"), "NO_ENGINE:office");
    assert_eq!(routes::missing("md", "docx"), "NO_ENGINE:pandoc");
}

#[test]
fn office_txt_to_pdf_odt_then_pdf_pages() {
    if doc::office().is_none() {
        return eprintln!("no soffice; skipping");
    }
    let d = tmpdir("office");
    let src = d.join("note.html");
    std::fs::write(&src, "<html><body><p>Page one</p><p style=\"page-break-before: always\">Page two</p></body></html>").unwrap();
    std::fs::write(d.join("note.pdf"), b"occupied").unwrap();
    let t = Instant::now();
    let r = dispatch::run_all(&[src.to_string_lossy().into()], &opts("pdf"), &|| false, &|_, _, _| {});
    eprintln!("LibreOffice html->pdf: {:?}", t.elapsed());
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    let pdf = d.join("note (converted).pdf");
    assert_eq!(PathBuf::from(&r.pairs[0].1), pdf);
    let t = Instant::now();
    let r = dispatch::run_all(&[src.to_string_lossy().into()], &opts("odt"), &|| false, &|_, _, _| {});
    eprintln!("LibreOffice html->odt: {:?}", t.elapsed());
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert!(d.join("note.odt").is_file());
    let r = dispatch::run_all(&[d.join("note.odt").to_string_lossy().into()], &opts("docx"), &|| false, &|_, _, _| {});
    assert!(r.failed.is_empty() && d.join("note.docx").is_file(), "{:?}", r.failed);
    if pdf::available() {
        assert_eq!(pdf::page_count(&pdf), Some(2));
        let r = dispatch::run_all(&[pdf.to_string_lossy().into()], &opts("png"), &|| false, &|_, _, _| {});
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        assert_eq!(r.pairs.len(), 2);
        assert!(d.join("note (converted) p1.png").is_file() && d.join("note (converted) p2.png").is_file());
    }
    // Cancelled before start: nothing new.
    let before = names(&d);
    let r = dispatch::run_all(&[src.to_string_lossy().into()], &opts("rtf"), &|| true, &|_, _, _| {});
    assert!(r.cancelled && r.pairs.is_empty());
    assert_eq!(names(&d), before);
    assert!(before.iter().all(|n| !n.starts_with(".orbit-convert")), "temp left: {before:?}");
    let _ = std::fs::remove_dir_all(&d);
}
