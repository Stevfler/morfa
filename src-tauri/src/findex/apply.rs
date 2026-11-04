//! Bring the index in line with the disk for paths something told us changed. There's no pairing of
//! rename events to rely on (FSEvents doesn't give any): every path is just looked at again, so a
//! rename is the old name gone and the new one found. A path in `deep` was created or renamed, so
//! nothing known below it can be trusted (`mv a t; mv b a` leaves `a/sub` naming another folder) and
//! it is walked again whole. Disk work happens outside the write lock.
use super::{
    tree::{os_bytes, Tree},
    walk::{self, Ctl, Rules},
};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{atomic::AtomicU64, RwLock},
};

/// This many changes in one folder: list it once rather than look at each (sibling lookups are linear).
const LIST_INSTEAD: usize = 64;

type Lock = RwLock<Option<Tree>>;
fn read(l: &Lock) -> std::sync::RwLockReadGuard<'_, Option<Tree>> {
    l.read().unwrap_or_else(|e| e.into_inner())
}
fn write(l: &Lock) -> std::sync::RwLockWriteGuard<'_, Option<Tree>> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

pub fn apply(lock: &Lock, rules: &Rules, paths: &BTreeSet<PathBuf>, deep: &BTreeSet<PathBuf>, stop: &(dyn Fn() -> bool + Sync)) {
    let mut per_dir: HashMap<&Path, usize> = HashMap::new();
    for p in paths {
        if let Some(d) = p.parent() {
            *per_dir.entry(d).or_default() += 1;
        }
    }
    let crowded: BTreeSet<PathBuf> = per_dir.iter().filter(|(_, n)| **n >= LIST_INSTEAD).map(|(d, _)| d.to_path_buf()).collect();
    for d in &crowded {
        if stop() {
            return;
        }
        refresh(lock, rules, d, deep.contains(d), stop);
    }
    for p in paths {
        if stop() {
            return;
        }
        let is_deep = deep.contains(p);
        // Listing the parent settled this one, unless it's a folder whose insides may have changed.
        let settled = p.parent().is_some_and(|d| crowded.contains(d)) && !(is_deep && p.symlink_metadata().is_ok_and(|m| m.is_dir()));
        if !settled {
            refresh(lock, rules, p, is_deep, stop);
        }
    }
    let mut g = write(lock);
    if let Some(t) = g.as_mut().filter(|t| t.wants_compaction()) {
        t.compact();
    }
}

fn root_info(lock: &Lock, path: &Path) -> Option<(PathBuf, u64)> {
    let g = read(lock);
    let t = g.as_ref()?;
    t.root_of(path).map(|r| t.roots[r].clone())
}

/// Look at one path again.
pub fn refresh(lock: &Lock, rules: &Rules, path: &Path, deep: bool, stop: &(dyn Fn() -> bool + Sync)) {
    let Some((root, dev)) = root_info(lock, path) else { return };
    if path == root {
        return sync_dir(lock, rules, path, &root, dev, stop);
    }
    if !rules.allows(path, &root) {
        return;
    }
    let meta = std::fs::symlink_metadata(path).ok();
    let is_dir = meta.as_ref().is_some_and(|m| m.file_type().is_dir());
    let enterable = meta.as_ref().is_some_and(|m| !is_dir || !(rules.crosses(path, &root, walk::dev_of(m), dev) || walk::online_only(m)));
    let Some(m) = meta.filter(|_| enterable) else {
        // Gone, or a folder that has become a mount point.
        let mut g = write(lock);
        if let Some(t) = g.as_mut() {
            if let Some(id) = t.lookup(path) {
                t.remove(id);
            }
        }
        return;
    };
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return };
    let existing = read(lock).as_ref().and_then(|t| t.lookup(path));
    if existing.is_none() && read(lock).as_ref().and_then(|t| t.lookup(parent)).is_none() {
        // A parent we haven't heard about: looking at it picks up this path as well.
        return refresh(lock, rules, parent, false, stop);
    }
    let raw = os_bytes(name).into_owned();
    let (mtime, flags) = (walk::mtime_of(&m), walk::flags_of(&raw, &m));
    if let Some(id) = existing {
        let same_kind = read(lock).as_ref().is_some_and(|t| t.is_dir(id) == is_dir);
        if same_kind && is_dir && deep {
            return add_new(lock, rules, parent, path, &raw, mtime, flags, true, &root, dev, true, stop);
        }
        if same_kind {
            if let Some(t) = write(lock).as_mut() {
                t.touch(id, mtime, flags);
            }
            if is_dir {
                sync_dir(lock, rules, path, &root, dev, stop);
            }
            return;
        }
        if let Some(t) = write(lock).as_mut() {
            t.remove(id);
        }
    }
    add_new(lock, rules, parent, path, &raw, mtime, flags, is_dir, &root, dev, false, stop);
}

