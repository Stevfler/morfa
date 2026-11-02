//! Converts a single file: run the engine into a temp file, then publish it.
use super::{engine::{command, Engine}, naming, ConvertOpts};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

/// Convert `src` to `dest` (a temp file is used and removed on any failure). `cancelled` is polled
/// while the child runs; returning true kills it.
pub fn convert_to(e: &Engine, src: &Path, tmp: &Path, o: &ConvertOpts, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let res = run(e, src, tmp, o, cancelled);
    if res.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    res
}

fn run(e: &Engine, src: &Path, tmp: &Path, o: &ConvertOpts, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    if !src.is_file() {
        return Err("not a file".into());
    }
    let mut cmd = command(e, src, tmp, o)?;
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
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
    let status = loop {
        if cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err("Cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(err) => return Err(err.to_string()),
        }
    };
    let err = reader.join().unwrap_or_default();
    let ok = std::fs::metadata(tmp).map(|m| m.len() > 0).unwrap_or(false);
    if status.success() && ok {
        return Ok(());
    }
    let msg = err.lines().map(str::trim).rfind(|l| !l.is_empty() && !l.contains("unable to load")).unwrap_or("conversion failed").to_string();
    Err(msg)
}

/// Full per-file pipeline used by `convert_images`: pick a name, convert, publish.
pub fn convert_file(e: &Engine, src: &Path, o: &ConvertOpts, cancelled: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    let ext = o.format.as_str();
    let dest = naming::dest_for(src, o.out_dir.as_deref(), &o.name_template, ext);
    let tmp = naming::temp_for(&dest);
    convert_to(e, src, &tmp, o, cancelled)?;
    if cancelled() {
        let _ = std::fs::remove_file(&tmp);
        return Err("Cancelled".into());
    }
    naming::publish(&tmp, src, dest, o.out_dir.as_deref(), &o.name_template, ext).map_err(|err| {
        let _ = std::fs::remove_file(&tmp);
        err.to_string()
    })
}

/// Convert `source` into a throwaway temp file and report both sizes.
pub fn estimate(source: &str, mut opts: ConvertOpts) -> Result<super::Estimate, String> {
    let e = super::ready(&mut opts)?;
    let src = PathBuf::from(source);
    let before = std::fs::metadata(&src).map_err(|err| err.to_string())?.len();
    let tmp = naming::temp_for(&std::env::temp_dir().join(format!("e.{}", opts.format)));
    let r = convert_to(e, &src, &tmp, &opts, &|| false).and_then(|_| std::fs::metadata(&tmp).map_err(|err| err.to_string()));
    let _ = std::fs::remove_file(&tmp);
    Ok(super::Estimate { before, after: r?.len() })
}
