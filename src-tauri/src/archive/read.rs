//! One streaming walk over every supported format.
use crate::CmdResult;
use std::{fs::File, io::{self, BufReader, Read}, path::Path};

#[derive(Clone)]
pub(crate) struct Item {
    /// Cleaned, '/'-separated, relative. Guaranteed free of `..` and roots.
    pub rel: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    /// Milliseconds since the epoch; 0 if unknown.
    pub modified: u64,
    pub mode: Option<u32>,
}

impl Item {
    pub fn folder(rel: String) -> Self {
        Item { rel, is_dir: true, is_link: false, size: 0, modified: 0, mode: None }
    }
    pub fn name(&self) -> &str {
        self.rel.rsplit('/').next().unwrap_or(&self.rel)
    }
}

pub(crate) trait Visitor {
    fn want(&mut self, item: &Item) -> bool;
    fn take(&mut self, item: &Item, data: &mut dyn Read) -> CmdResult<()>;
    /// True once nothing more is needed (or the job was cancelled): the walk ends early.
    fn finished(&self) -> bool {
        false
    }
}

/// Reads past data nobody wants, stopping as soon as the visitor is finished. Returns false if it did.
fn skip(data: &mut dyn Read, v: &dyn Visitor) -> io::Result<bool> {
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if v.finished() {
            return Ok(false);
        }
        match data.read(&mut buf) {
            Ok(0) => return Ok(true),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

/// Normalise an untrusted entry name; `None` means it tries to leave the extraction root.
pub(crate) fn clean_rel(name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    let mut parts = vec![];
    for seg in name.split('/') {
        match seg {
            "" | "." => {}
            ".." => return None,
            // Drive letters ("C:") and NTFS alternate streams ("a:b") only mean something on Windows.
            s if cfg!(windows) && s.contains(':') => return None,
            s if s.contains('\0') => return None,
            s => parts.push(s),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

const COMPRESSED_TARS: [&str; 6] = [".tar.gz", ".tgz", ".tar.bz2", ".tbz2", ".tar.xz", ".txz"];

pub(crate) fn walk(path: &Path, v: &mut dyn Visitor) -> CmdResult<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    let open = || File::open(path).map(BufReader::new).map_err(|e| format!("{}: {e}", path.display()));
    if name.ends_with(".zip") {
        zip_walk(open()?, v)
    } else if name.ends_with(".7z") {
        sevenz_walk(path, v)
    } else if name.ends_with(".tar") {
        tar_walk(open()?, v)
    } else if name.ends_with(".tar.zst") || name.ends_with(".tzst") {
        tar_walk(ruzstd::decoding::StreamingDecoder::new(open()?).map_err(|e| e.to_string())?, v)
    } else if COMPRESSED_TARS[..2].iter().any(|x| name.ends_with(x)) {
        tar_walk(flate2::bufread::MultiGzDecoder::new(open()?), v)
    } else if COMPRESSED_TARS[2..4].iter().any(|x| name.ends_with(x)) {
        tar_walk(bzip2::bufread::MultiBzDecoder::new(open()?), v)
    } else if COMPRESSED_TARS[4..].iter().any(|x| name.ends_with(x)) {
        tar_walk(lzma_rust2::XzReader::new(open()?, true), v)
    } else {
        Err("This archive format isn't supported".into())
    }
}

fn tar_walk(data: impl Read, v: &mut dyn Visitor) -> CmdResult<()> {
    let mut tar = tar::Archive::new(data);
    for entry in tar.entries().map_err(|e| e.to_string())? {
        if v.finished() {
            break;
        }
        let mut entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.header().entry_type();
        let (is_dir, is_link) = (kind.is_dir(), kind.is_symlink() || kind.is_hard_link());
        // Devices and FIFOs have no place in a file manager's extraction.
        if !(is_dir || is_link || kind.is_file() || kind.is_gnu_sparse() || kind.is_contiguous()) {
            continue;
        }
        let raw = entry.path_bytes();
        let Some(rel) = clean_rel(&String::from_utf8_lossy(&raw)) else { continue };
        let h = entry.header();
        let item = Item { rel, is_dir, is_link, size: if is_dir || is_link { 0 } else { entry.size() }, modified: h.mtime().unwrap_or(0).saturating_mul(1000), mode: h.mode().ok() };
        if v.want(&item) {
            v.take(&item, &mut entry)?;
        } else if !skip(&mut entry, v).map_err(|e| e.to_string())? {
            break;
        }
    }
    Ok(())
}

fn zip_walk(data: BufReader<File>, v: &mut dyn Visitor) -> CmdResult<()> {
    let mut zip = zip::ZipArchive::new(data).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        if v.finished() {
            break;
        }
        let item = {
            let f = zip.by_index_raw(i).map_err(|e| e.to_string())?;
            let name = f.name().map_err(|e| e.to_string())?;
            let Some(rel) = clean_rel(&name) else { continue };
            let is_dir = f.is_dir();
            let d = f.last_modified();
            let modified = d.map_or(0, |d| epoch_ms(d.year() as i64, d.month() as i64, d.day() as i64, d.hour() as i64, d.minute() as i64, d.second() as i64));
            Item { rel, is_dir, is_link: f.is_symlink(), size: f.size(), modified, mode: f.unix_mode() }
        };
        if v.want(&item) {
            let mut f = zip.by_index(i).map_err(|e| match e {
                zip::result::ZipError::UnsupportedArchive(m) if m.to_lowercase().contains("password") => "Password-protected archives aren't supported yet".to_string(),
                e => format!("{}: {e}", item.rel),
            })?;
            v.take(&item, &mut f)?;
        }
    }
    Ok(())
}

fn sevenz_walk(path: &Path, v: &mut dyn Visitor) -> CmdResult<()> {
    let err = |e: sevenz_rust2::Error| {
        let s = e.to_string();
        if s.to_lowercase().contains("password") { "Password-protected archives aren't supported yet".to_string() } else { s }
    };
    let mut reader = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty()).map_err(err)?;
    // Decoding is solid-block based, so announce entries first and only decode when something is wanted.
    let items: Vec<(String, Item)> = reader
        .archive()
        .files
        .iter()
        .filter(|f| !f.is_anti_item)
        .filter_map(|f| {
            let rel = clean_rel(&f.name)?;
            let modified = if f.has_last_modified_date { std::time::SystemTime::from(f.last_modified_date).duration_since(std::time::UNIX_EPOCH).ok().map_or(0, |d| d.as_millis() as u64) } else { 0 };
            Some((f.name.clone(), Item { rel, is_dir: f.is_directory, is_link: false, size: f.size, modified, mode: None }))
        })
        .collect();
    let mut wanted = std::collections::HashMap::new();
    for (raw, item) in items {
        if v.want(&item) {
            wanted.insert(raw, item);
        }
    }
    if wanted.is_empty() {
        return Ok(());
    }
    let mut failure = None;
    let run = reader.for_each_entries(|entry, data| {
        if let Some(item) = wanted.get(&entry.name) {
            if let Err(e) = v.take(item, data) {
                failure = Some(e);
                return Ok(false);
            }
            Ok(!v.finished())
        } else {
            // The library needs each stream consumed to keep its position in solid blocks.
            skip(data, &*v).map_err(sevenz_rust2::Error::from)
        }
    });
    if let Some(e) = failure {
        return Err(e);
    }
    run.map_err(err)
}

/// Civil date (treated as UTC; zip stores no time zone) to epoch milliseconds.
fn epoch_ms(y: i64, m: i64, d: i64, h: i64, min: i64, s: i64) -> u64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    ((days * 86_400 + h * 3600 + min * 60 + s).max(0) as u64) * 1000
}
