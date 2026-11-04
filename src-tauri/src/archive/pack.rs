//! Create zip and tar.gz archives.
use super::{place, Job, Staging, CANCELLED};
use crate::CmdResult;
use std::{
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

struct Src {
    path: PathBuf,
    rel: String,
    meta: fs::Metadata,
}

fn collect(path: &Path, rel: String, skip: &Path, out: &mut Vec<Src>, job: &Job) -> CmdResult<()> {
    if job.is_cancelled() {
        return Err(CANCELLED.into());
    }
    if path == skip {
        return Ok(());
    }
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (is_dir, plain) = (meta.is_dir(), meta.is_file() || meta.file_type().is_symlink());
    if !is_dir && !plain {
        return Ok(()); // sockets, FIFOs and devices have no archive form
    }
    out.push(Src { path: path.to_path_buf(), rel: rel.clone(), meta });
    if is_dir {
        let mut kids: Vec<_> = fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        kids.sort_by_key(|e| e.file_name());
        for k in kids {
            collect(&k.path(), format!("{rel}/{}", k.file_name().to_string_lossy()), skip, out, job)?;
        }
    }
    Ok(())
}

/// Reads a file while reporting progress, so a big file can be cancelled midway.
struct Tracked<'a, R> {
    inner: R,
    job: &'a mut Job,
    rel: &'a str,
}
impl<R: Read> Read for Tracked<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.job.is_cancelled() {
            return Err(io::Error::other(CANCELLED));
        }
        let n = self.inner.read(buf)?;
        self.job.done += n as u64;
        self.job.tick(self.rel);
        Ok(n)
    }
}

fn write_tar_gz(file: File, items: &[Src], job: &mut Job) -> CmdResult<()> {
    let gz = flate2::write::GzEncoder::new(BufWriter::new(file), flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.follow_symlinks(false);
    for s in items {
        let e = |e: io::Error| format!("{}: {e}", s.rel);
        if s.meta.is_file() {
            let mut h = tar::Header::new_gnu();
            h.set_metadata(&s.meta);
            let data = File::open(&s.path).map_err(e)?;
            tar.append_data(&mut h, &s.rel, Tracked { inner: data, job, rel: &s.rel }).map_err(e)?;
        } else {
            tar.append_path_with_name(&s.path, &s.rel).map_err(e)?;
        }
    }
    let gz = tar.into_inner().map_err(|e| e.to_string())?;
    gz.finish().map_err(|e| e.to_string())?.flush().map_err(|e| e.to_string())
}

fn write_zip(file: File, items: &[Src], job: &mut Job) -> CmdResult<()> {
    use zip::write::SimpleFileOptions;
    let mut zip = zip::ZipWriter::new(BufWriter::new(file));
    for s in items {
        let e = |e: zip::result::ZipError| format!("{}: {e}", s.rel);
        let mut o = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(s.meta.len() >= u32::MAX as u64);
        if let Some(t) = s.meta.modified().ok().and_then(zip_time) {
            o = o.last_modified_time(t);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            o = o.unix_permissions(s.meta.permissions().mode() & 0o777);
        }
        if s.meta.file_type().is_symlink() {
            let target = fs::read_link(&s.path).map_err(|e| format!("{}: {e}", s.rel))?;
            zip.add_symlink(s.rel.as_str(), target.to_string_lossy(), o).map_err(e)?;
        } else if s.meta.is_dir() {
            zip.add_directory(s.rel.as_str(), o).map_err(e)?;
        } else {
            zip.start_file(s.rel.as_str(), o).map_err(e)?;
            let mut data = File::open(&s.path).map_err(|e| format!("{}: {e}", s.rel))?;
            job.copy(&mut data, &mut zip, &s.rel).map_err(|e| format!("{}: {e}", s.rel))?;
        }
    }
    let mut w = zip.finish().map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())
}

fn zip_time(t: std::time::SystemTime) -> Option<zip::DateTime> {
    let secs = t.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since the epoch to a civil date.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    zip::DateTime::from_date_and_time(y as u16, m as u8, d as u8, (rem / 3600) as u8, (rem % 3600 / 60) as u8, (rem % 60) as u8).ok()
}

/// Pack `sources` into a new archive in `dest`, returning its path. Never overwrites.
pub(crate) fn pack(job: &mut Job, sources: &[String], dest: &Path, format: &str) -> CmdResult<String> {
    let ext = match format {
        "zip" => "zip",
        "tar.gz" => "tar.gz",
        _ => return Err(format!("Can't create \"{format}\" archives")),
    };
    let part = dest.join(format!(".morfa-pack-{}-{}.part", std::process::id(), job.id));
    let mut items = vec![];
    for s in sources {
        let p = Path::new(s);
        let name = p.file_name().ok_or("bad path")?.to_string_lossy().to_string();
        collect(p, name, &part, &mut items, job)?;
    }
    job.total = items.iter().filter(|s| s.meta.is_file()).map(|s| s.meta.len()).sum();
    let file = fs::OpenOptions::new().write(true).create_new(true).open(&part).map_err(|e| format!("{}: {e}", dest.display()))?;
    let _guard = Staging(part.clone());
    if ext == "zip" { write_zip(file, &items, job)? } else { write_tar_gz(file, &items, job)? }
    if job.is_cancelled() {
        return Err(CANCELLED.into());
    }
    let stem = match sources {
        [one] => {
            let p = Path::new(one);
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            if p.is_dir() { name } else { super::split_name(&name).0.to_string() }
        }
        _ => dest.file_name().map_or("Archive".to_string(), |n| n.to_string_lossy().to_string()),
    };
    Ok(place(&part, dest, &format!("{stem}.{ext}"))?.to_string_lossy().to_string())
}
