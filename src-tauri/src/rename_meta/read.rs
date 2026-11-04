//! Reads metadata from a bounded part of a file. Parsers only see a counted reader or a prefix in
//! memory, so a malformed file costs at most these many bytes, and a panic in one stays with that file.
use super::sanitize::{camera_name, exif_date, sanitize_value};
use lofty::{
    config::{ParseOptions, ParsingMode},
    file::{FileType, TaggedFileExt},
    probe::Probe,
    tag::Accessor,
};
use serde::Serialize;
use std::{
    fs,
    io::{self, BufReader, Cursor, Read, Seek, SeekFrom},
    panic::{catch_unwind, AssertUnwindSafe},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::UNIX_EPOCH,
};

/// Image headers and EXIF are looked for here. A camera file's metadata sits at the start.
pub const IMAGE_PREFIX: u64 = 1024 * 1024;
/// Total bytes the tag parser may read from one file. Same as lofty's own allocation limit: a tag
/// past it (a huge embedded cover) reads as no tags.
pub const AUDIO_BUDGET: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Default, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileMeta {
    pub path: String,
    /// Milliseconds since the epoch.
    pub modified: Option<i64>,
    /// `YYYY-MM-DDTHH:mm:ss`, as written in the file, no time zone.
    pub taken: Option<String>,
    pub camera: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub title: Option<String>,
    pub track: Option<String>,
    pub year: Option<String>,
}

/// Fails past its budget or once cancelled, and reports progress so a slow share isn't taken for a dead one.
pub struct Counted<'a, R> {
    inner: R,
    pub left: u64,
    cancel: &'a AtomicBool,
    tick: &'a dyn Fn(),
}
impl<'a, R> Counted<'a, R> {
    pub fn new(inner: R, budget: u64, cancel: &'a AtomicBool, tick: &'a dyn Fn()) -> Self {
        Self { inner, left: budget, cancel, tick }
    }
}
impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        if self.left == 0 {
            return Err(io::Error::other("read limit reached"));
        }
        let want = buf.len().min(self.left.min(usize::MAX as u64) as usize);
        let n = self.inner.read(&mut buf[..want])?;
        self.left -= n as u64;
        (self.tick)();
        Ok(n)
    }
}
impl<R: Seek> Seek for Counted<'_, R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// Opens only a regular file. Non-blocking, so a pipe swapped in after the type check can't hang the open.
fn open_regular(path: &Path) -> Option<fs::File> {
    let mut o = fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(libc::O_NONBLOCK);
    }
    let f = o.open(path).ok()?;
    f.metadata().ok()?.is_file().then_some(f)
}

fn looks_like_image(h: &[u8]) -> bool {
    h.starts_with(&[0xFF, 0xD8, 0xFF])
        || h.starts_with(b"\x89PNG\r\n\x1a\n")
        || h.starts_with(b"GIF8")
        || h.starts_with(b"BM")
        || h.starts_with(b"II*\0")
        || h.starts_with(b"MM\0*")
        || (h.starts_with(b"RIFF") && h.get(8..12) == Some(b"WEBP"))
        || (h.get(4..8) == Some(b"ftyp") && h.get(8..12).is_some_and(|b| [&b"heic"[..], b"heix", b"hevc", b"heif", b"mif1", b"msf1", b"avif"].contains(&b)))
}

