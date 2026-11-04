//! What a rule is, and whether it may exist: refused locations and outputs that could feed back.
use crate::convert::{naming, ConvertOpts};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

pub const FAILURES_KEPT: usize = 50;

/// A saved recipe as the UI holds it; recipes live in the frontend, which sends them on every change.
#[derive(Deserialize, Clone, Debug)]
pub struct Recipe {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub opts: ConvertOpts,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub path: String,
    pub error: String,
    pub at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Stats {
    pub last_run: Option<u64>,
    pub done: u64,
    pub failed: u64,
    pub failures: Vec<Failure>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub folder: String,
    pub recipe_id: String,
    /// Last known name, to still say which recipe a flagged rule used.
    pub recipe_name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub subfolders: bool,
    /// Why it was switched off and can't be switched on as it stands: a refusal code.
    #[serde(default)]
    pub flag: Option<String>,
    #[serde(default)]
    pub stats: Stats,
}

/// What the filesystem says about a path. `dead`: it didn't answer.
#[derive(Default, Clone)]
pub struct Inspect {
    pub real: Option<PathBuf>,
    pub is_dir: bool,
    pub network: bool,
    pub dead: bool,
}

pub trait PathEnv {
    fn inspect(&self, p: &Path) -> Inspect;
    fn protected(&self, real: &Path) -> bool;
}

/// The real thing. Everything that may block goes through `guard`.
pub struct RealEnv;

impl PathEnv for RealEnv {
    fn inspect(&self, p: &Path) -> Inspect {
        if crate::netfs::network_mount_of(p).is_some() {
            return Inspect { network: true, ..Default::default() };
        }
        let s = p.to_string_lossy().into_owned();
        let owned = p.to_path_buf();
        crate::guard::run_blocking(&s, crate::guard::quick_for(&s), move |_| {
            let real = std::fs::canonicalize(&owned).ok();
            let network = crate::netfs::is_network_at(&owned) || real.as_deref().is_some_and(|r| crate::netfs::network_mount_of(r).is_some());
            Ok(Inspect { is_dir: real.as_deref().is_some_and(Path::is_dir), real, network, dead: false })
        })
        .unwrap_or(Inspect { dead: true, ..Default::default() })
    }

    /// `Review` counts too: nothing should run by itself in `~/Library` or `/opt`. All of `~/Library`,
    /// `Caches` included, though the clean-up tool calls that safe. `/Volumes` itself holds only
    /// mount points; a volume inside it is fine if local.
    fn protected(&self, real: &Path) -> bool {
        let home = dirs::home_dir().and_then(|h| std::fs::canonicalize(&h).ok().or(Some(h)));
        let library = cfg!(target_os = "macos") && home.is_some_and(|h| real.to_string_lossy().to_lowercase().starts_with(&h.join("library").to_string_lossy().to_lowercase()));
        library || (cfg!(target_os = "macos") && real.to_string_lossy().eq_ignore_ascii_case("/volumes")) || crate::cleanup::is_trash_dir(real) || crate::cleanup::location_protection(real) != crate::cleanup::Protection::Safe
    }
}

fn plain_local(p: &str) -> bool {
    let path = Path::new(p);
    path.is_absolute() && !p.contains("://") && !path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir))
}

/// The same folder, however it was reached (case-insensitive volumes included).
fn same_dir(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return x.dev() == y.dev() && x.ino() == y.ino();
        }
    }
    cfg!(any(windows, target_os = "macos")) && a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

pub fn inside(p: &Path, root: &Path) -> bool {
    p.ancestors().skip(1).any(|a| same_dir(a, root))
}

/// Refusals switch a rule off and stay until the rule changes. `missing` and `unavailable` are
/// passing states when the rule already exists, and only stop one from being created.
pub const SOFT: [&str; 2] = ["missing", "unavailable"];

/// The watched folder's real path, or why it can't be watched.
pub fn check_folder(folder: &str, env: &dyn PathEnv) -> Result<PathBuf, &'static str> {
    if !plain_local(folder) {
        return Err("pseudo");
    }
    let i = env.inspect(Path::new(folder));
    if i.network {
        return Err("network");
    }
    if i.dead {
        return Err("unavailable");
    }
    if env.protected(i.real.as_deref().unwrap_or(Path::new(folder))) {
        return Err("protected");
    }
    match i.real {
        Some(real) if i.is_dir => Ok(real),
        _ => Err("missing"),
    }
}

/// The recipe's output against this folder. The two dangerous layouts are refused; "next to source"
/// is safe only through the scan's own skips (see `filter::eligible`).
pub fn check_output(real_folder: &Path, subfolders: bool, opts: &ConvertOpts, env: &dyn PathEnv) -> Result<(), &'static str> {
    if !naming::template_ok(&opts.name_template) {
        return Err("recipeBad");
    }
    let Some(out) = opts.out_dir.as_deref() else { return Ok(()) };
    if !plain_local(out) {
        return Err("recipeBad");
    }
    let i = env.inspect(Path::new(out));
    if i.network {
        return Err("outputNetwork");
    }
    let real = i.real.unwrap_or_else(|| PathBuf::from(out));
    if env.protected(&real) {
        return Err("outputProtected");
    }
    if same_dir(&real, real_folder) {
        return Err("outputSame");
    }
    if subfolders && inside(&real, real_folder) {
        return Err("outputInside");
    }
    Ok(())
}

pub fn check_rule(folder: &str, subfolders: bool, recipe: Option<&Recipe>, env: &dyn PathEnv) -> Result<(), &'static str> {
    let recipe = recipe.ok_or("recipeMissing")?;
    let real = check_folder(folder, env)?;
    check_output(&real, subfolders, &recipe.opts, env)
}
