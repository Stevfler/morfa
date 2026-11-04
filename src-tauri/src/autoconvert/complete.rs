//! Is a file whole? A quiet period can't tell a paused writer from a finished one, but most formats
//! say where they end. `Unknown` means this format, or this content, offers no such evidence.
use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::Path,
};

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum Verdict {
    Complete,
    Incomplete,
    Unknown,
}
use Verdict::*;

const ZIP_LIKE: [&str; 9] = ["zip", "docx", "xlsx", "pptx", "odt", "ods", "odp", "epub", "cbz"];
const MOVIES: [&str; 7] = ["mp4", "m4v", "mov", "m4a", "3gp", "f4v", "mpeg4"];
const STILLS: [&str; 3] = ["heic", "heif", "avif"];
const RIFF: [&str; 4] = ["webp", "wav", "avi", "ani"];

/// Formats with a check below. The rest (see the README) only get the longer quiet period.
pub fn checkable(ext: &str) -> bool {
    let e = ext;
    matches!(e, "jpg" | "png" | "gif" | "bmp" | "pdf" | "tiff") || ZIP_LIKE.contains(&e) || MOVIES.contains(&e) || STILLS.contains(&e) || RIFF.contains(&e)
}

/// `ext` normalized. `tick` is called as long reads progress. An I/O error says nothing about the format.
pub fn check(path: &Path, ext: &str, tick: &dyn Fn()) -> Verdict {
    let Ok(mut f) = File::open(path) else { return Unknown };
    let Ok(len) = f.metadata().map(|m| m.len()) else { return Unknown };
    let e = ext;
    match e {
        "jpg" => jpeg(&mut f, tick),
        "png" => png(&mut f, len),
        "gif" => gif(&mut f, tick),
        "bmp" => bmp(&mut f, len),
        "pdf" => pdf(&mut f, len),
        "tiff" => tiff(&mut f, len),
        _ if ZIP_LIKE.contains(&e) => zip(&mut f, len),
        _ if MOVIES.contains(&e) => boxes(&mut f, len, b"moov"),
        _ if STILLS.contains(&e) => boxes(&mut f, len, b"meta"),
        _ if RIFF.contains(&e) => riff(&mut f, len),
        _ => Unknown,
    }
}

/// Compares what's there with an expected start: shorter and matching so far means it's cut.
fn magic(f: &mut File, want: &[u8]) -> Option<Verdict> {
    let mut head = vec![0u8; want.len()];
    let _ = f.seek(SeekFrom::Start(0));
    let n = read_up_to(f, &mut head);
    if head[..n] != want[..n] {
        return Some(Unknown); // not what its name says: the engine will report that
    }
    (n < want.len()).then_some(Incomplete)
}

fn read_up_to(f: &mut impl Read, buf: &mut [u8]) -> usize {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    n
}

/// Walks segments, so a thumbnail's end marker or data appended after the image can't mislead it.
fn jpeg(f: &mut File, tick: &dyn Fn()) -> Verdict {
    if let Some(v) = magic(f, &[0xFF, 0xD8]) {
        return v;
    }
    let mut r = BufReader::with_capacity(1 << 16, f);
    let mut byte = || {
        let mut b = [0u8];
        (read_up_to(&mut r, &mut b) == 1).then_some(b[0])
    };
    let mut in_scan = false;
    let mut seen = 0u64;
    loop {
        let Some(b) = byte() else { return Incomplete };
        seen += 1;
        if seen.is_multiple_of(1 << 20) {
            tick();
        }
        if b != 0xFF {
            if in_scan {
                continue;
            }
            return Unknown;
        }
        let mut m = match byte() {
            Some(m) => m,
            None => return Incomplete,
        };
        while m == 0xFF {
            m = match byte() {
                Some(x) => x,
                None => return Incomplete,
            };
        }
        match m {
            0xD9 => return Complete,
            0x00 | 0xD0..=0xD7 | 0x01 => continue,
            _ => {}
        }
        let (Some(a), Some(c)) = (byte(), byte()) else { return Incomplete };
        let size = u16::from_be_bytes([a, c]) as u64;
        if size < 2 {
            return Unknown;
        }
        for _ in 0..size - 2 {
            if byte().is_none() {
                return Incomplete;
            }
        }
        in_scan = m == 0xDA;
    }
}

