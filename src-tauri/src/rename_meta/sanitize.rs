//! Turns an untrusted metadata value into text that is safe inside a file name.
use crate::fsops::name_issue;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

/// Bytes one value may take; the whole name is checked against the disk's limit later.
pub const MAX_VALUE_BYTES: usize = 120;

fn replaced(c: char) -> bool {
    matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')
        // Look like a slash, and turn into one in some tools.
        || matches!(c, '\u{2044}' | '\u{2215}' | '\u{29F8}' | '\u{FF0F}' | '\u{FF3C}')
}

/// Invisible or junk characters: controls, format characters (direction overrides can disguise an
/// extension), variation selectors, blank "letters", tag characters and the replacement character.
fn dropped(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{AD}' | '\u{34F}' | '\u{61C}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}' | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{3164}' | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}' | '\u{FFA0}' | '\u{FFF0}'..='\u{FFFD}' | '\u{E0000}'..='\u{E0FFF}')
}

fn cut(s: &mut String, max: usize) {
    if s.len() > max {
        let mut i = max;
        while !s.is_char_boundary(i) {
            i -= 1;
        }
        s.truncate(i);
    }
}

/// `None`: nothing meaningful is left, and the value counts as missing. A value never decides the
/// folder (no separators, no `..`), never starts with a dot or ends with a dot or a space.
pub fn sanitize_value(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len().min(MAX_VALUE_BYTES));
    let mut last_space = true;
    // Bounded first: a tag may hold megabytes, and only the start can survive anyway.
    for c in raw.chars().take(MAX_VALUE_BYTES * 2).nfc() {
        if dropped(c) && !c.is_whitespace() {
            continue;
        }
        if c.is_whitespace() {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
            continue;
        }
        last_space = false;
        if c == '.' && out.ends_with('.') {
            continue;
        }
        out.push(if replaced(c) { '_' } else { c });
    }
    cut(&mut out, MAX_VALUE_BYTES);
    let v = out.trim_matches(|c: char| c == '.' || c == ' ');
    // "___", "★", a lone accent: with no visible letter or digit, a name made of it says nothing.
    if !v.chars().any(|c| c.is_alphanumeric() && !is_combining_mark(c)) {
        return None;
    }
    if name_issue(v).is_some() {
        // Only a reserved word is left to complain about.
        return Some(format!("_{v}"));
    }
    Some(v.to_string())
}

/// `1999:12:31 23:59:58` as `1999-12-31T23:59:58`, exactly as written: EXIF has no time zone.
/// Anything else, including the all-zero and blank dates cameras write, is `None`.
pub fn exif_date(raw: &str) -> Option<String> {
    let b = raw.trim_matches(|c: char| c == '\0' || c == ' ').as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<u32> {
        let s = &b[r];
        s.iter().all(u8::is_ascii_digit).then(|| s.iter().fold(0, |n, d| n * 10 + u32::from(d - b'0')))
    };
    let sep = |i: usize, set: &[u8]| set.contains(&b[i]);
    if !(sep(4, b":-") && sep(7, b":-") && sep(10, b" T") && sep(13, b":") && sep(16, b":")) {
        return None;
    }
    let (y, mo, d, h, mi, s) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    let valid = y >= 1 && (1..=12).contains(&mo) && (1..=31).contains(&d) && h < 24 && mi < 60 && s < 61;
    valid.then(|| format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}"))
}

/// Model alone when it already names the maker ("NIKON CORPORATION" + "NIKON D850").
pub fn camera_name(make: Option<&str>, model: Option<&str>) -> Option<String> {
    let clean = |s: Option<&str>| s.map(|s| s.trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string()).filter(|s| !s.is_empty());
    let joined = match (clean(make), clean(model)) {
        (Some(ma), Some(mo)) => {
            let word = ma.split_whitespace().next().unwrap_or("").to_lowercase();
            if mo.to_lowercase().contains(&word) { mo } else { format!("{ma} {mo}") }
        }
        (a, b) => a.or(b)?,
    };
    sanitize_value(&joined)
}
