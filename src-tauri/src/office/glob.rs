//! Minimal path globbing for the prune list (no dependency needed).
use std::path::{Path, PathBuf};

/// Glob for one name: `*`, `?`, `[abc]` / `[!abc]` (no ranges needed).
pub fn matches(pat: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|i| go(&p[1..], &n[i..])),
            Some('?') => !n.is_empty() && go(&p[1..], &n[1..]),
            Some('[') => {
                let Some(end) = p.iter().position(|&c| c == ']') else { return false };
                let (neg, set) = if p.get(1) == Some(&'!') { (true, &p[2..end]) } else { (false, &p[1..end]) };
                !n.is_empty() && set.contains(&n[0]) != neg && go(&p[end + 1..], &n[1..])
            }
            Some(&c) => n.first() == Some(&c) && go(&p[1..], &n[1..]),
        }
    }
    go(&pat.chars().collect::<Vec<_>>(), &name.chars().collect::<Vec<_>>())
}

/// Existing paths under `base` matching a `/`-separated glob, each segment matched by `matches`.
pub fn expand(base: &Path, pat: &str) -> Vec<PathBuf> {
    let mut cur = vec![base.to_path_buf()];
    for seg in pat.split('/') {
        let mut next = vec![];
        // Only descend into real directories: a symlink must never lead the prune out of the tree.
        for dir in cur.iter().filter(|d| std::fs::symlink_metadata(d).is_ok_and(|m| m.is_dir())) {
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            next.extend(rd.flatten().filter(|e| matches(seg, &e.file_name().to_string_lossy())).map(|e| e.path()));
        }
        cur = next;
    }
    cur
}
