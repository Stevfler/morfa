//! The walk over both trees, and how a pair of items is classified.
use super::{is_own_temp, kind_of, snap, Job, Kind, Snap, CANCELLED, MAX_DEPTH, TOLERANCE_MS, UNREADABLE};
use crate::{
    cloud::sync::{fold_key, Fold},
    fsops::rename::same_file,
    CmdResult,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Read, path::Path, time::Instant};
use unicode_normalization::UnicodeNormalization;

/// Identical items beyond this are only counted: a huge tree mustn't flood the window.
const SAME_LIMIT: usize = 20_000;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Quick,
    Content,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Class {
    OnlyLeft,
    OnlyRight,
    NewerLeft,
    NewerRight,
    /// Sizes or link targets differ and the times don't say which is newer.
    Differs,
    SameSizeDifferent,
    TypeConflict,
    Same,
    Skipped,
}

/// What a result rests on.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Basis {
    Presence,
    SizeTime,
    Sha256,
    Target,
    Type,
    None,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// Spelling on each side; where the item is absent, where it would land.
    pub left_rel: String,
    pub right_rel: String,
    pub class: Class,
    pub basis: Basis,
    pub reason: Option<&'static str>,
    pub left: Option<Snap>,
    pub right: Option<Snap>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CompareResult {
    /// What a plan built on this result names.
    pub scan_id: u64,
    pub left_root: String,
    pub right_root: String,
    pub mode: Mode,
    pub items: Vec<Item>,
    pub same_count: u64,
    pub same_bytes: u64,
    /// Not every identical item is in `items`.
    pub same_truncated: bool,
    pub elapsed_ms: u64,
}

/// Size and time of two regular files; `hashes_equal` is known only in content mode, equal sizes.
pub fn classify_files(l: &Snap, r: &Snap, hashes_equal: Option<bool>) -> (Class, Basis) {
    if l.size == r.size {
        match hashes_equal {
            Some(true) => return (Class::Same, Basis::Sha256),
            Some(false) => return (Class::SameSizeDifferent, Basis::Sha256),
            None => {}
        }
    }
    let close = l.mtime.abs_diff(r.mtime) <= TOLERANCE_MS;
    if close && l.size == r.size {
        (Class::Same, Basis::SizeTime)
    } else if close || l.mtime == 0 || r.mtime == 0 {
        (Class::Differs, Basis::SizeTime)
    } else if l.mtime > r.mtime {
        (Class::NewerLeft, Basis::SizeTime)
    } else {
        (Class::NewerRight, Basis::SizeTime)
    }
}

/// Does this disk treat the two spellings as one name? Asked without writing anything: looks up
/// another spelling of a name that exists. With no usable name, the platform's usual behaviour.
pub fn detect_fold(root: &Path) -> Fold {
    let names: Vec<String> = fs::read_dir(root).into_iter().flatten().flatten().take(500).filter_map(|e| e.file_name().into_string().ok()).collect();
    let same_as = |name: &str, other: String| other != name && same_file(&root.join(name), &root.join(other));
    let swap = |s: &str| s.chars().map(|c| if c.is_lowercase() { c.to_uppercase().next().unwrap_or(c) } else { c.to_lowercase().next().unwrap_or(c) }).collect::<String>();
    let other_form = |s: &str| -> String {
        let nfc: String = s.nfc().collect();
        if nfc == s { s.nfd().collect() } else { nfc }
    };
    let case = names.iter().find(|n| swap(n) != **n).map(|n| same_as(n, swap(n)));
    let unicode = names.iter().find(|n| other_form(n) != **n).map(|n| same_as(n, other_form(n)));
    Fold { case: case.unwrap_or(cfg!(any(target_os = "macos", windows))), unicode: unicode.unwrap_or(cfg!(target_os = "macos")) }
}

/// Names on both sides grouped by what the disks consider one name. More than one spelling on a
/// side means they can't be paired up.
pub fn merge_names(left: Vec<String>, right: Vec<String>, fold: Fold) -> Vec<(Vec<String>, Vec<String>)> {
    let mut groups: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for n in left {
        groups.entry(fold_key(&n, fold)).or_default().0.push(n);
    }
    for n in right {
        groups.entry(fold_key(&n, fold)).or_default().1.push(n);
    }
    groups.into_values().collect()
}

fn join(rel: &str, name: &str) -> String {
    if rel.is_empty() { name.to_string() } else { format!("{rel}/{name}") }
}

/// Streams the file; each chunk counts as progress, and cancels.
fn sha256(p: &Path, job: &Job, on_bytes: &mut dyn FnMut(u64)) -> Result<[u8; 32], String> {
    let mut f = fs::File::open(p).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if job.stopped() {
            return Err(CANCELLED.into());
        }
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(h.finalize().into());
        }
        h.update(&buf[..n]);
        on_bytes(n as u64);
    }
}

