//! Video/audio conversion via ffmpeg: build args, run with progress, publish like images.
pub mod args;
pub mod run;

use super::{naming, ConvertOpts};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::OnceLock,
};

/// ffmpeg found (bundled or on PATH), detected once.
pub fn available() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    *F.get_or_init(|| {
        let mut c = crate::sidecar::command("ffmpeg");
        c.arg("-version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        c.status().is_ok_and(|s| s.success())
    })
}

/// Convert `src` to `o.format`; `part` gets the current file's progress (0..1, throttled).
pub fn convert(src: &Path, o: &ConvertOpts, cancelled: &dyn Fn() -> bool, part: &dyn Fn(f64)) -> Result<PathBuf, String> {
    if !available() {
        return Err("NO_ENGINE:ffmpeg".into());
    }
    if !src.is_file() {
        return Err("not a file".into());
    }
    let ext = o.format.as_str();
    let dest = naming::dest_for(src, o.out_dir.as_deref(), &o.name_template, ext);
    let tmp = naming::temp_for(&dest); // keeps the extension, so ffmpeg picks the muxer
    let mut cmd = crate::sidecar::command("ffmpeg");
    cmd.args(args::build(src, &tmp, o));
    let res = run::run(cmd, cancelled, part).and_then(|_| {
        if cancelled() {
            Err("Cancelled".into())
        } else if super::proc::nonempty(&tmp) {
            Ok(())
        } else {
            Err("conversion failed".into())
        }
    });
    if let Err(err) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    naming::publish(&tmp, src, dest, o.out_dir.as_deref(), &o.name_template, ext).map_err(|err| {
        let _ = std::fs::remove_file(&tmp);
        err.to_string()
    })
}
