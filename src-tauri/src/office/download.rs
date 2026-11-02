//! Streams the installer to `<dest>` (a `.partial` file), resuming with Range after dropped
//! connections, then checks its SHA-256 against the pinned hash.
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
    time::Duration,
};

const RETRIES: usize = 4;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_body(None)
        .user_agent(concat!("Orbit/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Total size from Content-Length (HEAD); None if the server doesn't say.
#[cfg(test)]
pub fn size(url: &str) -> Option<u64> {
    let r = agent().head(url).call().ok()?;
    r.headers().get("content-length")?.to_str().ok()?.parse().ok()
}

/// One attempt: (re)open the stream at the current partial length. Err(true) = retryable.
fn attempt(url: &str, dest: &Path, progress: &mut dyn FnMut(u64, u64), cancelled: &dyn Fn() -> bool) -> Result<(), (bool, String)> {
    let have = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    // identity: byte ranges must refer to the raw file, and some mirrors mislabel gzip.
    let mut req = agent().get(url).header("Accept-Encoding", "identity");
    if have > 0 {
        req = req.header("Range", &format!("bytes={have}-"));
    }
    let mut resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(416)) => return Ok(()), // already complete
        Err(ureq::Error::StatusCode(c)) => return Err((false, format!("download failed (HTTP {c})"))),
        Err(e) => return Err((true, e.to_string())),
    };
    let resumed = resp.status().as_u16() == 206;
    let len: u64 = resp.headers().get("content-length").and_then(|v| v.to_str().ok()?.parse().ok()).unwrap_or(0);
    let mut done = if resumed { have } else { 0 };
    let total = done + len;
    let mut file = OpenOptions::new().create(true).write(true).append(resumed).truncate(!resumed).open(dest).map_err(|e| (false, e.to_string()))?;
    let mut body = resp.body_mut().as_reader();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if cancelled() {
            return Err((false, "Cancelled".into()));
        }
        let n = body.read(&mut buf).map_err(|e| (true, e.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| (false, e.to_string()))?;
        done += n as u64;
        progress(done, total);
    }
    if len > 0 && done < total {
        return Err((true, "connection closed early".into()));
    }
    Ok(())
}

/// Download `url` into `dest`, retrying (with resume) on network errors.
pub fn fetch(url: &str, dest: &Path, progress: &mut dyn FnMut(u64, u64), cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let mut last = String::new();
    for i in 0..RETRIES {
        match attempt(url, dest, progress, cancelled) {
            Ok(()) => return Ok(()),
            Err((false, e)) => return Err(e),
            Err((true, e)) => last = e,
        }
        std::thread::sleep(Duration::from_secs(1 << i));
    }
    Err(format!("download failed: {last}"))
}

pub fn sha256_hex(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        if cancelled() {
            return Err("Cancelled".into());
        }
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
