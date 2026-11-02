//! Pure path classification: what the clean-up tool must never delete.
//! Works on normalised strings so every OS's rules are testable on any host.
use serde::Serialize;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Os {
    Linux,
    Mac,
    Windows,
}

impl Os {
    pub const fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Linux
        }
    }
}

/// Ordered: `Protected` > `Review` > `Safe`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Protection {
    Safe,
    Review,
    Protected,
}

const LINUX_SYS: &[&str] = &["/usr", "/etc", "/boot", "/bin", "/sbin", "/var/lib", "/root", "/proc", "/sys", "/dev", "/run", "/efi"];
const LINUX_REVIEW: &[&str] = &["/opt", "/var", "/srv", "/snap", "/nix"];
const MAC_SYS: &[&str] = &["/system", "/library", "/usr", "/bin", "/sbin", "/private", "/etc", "/var", "/cores", "/dev"];
const MAC_REVIEW: &[&str] = &["/applications", "/opt"];
const WIN_SYS: &[&str] = &["windows", "program files", "program files (x86)", "programdata", "pagefile.sys", "hiberfil.sys", "swapfile.sys", "system volume information", "$recycle.bin", "recovery", "boot", "bootmgr"];
/// Relative to home; secrets and app configuration.
const HOME_SYS: &[&str] = &[".config", ".ssh", ".gnupg", ".local/share/keyrings", ".password-store", ".pki", ".local/share/gnupg", "library/keychains", ".aws", ".kube", "appdata/roaming/microsoft"];
const HOME_REVIEW: &[&str] = &[".local/share", "library", "appdata/roaming", ".mozilla", ".thunderbird"];

/// Normalised components (lowercase on mac/win; `\\` is a separator on Windows).
/// `None` for anything we refuse to reason about: UNC/device paths, `..`, relative paths.
fn parts(p: &str, os: Os) -> Option<Vec<String>> {
    let mut s = if os == Os::Windows { p.replace('\\', "/") } else { p.to_string() };
    if os != Os::Linux {
        s = s.to_lowercase();
    }
    if os == Os::Windows {
        if s.starts_with("//?/unc/") {
            return None;
        }
        if let Some(r) = s.strip_prefix("//?/").or_else(|| s.strip_prefix("//./")) {
            s = r.to_string();
        } else if s.starts_with("//") {
            return None; // \\server\share
        }
        let b = s.as_bytes();
        if b.len() < 2 || b[1] != b':' || !b[0].is_ascii_alphabetic() {
            return None;
        }
    } else if !s.starts_with('/') {
        return None;
    }
    let v: Vec<String> = s.split('/').filter(|c| !c.is_empty() && *c != ".").map(String::from).collect();
    (!v.iter().any(|c| c == "..")).then_some(v)
}

/// Components of a built-in (already lowercase, `/`-separated) relative rule.
fn parts_rel(s: &str) -> Vec<String> {
    s.split('/').filter(|c| !c.is_empty()).map(String::from).collect()
}

fn under(p: &[String], prefix: &[String]) -> bool {
    p.len() >= prefix.len() && p[..prefix.len()] == *prefix
}

/// Classify `path` (absolute) for `os`; `home` is the user's home directory.
/// A path is protected if it is, or contains, a protected location (so `~` and `/` are protected too).
pub fn classify(path: &str, os: Os, home: &str) -> Protection {
    let (Some(p), Some(h)) = (parts(path, os), parts(home, os)) else { return Protection::Protected };
    let abs = |list: &[&str], drive: Option<&String>| -> Vec<Vec<String>> {
        list.iter()
            .map(|s| {
                let mut v: Vec<String> = drive.into_iter().cloned().collect();
                v.extend(parts_rel(s));
                v
            })
            .collect()
    };
    let in_home = |list: &[&str]| -> Vec<Vec<String>> { list.iter().map(|s| h.iter().cloned().chain(parts_rel(s)).collect()).collect() };
    let (mut sys, mut review) = match os {
        Os::Linux => (abs(LINUX_SYS, None), abs(LINUX_REVIEW, None)),
        Os::Mac => (abs(MAC_SYS, None), abs(MAC_REVIEW, None)),
        Os::Windows => (abs(WIN_SYS, p.first()), vec![]),
    };
    sys.extend(in_home(HOME_SYS));
    review.extend(in_home(HOME_REVIEW));
    // Roots: `/`, a drive (`c:`), and the home directory itself.
    if p.is_empty() || (os == Os::Windows && p.len() == 1) || h.starts_with(&p) {
        return Protection::Protected;
    }
    if os == Os::Linux && p.len() == 1 && p[0].starts_with("lib") {
        return Protection::Protected; // /lib, /lib32, /lib64, /libx32
    }
    if sys.iter().any(|s| under(&p, s) || under(s, &p)) {
        return Protection::Protected;
    }
    // ~/Library/Caches and %LOCALAPPDATA%\Temp are the point of the tool.
    let ok_inside = ["library/caches", "appdata/local/temp"].iter().any(|s| under(&p, &h.iter().cloned().chain(parts_rel(s)).collect::<Vec<_>>()));
    if !ok_inside && review.iter().any(|s| under(&p, s)) {
        return Protection::Review;
    }
    Protection::Safe
}

