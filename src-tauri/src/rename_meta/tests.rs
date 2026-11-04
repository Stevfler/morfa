use super::{
    check::{check_rows, Row, RowCheck},
    read::{parse_audio, parse_image, read_meta, Counted, FileMeta, AUDIO_BUDGET},
    sanitize::{camera_name, exif_date, sanitize_value, MAX_VALUE_BYTES},
};
use crate::{cloud::sync::Fold, fsops::name_issue};
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

static NO: AtomicBool = AtomicBool::new(false);
fn nop() {}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("morfa-rmeta-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

// ---------- fixtures, built byte by byte so that nothing else is needed to produce them

/// A little-endian TIFF block: IFD0 with Make, Model, Orientation and a pointer to the Exif IFD.
fn tiff(make: &str, model: &str, orientation: u16, taken: &str) -> Vec<u8> {
    let z = |s: &str| {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        v
    };
    let (make, model, taken) = (z(make), z(model), z(taken));
    let ifd0 = 8usize;
    let ifd0_len = 2 + 4 * 12 + 4;
    let exif_at = ifd0 + ifd0_len;
    let exif_len = 2 + 12 + 4;
    let data = exif_at + exif_len;
    let (make_at, model_at, taken_at) = (data, data + make.len(), data + make.len() + model.len());
    let mut o = b"II*\0".to_vec();
    o.extend(8u32.to_le_bytes());
    let entry = |o: &mut Vec<u8>, tag: u16, ty: u16, n: u32, v: u32| {
        o.extend(tag.to_le_bytes());
        o.extend(ty.to_le_bytes());
        o.extend(n.to_le_bytes());
        o.extend(v.to_le_bytes());
    };
    o.extend(4u16.to_le_bytes());
    entry(&mut o, 0x010F, 2, make.len() as u32, make_at as u32);
    entry(&mut o, 0x0110, 2, model.len() as u32, model_at as u32);
    entry(&mut o, 0x0112, 3, 1, u32::from(orientation));
    entry(&mut o, 0x8769, 4, 1, exif_at as u32);
    o.extend(0u32.to_le_bytes());
    o.extend(1u16.to_le_bytes());
    entry(&mut o, 0x9003, 2, taken.len() as u32, taken_at as u32);
    o.extend(0u32.to_le_bytes());
    o.extend(make);
    o.extend(model);
    o.extend(taken);
    o
}

/// The start of a JPEG: APP1 with Exif, then a frame header giving the size.
fn jpeg(t: &[u8], w: u16, h: u16) -> Vec<u8> {
    let mut o = vec![0xFF, 0xD8, 0xFF, 0xE1];
    o.extend(((t.len() + 8) as u16).to_be_bytes());
    o.extend(b"Exif\0\0");
    o.extend(t);
    o.extend([0xFF, 0xC0, 0, 11, 8]);
    o.extend(h.to_be_bytes());
    o.extend(w.to_be_bytes());
    o.extend([1, 1, 0x11, 0]);
    o.extend([0xFF, 0xD9]);
    o
}

/// ID3v2.3 tag with Latin-1 text frames, followed by one MPEG frame header's worth of audio.
fn mp3(frames: &[(&str, &str)]) -> Vec<u8> {
    let mut body = vec![];
    for (id, text) in frames {
        body.extend(id.as_bytes());
        body.extend(((text.len() + 1) as u32).to_be_bytes());
        body.extend([0, 0, 0]);
        body.extend(text.as_bytes());
    }
    let n = body.len() as u32;
    let mut o = b"ID3\x03\0\0".to_vec();
    o.extend([(n >> 21) as u8 & 0x7F, (n >> 14) as u8 & 0x7F, (n >> 7) as u8 & 0x7F, n as u8 & 0x7F]);
    o.extend(body);
    for _ in 0..4 {
        o.extend([0xFF, 0xFB, 0x90, 0x00]);
        o.extend([0u8; 413]);
    }
    o
}

fn audio_of(bytes: &[u8], name: &str) -> FileMeta {
    let mut m = FileMeta::default();
    parse_audio(Counted::new(Cursor::new(bytes), AUDIO_BUDGET, &NO, &nop), Path::new(name), &mut m);
    m
}

/// Same every run.
fn noise(n: usize, mut seed: u32) -> Vec<u8> {
    (0..n)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 24) as u8
        })
        .collect()
}

// ---------- sanitising

