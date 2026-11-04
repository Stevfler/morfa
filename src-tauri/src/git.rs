//! The branch a folder's git repository is on, for the badge on its row. Reads `HEAD` directly: this
//! runs per visible folder, so no process per row, and it works without git installed. Only a folder
//! that is itself a repository's root counts, and shares are skipped, as this is decoration.
use crate::{netfs, tags, CmdResult};
use std::{collections::HashMap, io::Read, path::Path};

/// Longer than any `HEAD` or gitfile git writes; a planted huge file isn't read in full.
const MAX_READ: u64 = 4096;

fn read_small(path: &Path) -> Option<String> {
    let mut s = String::new();
    std::fs::File::open(path).ok()?.take(MAX_READ).read_to_string(&mut s).ok()?;
    Some(s)
}

/// Branch name from `HEAD`'s contents, or a short hash when detached.
fn parse_head(head: &str) -> Option<String> {
    let head = head.trim();
    if let Some(target) = head.strip_prefix("ref:") {
        let target = target.trim();
        let name = target.strip_prefix("refs/heads/").or_else(|| target.strip_prefix("refs/")).unwrap_or(target);
        return (!name.is_empty()).then(|| name.to_string());
    }
    (matches!(head.len(), 40 | 64) && head.bytes().all(|b| b.is_ascii_hexdigit())).then(|| head[..7].to_string())
}

/// `.git` is a directory, or in a linked worktree or submodule a file pointing to one.
fn branch_of(dir: &Path) -> Option<String> {
    let dot_git = dir.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let file = read_small(&dot_git)?;
        dir.join(file.trim().strip_prefix("gitdir:")?.trim())
    };
    parse_head(&read_small(&git_dir.join("HEAD"))?)
}

#[tauri::command]
pub async fn git_branches(paths: Vec<String>) -> CmdResult<HashMap<String, String>> {
    let paths: Vec<String> = paths.into_iter().filter(|p| netfs::network_mount_of(Path::new(p)).is_none()).collect();
    tauri::async_runtime::spawn_blocking(move || {
        tags::batch("git-head", paths, |p| (p.to_string(), branch_of(Path::new(p)))).into_iter().filter_map(|(p, b)| Some((p, b?))).collect()
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("morfa-git-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parses_head() {
        assert_eq!(parse_head("ref: refs/heads/feature/x\n").as_deref(), Some("feature/x"));
        assert_eq!(parse_head(&"a1b2c3d".repeat(10)[..40]).as_deref(), Some("a1b2c3d"));
        assert_eq!(parse_head("ref: "), None);
        assert_eq!(parse_head("not a head"), None);
    }

    #[test]
    fn reads_a_repository() {
        let d = temp("plain");
        assert_eq!(branch_of(&d), None);
        std::fs::create_dir(d.join(".git")).unwrap();
        assert_eq!(branch_of(&d), None);
        std::fs::write(d.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(branch_of(&d).as_deref(), Some("main"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn follows_a_gitfile() {
        let d = temp("worktree");
        std::fs::create_dir_all(d.join("store/wt")).unwrap();
        std::fs::write(d.join("store/wt/HEAD"), "ref: refs/heads/topic\n").unwrap();
        std::fs::create_dir(d.join("checkout")).unwrap();
        std::fs::write(d.join("checkout/.git"), "gitdir: ../store/wt\n").unwrap();
        assert_eq!(branch_of(&d.join("checkout")).as_deref(), Some("topic"));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
