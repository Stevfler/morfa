//! On-disk state in `<config>/morfa/cloud/`: `accounts.json`, `secrets.json` (app passwords,
//! owner-only permissions) and one `<id>.json` sync database per account.
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    /// Provider; only "nextcloud" for now.
    pub kind: String,
    pub name: String,
    pub server: String,
    pub login: String,
    /// Local folder the remote structure is mirrored into.
    pub local_root: String,
}

/// What we last saw of a synced item on both sides.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Record {
    pub dir: bool,
    pub etag: String,
    pub size: u64,
    /// Local mtime, Unix ms.
    pub mtime: u64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct SyncDb {
    /// Rel paths the user chose to keep on the device (folders include everything inside).
    pub pins: BTreeSet<String>,
    pub items: BTreeMap<String, Record>,
}

/// `child` is `root` or inside it ("" is the account root).
pub fn under(child: &str, root: &str) -> bool {
    root.is_empty() || child == root || (child.starts_with(root) && child.as_bytes().get(root.len()) == Some(&b'/'))
}

impl SyncDb {
    /// The pin covering `rel`, if any.
    pub fn pin_of(&self, rel: &str) -> Option<&String> {
        self.pins.iter().find(|p| under(rel, p))
    }
    /// Some pin lies strictly inside the folder `rel`.
    pub fn has_pins_inside(&self, rel: &str) -> bool {
        self.pins.iter().any(|p| p != rel && under(p, rel))
    }
    pub fn drop_under(&mut self, rel: &str) {
        self.items.retain(|k, _| !under(k, rel));
    }
}

fn dir() -> Result<PathBuf, String> {
    let d = dirs::config_dir().ok_or("No config folder")?.join("morfa").join("cloud");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

fn read<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Write via a temp file + rename so a crash never leaves half a JSON file.
fn write<T: Serialize>(path: &Path, v: &T, private: bool) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    {
        use std::io::Write;
        let mut f = opts.open(&tmp).map_err(|e| e.to_string())?;
        f.write_all(&data).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn load_accounts() -> Vec<Account> {
    dir().map(|d| read(&d.join("accounts.json"))).unwrap_or_default()
}
pub fn save_accounts(a: &[Account]) -> Result<(), String> {
    write(&dir()?.join("accounts.json"), &a, false)
}

pub fn secret(id: &str) -> Option<String> {
    let m: HashMap<String, String> = read(&dir().ok()?.join("secrets.json"));
    m.get(id).cloned()
}
pub fn set_secret(id: &str, value: Option<&str>) -> Result<(), String> {
    let path = dir()?.join("secrets.json");
    let mut m: HashMap<String, String> = read(&path);
    match value {
        Some(v) => m.insert(id.into(), v.into()),
        None => m.remove(id),
    };
    write(&path, &m, true)
}

fn db_path(id: &str) -> Result<PathBuf, String> {
    Ok(dir()?.join(format!("{id}.json")))
}
pub fn load_db(id: &str) -> SyncDb {
    db_path(id).map(|p| read(&p)).unwrap_or_default()
}
pub fn save_db(id: &str, db: &SyncDb) -> Result<(), String> {
    write(&db_path(id)?, db, false)
}
pub fn remove_db(id: &str) {
    if let Ok(p) = db_path(id) {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_matches_segments() {
        assert!(under("a/b", ""));
        assert!(under("a/b", "a"));
        assert!(under("a", "a"));
        assert!(!under("ab", "a"));
        assert!(!under("a", "a/b"));
    }
}
