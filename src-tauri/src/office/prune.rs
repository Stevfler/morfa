//! What gets deleted from the downloaded LibreOffice to leave a headless converter for
//! Writer/Calc/Impress <-> pdf, docx, odt, rtf, txt, html, epub, xlsx, ods, csv, pptx, odp.
//! Rules are data: (where, glob). `where` maps to the platform layout, so one rule covers
//! Windows (`<root>/program`, `<root>/share`), Linux (same) and macOS (`LibreOffice.app/Contents/...`).
//! Validated on Linux by `tests::prune_real_install` (see docs/BUILDING.md).
use super::glob;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Os {
    Win,
    Mac,
    Linux,
}

#[derive(Clone, Copy, Debug)]
pub enum At {
    /// Install root: help, readmes, credits.            mac: Contents/Resources
    Top,
    /// `share/`.                                        mac: Contents/Resources
    Share,
    /// `program/` non-library files (scripts, exes).    mac: Contents/Resources (and MacOS for exes)
    Prog,
    /// Shared library by stem: `lib<s>.so` / `<s>.dll` / `lib<s>.dylib`. mac: Contents/Frameworks
    Lib,
}

use At::*;
pub const RULES: &[(At, &str)] = &[
    // Documentation, credits, readmes (LICENSE/NOTICE stay for licence compliance).
    (Top, "help"),
    (Top, "readmes"),
    (Top, "CREDITS.fodt"),
    (Top, "sdk"),
    // Extensions: dictionaries, hyphenation, thesaurus, wiki publisher, NLP solver, LanguageTool...
    (Share, "extensions/*"),
    // Galleries, templates, wizards, autotext, samples, Basic/script libraries, tips.
    (Share, "gallery"),
    (Share, "template"),
    (Share, "autotext"),
    (Share, "autocorr"),
    (Share, "wizards"),
    (Share, "config/wizard"),
    (Share, "basic"),
    (Share, "Scripts"),
    (Share, "tipoftheday"),
    (Share, "fingerprint"), // language guessing
    (Share, "wordbook"),
    (Share, "labels"),
    (Share, "xslt"),     // XSLT-based filters (DocBook, MS Office 2003 XML...): none we offer
    (Share, "firebird"), // Base's embedded database
    // UI definitions of Base, Basic IDE and other non-converting modules.
    (Share, "config/soffice.cfg/modules/db*"),
    (Share, "config/soffice.cfg/modules/BasicIDE"),
    (Share, "config/soffice.cfg/modules/sabpilot"),
    (Share, "config/soffice.cfg/modules/sbibliography"),
    (Share, "config/soffice.cfg/modules/scanner"),
    (Share, "config/soffice.cfg/modules/swreport"),
    // Icon themes: headless needs none, keep colibre (the Windows/macOS default) as a fallback.
    (Share, "config/images_*"),
    // UI translations other than en-US (see KEEP). Fonts (share/fonts) stay: conversions need them.
    (Share, "registry/res/*"),
    (Share, "registry/Langpack-*"),
    (Share, "registry/oo-*.sample"),
    (Share, "registry/librelogo.xcd"),
    (Share, "registry/postgresql.xcd"),
    (Prog, "resource/*"), // per-language .mo dirs (Windows/macOS); `common` stays
    // Java: classes, JRE glue.
    (Prog, "classes"),
    (Share, "java"), // macOS: Contents/Resources/java
    (Prog, "JREProperties.class"),
    (Prog, "javaldx"),
    (Prog, "java-set-classpath"),
    (Prog, "javavendors.xml"),
    (Lib, "java_uno"),
    (Lib, "javaloaderlo"),
    (Lib, "javavmlo"),
    // Python (no import/export filter we use needs it).
    (Prog, "*.py"),
    (Prog, "*.pyi"),
    (Prog, "pythonloader.unorc"),
    (Prog, "pyuno*"),
    (Prog, "python*"),          // Windows: python.exe, python-core-*
    (Prog, "wizards"),          // Windows: python wizards
    (Lib, "LibreOfficePython.framework"),
    (Lib, "pyuno"),
    (Lib, "pythonloaderlo"),
    // Base drivers and embedded databases (Writer/Calc still load the core dba libs).
    (Lib, "firebird_sdbclo"),
    (Lib, "fbclient"),
    (Lib, "hsqldb"),
    (Lib, "mysqlclo"),
    (Lib, "mysql_jdbclo"),
    (Lib, "postgresql-sdbc*"),
    (Lib, "evoablo"),
    (Lib, "abplo"),
    (Lib, "biblo"),
    (Lib, "mozbootstraplo"),
    (Lib, "ldapbe2lo"),
    (Lib, "LanguageToollo"),
    (Lib, "Engine12"), // Calc solver
    // Crash reporter, online update, misc launchers and tools.
    (Lib, "updatefeedlo"),
    (Prog, "updater*"),
    (Prog, "minidump_upload*"),
    (Prog, "crashreport*"),
    (Prog, "gengal*"),
    (Prog, "ui-previewer*"),
    (Prog, "senddoc*"),
    (Prog, "unopkg*"),
    (Prog, "quickstart*"),
    (Prog, "s[bdimw]*.exe"), // sbase/sdraw/simpress/smath/swriter/sweb launchers (Windows)
    (Prog, "scalc.exe"),
    (Prog, "intro*.png"),
    (Prog, "spsupp*"),       // SharePoint integration (Windows)
    (Prog, "*shlxthdl*"),    // Explorer shell extensions (Windows)
    (Prog, "twain*"),
];

/// File names never deleted even when a rule matches them.
pub const KEEP: &[&str] = &[
    "images_colibre.zip",
    "fcfg_langpack_en-US.xcd",
    "registry_en-US.xcd",
    "Langpack-en-US.xcd",
    "common",
    "en-US",
];

/// Directories (relative to the install root) a rule base maps to on `os`.
pub fn bases(at: At, os: Os) -> &'static [&'static str] {
    match (os, at) {
        (Os::Mac, Top | Share) => &["Contents/Resources"],
        (Os::Mac, Prog) => &["Contents/Resources", "Contents/MacOS"],
        (Os::Mac, Lib) => &["Contents/Frameworks"],
        (_, Top) => &[""],
        (_, Share) => &["share"],
        (_, Prog | Lib) => &["program"],
    }
}

/// File-name glob for a rule on `os` (libraries get the platform prefix/suffix).
pub fn pattern(at: At, glob: &str, os: Os) -> String {
    match (at, os) {
        (Lib, _) if glob.ends_with(".framework") => glob.into(),
        (Lib, Os::Win) => format!("{glob}.dll"),
        (Lib, Os::Mac) => format!("lib{glob}.dylib"),
        (Lib, Os::Linux) => format!("lib{glob}.so*"),
        _ => glob.into(),
    }
}

/// Every existing path under `root` matched by the rules (segment-wise glob).
pub fn targets(root: &Path, os: Os) -> Vec<PathBuf> {
    let mut out = vec![];
    for &(at, glob) in RULES {
        let pat = pattern(at, glob, os);
        for base in bases(at, os) {
            out.extend(glob::expand(&root.join(base), &pat).into_iter().filter(|p| !p.file_name().is_some_and(|n| KEEP.iter().any(|k| n == *k))));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Delete everything matched.
pub fn apply(root: &Path, os: Os) -> std::io::Result<()> {
    for p in targets(root, os) {
        let md = std::fs::symlink_metadata(&p)?;
        if md.is_dir() { std::fs::remove_dir_all(&p)? } else { std::fs::remove_file(&p)? }
    }
    Ok(())
}
