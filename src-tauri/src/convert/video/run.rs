//! Runs ffmpeg with progress: parses `Duration:` (stderr) and `out_time_us=` (stdout).
use std::{
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
    sync::{atomic::{AtomicU64, Ordering}, Arc, Mutex},
    time::{Duration, Instant},
};

/// `Duration: HH:MM:SS.xx, …` -> microseconds.
pub fn parse_duration(line: &str) -> Option<u64> {
    let t = line.trim().strip_prefix("Duration:")?.trim().split(',').next()?;
    let mut it = t.split(':');
    let (h, m, s): (f64, f64, f64) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    Some(((h * 3600.0 + m * 60.0 + s) * 1e6) as u64)
}

/// `out_time_us=N` / `out_time_ms=N` (both microseconds) -> N.
pub fn parse_out_time(line: &str) -> Option<u64> {
    let l = line.trim();
    l.strip_prefix("out_time_us=").or_else(|| l.strip_prefix("out_time_ms="))?.parse().ok()
}

/// Read `r` line by line (lossy UTF-8; ffmpeg ends progress lines with \r or \n).
fn lines(r: impl Read, mut f: impl FnMut(&str)) {
    let mut r = BufReader::new(r);
    let mut buf = vec![];
    while r.read_until(b'\n', &mut buf).is_ok_and(|n| n > 0) {
        for l in String::from_utf8_lossy(&buf).split('\r') {
            f(l);
        }
        buf.clear();
    }
}

/// Like `proc::run` (process group kill on cancel, last stderr line as error), plus `part` progress.
pub fn run(mut cmd: Command, cancelled: &dyn Fn() -> bool, part: &dyn Fn(f64)) -> Result<(), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn().map_err(|err| format!("can't start converter: {err}"))?;
    let (dur, at) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let last = Arc::new(Mutex::new(String::new()));
    let (out, err) = (child.stdout.take(), child.stderr.take());
    let a = at.clone();
    let t_out = std::thread::spawn(move || out.map(|o| lines(o, |l| if let Some(t) = parse_out_time(l) { a.store(t, Ordering::Relaxed) })));
    let (d, ls) = (dur.clone(), last.clone());
    let t_err = std::thread::spawn(move || {
        err.map(|e| {
            lines(e, |l| {
                if let Some(t) = parse_duration(l) {
                    let _ = d.compare_exchange(0, t, Ordering::Relaxed, Ordering::Relaxed);
                }
                let l = l.trim();
                if !l.is_empty() && l != "Conversion failed!" && !l.starts_with("Exiting normally") {
                    *ls.lock().unwrap_or_else(|x| x.into_inner()) = l.into();
                }
            })
        })
    });
    let (mut tick, mut sent) = (Instant::now(), -1.0);
    let status = loop {
        if cancelled() {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            let _ = (t_out.join(), t_err.join());
            return Err("Cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(err) => return Err(err.to_string()),
        }
        let total = dur.load(Ordering::Relaxed);
        if total > 0 && tick.elapsed() >= Duration::from_millis(250) {
            let f = (at.load(Ordering::Relaxed) as f64 / total as f64).clamp(0.0, 1.0);
            if f != sent {
                part(f);
                sent = f;
            }
            tick = Instant::now();
        }
    };
    let _ = (t_out.join(), t_err.join());
    if status.success() {
        return Ok(());
    }
    let l = last.lock().unwrap_or_else(|x| x.into_inner()).clone();
    Err(if l.is_empty() { "conversion failed".into() } else { l })
}