#[allow(clippy::too_many_arguments)]
fn add_new(lock: &Lock, rules: &Rules, parent: &Path, path: &Path, raw: &[u8], mtime: u32, flags: u8, is_dir: bool, root: &Path, dev: u64, replace: bool, stop: &(dyn Fn() -> bool + Sync)) {
    let sub = if is_dir {
        // Walked on the side: a big folder moved in must not hold up searches.
        let mut small = Tree::new(vec![(path.to_path_buf(), dev)]);
        let seen = AtomicU64::new(0);
        let ctl = Ctl { stop, paused: &|| false, seen: &seen, throttle: true };
        if !walk::walk_into(&mut small, 0, path, root, dev, rules, &ctl) {
            return;
        }
        Some(small)
    } else {
        None
    };
    let mut g = write(lock);
    let Some(t) = g.as_mut() else { return };
    let Some(pid) = t.lookup(parent) else { return };
    if let Some(old) = t.child(pid, raw) {
        if !replace {
            return;
        }
        t.remove(old); // only now: searches kept the old contents while the new ones were walked
    }
    if t.is_full() {
        return;
    }
    match sub {
        Some(s) => t.graft(pid, raw, mtime, flags, &s),
        None => {
            t.add(pid, raw, mtime, flags);
        }
    }
}

/// Make one folder's direct children match its listing. Subfolders already known aren't descended.
fn sync_dir(lock: &Lock, rules: &Rules, dir: &Path, root: &Path, dev: u64, stop: &(dyn Fn() -> bool + Sync)) {
    let Ok(list) = std::fs::read_dir(dir) else { return };
    let mut found: HashMap<Vec<u8>, std::fs::Metadata> = HashMap::new();
    for e in list.flatten() {
        let p = dir.join(e.file_name());
        if rules.excluded(&p) {
            continue;
        }
        if let Ok(m) = e.metadata() {
            if !(m.file_type().is_dir() && (rules.crosses(&p, root, walk::dev_of(&m), dev) || walk::online_only(&m))) {
                found.insert(os_bytes(&e.file_name()).into_owned(), m);
            }
        }
    }
    let mut fresh = vec![];
    {
        let mut g = write(lock);
        let Some(t) = g.as_mut() else { return };
        let Some(id) = t.lookup(dir) else { return };
        let known: Vec<(u32, Vec<u8>)> = t.children(id).map(|c| (c, t.raw_name(c as usize).to_vec())).collect();
        for (c, name) in known {
            match found.get(&name) {
                Some(m) if m.file_type().is_dir() == t.is_dir(c) => {
                    t.touch(c, walk::mtime_of(m), walk::flags_of(&name, m));
                    found.remove(&name);
                }
                _ => t.remove(c),
            }
        }
        for (name, m) in found {
            if m.file_type().is_dir() {
                fresh.push((name, m));
            } else if !t.is_full() {
                t.add(id, &name, walk::mtime_of(&m), walk::flags_of(&name, &m));
            }
        }
    }
    for (name, m) in fresh {
        if stop() {
            return;
        }
        let p = dir.join(super::tree::bytes_os(&name));
        add_new(lock, rules, dir, &p, &name, walk::mtime_of(&m), walk::flags_of(&name, &m), true, root, dev, false, stop);
    }
}