#[test]
fn values_cant_leave_the_folder_or_break_a_name() {
    let cases = [
        "../../etc/passwd", "..\\..\\windows\\system32", "/abs/path", "C:\\x", "a/b", "a\\b", "a\0b", "a\nb\tc", "x\u{202E}gpj.exe",
        "CON", "con.txt", "nul", "COM1", "LPT¹", "name.", "name   ", "name...", " .. name .. ", "<>:\"|?*x", "a..b", "a／b", "a∕b",
    ];
    for raw in cases {
        let v = sanitize_value(raw).unwrap_or_else(|| panic!("{raw:?} left nothing"));
        assert_eq!(name_issue(&v), None, "{raw:?} became {v:?}");
        assert!(!v.contains(".."), "{v:?}");
        assert!(!v.starts_with('.') && !v.ends_with('.') && !v.ends_with(' '), "{v:?}");
        assert!(!v.chars().any(|c| c.is_control() || matches!(c, '\u{202E}' | '/' | '\\' | '／' | '∕')), "{v:?}");
        // The file name it goes into stays one component in this folder.
        let name = format!("{v}.jpg");
        assert_eq!(Path::new("/d").join(&name).parent(), Some(Path::new("/d")), "{name:?}");
    }
}

#[test]
fn values_with_nothing_in_them_are_missing() {
    for raw in ["", "   ", "..", "...", "/", "\\\\", "\0\0", "???", "- -", "___", ". .", "\u{202E}", "<>"] {
        assert_eq!(sanitize_value(raw), None, "{raw:?}");
    }
    for raw in ["\u{200B}", "\u{200C}\u{200D}", "\u{AD}", "\u{3164}", "\u{FE0F}", "\u{301}", "\u{301}\u{302}", "\u{FFFD}", "\u{61C}", "\u{E0001}", "\u{200B}.\u{FE0F}", "★"] {
        assert_eq!(sanitize_value(raw), None, "{raw:?}");
    }
    assert_eq!(sanitize_value("a\u{200B}b\u{AD}c\u{FFFD}").as_deref(), Some("abc"));
    assert_eq!(sanitize_value("Кино").as_deref(), Some("Кино"));
}

#[test]
fn values_are_normalised_collapsed_and_limited() {
    assert_eq!(sanitize_value("Cafe\u{301}").as_deref(), Some("Caf\u{e9}"));
    assert_eq!(sanitize_value("  a \n\t b  ").as_deref(), Some("a b"));
    assert_eq!(sanitize_value("AC/DC").as_deref(), Some("AC_DC"));
    let long = sanitize_value(&"я".repeat(100_000)).unwrap();
    assert!(long.len() <= MAX_VALUE_BYTES && long.chars().all(|c| c == 'я'));
    // The cut leaves a clean end.
    let cut = sanitize_value(&format!("{}. tail", "a".repeat(MAX_VALUE_BYTES - 1))).unwrap();
    assert!(!cut.ends_with('.'));
}

#[test]
fn exif_dates_stay_as_written() {
    assert_eq!(exif_date("2024:03:05 23:59:58").as_deref(), Some("2024-03-05T23:59:58"));
    assert_eq!(exif_date("2024:03:05 23:59:58\0").as_deref(), Some("2024-03-05T23:59:58"));
    for bad in ["", "0000:00:00 00:00:00", "    :  :     :  :  ", "2024:13:05 10:00:00", "2024:03:05", "../../../../../../..", "2024:03:05 25:00:00", "2024/03/05 10:00:00"] {
        assert_eq!(exif_date(bad), None, "{bad:?}");
    }
}

#[test]
fn camera_drops_a_repeated_maker() {
    assert_eq!(camera_name(Some("NIKON CORPORATION"), Some("NIKON D850")).as_deref(), Some("NIKON D850"));
    assert_eq!(camera_name(Some("Apple"), Some("iPhone 15")).as_deref(), Some("Apple iPhone 15"));
    assert_eq!(camera_name(None, Some("X/../Y")).as_deref(), Some("X_._Y"));
    assert_eq!(camera_name(Some("  "), None), None);
}

// ---------- parsers

#[test]
fn reads_a_real_jpeg() {
    let j = jpeg(&tiff("Canon", "Canon EOS R5", 1, "2021:07:04 10:20:30"), 640, 480);
    let mut m = FileMeta::default();
    parse_image(&j, &mut m);
    assert_eq!(m.taken.as_deref(), Some("2021-07-04T10:20:30"));
    assert_eq!(m.camera.as_deref(), Some("Canon EOS R5"));
    assert_eq!((m.width, m.height), (Some(640), Some(480)));
}

