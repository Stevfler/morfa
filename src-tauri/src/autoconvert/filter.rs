//! Which files a rule may touch, and when a file has stopped changing. No side effects.
use crate::convert::routes::{self, Via};
use std::path::Path;

/// Unchanged size and mtime for this long, then openable and whole (`complete`).
pub const STABLE_MS: u64 = 3000;
/// For formats that can't say whether they're whole, the quiet period is all there is.
pub const UNVERIFIED_QUIET_MS: u64 = 30_000;

const PARTIAL: [&str; 4] = [".crdownload", ".part", ".download", ".tmp"];

/// Hidden, in progress, or ours. Works on the file name alone.
pub fn ignored_name(name: &str) -> bool {
    let low = name.to_lowercase();
    name.starts_with('.') || name.starts_with("~$") || PARTIAL.iter().any(|s| low.ends_with(s))
}

/// A folder a recursive scan doesn't enter: hidden ones, and Safari's `.download` bundles.
pub fn ignored_dir(name: &str) -> bool {
    ignored_name(name)
}

/// Spellings of one format count as one, so `.heif` next to a HEIC output is "the target format".
pub fn fold_ext(ext: &str) -> String {
    let e = routes::norm(ext);
    if e == "heif" { "heic".into() } else { e }
}

/// What a scan needs from the rule's recipe.
#[derive(Clone, Debug)]
pub struct Spec {
    pub format: String,
    pub next_to_source: bool,
    pub limited: bool,
}

/// Can this recipe take the file? Next to the source, a file already in the target format is refused
/// outright: it may be one of our outputs, and re-encoding it would produce another one to refuse.
pub fn eligible(path: &Path, spec: &Spec, via: &dyn Fn(&str, &str) -> Option<Via>) -> bool {
    let Some(ext) = path.extension().map(|e| e.to_string_lossy()) else { return false };
    let (e, target) = (fold_ext(&ext), fold_ext(&spec.format));
    if spec.next_to_source && e == target {
        return false;
    }
    if spec.limited && !routes::is_image(&routes::norm(&ext)) {
        return false;
    }
    via(&routes::norm(&ext), &routes::norm(&spec.format)).is_some()
}

/// When the file was first seen as it is now: kept while size and mtime hold, restarted otherwise.
pub fn observed_since(prev: Option<(u64, i64, u64)>, size: u64, mtime: i64, now: u64) -> u64 {
    match prev {
        Some((s, m, since)) if s == size && m == mtime => since,
        _ => now,
    }
}

pub fn quiet_for(path: &str) -> u64 {
    let ext = Path::new(path).extension().map(|e| fold_ext(&e.to_string_lossy())).unwrap_or_default();
    if super::complete::checkable(&ext) { STABLE_MS } else { UNVERIFIED_QUIET_MS }
}

/// Has it been quiet long enough? Empty files never get here: they're placeholders, not tracked.
pub fn is_stable(since: u64, quiet: u64, now: u64) -> bool {
    now.saturating_sub(since) >= quiet
}

/// Exactly a temp file of ours: `.morfa-convert-<pid>-<n>.<ext>`.
pub fn temp_owner_pid(name: &str) -> Option<u32> {
    let rest = name.strip_prefix(crate::convert::naming::TEMP_PREFIX)?;
    let (stem, ext) = rest.rsplit_once('.').unwrap_or((rest, ""));
    let (pid, n) = stem.split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(pid) && digits(n) && ext.bytes().all(|b| b.is_ascii_alphanumeric())).then(|| pid.parse().ok())?
}

/// One of our suffixes, whatever made it.
pub fn named_like_output(path: &Path) -> bool {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(open) = stem.strip_suffix(')').and_then(|s| s.rfind(" (converted").map(|i| &stem[i + 11..stem.len() - 1])) else { return false };
    open.is_empty() || open.trim().bytes().all(|b| b.is_ascii_digit())
}

/// The recipe's options for this file, with what doesn't apply to it off (the UI's `gateOpts`).
pub fn gate(opts: &crate::convert::ConvertOpts, ext: &str, via: Option<Via>) -> crate::convert::ConvertOpts {
    let mut o = opts.clone();
    let e = routes::norm(ext);
    match via {
        Some(Via::Image) => {
            if !crate::convert::engine::lossy(&o.format) {
                o.max_bytes = None;
            }
        }
        Some(Via::Video) => {
            o.max_bytes = None;
            if routes::is_audio(&o.format) || !routes::is_video_like(&e) {
                o.resize = None;
            }
        }
        _ => {
            o.max_bytes = None;
            o.resize = None;
            o.strip_metadata = false;
        }
    }
    o
}
