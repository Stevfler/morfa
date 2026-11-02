//! Pinned LibreOffice build for the Office module: version, download URLs and SHA-256.
//! Hashes are pinned here (copied from TDF's `<file>.sha256` / Metalink), so a compromised
//! mirror can't serve a different file. To bump: change VERSION and the four hashes
//! (`curl -sL <url>.sha256`), then run `cargo test office -- --ignored` to HEAD-check the URLs.

pub const VERSION: &str = "26.8.2";

/// Size estimate for the UI before the download starts (real size comes from Content-Length).
pub const DOWNLOAD_ESTIMATE: u64 = 350 * 1024 * 1024;

const PRIMARY: &str = "https://download.documentfoundation.org/libreoffice/stable";
/// Where TDF moves a version once it leaves `stable/`.
const ARCHIVE: &str = "https://downloadarchive.documentfoundation.org/libreoffice/old";

pub struct Package {
    /// `win/x86_64` style directory under the version.
    pub dir: &'static str,
    pub file: &'static str,
    pub sha256: &'static str,
}

const WIN_X64: Package = Package {
    dir: "win/x86_64",
    file: "LibreOffice_{v}_Win_x86-64.msi",
    sha256: "ff4b911403b67ee9e6249f820f9782c44bb32c94fdaa45b0da53a8e419e5307a",
};
const WIN_ARM: Package = Package {
    dir: "win/aarch64",
    file: "LibreOffice_{v}_Win_aarch64.msi",
    sha256: "73e6a15f5b2b9e6a87ffad7fdaacd6e992a597b08c911ee7e3baddd15e29bcc7",
};
const MAC_X64: Package = Package {
    dir: "mac/x86_64",
    file: "LibreOffice_{v}_MacOS_x86-64.dmg",
    sha256: "0af062779cc17f6f852918b2424c745f7dd2d20d51d7fb8a0925eb92ec0417a7",
};
const MAC_ARM: Package = Package {
    dir: "mac/aarch64",
    file: "LibreOffice_{v}_MacOS_aarch64.dmg",
    sha256: "9f24fe83ab1ed5c95eb3df580119f32932f713e577feeac02bf619fb00749b55",
};

/// Package for an (os, arch) pair as named by `std::env::consts`.
pub fn package_for(os: &str, arch: &str) -> Option<&'static Package> {
    match (os, arch) {
        ("windows", "x86_64") => Some(&WIN_X64),
        ("windows", "aarch64") => Some(&WIN_ARM),
        ("macos", "x86_64") => Some(&MAC_X64),
        ("macos", "aarch64") => Some(&MAC_ARM),
        _ => None,
    }
}

/// Package for this machine (None on Linux: use the distro's LibreOffice).
pub fn package() -> Option<&'static Package> {
    package_for(std::env::consts::OS, std::env::consts::ARCH)
}

impl Package {
    pub fn file_name(&self) -> String {
        self.file.replace("{v}", VERSION)
    }
    /// Candidate URLs in order: mirror redirector, then the long-term archive.
    pub fn urls(&self) -> Vec<String> {
        let f = self.file_name();
        [PRIMARY, ARCHIVE].iter().map(|b| format!("{b}/{VERSION}/{}/{f}", self.dir)).collect()
    }
}
