use super::*;
use routes::{route, targets, Avail, Via};
use video::{args, run};
use std::path::Path;

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn opts(format: &str, quality: u8, resize: Option<Resize>, strip: bool) -> ConvertOpts {
    ConvertOpts { format: format.into(), quality, resize, strip_metadata: strip, out_dir: None, name_template: "{name}".into() }
}

#[test]
fn media_routes() {
    let img = s(&["jpg", "png", "webp", "gif"]);
    let a = Avail { image: &img, pdf: false, office: false, pandoc: false, ffmpeg: true };
    assert_eq!(targets(&s(&["MOV"]), &a), s(&["mp4", "webm", "mkv", "gif", "mp3", "m4a", "ogg", "opus", "flac", "wav"]));
    assert_eq!(targets(&s(&["mp4"]), &a)[..2], s(&["webm", "mkv"]));
    assert_eq!(targets(&s(&["oga"]), &a), s(&["mp3", "m4a", "opus", "flac", "wav"]));
    assert_eq!(targets(&s(&["mp4", "flac"]), &a), s(&["mp3", "m4a", "ogg", "opus", "wav"]));
    assert_eq!(targets(&s(&["gif"]), &a), s(&["webp", "jpg", "png", "mp4", "webm"]));
    assert_eq!(route("mpg", "mp4", &a), Some(Via::Video));
    assert_eq!(routes::norm("MPEG"), "mpg");
    assert!(routes::is_media("m4v") && !routes::is_media("ts") && routes::is_media("wma") && !routes::is_media("gif"));
    let none = Avail { ffmpeg: false, ..a };
    assert!(targets(&s(&["mp4"]), &none).is_empty());
    assert_eq!(targets(&s(&["gif"]), &none), s(&["webp", "jpg", "png"]));
    assert_eq!(routes::missing("mkv", "mp4"), "NO_ENGINE:ffmpeg");
    assert_eq!(routes::missing("gif", "webm"), "NO_ENGINE:ffmpeg");
}

#[test]
fn quality_maps() {
    assert_eq!((args::crf_x264(100), args::crf_x264(1)), (16, 36));
    assert_eq!((args::crf_vp9(100), args::crf_vp9(1)), (20, 45));
    assert!(args::crf_x264(50) > 16 && args::crf_x264(50) < 36);
    assert_eq!([100, 82, 65, 10].map(|q| args::audio_kbps(q, false)), [320, 192, 128, 96]);
    assert_eq!(args::audio_kbps(100, true), 192);
}

#[test]
fn builds_args() {
    let (src, tmp) = (Path::new("/a/in.mkv"), Path::new("/a/.t.mp4"));
    let a = args::build(src, tmp, &opts("mp4", 100, Some(Resize { mode: "max".into(), value: 720 }), true)).join(" ");
    assert!(a.starts_with("-hide_banner -nostdin -y -i /a/in.mkv"), "{a}");
    assert!(a.ends_with("-progress pipe:1 -nostats /a/.t.mp4"), "{a}");
    for x in ["-c:v libx264", "-crf 16", "yuv420p", "+faststart", "-c:a aac", "min(720,", "-map_metadata -1", "-map_chapters -1"] {
        assert!(a.contains(x), "{x} in {a}");
    }
    let w = args::build(src, tmp, &opts("webm", 1, None, false)).join(" ");
    assert!(w.contains("libvpx-vp9 -b:v 0 -crf 45 -row-mt 1") && w.contains("libopus") && !w.contains("map_metadata"), "{w}");
    let g = args::build(src, tmp, &opts("gif", 80, Some(Resize { mode: "percent".into(), value: 50 }), false)).join(" ");
    assert!(g.contains("fps=15,scale=trunc(iw*50/200)*2:-2,split") && g.contains("paletteuse") && g.contains("-an -loop 0"), "{g}");
    let m = args::build(src, tmp, &opts("mp3", 65, Some(Resize { mode: "max".into(), value: 9 }), false)).join(" ");
    assert!(m.contains("-vn -c:a libmp3lame -b:a 128k") && !m.contains("scale"), "{m}");
    let f = args::build(src, tmp, &opts("flac", 65, None, false)).join(" ");
    assert!(f.contains("-c:a flac") && !f.contains("-b:a"), "{f}");
}

