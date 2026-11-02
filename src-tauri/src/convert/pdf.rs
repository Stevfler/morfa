//! PDF -> one image per page via libvips `pdfload` (poppler).
use super::{naming, proc, ConvertOpts};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
};

const DPI: u32 = 150;

/// libvips with a pdf loader, detected once.
pub fn available() -> bool {
    static A: OnceLock<bool> = OnceLock::new();
    *A.get_or_init(|| {
        let o = Command::new("vips").args(["-l", "foreign"]).stdin(Stdio::null()).stderr(Stdio::null()).output();
        o.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("pdfload"))
    })
}

pub fn page_count(src: &Path) -> Option<u32> {
    let o = Command::new("vipsheader").args(["-f", "n-pages"]).arg(src).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !o.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&o.stdout).trim().parse().unwrap_or(1).max(1))
}

/// Render every page to "<name> p<N>.<fmt>". All-or-nothing: on failure/cancel, published pages are removed.
pub fn convert(src: &Path, o: &ConvertOpts, cancelled: &dyn Fn() -> bool) -> Result<Vec<PathBuf>, String> {
    if !available() {
        return Err(super::engine::NO_ENGINE.into());
    }
    if !src.is_file() {
        return Err("not a file".into());
    }
    let pages = page_count(src).ok_or("can't read PDF")?;
    let tpl = if o.name_template.trim().is_empty() { "{name}" } else { o.name_template.as_str() };
    let mut done = vec![];
    let res = (1..=pages).try_for_each(|n| {
        let page_tpl = format!("{tpl} p{n}");
        let dest = naming::dest_for(src, o.out_dir.as_deref(), &page_tpl, &o.format);
        let tmp = naming::temp_for(&dest);
        let save = if matches!(o.format.as_str(), "jpg" | "webp") {
            format!("{}[Q={}]", tmp.display(), o.quality.clamp(1, 100))
        } else {
            tmp.display().to_string()
        };
        let mut cmd = Command::new("vips");
        cmd.arg("copy").arg(format!("{}[page={},dpi={DPI}]", src.display(), n - 1)).arg(save);
        let r = proc::run(cmd, cancelled, None).and_then(|_| {
            if !proc::nonempty(&tmp) {
                return Err("conversion failed".to_string());
            }
            if cancelled() {
                return Err("Cancelled".into());
            }
            naming::publish(&tmp, src, dest, o.out_dir.as_deref(), &page_tpl, &o.format).map_err(|e| e.to_string())
        });
        let _ = std::fs::remove_file(&tmp);
        done.push(r?);
        Ok::<_, String>(())
    });
    if let Err(e) = res {
        for p in &done {
            let _ = std::fs::remove_file(p);
        }
        return Err(e);
    }
    Ok(done)
}
