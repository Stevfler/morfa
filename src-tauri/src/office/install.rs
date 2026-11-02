//! Download -> verify -> unpack -> prune -> (re-sign) -> atomic rename into `<office>`.
//! Everything is built in `<office>.staging`; on failure or cancel the staging dir, the
//! scratch dir and the `.partial` download are deleted, leaving any old install untouched.
use super::{download, extract, manifest, prune, Progress};
use std::path::{Path, PathBuf};

pub struct Paths {
    pub office: PathBuf,
    pub partial: PathBuf,
    pub staging: PathBuf,
    pub scratch: PathBuf,
}

impl Paths {
    pub fn new(office: PathBuf) -> Self {
        let sib = |s: &str| office.with_file_name(format!("office.{s}"));
        Paths { partial: sib("partial"), staging: sib("staging"), scratch: sib("tmp"), office }
    }
    pub fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.partial);
        let _ = std::fs::remove_dir_all(&self.staging);
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

pub fn host_os() -> prune::Os {
    if cfg!(windows) {
        prune::Os::Win
    } else if cfg!(target_os = "macos") {
        prune::Os::Mac
    } else {
        prune::Os::Linux
    }
}

/// Prune root inside the staging dir (the .app bundle on macOS).
fn prune_root(staging: &Path) -> PathBuf {
    if cfg!(target_os = "macos") { staging.join("LibreOffice.app") } else { staging.to_path_buf() }
}

pub fn run(p: &Paths, emit: &dyn Fn(Progress), cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let pkg = manifest::package().ok_or("The Office module isn't offered on this OS")?;
    if let Some(parent) = p.office.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_dir_all(&p.staging);
    let _ = std::fs::remove_dir_all(&p.scratch);

    let mut last = String::new();
    let mut ok = false;
    let mut tick = std::time::Instant::now();
    for url in pkg.urls() {
        let mut progress = |done: u64, total: u64| {
            if tick.elapsed().as_millis() > 150 {
                tick = std::time::Instant::now();
                emit(Progress::new("download", done, total));
            }
        };
        match download::fetch(&url, &p.partial, &mut progress, cancelled) {
            Ok(()) => {
                ok = true;
                break;
            }
            Err(e) if e == "Cancelled" => return Err(e),
            Err(e) => {
                last = e;
                let _ = std::fs::remove_file(&p.partial); // next URL starts fresh
            }
        }
    }
    if !ok {
        return Err(last);
    }
    emit(Progress::new("extract", 0, 0));
    if download::sha256_hex(&p.partial, cancelled)? != pkg.sha256 {
        return Err("downloaded file is corrupt (checksum mismatch)".into());
    }
    // The unpack tools expect the real extension.
    std::fs::create_dir_all(&p.scratch).map_err(|e| e.to_string())?;
    let pkg_file = p.scratch.join(pkg.file_name());
    std::fs::rename(&p.partial, &pkg_file).map_err(|e| e.to_string())?;
    extract::unpack(&pkg_file, &p.scratch.join("x"), &p.staging, cancelled)?;
    let _ = std::fs::remove_dir_all(&p.scratch);
    if cancelled() {
        return Err("Cancelled".into());
    }

    emit(Progress::new("prune", 0, 0));
    prune::apply(&prune_root(&p.staging), host_os()).map_err(|e| format!("prune: {e}"))?;
    extract::finalize(&p.staging, cancelled)?;
    std::fs::write(p.staging.join("morfa-office-version"), manifest::VERSION).map_err(|e| e.to_string())?;
    if super::soffice_in(&p.staging).is_none() {
        return Err("unpacked module has no soffice".into());
    }
    if cancelled() {
        return Err("Cancelled".into());
    }
    // Move the old install aside first: on Windows open files can block deleting it, but not renaming the tree.
    let old = p.office.with_extension("old");
    let _ = std::fs::remove_dir_all(&old);
    if p.office.exists() {
        std::fs::rename(&p.office, &old).map_err(|e| format!("old module is in use: {e}"))?;
    }
    std::fs::rename(&p.staging, &p.office).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}