fn png(f: &mut File, len: u64) -> Verdict {
    if let Some(v) = magic(f, &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return v;
    }
    let mut at = 8u64;
    loop {
        let mut h = [0u8; 8];
        if f.seek(SeekFrom::Start(at)).is_err() || read_up_to(f, &mut h) < 8 {
            return Incomplete;
        }
        let size = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64;
        at += 12 + size;
        if at > len {
            return Incomplete;
        }
        if &h[4..8] == b"IEND" {
            return Complete;
        }
    }
}

fn gif(f: &mut File, tick: &dyn Fn()) -> Verdict {
    if let Some(v) = magic(f, b"GIF8") {
        return v;
    }
    let _ = f.seek(SeekFrom::Start(0));
    let mut r = BufReader::new(f);
    let mut take = |n: usize| {
        let mut b = vec![0u8; n];
        (read_up_to(&mut r, &mut b) == n).then_some(b)
    };
    let Some(h) = take(13) else { return Incomplete };
    let table = |flags: u8| if flags & 0x80 != 0 { 3usize << ((flags & 7) + 1) } else { 0 };
    if take(table(h[10])).is_none() {
        return Incomplete;
    }
    loop {
        tick();
        let Some(k) = take(1) else { return Incomplete };
        match k[0] {
            0x3B => return Complete,
            0x2C => {
                let Some(d) = take(9) else { return Incomplete };
                if take(table(d[8]) + 1).is_none() {
                    return Incomplete;
                }
            }
            0x21 => {
                if take(1).is_none() {
                    return Incomplete;
                }
            }
            _ => return Unknown,
        }
        loop {
            let Some(n) = take(1) else { return Incomplete };
            if n[0] == 0 {
                break;
            }
            if take(n[0] as usize).is_none() {
                return Incomplete;
            }
        }
    }
}

fn bmp(f: &mut File, len: u64) -> Verdict {
    if let Some(v) = magic(f, b"BM") {
        return v;
    }
    let mut s = [0u8; 4];
    if read_up_to(f, &mut s) < 4 {
        return Incomplete;
    }
    let want = u32::from_le_bytes(s) as u64;
    if want == 0 { Unknown } else if len >= want { Complete } else { Incomplete }
}

fn tail(f: &mut File, len: u64, n: u64) -> Vec<u8> {
    let start = len.saturating_sub(n);
    let mut b = vec![0u8; (len - start) as usize];
    let _ = f.seek(SeekFrom::Start(start));
    let got = read_up_to(f, &mut b);
    b.truncate(got);
    b
}

fn pdf(f: &mut File, len: u64) -> Verdict {
    if let Some(v) = magic(f, b"%PDF") {
        return v;
    }
    if tail(f, len, 2048).windows(5).any(|w| w == b"%%EOF") { Complete } else { Incomplete }
}

fn zip(f: &mut File, len: u64) -> Verdict {
    if let Some(v) = magic(f, b"PK") {
        return v;
    }
    if tail(f, len, 65_557).windows(4).any(|w| w == [b'P', b'K', 5, 6]) { Complete } else { Incomplete }
}

fn riff(f: &mut File, len: u64) -> Verdict {
    if let Some(v) = magic(f, b"RIFF") {
        return v;
    }
    let mut s = [0u8; 4];
    if read_up_to(f, &mut s) < 4 {
        return Incomplete;
    }
    let size = u32::from_le_bytes(s) as u64;
    // Streaming writers leave these.
    if size == 0 || size == u32::MAX as u64 {
        return Unknown;
    }
    if len >= size + 8 { Complete } else { Incomplete }
}

