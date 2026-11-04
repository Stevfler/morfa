use super::*;
use crate::convert::{dispatch, engine, ConvertOpts};
use std::cell::{Cell, RefCell};

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("morfa-fit-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn files(d: &Path) -> Vec<String> {
    std::fs::read_dir(d).unwrap().map(|x| x.unwrap().file_name().to_string_lossy().into_owned()).collect()
}

/// Runs `search` with a fake encoder writing `size(q)` bytes; returns the result and qualities tried.
fn fake(d: &Path, ceiling: u8, limit: u64, lossy: bool, size: &dyn Fn(u8) -> usize, cancelled: &dyn Fn(u32) -> bool) -> (Result<Found, String>, Vec<u8>) {
    let n = Cell::new(0);
    let tried = RefCell::new(vec![]);
    let r = search(
        ceiling,
        limit,
        lossy,
        &mut || {
            n.set(n.get() + 1);
            d.join(format!("t{}", n.get()))
        },
        &mut |q, p| {
            tried.borrow_mut().push(q);
            std::fs::write(p, vec![0u8; size(q)]).map_err(|e| e.to_string())
        },
        &|| cancelled(tried.borrow().len() as u32),
        &|_| {},
    );
    (r, tried.into_inner())
}

const NEVER: &dyn Fn(u32) -> bool = &|_| false;

