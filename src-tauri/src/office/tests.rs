use super::{glob::matches, manifest, prune};
use prune::{pattern, At, Os};
use std::path::Path;

#[test]
fn glob_matching() {
    assert!(matches("images_*", "images_colibre_svg.zip"));
    assert!(matches("lib*.so*", "libfbclient.so.2"));
    assert!(matches("s[bdimw]*.exe", "swriter.exe"));
    assert!(!matches("s[bdimw]*.exe", "soffice.exe"));
    assert!(matches("[!c]*", "de") && !matches("[!c]*", "common"));
    assert!(matches("python*", "python-core-3.11.9") && !matches("*.py", "pyuno.pyd"));
    assert!(!matches("help", "helper") && matches("?elp", "help"));
}

#[test]
fn lib_patterns_per_os() {
    assert_eq!(pattern(At::Lib, "hsqldb", Os::Win), "hsqldb.dll");
    assert_eq!(pattern(At::Lib, "hsqldb", Os::Mac), "libhsqldb.dylib");
    assert_eq!(pattern(At::Lib, "hsqldb", Os::Linux), "libhsqldb.so*");
    assert_eq!(pattern(At::Lib, "LibreOfficePython.framework", Os::Mac), "LibreOfficePython.framework");
    assert_eq!(prune::bases(At::Share, Os::Mac), ["Contents/Resources"]);
    assert_eq!(prune::bases(At::Share, Os::Win), ["share"]);
}

#[test]
fn prune_fake_tree() {
    let root = std::env::temp_dir().join(format!("orbit-prune-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let files = [
        "help/en-US/x.html", "share/extensions/dict-de/a", "share/config/images_colibre.zip",
        "share/config/images_breeze.zip", "share/registry/res/fcfg_langpack_en-US.xcd",
        "share/registry/res/fcfg_langpack_de.xcd", "share/registry/main.xcd", "share/fonts/truetype/a.ttf",
        "program/soffice.exe", "program/swriter.exe", "program/hsqldb.dll", "program/swlo.dll",
        "program/resource/common/x", "program/resource/de/LC_MESSAGES/sw.mo", "program/python-core-3.11/lib",
    ];
    for f in files {
        let p = root.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }
    prune::apply(&root, Os::Win).unwrap();
    let left = |f: &str| root.join(f).exists();
    for kept in ["share/config/images_colibre.zip", "share/registry/res/fcfg_langpack_en-US.xcd", "share/registry/main.xcd",
        "share/fonts/truetype/a.ttf", "program/soffice.exe", "program/swlo.dll", "program/resource/common/x"] {
        assert!(left(kept), "{kept} should stay");
    }
    for gone in ["help", "share/extensions/dict-de", "share/config/images_breeze.zip", "share/registry/res/fcfg_langpack_de.xcd",
        "program/swriter.exe", "program/hsqldb.dll", "program/resource/de", "program/python-core-3.11"] {
        assert!(!left(gone), "{gone} should be pruned");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn manifest_urls() {
    assert!(manifest::package_for("linux", "x86_64").is_none());
    let p = manifest::package_for("windows", "x86_64").unwrap();
    let v = manifest::VERSION;
    assert_eq!(p.urls()[0], format!("https://download.documentfoundation.org/libreoffice/stable/{v}/win/x86_64/LibreOffice_{v}_Win_x86-64.msi"));
    assert!(p.urls()[1].starts_with("https://downloadarchive.documentfoundation.org/libreoffice/old/"));
    let m = manifest::package_for("macos", "aarch64").unwrap();
    assert!(m.urls()[0].ends_with(&format!("mac/aarch64/LibreOffice_{v}_MacOS_aarch64.dmg")));
    for (os, a) in [("windows", "x86_64"), ("windows", "aarch64"), ("macos", "x86_64"), ("macos", "aarch64")] {
        let h = manifest::package_for(os, a).unwrap().sha256;
        assert!(h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}

/// Network: HEAD-check every pinned URL (`cargo test office -- --ignored`). No body is downloaded.
#[test]
#[ignore]
fn manifest_urls_exist() {
    for (os, a) in [("windows", "x86_64"), ("windows", "aarch64"), ("macos", "x86_64"), ("macos", "aarch64")] {
        let url = &manifest::package_for(os, a).unwrap().urls()[0];
        let n = super::download::size(url).unwrap_or(0);
        assert!(n > 100_000_000, "{url}: {n}");
    }
}

/// Prunes a COPY of a real LibreOffice in $ORBIT_PRUNE_DIR (e.g. of /usr/lib/libreoffice) in place.
#[test]
#[ignore]
fn prune_real_install() {
    let Ok(dir) = std::env::var("ORBIT_PRUNE_DIR") else { return };
    prune::apply(Path::new(&dir), Os::Linux).unwrap();
}

/// Network: download a tiny file, then resume it from a truncated partial via Range.
#[test]
#[ignore]
fn download_resumes() {
    let url = format!("{}.sha256", manifest::package_for("windows", "x86_64").unwrap().urls()[0]);
    let dir = std::env::temp_dir().join(format!("orbit-dl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let full = dir.join("full");
    super::download::fetch(&url, &full, &mut |_, _| {}, &|| false).unwrap();
    let body = std::fs::read(&full).unwrap();
    let part = dir.join("part");
    std::fs::write(&part, &body[..10]).unwrap();
    super::download::fetch(&url, &part, &mut |_, _| {}, &|| false).unwrap();
    assert_eq!(std::fs::read(&part).unwrap(), body);
    assert!(String::from_utf8_lossy(&body).starts_with(manifest::package_for("windows", "x86_64").unwrap().sha256));
    let _ = std::fs::remove_dir_all(&dir);
}