#[test]
fn rotated_photo_reports_its_displayed_size() {
    let j = jpeg(&tiff("Canon", "R5", 6, "2021:07:04 10:20:30"), 640, 480);
    let mut m = FileMeta::default();
    parse_image(&j, &mut m);
    assert_eq!((m.width, m.height), (Some(480), Some(640)));
}

#[test]
fn hostile_exif_text_comes_out_clean() {
    let j = jpeg(&tiff("../../evil", "a/b\\c", 1, "../../../../../../.."), 10, 10);
    let mut m = FileMeta::default();
    parse_image(&j, &mut m);
    assert_eq!(m.taken, None);
    let cam = m.camera.unwrap();
    assert!(!cam.contains('/') && !cam.contains('\\') && !cam.contains(".."), "{cam}");
}

#[test]
fn reads_png_size_without_exif() {
    let mut p = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    p.extend(300u32.to_be_bytes());
    p.extend(200u32.to_be_bytes());
    p.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
    let mut m = FileMeta::default();
    parse_image(&p, &mut m);
    assert_eq!((m.width, m.height, m.taken), (Some(300), Some(200), None));
}

#[test]
fn reads_real_id3_tags() {
    let m = audio_of(&mp3(&[("TPE1", "Kino"), ("TALB", "Album"), ("TIT2", "Song"), ("TRCK", "3/12"), ("TYER", "1988")]), "a.mp3");
    assert_eq!(m.artist.as_deref(), Some("Kino"));
    assert_eq!(m.album.as_deref(), Some("Album"));
    assert_eq!(m.title.as_deref(), Some("Song"));
    assert_eq!(m.track.as_deref(), Some("03"));
    assert_eq!(m.year.as_deref(), Some("1988"));
}

#[test]
fn hostile_tags_come_out_clean() {
    let m = audio_of(&mp3(&[("TPE1", "../../../etc/passwd"), ("TALB", ".."), ("TIT2", "CON"), ("TRCK", "garbage"), ("TYER", "x")]), "a.mp3");
    let a = m.artist.unwrap();
    assert!(!a.contains('/') && !a.contains(".."), "{a}");
    assert_eq!(m.album, None);
    assert_eq!(m.title.as_deref(), Some("_CON"));
    assert_eq!(m.track, None);
    assert_eq!(m.year, None);
}

#[test]
fn every_truncation_is_survived() {
    let j = jpeg(&tiff("Canon", "EOS", 1, "2021:07:04 10:20:30"), 64, 48);
    for n in 0..j.len() {
        parse_image(&j[..n], &mut FileMeta::default());
    }
    let a = mp3(&[("TPE1", "Artist"), ("TALB", "Album")]);
    for n in 0..a.len() {
        audio_of(&a[..n], "a.mp3");
    }
}

#[test]
fn garbage_is_survived_and_gives_nothing() {
    let magic: [&[u8]; 9] = [b"", &[0xFF, 0xD8, 0xFF, 0xE1], b"\x89PNG\r\n\x1a\n", b"II*\0", b"MM\0*", b"RIFF\0\0\0\0WEBP", b"\0\0\0\x18ftypheic", b"ID3\x03\0\0\x7f\x7f\x7f\x7f", b"fLaC"];
    for (i, head) in magic.iter().enumerate() {
        for seed in 0..40u32 {
            let mut b = head.to_vec();
            b.extend(noise(4096, seed * 7 + i as u32));
            let mut m = FileMeta::default();
            // A panic in a parser is contained in production; here it should not happen for these.
            parse_image(&b, &mut m);
            assert_eq!(m.taken, None);
            for name in ["a.mp3", "a.flac", "a.m4a", "a.ogg", "a.wav", "a.ape"] {
                let m = audio_of(&b, name);
                assert_eq!(m.track.filter(|t| !t.chars().all(|c| c.is_ascii_digit())), None);
            }
        }
    }
}

#[test]
fn counted_reader_stops_at_the_budget() {
    let data = vec![0u8; 1000];
    let mut r = Counted::new(Cursor::new(&data), 100, &NO, &nop);
    let mut sink = vec![];
    assert!(r.read_to_end(&mut sink).is_err());
    assert_eq!(sink.len(), 100);
    let stop = AtomicBool::new(true);
    assert!(Counted::new(Cursor::new(&data), 100, &stop, &nop).read(&mut [0u8; 8]).is_err());
}