struct Walk<'a, 'j> {
    left: &'a Path,
    right: &'a Path,
    mode: Mode,
    fold: Fold,
    job: &'a Job<'j>,
    report: &'a mut dyn FnMut(u64, u64, &str),
    out: CompareResult,
    files: u64,
    bytes: u64,
}

/// `report(files, bytes, current)` is called often; throttling is the caller's.
pub fn scan(left: &Path, right: &Path, mode: Mode, job: &Job, report: &mut dyn FnMut(u64, u64, &str)) -> CmdResult<CompareResult> {
    let (a, b) = (detect_fold(left), detect_fold(right));
    let started = Instant::now();
    let out = CompareResult {
        scan_id: 0,
        left_root: left.to_string_lossy().into(),
        right_root: right.to_string_lossy().into(),
        mode,
        items: vec![],
        same_count: 0,
        same_bytes: 0,
        same_truncated: false,
        elapsed_ms: 0,
    };
    // One folding disk is enough: a copy onto it would land on the other spelling.
    let fold = Fold { case: a.case || b.case, unicode: a.unicode || b.unicode };
    let mut w = Walk { left, right, mode, fold, job, report, out, files: 0, bytes: 0 };
    w.dir("", "", 0)?;
    w.out.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(w.out)
}

/// Names in a folder, or `None` if any part of it can't be read. Names that aren't valid text can't
/// travel to the UI and are returned apart.
fn list(dir: &Path) -> Result<(Vec<String>, Vec<String>), std::io::Error> {
    let (mut ok, mut bad) = (vec![], vec![]);
    for e in fs::read_dir(dir)? {
        let name = e?.file_name();
        if is_own_temp(&name) {
            continue;
        }
        match name.into_string() {
            Ok(n) => ok.push(n),
            Err(n) => bad.push(n.to_string_lossy().into_owned()),
        }
    }
    Ok((ok, bad))
}

fn item(left_rel: String, right_rel: String, class: Class, basis: Basis, reason: Option<&'static str>, left: Option<Snap>, right: Option<Snap>) -> Item {
    Item { left_rel, right_rel, class, basis, reason, left, right }
}

impl Walk<'_, '_> {
    fn push(&mut self, item: Item) {
        if item.class == Class::Same {
            self.out.same_count += 1;
            self.out.same_bytes += item.left.as_ref().map_or(0, |s| s.size);
            if self.out.same_count as usize > SAME_LIMIT {
                self.out.same_truncated = true;
                return;
            }
        }
        self.out.items.push(item);
    }

    fn skip(&mut self, lrel: String, rrel: String, reason: &'static str) {
        self.push(item(lrel, rrel, Class::Skipped, Basis::None, Some(reason), None, None));
    }

    fn dir(&mut self, lrel: &str, rrel: &str, depth: usize) -> CmdResult<()> {
        if self.job.stopped() {
            return Err(CANCELLED.into());
        }
        if depth >= MAX_DEPTH {
            self.skip(lrel.into(), rrel.into(), "tooDeep");
            return Ok(());
        }
        let (ld, rd) = (self.left.join(lrel), self.right.join(rrel));
        // An unreadable side says nothing about what's in it: its contents must not read as missing.
        let (l, r) = match (list(&ld), list(&rd)) {
            (Ok(l), Ok(r)) => (l, r),
            (Err(e), _) | (_, Err(e)) if depth == 0 => return Err(e.to_string()),
            _ => {
                self.skip(lrel.into(), rrel.into(), "unreadable");
                return Ok(());
            }
        };
        for n in l.1 {
            self.skip(join(lrel, &n), join(rrel, &n), "badName");
        }
        for n in r.1 {
            self.skip(join(lrel, &n), join(rrel, &n), "badName");
        }
        for (ls, rs) in merge_names(l.0, r.0, self.fold) {
            if ls.len() > 1 || rs.len() > 1 {
                for n in ls.iter().chain(&rs) {
                    self.skip(join(lrel, n), join(rrel, n), "collision");
                }
                continue;
            }
            let (ln, rn) = (ls.first().or(rs.first()), rs.first().or(ls.first()));
            let (Some(ln), Some(rn)) = (ln, rn) else { continue };
            // Folding is a guess at what the disks do. Spellings are one item only if a disk says so.
            if ln != rn && !same_file(&ld.join(ln), &ld.join(rn)) && !same_file(&rd.join(ln), &rd.join(rn)) {
                self.skip(join(lrel, ln), join(rrel, ln), "nameClash");
                self.skip(join(lrel, rn), join(rrel, rn), "nameClash");
                continue;
            }
            let (a, b) = (join(lrel, ln), join(rrel, rn));
            match self.pair(a.clone(), b.clone(), !ls.is_empty(), !rs.is_empty(), depth) {
                Err(e) if e == UNREADABLE => self.skip(a, b, "unreadable"),
                r => r?,
            }
        }
        Ok(())
    }