#[test]
fn parses_progress() {
    assert_eq!(run::parse_duration("  Duration: 00:01:02.50, start: 0.000000, bitrate: 1 kb/s"), Some(62_500_000));
    assert_eq!(run::parse_duration("  Duration: N/A, bitrate: N/A"), None);
    assert_eq!(run::parse_out_time("out_time_us=1500000"), Some(1_500_000));
    assert_eq!(run::parse_out_time("out_time_ms=1500000\n"), Some(1_500_000));
    assert_eq!(run::parse_out_time("out_time=00:00:01.5"), None);
}

#[test]
fn converts_real_video() {
    if !video::available() {
        return eprintln!("no ffmpeg; skipping");
    }
    let d = std::env::temp_dir().join(format!("morfa-convert-test-video-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let src = d.join("clip.mp4");
    let ok = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-f", "lavfi", "-i", "testsrc=duration=3:size=321x241:rate=25", "-f", "lavfi", "-i", "sine=duration=3", "-shortest"])
        .arg(&src)
        .status();
    assert!(ok.unwrap().success());
    let parts = std::sync::Mutex::new(vec![]);
    let srcs = vec![src.to_string_lossy().into_owned()];
    for fmt in ["webm", "gif", "mp3", "mkv"] {
        let r = dispatch::run_parts(&srcs, &opts(fmt, 60, None, true), &|| false, &|_, _, _| {}, &|_, _, f| parts.lock().unwrap().push(f));
        assert_eq!(r.pairs.len(), 1, "{fmt}: {:?}", r.failed);
    }
    let gif = d.join("clip.gif").to_string_lossy().into_owned();
    let r = dispatch::run_all(&[gif], &opts("mp4", 60, None, false), &|| false, &|_, _, _| {});
    assert_eq!(r.pairs.len(), 1, "gif->mp4: {:?}", r.failed);
    assert!(parts.lock().unwrap().iter().all(|f| (0.0..=1.0).contains(f)));
    let r = dispatch::run_all(&srcs, &opts("mov", 60, None, false), &|| true, &|_, _, _| {});
    assert!(r.cancelled && r.pairs.is_empty());
    let names: Vec<_> = std::fs::read_dir(&d).unwrap().map(|x| x.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(names.iter().all(|n| !n.starts_with(".morfa-convert")), "temp left: {names:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn converts_edge_cases() {
    if !video::available() {
        return eprintln!("no ffmpeg; skipping");
    }
    let d = std::env::temp_dir().join(format!("morfa-convert-test-edge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let gen = |name: &str, input: &[&str]| {
        let p = d.join(name);
        let mut c = std::process::Command::new("ffmpeg");
        c.args(["-v", "error", "-nostdin"]).args(input).arg(&p);
        assert!(c.status().unwrap().success());
        vec![p.to_string_lossy().into_owned()]
    };
    let audio = gen("tone.wav", &["-f", "lavfi", "-i", "sine=duration=2"]);
    let mute = gen("mute.mkv", &["-f", "lavfi", "-i", "testsrc=duration=2:size=333x199:rate=10"]);
    let run = |s: &[String], o: ConvertOpts| dispatch::run_all(s, &o, &|| false, &|_, _, _| {});
    for fmt in ["mp3", "m4a", "opus", "ogg", "flac"] {
        assert_eq!(run(&audio, opts(fmt, 60, None, true)).pairs.len(), 1, "wav->{fmt}");
    }
    assert_eq!(run(&gen("in.flac", &["-f", "lavfi", "-i", "sine=duration=1"]), opts("wav", 60, None, false)).pairs.len(), 1);
    for (fmt, r) in [("webm", Resize { mode: "percent".into(), value: 50 }), ("mp4", Resize { mode: "max".into(), value: 101 })] {
        assert_eq!(run(&mute, opts(fmt, 60, Some(r), true)).pairs.len(), 1, "mute->{fmt}");
    }
    let _ = std::fs::remove_dir_all(&d);
}
