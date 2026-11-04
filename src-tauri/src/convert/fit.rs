//! Target file size: searches encoder quality in a bounded number of attempts and keeps the
//! highest quality that fits. The encoder is a parameter so the search is testable without an engine.
use std::path::{Path, PathBuf};

/// Attempts per file, including the probes at the quality ceiling and at the floor.
pub const MAX_ATTEMPTS: u32 = 6;
/// Lowest quality tried. It's probed early: bisecting from the top could use up the attempts
/// before reaching it and call a reachable limit impossible.
const FLOOR: u8 = 1;

/// Prefix of the error for a limit no attempt could meet: `OVER_LIMIT:<smallest size>:<limit>`.
pub const OVER_LIMIT: &str = "OVER_LIMIT";

#[derive(Debug, PartialEq)]
pub struct Found {
    pub tmp: PathBuf,
    pub quality: u8,
    pub size: u64,
}

/// Removes its file on drop unless the file was handed over, so every exit path cleans up.
struct Temp(Option<PathBuf>);

impl Drop for Temp {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

struct Search<'a> {
    limit: u64,
    attempts: u32,
    best: Option<(Temp, u8, u64)>,
    smallest: u64,
    temp: &'a mut dyn FnMut() -> PathBuf,
    encode: &'a mut dyn FnMut(u8, &Path) -> Result<(), String>,
    cancelled: &'a dyn Fn() -> bool,
    progress: &'a dyn Fn(f64),
}

impl Search<'_> {
    /// One encode; a result under the limit replaces the current best. True if it fit.
    fn attempt(&mut self, quality: u8) -> Result<bool, String> {
        if (self.cancelled)() {
            return Err("Cancelled".into());
        }
        let tmp = Temp(Some((self.temp)()));
        let path = tmp.0.clone().unwrap_or_default();
        (self.encode)(quality, &path)?;
        let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
        self.attempts += 1;
        (self.progress)(self.attempts as f64 / MAX_ATTEMPTS as f64);
        self.smallest = self.smallest.min(size);
        if size > self.limit {
            return Ok(false);
        }
        self.best = Some((tmp, quality, size));
        Ok(true)
    }

    fn finish(mut self) -> Result<Found, String> {
        if (self.cancelled)() {
            return Err("Cancelled".into());
        }
        match self.best.take() {
            Some((mut tmp, quality, size)) => Ok(Found { tmp: tmp.0.take().unwrap_or_default(), quality, size }),
            None => Err(format!("{OVER_LIMIT}:{}:{}", self.smallest, self.limit)),
        }
    }
}

/// Best encode of at most `limit` bytes, no better than `ceiling` quality. `lossy` false means
/// quality has no effect on size, so one attempt decides. On success the caller owns `Found::tmp`;
/// on any error (over limit, encoder failure, cancel) no temp file is left.
pub fn search(
    ceiling: u8,
    limit: u64,
    lossy: bool,
    temp: &mut dyn FnMut() -> PathBuf,
    encode: &mut dyn FnMut(u8, &Path) -> Result<(), String>,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(f64),
) -> Result<Found, String> {
    let ceiling = ceiling.clamp(FLOOR, 100);
    let mut s = Search { limit, attempts: 0, best: None, smallest: u64::MAX, temp, encode, cancelled, progress };
    if s.attempt(ceiling)? || !lossy || ceiling == FLOOR || !s.attempt(FLOOR)? {
        return s.finish();
    }
    // Invariant: `low` fits and `high` doesn't, so the best kept is always the highest that fit.
    let (mut low, mut high) = (FLOOR, ceiling);
    while high - low > 1 && s.attempts < MAX_ATTEMPTS {
        let mid = low + (high - low) / 2;
        if s.attempt(mid)? {
            low = mid;
        } else {
            high = mid;
        }
    }
    s.finish()
}

#[cfg(test)]
#[path = "tests_fit.rs"]
mod tests;