    fn pair(&mut self, lrel: String, rrel: String, has_l: bool, has_r: bool, depth: usize) -> CmdResult<()> {
        let (lp, rp) = (self.left.join(&lrel), self.right.join(&rrel));
        self.files += 1;
        (self.report)(self.files, self.bytes, &lrel);
        // A failed look at an item is not its absence.
        let look = |p: &Path| match fs::symlink_metadata(p) {
            Ok(m) => Ok(Some(kind_of(&m))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(UNREADABLE.to_string()),
        };
        let (lk, rk) = (look(&lp)?, look(&rp)?);
        // Listed on one side only, yet the other disk answers to the name: a spelling the folding missed.
        if (!has_l && lk.is_some()) || (!has_r && rk.is_some()) {
            self.skip(lrel, rrel, "nameClash");
            return Ok(());
        }
        match (lk, rk) {
            (None, None) => self.skip(lrel, rrel, "unreadable"),
            (Some(Kind::Special), _) | (_, Some(Kind::Special)) => self.skip(lrel, rrel, "special"),
            // Listed a moment ago and gone now: nothing can be said about it.
            (None, Some(_)) if has_l => self.skip(lrel, rrel, "unreadable"),
            (Some(_), None) if has_r => self.skip(lrel, rrel, "unreadable"),
            (Some(_), None) => {
                let s = self.snap_side(&lp, &lrel, &rrel)?;
                self.push(item(lrel, rrel, Class::OnlyLeft, Basis::Presence, None, s, None));
            }
            (None, Some(_)) => {
                let s = self.snap_side(&rp, &lrel, &rrel)?;
                self.push(item(lrel, rrel, Class::OnlyRight, Basis::Presence, None, None, s));
            }
            (Some(Kind::Dir), Some(Kind::Dir)) => self.dir(&lrel, &rrel, depth + 1)?,
            (Some(a), Some(b)) if a != b => {
                let (x, y) = (self.snap_side(&lp, &lrel, &rrel)?, self.snap_side(&rp, &lrel, &rrel)?);
                self.push(item(lrel, rrel, Class::TypeConflict, Basis::Type, None, x, y));
            }
            (Some(k), Some(_)) => {
                let (x, y) = (self.snap_side(&lp, &lrel, &rrel)?, self.snap_side(&rp, &lrel, &rrel)?);
                let (Some(a), Some(b)) = (&x, &y) else {
                    self.skip(lrel, rrel, "unreadable");
                    return Ok(());
                };
                let (class, basis) = if k == Kind::Symlink {
                    (if a.target == b.target { Class::Same } else { Class::Differs }, Basis::Target)
                } else if self.mode == Mode::Content && a.size == b.size {
                    match self.hash_pair(&lp, &rp) {
                        Ok(eq) => classify_files(a, b, Some(eq)),
                        Err(e) if e == CANCELLED => return Err(e),
                        Err(_) => {
                            self.skip(lrel, rrel, "unreadable");
                            return Ok(());
                        }
                    }
                } else {
                    classify_files(a, b, None)
                };
                self.bytes += a.size;
                self.push(item(lrel, rrel, class, basis, None, x, y));
            }
        }
        Ok(())
    }

    fn hash_pair(&mut self, l: &Path, r: &Path) -> Result<bool, String> {
        let (job, name) = (self.job, l.to_string_lossy().into_owned());
        let (files, bytes, report) = (self.files, &mut self.bytes, &mut *self.report);
        let mut on_bytes = |n: u64| {
            *bytes += n;
            report(files, *bytes, &name);
        };
        Ok(sha256(l, job, &mut on_bytes)? == sha256(r, job, &mut on_bytes)?)
    }

    /// What can't be copied inside a one-sided folder is listed, under its path on each side.
    fn snap_side(&mut self, p: &Path, lrel: &str, rrel: &str) -> CmdResult<Option<Snap>> {
        let mut bad: Vec<(String, &'static str)> = vec![];
        let s = snap(p, self.job, &mut |q, why| {
            if let Ok(sub) = q.strip_prefix(p) {
                bad.push((sub.to_string_lossy().replace('\\', "/"), why));
            }
        })?;
        for (sub, why) in bad {
            let at = |rel: &str| if sub.is_empty() { rel.to_string() } else { join(rel, &sub) };
            self.skip(at(lrel), at(rrel), why);
        }
        Ok(s)
    }
}
