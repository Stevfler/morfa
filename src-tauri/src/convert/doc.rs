//! Document conversion via LibreOffice headless (`soffice`) or pandoc.
use super::{naming, proc, routes, ConvertOpts};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
    time::Duration,
};

const OFFICE_TIMEOUT: Duration = Duration::from_secs(120);

fn works(cmd: &str) -> bool {
    Command::new(cmd).arg("--version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// LibreOffice on PATH, detected once.
pub fn system_office() -> Option<&'static str> {
    static B: OnceLock<Option<&'static str>> = OnceLock::new();
    *B.get_or_init(|| ["soffice", "libreoffice"].into_iter().find(|b| works(b)))
}

static OFFICE: Mutex<Option<Option<PathBuf>>> = Mutex::new(None);

/// LibreOffice to use: Morfa's Office module if installed, else PATH. Cached until `invalidate`.
pub fn office() -> Option<PathBuf> {
    let mut g = OFFICE.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(|| crate::office::soffice().or_else(|| system_office().map(PathBuf::from))).clone()
}

/// Re-detect on next use (after the Office module is installed or removed).
pub fn invalidate() {
    *OFFICE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn pandoc() -> bool {
    static P: OnceLock<bool> = OnceLock::new();
    *P.get_or_init(|| works("pandoc"))
}

/// `--convert-to` argument: explicit filters where LibreOffice's default pick is wrong or ambiguous.
fn filter(ext: &str, fmt: &str) -> String {
    let sheet = routes::SHEET_IN.contains(&ext);
    let f = match fmt {
        "csv" => "csv:Text - txt - csv (StarCalc):44,34,76",
        "xlsx" => "xlsx:Calc MS Excel 2007 XML",
        "html" if sheet => "html:HTML (StarCalc)",
        "html" => "html:HTML (StarWriter)",
        "pptx" => "pptx:Impress MS PowerPoint 2007 XML",
        "docx" => "docx:MS Word 2007 XML",
        "txt" => "txt:Text (encoded):UTF8",
        _ => fmt,
    };
    f.into()
}

/// file:// URL for a local path (percent-encodes anything unusual, e.g. spaces).
/// `C:\x` → `file:///C:/x`, `/x` → `file:///x`. A malformed URL makes LibreOffice report "bootstrap.ini is corrupt".
fn file_url(p: &Path) -> String {
    let path = p.to_string_lossy().replace('\\', "/");
    let mut s = String::from(if path.starts_with('/') { "file://" } else { "file:///" });
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// Publish the finished temp `out` (beside `dest`) under the usual naming rules.
fn finish(out: &Path, src: &Path, dest: PathBuf, o: &ConvertOpts, cancelled: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    if !proc::nonempty(out) {
        return Err("conversion failed".into());
    }
    if cancelled() {
        return Err("Cancelled".into());
    }
    naming::publish(out, src, dest, o.out_dir.as_deref(), &o.name_template, &o.format).map_err(|e| e.to_string())
}

/// Convert with LibreOffice using a private per-worker profile (works while the user's LO is open).
pub fn office_convert(src: &Path, o: &ConvertOpts, worker: usize, cancelled: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    let bin = office().ok_or("NO_ENGINE:office")?;
    if !src.is_file() {
        return Err("not a file".into());
    }
    let ext = routes::norm(&src.extension().unwrap_or_default().to_string_lossy());
    let dest = naming::dest_for(src, o.out_dir.as_deref(), &o.name_template, &o.format);
    let work = naming::temp_for(&dest).with_extension("d");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let profile = std::env::temp_dir().join(format!("morfa-lo-{worker}"));
    let mut cmd = Command::new(bin);
    cmd.args(["--headless", "--norestore", "--nolockcheck", "--nodefault", "--nologo"])
        .arg(format!("-env:UserInstallation={}", file_url(&profile)));
    if ext == "html" {
        cmd.arg("--infilter=HTML (StarWriter)"); // open in Writer, not Writer/Web
    }
    cmd.args(["--convert-to", &filter(&ext, &o.format), "--outdir"]).arg(&work).arg(src);
    let stem = src.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let res = proc::run(cmd, cancelled, Some(OFFICE_TIMEOUT))
        .and_then(|_| finish(&work.join(format!("{stem}.{}", o.format)), src, dest, o, cancelled));
    let _ = std::fs::remove_dir_all(&work);
    res
}

fn pandoc_fmt(ext: &str) -> &str {
    match ext {
        "md" | "txt" => "markdown",
        "tex" => "latex",
        e => e,
    }
}

pub fn pandoc_convert(src: &Path, o: &ConvertOpts, cancelled: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    if !pandoc() {
        return Err("NO_ENGINE:pandoc".into());
    }
    if !src.is_file() {
        return Err("not a file".into());
    }
    let ext = routes::norm(&src.extension().unwrap_or_default().to_string_lossy());
    let dest = naming::dest_for(src, o.out_dir.as_deref(), &o.name_template, &o.format);
    let tmp = naming::temp_for(&dest);
    let mut cmd = Command::new("pandoc");
    cmd.args(["-s", "-f", pandoc_fmt(&ext), "-t", pandoc_fmt(&o.format), "-o"]).arg(&tmp).arg(src);
    let res = proc::run(cmd, cancelled, Some(OFFICE_TIMEOUT)).and_then(|_| finish(&tmp, src, dest, o, cancelled));
    let _ = std::fs::remove_file(&tmp);
    res
}
