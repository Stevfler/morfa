//! Names that come from a server are untrusted: they become local paths in the mirror folder.
//! Rel paths are `/`-separated; every segment must be one plain file name.

/// One path segment that can't leave its folder or mean anything but a file name, on this OS.
pub fn safe_name(name: &str) -> bool {
    safe_name_on(name, cfg!(windows))
}

fn safe_name_on(name: &str, windows: bool) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.chars().any(|c| c == '/' || c == '\\' || c.is_control()) {
        return false;
    }
    if !windows {
        return true;
    }
    // Drive prefixes and alternate data streams (":"), reserved characters, and what Windows
    // silently rewrites: "x." and "x " are "x", "NUL.txt" is the NUL device.
    if name.chars().any(|c| matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*')) || name.ends_with(['.', ' ']) {
        return false;
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end().to_ascii_uppercase();
    let numbered = |p: &str| stem.strip_prefix(p).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
    !(matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT"))
}

/// `""` (the account root) or segments that are all `safe_name`.
pub fn safe_rel(rel: &str) -> bool {
    rel.is_empty() || rel.split('/').all(safe_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal() {
        for bad in ["..", "../x", "a/..", "a/../b", ".", "./a", "/etc/passwd", "a//b", "a/", "..\\x", "a\\b", "a\0b", "a\nb"] {
            assert!(!safe_rel(bad), "{bad:?}");
        }
    }

    #[test]
    fn keeps_ordinary_names() {
        for ok in ["", "a", "a/b c/d.txt", ".hidden", "..foo", "foo..", "Привет/мир", "a:b"] {
            assert_eq!(safe_rel(ok), !cfg!(windows) || !ok.contains(':') && !ok.ends_with('.'), "{ok:?}");
        }
    }

    #[test]
    fn windows_rules_hold_on_every_platform() {
        for bad in ["C:", "C:evil", "file.txt:stream", "NUL", "nul.txt", "COM1", "lpt9.log", "x.", "x ", "a?b"] {
            assert!(!safe_name_on(bad, true), "{bad:?}");
        }
        for ok in ["COM10", "console", "a.b", "nullable.txt"] {
            assert!(safe_name_on(ok, true), "{ok:?}");
        }
    }
}
