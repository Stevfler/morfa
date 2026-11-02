//! Runs an external converter: drains stderr, polls cancel, enforces an optional timeout.
use std::{
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Run `cmd` to completion. On failure returns the last meaningful stderr line.
pub fn run(mut cmd: Command, cancelled: &dyn Fn() -> bool, timeout: Option<Duration>) -> Result<(), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    // Own process group so cancel/timeout also reaches forked helpers (soffice -> oosplash -> soffice.bin).
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn().map_err(|err| format!("can't start converter: {err}"))?;
    let mut stderr = child.stderr.take();
    // Drain stderr on a thread so a chatty tool can't block on a full pipe.
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(r) = stderr.as_mut() {
            let _ = r.read_to_string(&mut s);
        }
        s
    });
    let start = Instant::now();
    let status = loop {
        let timed_out = timeout.is_some_and(|t| start.elapsed() > t);
        if cancelled() || timed_out {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(if timed_out { "Timed out".into() } else { "Cancelled".into() });
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(err) => return Err(err.to_string()),
        }
    };
    let err = reader.join().unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    Err(err.lines().map(str::trim).rfind(|l| !l.is_empty() && !l.contains("unable to load")).unwrap_or("conversion failed").to_string())
}

/// True if `p` exists and is non-empty.
pub fn nonempty(p: &std::path::Path) -> bool {
    std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false)
}