#[test]
fn huge_files_are_not_read_whole() {
    let d = tmp("huge");
    // Sparse: 64 GiB of nothing with audio and image names.
    for name in ["big.mp3", "big.jpg", "big.flac"] {
        let f = fs::File::create(d.join(name)).unwrap();
        if f.set_len(64 << 30).is_err() {
            f.set_len(1 << 30).unwrap();
        }
    }
    let start = Instant::now();
    for name in ["big.mp3", "big.jpg", "big.flac"] {
        let m = read_meta(d.join(name).to_str().unwrap(), true, true, &NO, &nop);
        assert_eq!(m.artist, None);
    }
    assert!(start.elapsed() < Duration::from_secs(30));
    // What the tag parser really consumed, however large the file is.
    let mut c = Counted::new(fs::File::open(d.join("big.mp3")).unwrap(), AUDIO_BUDGET, &NO, &nop);
    parse_audio(&mut c, Path::new("big.mp3"), &mut FileMeta::default());
    assert!(c.left <= AUDIO_BUDGET);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn reads_real_flac_tags() {
    let mut f = b"fLaC".to_vec();
    f.extend([0, 0, 0, 34]);
    f.extend([0x10, 0, 0x10, 0, 0, 0, 0, 0, 0, 0, 0x0A, 0xC4, 0x42, 0xF0, 0, 0, 0, 0]);
    f.extend([0u8; 16]);
    let mut c = 0u32.to_le_bytes().to_vec(); // vendor length
    let items = ["ARTIST=A/B", "ALBUM=Disc", "TITLE=Song", "TRACKNUMBER=7", "DATE=2001-05-06"];
    c.extend((items.len() as u32).to_le_bytes());
    for i in items {
        c.extend((i.len() as u32).to_le_bytes());
        c.extend(i.as_bytes());
    }
    f.push(0x80 | 4);
    f.extend(&(c.len() as u32).to_be_bytes()[1..]);
    f.extend(c);
    let m = audio_of(&f, "a.flac");
    assert_eq!(m.artist.as_deref(), Some("A_B"));
    assert_eq!(m.album.as_deref(), Some("Disc"));
    assert_eq!(m.track.as_deref(), Some("07"));
    assert_eq!(m.year.as_deref(), Some("2001"));
}

#[test]
fn read_meta_end_to_end() {
    let d = tmp("e2e");
    let jp = d.join("p.jpg");
    fs::write(&jp, jpeg(&tiff("Sony", "A7", 1, "2020:01:02 03:04:05"), 40, 30)).unwrap();
    let au = d.join("s.mp3");
    fs::write(&au, mp3(&[("TPE1", "Artist")])).unwrap();
    let junk = d.join("j.jpg");
    fs::write(&junk, noise(2000, 3)).unwrap();
    let m = read_meta(jp.to_str().unwrap(), true, false, &NO, &nop);
    assert_eq!(m.taken.as_deref(), Some("2020-01-02T03:04:05"));
    assert!(m.modified.is_some());
    assert_eq!(m.artist, None, "audio wasn't asked for");
    assert_eq!(read_meta(au.to_str().unwrap(), false, true, &NO, &nop).artist.as_deref(), Some("Artist"));
    let g = read_meta(junk.to_str().unwrap(), true, true, &NO, &nop);
    assert_eq!((g.taken, g.camera, g.width), (None, None, None));
    // A folder and a missing file: time or nothing, never an error.
    assert!(read_meta(d.to_str().unwrap(), true, true, &NO, &nop).modified.is_some());
    assert_eq!(read_meta(d.join("nope").to_str().unwrap(), true, true, &NO, &nop).modified, None);
    assert_eq!(read_meta("morfa://cloud/a/b", true, true, &NO, &nop).modified, None);
    fs::remove_dir_all(&d).unwrap();
}

#[cfg(unix)]
#[test]
fn a_pipe_doesnt_hang_the_read() {
    let d = tmp("fifo");
    let p = d.join("pipe.mp3");
    let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
    let m = read_meta(p.to_str().unwrap(), true, true, &NO, &nop);
    assert_eq!(m.artist, None);
    fs::remove_dir_all(&d).unwrap();
}

// ---------- the check

const PLAIN: Fold = Fold { case: false, unicode: false };
const FOLDING: Fold = Fold { case: true, unicode: true };

fn run(d: &Path, fold: Fold, rows: &[(&str, &str)]) -> Vec<RowCheck> {
    let rows: Vec<Row> = rows.iter().map(|(a, b)| Row { path: d.join(a).to_string_lossy().into(), next: (*b).into() }).collect();
    check_rows(&rows, &|_| fold, &|_| 255, &NO, &nop).unwrap()
}

/// Names that differ only by case would collide on a Mac test disk, so those tests use distinct files.
fn dir_with(tag: &str, names: &[&str]) -> PathBuf {
    let d = tmp(tag);
    for n in names {
        fs::write(d.join(n), n).unwrap();
    }
    d
}

#[test]
fn duplicates_follow_the_disk() {
    let d = dir_with("dup", &["a", "b"]);
    assert_eq!(run(&d, FOLDING, &[("a", "X.jpg"), ("b", "x.JPG")]), [RowCheck::Duplicate, RowCheck::Duplicate]);
    assert_eq!(run(&d, PLAIN, &[("a", "same"), ("b", "same")]), [RowCheck::Duplicate, RowCheck::Duplicate]);
    assert_eq!(run(&d, FOLDING, &[("a", "caf\u{e9}"), ("b", "cafe\u{301}")]), [RowCheck::Duplicate, RowCheck::Duplicate]);
    let plain = run(&d, PLAIN, &[("a", "Xnew.jpg"), ("b", "xnew.jpg")]);
    assert_eq!(plain, [RowCheck::Ok, RowCheck::Ok]);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn existing_files_conflict_unless_they_move_away() {
    let d = dir_with("exist", &["a", "b", "c", "other"]);
    assert_eq!(run(&d, PLAIN, &[("a", "other"), ("b", "b")]), [RowCheck::Exists, RowCheck::Same]);
    // A swap and a chain are fine: both names are freed by the batch.
    assert_eq!(run(&d, PLAIN, &[("a", "b"), ("b", "a")]), [RowCheck::Ok, RowCheck::Ok]);
    assert_eq!(run(&d, PLAIN, &[("a", "b"), ("b", "c"), ("c", "new")]), [RowCheck::Ok, RowCheck::Ok, RowCheck::Ok]);
    // Excluded row: it keeps its name, so that name isn't free.
    assert_eq!(run(&d, PLAIN, &[("a", "b"), ("b", "b")]), [RowCheck::Exists, RowCheck::Same]);
    fs::remove_dir_all(&d).unwrap();
}

#[cfg(unix)]
#[test]
fn a_hard_link_under_another_name_is_an_existing_file() {
    let d = dir_with("hard", &["a"]);
    fs::hard_link(d.join("a"), d.join("linked")).unwrap();
    assert_eq!(run(&d, PLAIN, &[("a", "linked")]), [RowCheck::Exists]);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn invalid_names_are_named() {
    let d = dir_with("invalid", &["a"]);
    let one = |next: &str| run(&d, PLAIN, &[("a", next)]).remove(0);
    assert_eq!(one(""), RowCheck::Invalid("empty"));
    assert_eq!(one(".."), RowCheck::Invalid("dots"));
    assert_eq!(one("x/../../y"), RowCheck::Invalid("chars"));
    assert_eq!(one("x\\y"), RowCheck::Invalid("chars"));
    assert_eq!(one("x\0y"), RowCheck::Invalid("chars"));
    assert_eq!(one("x."), RowCheck::Invalid("trailing"));
    assert_eq!(one("NUL.txt"), RowCheck::Invalid("reserved"));
    assert_eq!(one(&"я".repeat(128)), RowCheck::Invalid("long"));
    assert_eq!(one(&"a".repeat(255)), RowCheck::Ok);
    assert_eq!(run(&d, PLAIN, &[("gone", "new")]), [RowCheck::Gone]);
    let pseudo = [Row { path: "morfa://cloud/x/a".into(), next: "b".into() }];
    assert_eq!(check_rows(&pseudo, &|_| PLAIN, &|_| 255, &NO, &nop).unwrap(), [RowCheck::Invalid("pseudo")]);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn the_disks_own_limit_counts() {
    let d = dir_with("max", &["a"]);
    let rows = [Row { path: d.join("a").to_string_lossy().into(), next: "x".repeat(144) }];
    assert_eq!(check_rows(&rows, &|_| PLAIN, &|_| 143, &NO, &nop).unwrap(), [RowCheck::Invalid("long")]);
    assert!(super::check::name_max(&d) >= 100);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn the_limit_counts_what_the_system_counts() {
    use super::check::name_len;
    assert_eq!(name_len("я", false), 2);
    assert_eq!(name_len("я", true), 1);
    assert_eq!(name_len("😀", true), 2);
}

#[test]
fn cancel_stops_the_check() {
    let stop = AtomicBool::new(true);
    let rows = [Row { path: "/x/a".into(), next: "b".into() }];
    assert!(check_rows(&rows, &|_| PLAIN, &|_| 255, &stop, &nop).is_err());
}
