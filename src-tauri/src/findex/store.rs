//! The on-disk form: versioned, checksummed, private and replaced atomically. Only original names
//! are written (folding is redone on load), parents as distances, so it stays compact. Anything
//! unexpected is an `Err`, and the caller rebuilds without telling anyone.
use super::tree::{os_bytes, Tree, DIR, HIDDEN};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 4] = b"MFIX";
/// Bump when the format or `query::fold` changes.
pub const VERSION: u32 = 1;
const HASH: usize = 32;
const BUF: usize = 1 << 20;
/// Far past what MAX_NODES can produce: a bigger file isn't ours, and won't be read into memory.
const MAX_FILE: u64 = 4 << 30;

pub fn file_in(dir: &Path) -> PathBuf {
    dir.join("names.idx")
}
fn tmp_in(dir: &Path) -> PathBuf {
    dir.join("names.idx.tmp")
}

/// Hash of what the index was built from: a file from other settings is not to be used.
pub fn spec_hash(roots: &[PathBuf], excludes: &[String]) -> [u8; HASH] {
    let mut h = Sha256::new();
    h.update(VERSION.to_le_bytes());
    for r in roots {
        h.update(os_bytes(r.as_os_str()));
        h.update([0]);
    }
    h.update([1]);
    for e in excludes {
        h.update(e.as_bytes());
        h.update([0]);
    }
    h.finalize().into()
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

struct Hashed<W: Write> {
    inner: W,
    hash: Sha256,
    written: u64,
}
impl<W: Write> Hashed<W> {
    fn put(&mut self, b: &[u8]) -> std::io::Result<()> {
        self.hash.update(b);
        self.written += b.len() as u64;
        self.inner.write_all(b)
    }
}

fn private_new_file(path: &Path) -> std::io::Result<std::fs::File> {
    let _ = std::fs::remove_file(path);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

/// Bytes written.
pub fn save(tree: &Tree, spec: &[u8; HASH], dir: &Path) -> std::io::Result<u64> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let tmp = tmp_in(dir);
    let res = (|| {
        let file = private_new_file(&tmp)?;
        let mut w = Hashed { inner: std::io::BufWriter::with_capacity(BUF, file), hash: Sha256::new(), written: 0 };
        w.put(MAGIC)?;
        w.put(&VERSION.to_le_bytes())?;
        w.put(spec)?;
        w.put(&(tree.roots.len() as u32).to_le_bytes())?;
        for (r, dev) in &tree.roots {
            let b = os_bytes(r.as_os_str());
            w.put(&(b.len() as u32).to_le_bytes())?;
            w.put(&b)?;
            w.put(&dev.to_le_bytes())?;
        }
        let r = tree.roots.len();
        w.put(&(tree.live_entries() as u64).to_le_bytes())?;
        // Ids change as dead slots drop out; parents always come first, so one pass can renumber.
        let mut map = vec![u32::MAX; tree.slots()];
        for (i, m) in map.iter_mut().enumerate().take(r) {
            *m = i as u32;
        }
        let mut next = r as u32;
        let mut rec = Vec::with_capacity(512);
        for i in r..tree.slots() {
            if !tree.is_live(i) {
                continue;
            }
            let name = tree.raw_name(i);
            rec.clear();
            varint(&mut rec, (next - map[tree.parent(i) as usize]) as u64);
            varint(&mut rec, tree.mtime(i) as u64);
            rec.push(tree.plain_flags(i));
            varint(&mut rec, name.len() as u64);
            rec.extend_from_slice(name);
            w.put(&rec)?;
            map[i] = next;
            next += 1;
        }
        let digest: [u8; HASH] = w.hash.clone().finalize().into();
        w.inner.write_all(&digest)?;
        let size = w.written + HASH as u64;
        let file = w.inner.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        std::fs::rename(&tmp, file_in(dir))?;
        Ok(size)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

struct Cur<'a>(&'a [u8]);
impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if n > self.0.len() {
            return Err("truncated".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| "u32")?))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| "u64")?))
    }
    fn varint(&mut self) -> Result<u64, String> {
        let (mut v, mut shift) = (0u64, 0);
        loop {
            let b = self.take(1)?[0];
            v |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
            if shift > 63 {
                return Err("varint".into());
            }
        }
    }
}

pub fn load(dir: &Path, spec: &[u8; HASH]) -> Result<Tree, String> {
    let size = std::fs::metadata(file_in(dir)).map_err(|e| e.to_string())?.len();
    if size > MAX_FILE {
        return Err("too big".into());
    }
    let data = std::fs::read(file_in(dir)).map_err(|e| e.to_string())?;
    parse(&data, spec)
}

pub fn parse(data: &[u8], spec: &[u8; HASH]) -> Result<Tree, String> {
    if data.len() < HASH + 44 {
        return Err("short".into());
    }
    let (body, sum) = data.split_at(data.len() - HASH);
    if Sha256::digest(body).as_slice() != sum {
        return Err("checksum".into());
    }
    let mut c = Cur(body);
    if c.take(4)? != MAGIC || c.u32()? != VERSION || c.take(HASH)? != spec {
        return Err("other version or settings".into());
    }
    let n_roots = c.u32()? as usize;
    if n_roots > 4096 {
        return Err("roots".into());
    }
    let mut roots = Vec::with_capacity(n_roots);
    for _ in 0..n_roots {
        let len = c.u32()? as usize;
        let p: PathBuf = super::tree::bytes_os(c.take(len)?).into();
        roots.push((p, c.u64()?));
    }
    let count = c.u64()?;
    if count > super::tree::MAX_NODES as u64 {
        return Err("count".into());
    }
    let mut t = Tree::new(roots);
    for _ in 0..count {
        let id = t.slots() as u64;
        let dist = c.varint()?;
        if dist == 0 || dist > id {
            return Err("parent".into());
        }
        let parent = (id - dist) as u32;
        if !t.is_dir(parent) {
            return Err("parent kind".into());
        }
        let mtime = u32::try_from(c.varint()?).map_err(|_| "mtime")?;
        let flags = c.take(1)?[0];
        if flags & !(DIR | HIDDEN) != 0 {
            return Err("flags".into());
        }
        let len = c.varint()? as usize;
        if len == 0 || len > 4096 {
            return Err("name".into());
        }
        let name = c.take(len)?;
        if name == b"." || name == b".." || name.contains(&0) || name.contains(&b'/') || (cfg!(windows) && name.contains(&b'\\')) {
            return Err("name".into());
        }
        t.add(parent, name, mtime, flags);
    }
    if !c.0.is_empty() {
        return Err("trailing".into());
    }
    t.shrink();
    Ok(t)
}

/// Remove the index and anything a crash left next to it.
pub fn delete(dir: &Path) {
    let _ = std::fs::remove_file(file_in(dir));
    let _ = std::fs::remove_file(tmp_in(dir));
    let _ = std::fs::remove_dir(dir);
}