#[test]
fn fits_at_ceiling_in_one_attempt() {
    let d = tmpdir("ceiling");
    let (r, tried) = fake(&d, 80, 1000, true, &|q| q as usize, NEVER);
    let f = r.unwrap();
    assert_eq!((f.quality, f.size, tried), (80, 80, vec![80]));
    assert_eq!(files(&d), vec!["t1"]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn monotonic_keeps_a_fitting_result_near_the_limit() {
    let d = tmpdir("mono");
    // Every limit: the result fits, stays within what the attempts can resolve, and is the only file left.
    for limit in [10u64, 150, 333, 500, 777, 990] {
        let (r, tried) = fake(&d, 100, limit, true, &|q| q as usize * 10, NEVER);
        let f = r.unwrap();
        assert!(f.size <= limit, "{} over {limit}", f.size);
        assert!(tried.len() as u32 <= MAX_ATTEMPTS, "{tried:?}");
        let exact = (limit / 10).clamp(1, 100) as u8;
        assert!(exact - f.quality <= 7, "limit {limit}: q{} vs q{exact}", f.quality);
        assert_eq!(files(&d), vec![f.tmp.file_name().unwrap().to_string_lossy().into_owned()]);
        std::fs::remove_file(&f.tmp).unwrap();
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn non_monotonic_encoder_never_returns_an_oversized_file() {
    let d = tmpdir("wobble");
    let (r, _) = fake(&d, 100, 500, true, &|q| if q % 2 == 0 { 900 } else { q as usize * 5 }, NEVER);
    let f = r.unwrap();
    assert!(f.size <= 500);
    assert_eq!(std::fs::metadata(&f.tmp).unwrap().len(), f.size);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn impossible_limit_is_an_error_and_leaves_nothing() {
    let d = tmpdir("impossible");
    let (r, tried) = fake(&d, 90, 50, true, &|q| 100 + q as usize, NEVER);
    assert_eq!(r.unwrap_err(), format!("{OVER_LIMIT}:101:50"));
    assert_eq!(tried, vec![90, 1], "the floor decides, no more attempts are spent");
    assert!(files(&d).is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn only_the_floor_fits() {
    let d = tmpdir("floor");
    let (r, tried) = fake(&d, 100, 50, true, &|q| if q == 1 { 40 } else { 100 }, NEVER);
    assert_eq!(r.unwrap().quality, 1);
    assert_eq!(tried.len() as u32, MAX_ATTEMPTS);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn lossless_gets_one_attempt() {
    let d = tmpdir("lossless");
    let (r, tried) = fake(&d, 80, 50, false, &|_| 100, NEVER);
    assert!(r.unwrap_err().starts_with(OVER_LIMIT));
    assert_eq!(tried, vec![80]);
    assert!(files(&d).is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn cancel_at_any_attempt_cleans_up() {
    for at in 0..=MAX_ATTEMPTS {
        let d = tmpdir(&format!("cancel{at}"));
        // Size that makes the search run all attempts while holding a fitting temp from attempt 2.
        let (r, tried) = fake(&d, 100, 500, true, &|q| q as usize * 10, &|n| n >= at);
        assert_eq!(r.unwrap_err(), "Cancelled", "at {at}");
        assert!(tried.len() as u32 <= at, "at {at}: {tried:?}");
        assert!(files(&d).is_empty(), "at {at}: {:?}", files(&d));
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[test]
fn encoder_failure_cleans_up_partial_and_best() {
    let d = tmpdir("fail");
    let n = Cell::new(0);
    let r = search(
        100,
        500,
        true,
        &mut || {
            n.set(n.get() + 1);
            d.join(format!("t{}", n.get()))
        },
        &mut |q, p| {
            std::fs::write(p, vec![0u8; q as usize * 10]).unwrap();
            if n.get() == 3 { Err("boom".into()) } else { Ok(()) }
        },
        &|| false,
        &|_| {},
    );
    assert_eq!(r.unwrap_err(), "boom");
    assert!(files(&d).is_empty(), "{:?}", files(&d));
    let _ = std::fs::remove_dir_all(&d);
}

fn opts(format: &str, max_bytes: u64) -> ConvertOpts {
    ConvertOpts { format: format.into(), quality: 95, resize: None, strip_metadata: true, out_dir: None, name_template: "{name}".into(), max_bytes: Some(max_bytes) }
}

/// Noise compresses badly, so quality really moves the size.
fn noise(e: &engine::Engine, src: &Path) -> bool {
    let made = match e.kind {
        engine::Kind::Vips => std::process::Command::new("vips").arg("gaussnoise").arg(src).args(["600", "400"]).status(),
        _ => std::process::Command::new("magick").args(["-size", "600x400", "xc:gray", "+noise", "Gaussian"]).arg(src).status(),
    };
    made.map(|s| s.success()).unwrap_or(false)
}

#[test]
fn real_engine_hits_the_limit_or_reports_it() {
    let Some(e) = engine::engine() else { return eprintln!("no engine; skipping") };
    if !e.formats.iter().any(|f| f == "jpg") {
        return eprintln!("no jpg; skipping");
    }
    let d = tmpdir("real");
    let src = d.join("pic.png");
    assert!(noise(e, &src));
    let before = std::fs::read(&src).unwrap();
    let s = vec![src.to_string_lossy().into_owned()];
    let full = dispatch::run_all(&s, &ConvertOpts { max_bytes: None, ..opts("jpg", 1) }, &|| false, &|_, _, _| {});
    let unlimited = std::fs::metadata(&full.pairs[0].1).unwrap().len();
    std::fs::remove_file(&full.pairs[0].1).unwrap();

    let limit = unlimited / 2;
    let r = dispatch::run_all(&s, &opts("jpg", limit), &|| false, &|_, _, _| {});
    assert_eq!(r.pairs.len(), 1, "failed: {:?}", r.failed);
    let size = std::fs::metadata(&r.pairs[0].1).unwrap().len();
    assert!(size <= limit, "{size} over {limit}");
    assert!(size > limit / 2, "{size} is far below {limit}: the search gave away too much quality");
    std::fs::remove_file(&r.pairs[0].1).unwrap();

    let r = dispatch::run_all(&s, &opts("jpg", 10), &|| false, &|_, _, _| {});
    assert!(r.pairs.is_empty());
    assert!(r.failed[0].1.starts_with(OVER_LIMIT), "{:?}", r.failed);

    // Lossless output: the one size either fits or the file fails.
    let r = dispatch::run_all(&s, &opts("png", 10), &|| false, &|_, _, _| {});
    assert!(r.failed[0].1.starts_with(OVER_LIMIT), "{:?}", r.failed);

    // A cancel during the search writes nothing.
    let polls = Cell::new(0u32);
    let polled = std::sync::Mutex::new(polls);
    let r = dispatch::run_all(&s, &opts("jpg", limit), &|| {
        let g = polled.lock().unwrap();
        g.set(g.get() + 1);
        g.get() > 3
    }, &|_, _, _| {});
    assert!(r.cancelled && r.pairs.is_empty() && r.failed.is_empty(), "{:?}", r.failed);

    assert_eq!(std::fs::read(&src).unwrap(), before, "source changed");
    assert_eq!(files(&d), vec!["pic.png"], "left behind");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn limit_is_refused_outside_image_routes_and_missing_folder_is_reported() {
    let d = tmpdir("route");
    let src = d.join("a.wav");
    std::fs::write(&src, b"x").unwrap();
    let s = vec![src.to_string_lossy().into_owned()];
    let r = dispatch::run_all(&s, &opts("mp3", 100), &|| false, &|_, _, _| {});
    // Without ffmpeg the missing engine is reported instead; either way nothing is written.
    assert!(r.pairs.is_empty());
    assert!(r.failed[0].1 == dispatch::LIMIT_IMAGES_ONLY || r.failed[0].1.contains("NO_ENGINE"), "{:?}", r.failed);
    let o = ConvertOpts { out_dir: Some(d.join("gone").to_string_lossy().into_owned()), max_bytes: None, ..opts("mp3", 0) };
    let r = dispatch::run_all(&s, &o, &|| false, &|_, _, _| {});
    assert_eq!(r.failed[0].1, dispatch::NO_OUT_DIR);
    let o = ConvertOpts { name_template: "D:evil".into(), max_bytes: None, ..opts("mp3", 0) };
    let r = dispatch::run_all(&s, &o, &|| false, &|_, _, _| {});
    assert_eq!(r.failed[0].1, dispatch::BAD_TEMPLATE);
    let _ = std::fs::remove_dir_all(&d);
}