/// Parses image metadata from the start of a file; tolerant of a cut-off or damaged one.
pub fn parse_image(prefix: &[u8], m: &mut FileMeta) {
    if !looks_like_image(prefix) {
        return;
    }
    let (mut width, mut height) = match imagesize::blob_size(prefix) {
        Ok(s) if s.width > 0 && s.height > 0 => (u32::try_from(s.width).ok(), u32::try_from(s.height).ok()),
        _ => (None, None),
    };
    let mut reader = exif::Reader::new();
    reader.continue_on_error(true);
    let parsed = match reader.read_from_container(&mut Cursor::new(prefix)) {
        Ok(e) => Some(e),
        Err(exif::Error::PartialResult(p)) => Some(p.into_inner().0),
        Err(_) => None,
    };
    if let Some(e) = parsed {
        let text = |tag| -> Option<String> {
            let f = e.get_field(tag, exif::In::PRIMARY)?;
            match &f.value {
                exif::Value::Ascii(v) => v.first().map(|b| String::from_utf8_lossy(b).into_owned()),
                _ => None,
            }
        };
        // Not the plain DateTime tag: that one is when the file was last changed.
        m.taken = text(exif::Tag::DateTimeOriginal).and_then(|s| exif_date(&s)).or_else(|| text(exif::Tag::DateTimeDigitized).and_then(|s| exif_date(&s)));
        m.camera = camera_name(text(exif::Tag::Make).as_deref(), text(exif::Tag::Model).as_deref());
        // Sizes as the picture is displayed: 5–8 are the rotated orientations.
        let turned = e.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0)).is_some_and(|o| (5..=8).contains(&o));
        if turned {
            std::mem::swap(&mut width, &mut height);
        }
    }
    m.width = width;
    m.height = height;
}

fn read_prefix(f: fs::File, cancel: &AtomicBool, tick: &dyn Fn()) -> Option<Vec<u8>> {
    let mut r = Counted::new(f, IMAGE_PREFIX, cancel, tick);
    // The signature first: most of a mixed selection stops here, after one small read.
    let mut out = Vec::new();
    (&mut r).take(16).read_to_end(&mut out).ok()?;
    if !looks_like_image(&out) {
        return None;
    }
    // Reaching the limit is the normal end for a large file.
    let mut rest = r.take(IMAGE_PREFIX - 16);
    let _ = rest.read_to_end(&mut out);
    Some(out)
}

/// Parses audio tags from a reader limited by `Counted`.
pub fn parse_audio<R: Read + Seek>(reader: R, path: &Path, m: &mut FileMeta) {
    let options = ParseOptions::new().read_properties(false).read_cover_art(false).parsing_mode(ParsingMode::Relaxed);
    let Ok(mut probe) = Probe::new(BufReader::with_capacity(64 * 1024, reader)).options(options).guess_file_type() else { return };
    if probe.file_type().is_none() {
        // Content wasn't recognised: the extension may still name a format that has no signature.
        let Some(t) = FileType::from_path(path) else { return };
        probe = probe.set_file_type(t);
    }
    let Ok(tagged) = probe.read() else { return };
    let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) else { return };
    m.artist = tag.artist().and_then(|v| sanitize_value(&v));
    m.album = tag.album().and_then(|v| sanitize_value(&v));
    m.title = tag.title().and_then(|v| sanitize_value(&v));
    m.track = tag.track().filter(|n| *n > 0).map(|n| format!("{n:02}"));
    m.year = tag.date().map(|d| d.year).filter(|y| *y > 0).map(|y| format!("{y:04}"));
}

/// One file. Never fails: whatever can't be read stays empty, and the preview flags it as missing.
pub fn read_meta(path: &str, image: bool, audio: bool, cancel: &AtomicBool, tick: &dyn Fn()) -> FileMeta {
    let mut m = FileMeta { path: path.to_string(), ..Default::default() };
    if path.starts_with("morfa://") {
        return m;
    }
    let p = Path::new(path);
    let Ok(st) = fs::metadata(p) else { return m };
    m.modified = st.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).and_then(|d| i64::try_from(d.as_millis()).ok());
    if !st.is_file() {
        return m;
    }
    if image {
        if let Some(prefix) = open_regular(p).and_then(|f| read_prefix(f, cancel, tick)) {
            let mut got = m.clone();
            if catch_unwind(AssertUnwindSafe(|| parse_image(&prefix, &mut got))).is_ok() {
                m = got;
            }
        }
    }
    if audio {
        if let Some(f) = open_regular(p) {
            let mut got = m.clone();
            let run = AssertUnwindSafe(|| parse_audio(Counted::new(f, AUDIO_BUDGET, cancel, tick), p, &mut got));
            if catch_unwind(run).is_ok() {
                m = got;
            }
        }
    }
    m
}
