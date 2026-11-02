//! File-name validation and race-free "name (n)" creation.
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

const MAX_ATTEMPTS: u32 = 10_000;

/// Rejects names that are invalid on any of Linux, macOS or Windows.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Name can't be empty".into());
    }
    if name == "." || name == ".." {
        return Err(format!("\"{name}\" is not a valid name"));
    }
    if let Some(c) = name.chars().find(|c| matches!(c, '/' | '\\' | '\0' | '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control()) {
        let shown = if c.is_control() { "control characters".to_string() } else { format!("\"{c}\"") };
        return Err(format!("Names can't contain {shown}"));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err("Names can't end with a dot or a space".into());
    }
    let base = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || ["COM", "LPT"].iter().any(|p| base.strip_prefix(p).is_some_and(|d| matches!(d, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")));
    if reserved {
        return Err(format!("\"{name}\" is a reserved name on Windows"));
    }
    Ok(())
}

fn candidate(p: &Path, is_dir: bool, n: u32) -> PathBuf {
    if n < 2 {
        return p.to_path_buf();
    }
    let parent = p.parent().unwrap_or(Path::new(""));
    let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
    let (stem, ext) = match (is_dir, name.rfind('.')) {
        (false, Some(i)) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    parent.join(format!("{stem} ({n}){ext}"))
}

/// Atomically creates `p` (or "p (2)", "p (3)"…) as a new file or dir. Never overwrites.
pub fn create_unique(p: &Path, is_dir: bool) -> Result<PathBuf, String> {
    for n in 1..MAX_ATTEMPTS {
        let c = candidate(p, is_dir, n);
        let res = if is_dir { fs::create_dir(&c) } else { fs::OpenOptions::new().write(true).create_new(true).open(&c).map(|_| ()) };
        match res {
            Ok(()) => return Ok(c),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Couldn't find a free name".into())
}