/// ISO BMFF: every top-level box fits, and the index box exists.
fn boxes(f: &mut File, len: u64, needed: &[u8; 4]) -> Verdict {
    let mut at = 0u64;
    let mut seen = false;
    while at < len {
        let mut h = [0u8; 16];
        if f.seek(SeekFrom::Start(at)).is_err() || read_up_to(f, &mut h) < 8 {
            return Incomplete;
        }
        if at == 0 && &h[4..8] != b"ftyp" && &h[4..8] != b"moov" && &h[4..8] != b"mdat" && &h[4..8] != b"wide" && &h[4..8] != b"free" {
            return Unknown;
        }
        let mut size = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64;
        if size == 1 {
            size = u64::from_be_bytes([h[8], h[9], h[10], h[11], h[12], h[13], h[14], h[15]]);
        } else if size == 0 {
            size = len - at; // to the end of the file: can't be judged, the index decides
        }
        if size < 8 {
            return Unknown;
        }
        seen |= &h[4..8] == needed;
        at = at.saturating_add(size);
        if at > len {
            return Incomplete;
        }
    }
    if seen { Complete } else { Incomplete }
}

/// Classic TIFF: the chain of directories is all there, and so is the image data they point at.
fn tiff(f: &mut File, len: u64) -> Verdict {
    let mut h = [0u8; 8];
    let _ = f.seek(SeekFrom::Start(0));
    let n = read_up_to(f, &mut h);
    let le = h[..n.min(4)] == [b'I', b'I', 42, 0][..n.min(4)];
    let be = h[..n.min(4)] == [b'M', b'M', 0, 42][..n.min(4)];
    if !le && !be {
        return Unknown; // BigTIFF included
    }
    if n < 8 {
        return Incomplete;
    }
    let u16_at = |b: &[u8]| if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) };
    let u32_at = |b: &[u8]| if le { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) };
    let mut ifd = u32_at(&h[4..8]) as u64;
    let mut dirs = 0;
    while ifd != 0 {
        dirs += 1;
        if dirs > 4096 {
            return Unknown;
        }
        let mut c = [0u8; 2];
        if ifd + 2 > len || f.seek(SeekFrom::Start(ifd)).is_err() || read_up_to(f, &mut c) < 2 {
            return Incomplete;
        }
        let count = u16_at(&c) as usize;
        let mut d = vec![0u8; count * 12 + 4];
        if read_up_to(f, &mut d) < d.len() {
            return Incomplete;
        }
        // Largest single strip or tile end, from the simple one-value layout; arrays are read too.
        let (mut offs, mut sizes) = (vec![], vec![]);
        for e in d[..count * 12].chunks(12) {
            let (tag, typ, cnt) = (u16_at(e), u16_at(&e[2..]), u32_at(&e[4..]) as usize);
            let w = if typ == 3 { 2 } else { 4 };
            if !matches!(tag, 273 | 279 | 324 | 325) || !matches!(typ, 3 | 4) || cnt > 1 << 20 {
                continue;
            }
            let mut raw = vec![0u8; cnt * w];
            if cnt * w <= 4 {
                raw.copy_from_slice(&e[8..8 + cnt * w]);
            } else {
                let at = u32_at(&e[8..]) as u64;
                if at + raw.len() as u64 > len {
                    return Incomplete;
                }
                let back = f.stream_position().unwrap_or(0);
                let _ = f.seek(SeekFrom::Start(at));
                if read_up_to(f, &mut raw) < raw.len() {
                    return Incomplete;
                }
                let _ = f.seek(SeekFrom::Start(back));
            }
            let vals: Vec<u64> = raw.chunks(w).map(|b| if w == 2 { u16_at(b) as u64 } else { u32_at(b) as u64 }).collect();
            if matches!(tag, 273 | 324) { offs = vals } else { sizes = vals }
        }
        if offs.iter().zip(&sizes).any(|(o, s)| o + s > len) {
            return Incomplete;
        }
        ifd = u32_at(&d[count * 12..]) as u64;
    }
    Complete
}
