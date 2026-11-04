//! Which engine converts which input extension into which output format.
use super::{doc, engine, pdf, video};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Via {
    Image,
    PdfPages,
    Office,
    Pandoc,
    Video,
}

/// What's installed. `image` is the image engine's writable formats.
pub struct Avail<'a> {
    pub image: &'a [String],
    pub pdf: bool,
    pub office: bool,
    pub pandoc: bool,
    pub ffmpeg: bool,
}

const IMAGE_IN: [&str; 10] = ["jpg", "png", "webp", "avif", "heic", "heif", "tiff", "gif", "bmp", "jxl"];
const WRITER_IN: [&str; 6] = ["doc", "docx", "odt", "rtf", "txt", "html"];
const WRITER_OUT: [&str; 7] = ["pdf", "docx", "odt", "rtf", "txt", "html", "epub"];
pub const SHEET_IN: [&str; 4] = ["xls", "xlsx", "ods", "csv"];
const SHEET_OUT: [&str; 5] = ["pdf", "xlsx", "ods", "csv", "html"];
pub const SLIDE_IN: [&str; 3] = ["ppt", "pptx", "odp"];
const SLIDE_OUT: [&str; 3] = ["pdf", "pptx", "odp"];
const PANDOC_IN: [&str; 9] = ["md", "rst", "org", "tex", "epub", "html", "docx", "odt", "txt"];
const PANDOC_OUT: [&str; 6] = ["md", "html", "docx", "odt", "epub", "rst"];
// No "ts": too often TypeScript source.
const VIDEO_IN: [&str; 11] = ["mp4", "mkv", "webm", "mov", "avi", "m4v", "wmv", "flv", "mpg", "3gp", "mpeg"];
const AUDIO_IN: [&str; 8] = ["mp3", "wav", "flac", "ogg", "m4a", "aac", "opus", "wma"];
/// Video outputs first, then audio extraction.
const VIDEO_OUT: [&str; 11] = ["mp4", "webm", "mkv", "mov", "gif", "mp3", "m4a", "ogg", "opus", "flac", "wav"];
const AUDIO_OUT: [&str; 6] = ["mp3", "m4a", "opus", "ogg", "flac", "wav"];
const GIF_VIDEO: [&str; 2] = ["mp4", "webm"];
const PDF_OUT: [&str; 3] = ["png", "jpg", "webp"];
/// Display order for documents (pdf first).
const DOC_ORDER: [&str; 13] = ["pdf", "docx", "odt", "rtf", "txt", "html", "epub", "md", "rst", "xlsx", "ods", "csv", "pptx"];

/// Lowercase and fold aliases so "JPEG", "jpg", "tif", "htm", "markdown" compare equal.
pub fn norm(ext: &str) -> String {
    let e = ext.trim().trim_start_matches('.').to_lowercase();
    match e.as_str() {
        "jpeg" => "jpg".into(),
        "tif" => "tiff".into(),
        "htm" | "xhtml" => "html".into(),
        "markdown" => "md".into(),
        "mpeg" => "mpg".into(),
        "oga" => "ogg".into(),
        _ => e,
    }
}

pub fn is_image(ext: &str) -> bool {
    IMAGE_IN.contains(&ext)
}

/// Video or audio input handled by ffmpeg.
#[allow(dead_code)] // public API for callers outside routing
pub fn is_media(ext: &str) -> bool {
    VIDEO_IN.contains(&ext) || AUDIO_IN.contains(&ext)
}

/// Video input, or a GIF: what a video resize applies to.
pub fn is_video_like(ext: &str) -> bool {
    VIDEO_IN.contains(&ext) || ext == "gif"
}

/// True if `fmt` is an audio-only output.
pub fn is_audio(fmt: &str) -> bool {
    AUDIO_OUT.contains(&fmt)
}

