//! Detects the external image tool (libvips CLI, else ImageMagick) and builds its command lines.
use super::ConvertOpts;
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::OnceLock,
};

pub const NO_ENGINE: &str = "NO_ENGINE: no image converter found — install libvips (vips) or ImageMagick";
const ALL: [&str; 9] = ["jpg", "png", "webp", "avif", "heic", "tiff", "gif", "bmp", "jxl"];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Vips,
    Magick, // IM7 `magick`
    Convert, // IM6 `convert`
}

pub struct Engine {
    pub kind: Kind,
    pub formats: Vec<String>,
}

impl Engine {
    pub fn name(&self) -> &'static str {
        if self.kind == Kind::Vips { "vips" } else { "magick" }
    }
}

fn quiet(cmd: &str) -> Command {
    let mut c = Command::new(cmd);
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    c
}

fn works(cmd: &str, arg: &str) -> bool {
    quiet(cmd).arg(arg).status().map(|s| s.success()).unwrap_or(false)
}

/// Save a tiny generated image to every candidate format; keep those that produce a file.
fn probe(kind: Kind) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("orbit-convert-probe-{}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return vec![];
    }
    let src = dir.join("probe.png");
    let made = match kind {
        Kind::Vips => quiet("vips").arg("black").arg(&src).args(["8", "8"]).status(),
        _ => quiet(cmd_name(kind)).args(["-size", "8x8", "xc:gray"]).arg(&src).status(),
    };
    let mut out = vec![];
    if made.map(|s| s.success()).unwrap_or(false) {
        for f in ALL {
            let dst = dir.join(format!("p.{f}"));
            let ok = match kind {
                Kind::Vips => quiet("vips").arg("copy").arg(&src).arg(&dst).status(),
                _ => quiet(cmd_name(kind)).arg(&src).arg(&dst).status(),
            };
            if ok.map(|s| s.success()).unwrap_or(false) && std::fs::metadata(&dst).map(|m| m.len() > 0).unwrap_or(false) {
                out.push(f.to_string());
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn cmd_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Vips => "vips",
        Kind::Magick => "magick",
        Kind::Convert => "convert",
    }
}

/// Detected once per process.
pub fn engine() -> Option<&'static Engine> {
    static E: OnceLock<Option<Engine>> = OnceLock::new();
    E.get_or_init(|| {
        let kind = if works("vips", "--version") && works("vipsheader", "--version") {
            Kind::Vips
        } else if works("magick", "-version") {
            Kind::Magick
        } else if cfg!(not(windows)) && works("convert", "-version") {
            Kind::Convert // on Windows `convert` is the FAT->NTFS tool
        } else {
            return None;
        };
        let formats = probe(kind);
        (!formats.is_empty()).then_some(Engine { kind, formats })
    })
    .as_ref()
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn animated(src: &Path, fmt: &str) -> bool {
    matches!(ext_of(src).as_str(), "gif" | "webp") && matches!(fmt, "gif" | "webp")
}

fn lossy(fmt: &str) -> bool {
    matches!(fmt, "jpg" | "webp" | "avif" | "heic" | "jxl")
}

/// Longest side (using page height for animations) via vipsheader.
fn vips_long_side(src: &Path) -> Option<u32> {
    let field = |f: &str| -> Option<u32> {
        let o = Command::new("vipsheader").args(["-f", f]).arg(src).stderr(Stdio::null()).output().ok()?;
        String::from_utf8_lossy(&o.stdout).trim().parse().ok()
    };
    let w = field("width")?;
    let h = field("page-height").filter(|&h| h > 0).or_else(|| field("height"))?;
    Some(w.max(h))
}

/// Build the conversion command writing `src` to `dst` (dst's extension selects the saver).
pub fn command(e: &Engine, src: &Path, dst: &Path, o: &ConvertOpts) -> Result<Command, String> {
    let fmt = o.format.as_str();
    let q = o.quality.clamp(1, 100);
    let anim = animated(src, fmt);
    let resize = o.resize.as_ref().filter(|r| r.value > 0);
    if e.kind == Kind::Vips {
        // `thumbnail` auto-rotates, handles animation with n=-1 and never upscales with --size down.
        let size = match resize {
            Some(r) if r.mode == "percent" => {
                let long = vips_long_side(src).ok_or("can't read image size")?;
                ((long as u64 * r.value.min(100) as u64).div_ceil(100)).max(1) as u32
            }
            Some(r) => r.value,
            None => 10_000_000,
        };
        let input = format!("{}{}", src.display(), if anim { "[n=-1]" } else { "" });
        let mut opts = vec![];
        if lossy(fmt) {
            opts.push(format!("Q={q}"));
        }
        if o.strip_metadata {
            opts.push("strip".into());
        }
        let out = if opts.is_empty() { dst.display().to_string() } else { format!("{}[{}]", dst.display(), opts.join(",")) };
        let mut c = Command::new("vips");
        c.arg("thumbnail").arg(input).arg(out).arg(size.to_string()).args(["--height", &size.to_string(), "--size", "down"]);
        return Ok(c);
    }
    let mut c = Command::new(cmd_name(e.kind));
    c.arg(if anim { src.display().to_string() } else { format!("{}[0]", src.display()) });
    if anim {
        c.arg("-coalesce");
    }
    c.arg("-auto-orient");
    match resize {
        Some(r) if r.mode == "percent" => c.args(["-resize", &format!("{}%", r.value.min(100))]),
        Some(r) => c.args(["-resize", &format!("{0}x{0}>", r.value)]),
        None => &mut c,
    };
    if lossy(fmt) {
        c.args(["-quality", &q.to_string()]);
    }
    if o.strip_metadata {
        c.arg("-strip");
    }
    c.arg(dst);
    Ok(c)
}