#[cfg(test)]
mod tests {
    use super::{classify, Os::*, Protection::*};

    #[test]
    fn linux() {
        let h = "/home/u";
        for p in ["/", "/usr", "/usr/lib/libc.so", "/etc/fstab", "/boot", "/lib64", "/var/lib/pacman", "/home", "/home/u", "/home/u/.ssh/id_ed25519", "/home/u/.config", "/home/u/.local", "/home/u/.local/share/keyrings/x"] {
            assert_eq!(classify(p, Linux, h), Protected, "{p}");
        }
        for p in ["/opt/app", "/var/cache/x", "/home/u/.local/share/Steam"] {
            assert_eq!(classify(p, Linux, h), Review, "{p}");
        }
        for p in ["/home/u/.cache/foo", "/home/u/Downloads/a.iso", "/home/u/code/node_modules", "/data/x", "/home/u/.local/share/Trash"] {
            assert!(classify(p, Linux, h) <= Review, "{p}");
        }
        assert_eq!(classify("/home/u/.cache", Linux, h), Safe);
        assert_eq!(classify("/home/u/.configs", Linux, h), Safe);
    }

    #[test]
    fn mac() {
        let h = "/Users/u";
        for p in ["/System/Library", "/Library/Fonts", "/usr/bin", "/private/var", "/Users/u", "/Users/u/Library/Keychains/k", "/Users/u/Library"] {
            assert_eq!(classify(p, Mac, h), Protected, "{p}");
        }
        assert_eq!(classify("/Applications/Xcode.app", Mac, h), Review);
        assert_eq!(classify("/Users/u/Library/Caches/com.x", Mac, h), Safe);
        assert_eq!(classify("/Users/u/Library/Application Support/x", Mac, h), Review);
        assert_eq!(classify("/Users/u/Movies/big.mov", Mac, h), Safe);
    }

    #[test]
    fn windows() {
        let h = r"C:\Users\u";
        for p in [r"C:\", "C:", r"C:\Windows\System32", r"c:\program files\x", r"C:\Program Files (x86)", r"C:\ProgramData\x", r"C:\pagefile.sys", r"D:\hiberfil.sys", r"D:\System Volume Information", r"C:\$Recycle.Bin\S-1", r"C:\Users", r"C:\Users\u\.ssh"] {
            assert_eq!(classify(p, Windows, h), Protected, "{p}");
        }
        assert_eq!(classify(r"C:\Users\u\AppData\Local\Temp\x", Windows, h), Safe);
        assert_eq!(classify(r"C:\Users\u\AppData\Roaming\App", Windows, h), Review);
        assert_eq!(classify(r"D:\Games\big.bin", Windows, h), Safe);
    }

    #[test]
    fn normalisation() {
        let w = r"C:\Users\u";
        for p in [r"\\?\C:\Windows", r"\\.\C:\Windows\x", "C:/Windows", r"C:\WINDOWS\", r"\\server\share\x", r"\\?\UNC\srv\s\x", r"C:\Users\u\x\..\..", "relative\\x"] {
            assert_eq!(classify(p, Windows, w), Protected, "{p}");
        }
        let h = "/home/u";
        for p in ["/home/u/", "//usr", "/./usr", "/home/u/.config/..", "/run/media/u/disk", "rel/x", "/home/u/.cache/../.ssh"] {
            assert_eq!(classify(p, Linux, h), Protected, "{p}");
        }
        assert_eq!(classify("/Users/u/library/keychains", Mac, "/Users/u"), Protected);
        assert_eq!(classify("/home/u/.cache//x/./y/", Linux, h), Safe);
    }
}
