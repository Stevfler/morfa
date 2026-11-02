//! In-memory size index of the last scan, served to the space map on demand.
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub struct FileHit {
    pub path: PathBuf,
    pub size: u64,
    /// Seconds since the epoch (0 if unknown).
    pub mtime: u64,
}

#[derive(Default, Clone, Copy)]
struct Dir {
    /// Bytes / file count directly inside.
    own: u64,
    own_files: u64,
    total: u64,
    files: u64,
}

pub struct Index {
    pub root: PathBuf,
    dirs: HashMap<PathBuf, Dir>,
    kids: HashMap<PathBuf, Vec<PathBuf>>,
    /// Individually tracked files (big ones and logs).
    pub files: Vec<FileHit>,
    big: HashMap<PathBuf, Vec<usize>>,
}

/// One bar in the space map. `kind`: "dir" | "file" | "rest" (aggregate of small items).
#[derive(Serialize, Clone)]
pub struct Node {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub files: u64,
    pub kind: &'static str,
}

impl Index {
    pub fn build(root: PathBuf, dirs: Vec<PathBuf>, own: HashMap<PathBuf, (u64, u64)>, files: Vec<FileHit>) -> Index {
        let mut map: HashMap<PathBuf, Dir> = dirs.into_iter().map(|d| (d, Dir::default())).collect();
        map.entry(root.clone()).or_default();
        for (k, (b, f)) in own {
            let d = map.entry(k).or_default();
            d.own = b;
            d.own_files = f;
        }
        // Deepest first, so every child is final before it's added to its parent.
        let mut order: Vec<PathBuf> = map.keys().cloned().collect();
        order.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
        let mut kids: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        for p in order {
            let d = map.get_mut(&p).expect("present");
            d.total += d.own;
            d.files += d.own_files;
            let (t, f) = (d.total, d.files);
            if p == root {
                continue;
            }
            if let Some(parent) = p.parent().filter(|q| map.contains_key(*q)) {
                let pd = map.get_mut(parent).expect("present");
                pd.total += t;
                pd.files += f;
                kids.entry(parent.to_path_buf()).or_default().push(p);
            }
        }
        let mut big: HashMap<PathBuf, Vec<usize>> = HashMap::new();
        for (i, f) in files.iter().enumerate() {
            if f.size >= super::scan::KEEP_FILE {
                if let Some(p) = f.path.parent() {
                    big.entry(p.to_path_buf()).or_default().push(i);
                }
            }
        }
        Index { root, dirs: map, kids, files, big }
    }

    pub fn total(&self, p: &Path) -> u64 {
        self.dirs.get(p).map_or(0, |d| d.total)
    }

    pub fn contains_dir(&self, p: &Path) -> bool {
        self.dirs.contains_key(p)
    }

    pub fn node(&self, p: &Path) -> Node {
        let d = self.dirs.get(p).copied().unwrap_or_default();
        Node { path: p.to_string_lossy().into(), name: name_of(p), size: d.total, files: d.files, kind: "dir" }
    }

    /// Largest first; beyond `limit`, and all small loose files, folded into one "rest" node.
    pub fn children(&self, p: &Path, limit: usize) -> Vec<Node> {
        let mut out: Vec<Node> = self.kids.get(p).into_iter().flatten().map(|k| self.node(k)).collect();
        let d = self.dirs.get(p).copied().unwrap_or_default();
        let (mut loose, mut loose_n) = (d.own, d.own_files);
        for &i in self.big.get(p).into_iter().flatten() {
            let f = &self.files[i];
            loose -= f.size.min(loose);
            loose_n = loose_n.saturating_sub(1);
            out.push(Node { path: f.path.to_string_lossy().into(), name: name_of(&f.path), size: f.size, files: 1, kind: "file" });
        }
        out.sort_by_key(|n| std::cmp::Reverse(n.size));
        for n in out.drain(limit.min(out.len())..).collect::<Vec<_>>() {
            loose += n.size;
            loose_n += n.files;
        }
        if loose_n > 0 {
            out.push(Node { path: p.to_string_lossy().into(), name: format!("{loose_n} smaller items"), size: loose, files: loose_n, kind: "rest" });
        }
        out
    }

    /// Directories whose last component is one of `names` (for build-artifact detection).
    pub fn dirs_named<'a>(&'a self, names: &'a [&str]) -> impl Iterator<Item = &'a PathBuf> + 'a {
        self.dirs.keys().filter(move |p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| names.contains(&n)))
    }
}

fn name_of(p: &Path) -> String {
    p.file_name().map_or_else(|| p.to_string_lossy().into(), |n| n.to_string_lossy().into())
}

impl Index {
    /// Folder total, tracked file size, or the file's length as a fallback.
    pub fn size_of(&self, p: &Path) -> u64 {
        if let Some(d) = self.dirs.get(p) {
            return d.total;
        }
        self.files.iter().find(|f| f.path == p).map(|f| f.size).or_else(|| p.symlink_metadata().ok().map(|m| m.len())).unwrap_or(0)
    }
}

impl Index {
    /// A folder or individually tracked file from this scan (what the UI can list).
    pub fn knows(&self, p: &Path) -> bool {
        self.dirs.contains_key(p) || self.files.iter().any(|f| f.path == p)
    }
}
