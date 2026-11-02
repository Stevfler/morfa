//! Destination naming: never clobber, temp files beside the destination.
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

fn same(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

fn sanitize(s: &str) -> String {
    let s: String = s.chars().map(|c| if matches!(c, '/' | '\\' | '\0') { '_' } else { c }).collect();
    if s.trim().is_empty() || s == "." || s == ".." { "image".into() } else { s }
}

/// First free destination for `src` under the template/format/outDir rules.
pub fn dest_for(src: &Path, out_dir: Option<&str>, template: &str, ext: &str) -> PathBuf {
    let dir = out_dir.map(PathBuf::from).unwrap_or_else(|| src.parent().map(Path::to_path_buf).unwrap_or_default());
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let tpl = if template.trim().is_empty() { "{name}" } else { template };
    let base = sanitize(&tpl.replace("{name}", &stem));
    let free = |p: &PathBuf| !same(p, src) && std::fs::symlink_metadata(p).is_err();
    let first = dir.join(format!("{base}.{ext}"));
    if free(&first) {
        return first;
    }
    for n in 1.. {
        let tag = if n == 1 { "converted".to_string() } else { format!("converted {n}") };
        let p = dir.join(format!("{base} ({tag}).{ext}"));
        if free(&p) {
            return p;
        }
    }
    unreachable!()
}

/// Hidden temp path in the same dir as `dest`, keeping the extension (it selects the saver).
pub fn temp_for(dest: &Path) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let ext = dest.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let n = N.fetch_add(1, Ordering::Relaxed);
    dest.with_file_name(format!(".morfa-convert-{}-{n}.{ext}", std::process::id()))
}

/// Move the finished temp into place without replacing anything; if `dest` got taken meanwhile,
/// pick the next free name. Returns the final path.
pub fn publish(tmp: &Path, src: &Path, mut dest: PathBuf, out_dir: Option<&str>, template: &str, ext: &str) -> std::io::Result<PathBuf> {
    for _ in 0..50 {
        match std::fs::hard_link(tmp, &dest) {
            Ok(()) => {
                let _ = std::fs::remove_file(tmp);
                return Ok(dest);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => dest = dest_for(src, out_dir, template, ext),
            Err(_) => {
                // Filesystem without hard links (FAT, some network mounts): checked rename.
                if std::fs::symlink_metadata(&dest).is_ok() {
                    dest = dest_for(src, out_dir, template, ext);
                    continue;
                }
                std::fs::rename(tmp, &dest)?;
                return Ok(dest);
            }
        }
    }
    Err(std::io::Error::other("couldn't find a free file name"))
}
