//! Clean-up categories derived from the scan index. Only paths inside the scanned root are reported.
use super::{index::Index, protection, Protection};
use serde::Serialize;
use std::path::{Path, PathBuf};

const MAX_ITEMS: usize = 300;
const LARGE: u64 = 500 << 20;
const OLD_DOWNLOAD: u64 = 50 << 20;
const OLD_SECS: u64 = 90 * 86_400;
const MIN_LOG: u64 = 256 << 10;

#[derive(Serialize, Clone)]
pub struct Item {
    pub path: String,
    pub size: u64,
    pub category: &'static str,
    pub risk: Protection,
    pub protected: bool,
}

#[derive(Serialize)]
pub struct Category {
    pub id: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    pub total: u64,
    pub items: Vec<Item>,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// Package-manager caches (user-owned, re-downloadable).
fn pkg_dirs() -> Vec<PathBuf> {
    let h = home();
    let mut v: Vec<PathBuf> = [".cargo/registry/cache", ".npm/_cacache", ".bun/install/cache", ".cache/pip", ".cache/yarn", ".cache/go-build", ".gradle/caches", "Library/Caches/pip", "Library/Caches/Yarn"].iter().map(|s| h.join(s)).collect();
    if let Some(l) = dirs::cache_dir() {
        v.push(l.join("pip").join("Cache")); // Windows %LOCALAPPDATA%\pip\Cache
    }
    v
}

fn thumb_dirs() -> Vec<PathBuf> {
    let h = home();
    vec![h.join(".cache/thumbnails"), h.join(".thumbnails")]
}

fn trash_dirs() -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![home().join(".Trash")]
    } else if cfg!(windows) {
        vec![]
    } else {
        dirs::data_dir().map(|d| vec![d.join("Trash")]).unwrap_or_default()
    }
}

/// Folders whose *children* are each a cache entry.
fn cache_roots() -> Vec<PathBuf> {
    let mut v = vec![];
    if cfg!(windows) {
        v.extend(dirs::cache_dir().map(|d| d.join("Temp"))); // %LOCALAPPDATA%\Temp
        v.push(std::env::temp_dir());
    } else if cfg!(target_os = "macos") {
        v.push(home().join("Library/Caches"));
    } else {
        v.push(home().join(".cache"));
    }
    v.dedup();
    v
}

/// Build output dir name → markers, any of which must sit next to it (or, for `target`, `CACHEDIR.TAG` inside).
const BUILD: &[(&str, &[&str])] = &[
    ("node_modules", &["package.json"]),
    ("target", &["Cargo.toml"]),
    ("__pycache__", &[]),
    (".gradle", &["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"]),
    (".next", &["package.json"]),
    (".nuxt", &["package.json"]),
    (".parcel-cache", &["package.json"]),
    (".pytest_cache", &[]),
    (".mypy_cache", &[]),
];

fn is_build_dir(p: &Path) -> bool {
    let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let Some((_, markers)) = BUILD.iter().find(|(d, _)| *d == n) else { return false };
    let parent = p.parent().unwrap_or(p);
    // Tool installs (~/.local/share/mise/…/node_modules, ~/.vscode/extensions…) live under dot-folders; projects rarely do.
    let rel = parent.strip_prefix(home()).unwrap_or(parent);
    if rel.components().any(|c| c.as_os_str().to_str().is_some_and(|s| s.starts_with('.'))) {
        return false;
    }
    markers.is_empty() || markers.iter().any(|m| parent.join(m).is_file()) || (n == "target" && p.join("CACHEDIR.TAG").is_file())
}

fn item(ix_size: u64, p: &Path, cat: &'static str, base: Protection) -> Item {
    let prot = protection(p);
    Item { path: p.to_string_lossy().into(), size: ix_size, category: cat, risk: prot.max(base).min(Protection::Review), protected: prot == Protection::Protected }
}

fn cat(id: &'static str, label: &'static str, desc: &'static str, mut items: Vec<Item>) -> Category {
    items.retain(|i| i.size > 0);
    items.sort_by_key(|i| std::cmp::Reverse(i.size));
    items.truncate(MAX_ITEMS);
    Category { id, label, desc, total: items.iter().map(|i| i.size).sum(), items }
}