/// Engine for `ext` -> `fmt` given what's available (both already normalized).
pub fn route(ext: &str, fmt: &str, a: &Avail) -> Option<Via> {
    if ext == fmt {
        return None;
    }
    let has = |l: &[&str], x: &str| l.contains(&x);
    if has(&VIDEO_IN, ext) || has(&AUDIO_IN, ext) {
        let outs: &[&str] = if has(&VIDEO_IN, ext) { &VIDEO_OUT } else { &AUDIO_OUT };
        return (a.ffmpeg && has(outs, fmt)).then_some(Via::Video);
    }
    if ext == "gif" && has(&GIF_VIDEO, fmt) {
        return a.ffmpeg.then_some(Via::Video);
    }
    if has(&IMAGE_IN, ext) {
        return a.image.iter().any(|f| f == fmt).then_some(Via::Image);
    }
    if ext == "pdf" {
        return (a.pdf && has(&PDF_OUT, fmt) && a.image.iter().any(|f| f == fmt)).then_some(Via::PdfPages);
    }
    let office = (has(&WRITER_IN, ext) && has(&WRITER_OUT, fmt))
        || (has(&SHEET_IN, ext) && has(&SHEET_OUT, fmt))
        || (has(&SLIDE_IN, ext) && has(&SLIDE_OUT, fmt));
    if a.office && office {
        return Some(Via::Office);
    }
    (a.pandoc && has(&PANDOC_IN, ext) && has(&PANDOC_OUT, fmt)).then_some(Via::Pandoc)
}

/// `route`, plus same-format image re-encodes (resize, strip, quality), which aren't listed as targets.
pub fn route_or_same(ext: &str, fmt: &str, a: &Avail) -> Option<Via> {
    route(ext, fmt, a).or_else(|| (ext == fmt && a.image.iter().any(|f| f == fmt)).then_some(Via::Image))
}

fn order(ext: &str, a: &Avail) -> Vec<String> {
    let mut all: Vec<String> = vec!["webp".into()];
    if VIDEO_IN.contains(&ext) {
        all = VIDEO_OUT.iter().map(|s| s.to_string()).collect();
    } else if AUDIO_IN.contains(&ext) {
        all = AUDIO_OUT.iter().map(|s| s.to_string()).collect();
    } else if ext == "pdf" {
        all = PDF_OUT.iter().map(|s| s.to_string()).collect();
    } else if !IMAGE_IN.contains(&ext) {
        all = DOC_ORDER.iter().chain(SLIDE_OUT.iter()).map(|s| s.to_string()).collect();
    }
    all.extend(a.image.iter().cloned());
    if ext == "gif" {
        all.extend(GIF_VIDEO.iter().map(|s| s.to_string()));
    }
    let mut seen = std::collections::HashSet::new();
    all.retain(|f| seen.insert(f.clone()) && route(ext, f, a).is_some());
    all
}

/// Formats valid for every input, ordered by the first input's preference.
pub fn targets(exts: &[String], a: &Avail) -> Vec<String> {
    let exts: Vec<String> = exts.iter().map(|e| norm(e)).collect();
    let Some(first) = exts.first() else { return vec![] };
    order(first, a).into_iter().filter(|f| exts.iter().all(|e| route(e, f, a).is_some())).collect()
}

/// The real, detected availability (detection is cached).
pub fn detected<R>(f: impl FnOnce(&Avail) -> R) -> R {
    let e = engine::engine();
    let image: &[String] = e.map(|e| e.formats.as_slice()).unwrap_or(&[]);
    f(&Avail { image, pdf: e.is_some() && pdf::available(), office: doc::office().is_some(), pandoc: doc::pandoc(), ffmpeg: video::available() })
}

/// Error for a route that's unavailable here: which engine would be needed.
pub fn missing(ext: &str, fmt: &str) -> String {
    let all: Vec<String> = IMAGE_IN.iter().map(|s| s.to_string()).collect();
    match route(ext, fmt, &Avail { image: &all, pdf: true, office: true, pandoc: true, ffmpeg: true }) {
        Some(Via::Office) => "NO_ENGINE:office".into(),
        Some(Via::Pandoc) => "NO_ENGINE:pandoc".into(),
        Some(Via::Video) => "NO_ENGINE:ffmpeg".into(),
        Some(_) => engine::NO_ENGINE.into(),
        None => format!("Can't convert {ext} to {fmt}"),
    }
}
