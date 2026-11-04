//! Converts a single file: run the engine into a temp file, then publish it.
use super::{engine::{command, lossy, Engine}, fit, naming, ConvertOpts};
use std::path::{Path, PathBuf};

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
    super::proc::run(command(e, src, tmp, o)?, cancelled, None)?;
    if super::proc::nonempty(tmp) { Ok(()) } else { Err("conversion failed".into()) }
}

/// Full per-file pipeline used by `convert_images`: pick a name, convert, publish.
pub fn convert_file(e: &Engine, src: &Path, o: &ConvertOpts, cancelled: &dyn Fn() -> bool, part: &dyn Fn(f64)) -> Result<PathBuf, String> {
    let ext = o.format.as_str();
    let dest = naming::dest_for(src, o.out_dir.as_deref(), &o.name_template, ext);
    let tmp = match o.max_bytes {
        Some(limit) => {
            let mut temp = || naming::temp_for(&dest);
            let mut encode = |quality: u8, tmp: &Path| convert_to(e, src, tmp, &ConvertOpts { quality, ..o.clone() }, cancelled);
            fit::search(o.quality, limit, lossy(ext), &mut temp, &mut encode, cancelled, part)?.tmp
        }
        None => {
            let tmp = naming::temp_for(&dest);
            convert_to(e, src, &tmp, o, cancelled)?;
            tmp
        }
    };
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