pub fn categorize(ix: &Index, now: u64) -> Vec<Category> {
    let dir_items = |ps: Vec<PathBuf>, c: &'static str, r: Protection| -> Vec<Item> { ps.into_iter().filter(|p| ix.contains_dir(p)).map(|p| item(ix.total(&p), &p, c, r)).collect() };
    let pkg = pkg_dirs();
    let thumbs = thumb_dirs();
    let elsewhere = |p: &Path| pkg.iter().chain(&thumbs).any(|q| q.starts_with(p) || p.starts_with(q));
    let caches: Vec<Item> = cache_roots()
        .iter()
        .flat_map(|r| ix.children(r, usize::MAX))
        .filter(|n| n.kind != "rest" && !elsewhere(Path::new(&n.path)))
        .map(|n| item(n.size, Path::new(&n.path), "cache", Protection::Safe))
        .collect();
    let names: Vec<&str> = BUILD.iter().map(|b| b.0).collect();
    let mut builds: Vec<&PathBuf> = ix.dirs_named(&names).filter(|p| is_build_dir(p)).collect();
    builds.sort_by_key(|p| p.components().count());
    let mut outer: Vec<&PathBuf> = vec![];
    for b in builds {
        if !outer.iter().any(|o| b.starts_with(o)) {
            outer.push(b);
        }
    }
    let h = home();
    let downloads = dirs::download_dir().unwrap_or_else(|| h.join("Downloads"));
    let file_items = |f: &dyn Fn(&super::index::FileHit) -> bool, c: &'static str| -> Vec<Item> { ix.files.iter().filter(|x| f(x)).map(|x| item(x.size, &x.path, c, Protection::Review)).collect() };
    vec![
        cat("trash", "Trash", "Items you already deleted. Emptying is permanent.", trash_items(ix, dir_items(trash_dirs(), "trash", Protection::Safe))),
        cat("cache", "App caches", "Rebuilt automatically by apps when needed.", caches),
        cat("thumbnails", "Thumbnails", "Image previews; regenerated on demand.", dir_items(thumbs.clone(), "thumbnails", Protection::Safe)),
        cat("packages", "Package caches", "Downloaded packages (cargo, npm, bun, pip…); re-downloaded when needed.", dir_items(pkg.clone(), "packages", Protection::Safe)),
        cat("build", "Build artifacts", "node_modules, target, __pycache__… next to their project files. Rebuild to restore.", outer.into_iter().map(|p| item(ix.total(p), p, "build", Protection::Safe)).collect()),
        cat("logs", "Logs", "Log files in your home folder.", file_items(&|x| x.size >= MIN_LOG && x.path.starts_with(&h) && super::scan::is_log(&x.path), "logs")),
        cat("large", "Large files", "Files over 500 MB. Check before removing.", file_items(&|x| x.size >= LARGE, "large")),
        cat("downloads", "Old downloads", "Downloads over 50 MB untouched for 90+ days.", file_items(&|x| x.size >= OLD_DOWNLOAD && x.path.starts_with(&downloads) && x.mtime > 0 && now.saturating_sub(x.mtime) > OLD_SECS, "downloads")),
    ]
}

/// Windows has no single trash folder to size; ask the trash crate instead.
#[cfg(windows)]
fn trash_items(_ix: &Index, _found: Vec<Item>) -> Vec<Item> {
    let Ok(list) = trash::os_limited::list() else { return vec![] };
    let size: u64 = list.iter().filter_map(|i| trash::os_limited::metadata(i).ok()).filter_map(|m| m.size.size()).sum();
    vec![Item { path: "Recycle Bin".into(), size, category: "trash", risk: Protection::Safe, protected: false }]
}
#[cfg(not(windows))]
fn trash_items(_ix: &Index, found: Vec<Item>) -> Vec<Item> {
    found
}

pub fn is_trash_dir(p: &Path) -> bool {
    trash_dirs().iter().any(|t| t == p)
}
