//! On-disk state in `<config>/morfa/cloud/`: `accounts.json`, `secrets.json` (app passwords,
//! SFTP passwords and key passphrases, owner-only permissions) and one `<id>.json` sync database per account.
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    /// "nextcloud" | "webdav" | "sftp"
    pub kind: String,
    pub name: String,
    pub server: String,
    pub login: String,
    /// Local folder the remote structure is mirrored into.
    pub local_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sftp: Option<SftpConfig>,
}

/// The non-secret half of an SFTP account; password or passphrase live in `secrets.json`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SftpConfig {
    pub host: String,
    pub port: u16,
    /// Path of the private key file; empty means password sign-in. The key itself is never stored.
    pub key_path: String,
    /// Folder shown as the account root; empty means the login folder.
    pub remote_path: String,
    /// SHA-256 fingerprint the user confirmed. Any other key is refused.
    pub host_key: String,
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
    /// Deletions waiting for the user; their records stay in `items` meanwhile.
    #[serde(default)]
    pub holds: Vec<Hold>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Hold {
    pub pin: String,
    /// On the server (true) or on this device (false).
    pub to_server: bool,
    pub items: BTreeSet<String>,
    /// That whole side looked missing or empty, rather than merely too many at once.
    pub gone: bool,
    /// The user approved these items; the next run deletes them.
    pub confirmed: bool,
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
        self.holds.retain(|h| !under(&h.pin, rel));
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
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let tmp = path.with_extension(format!("{}.{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let data = serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?;
    let mut opts = std::fs::OpenOptions::new();
    // New every time, so owner-only permissions always apply.
    opts.write(true).create_new(true);
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
        let done = f.write_all(&data).and_then(|_| f.sync_all());
        if let Err(e) = done {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.to_string());
        }
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

pub fn load_accounts() -> Vec<Account> {
    dir().map(|d| read(&d.join("accounts.json"))).unwrap_or_default()
}
pub fn save_accounts(a: &[Account]) -> Result<(), String> {
    write(&dir()?.join("accounts.json"), &a, false)
}

/// Missing is empty; unreadable or corrupt is an error, so a bad file never gets overwritten with
/// just the one new secret, and never looks like "no password saved".
fn read_secrets(path: &Path) -> Result<HashMap<String, String>, String> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).map_err(|_| "The saved credentials file is damaged. Fix or remove secrets.json in the Morfa settings folder".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(e) => Err(format!("Can't read the saved credentials: {e}")),
    }
}

/// One read-modify-write at a time, or concurrent connects lose each other's secrets.
static SECRETS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn update_secret(path: &Path, id: &str, value: Option<&str>) -> Result<(), String> {
    let _g = SECRETS.lock().unwrap_or_else(|e| e.into_inner());
    let mut m = read_secrets(path)?;
    match value {
        Some(v) => m.insert(id.into(), v.into()),
        None => m.remove(id),
    };
    write(path, &m, true)
}

pub fn secret(id: &str) -> Result<Option<String>, String> {
    let _g = SECRETS.lock().unwrap_or_else(|e| e.into_inner());
    Ok(read_secrets(&dir()?.join("secrets.json"))?.get(id).cloned())
}
pub fn set_secret(id: &str, value: Option<&str>) -> Result<(), String> {
    update_secret(&dir()?.join("secrets.json"), id, value)
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

    fn tmp_secrets(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("morfa-secrets-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn corrupt_secrets_are_an_error_and_stay_untouched() {
        let d = tmp_secrets("corrupt");
        let p = d.join("secrets.json");
        std::fs::write(&p, b"{not json").unwrap();
        assert!(read_secrets(&p).is_err());
        assert!(update_secret(&p, "a", Some("x")).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"{not json");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn concurrent_secret_updates_all_land() {
        let d = tmp_secrets("race");
        let p = d.join("secrets.json");
        let ts: Vec<_> = (0..16).map(|i| {
            let p = p.clone();
            std::thread::spawn(move || update_secret(&p, &format!("k{i}"), Some("v")).unwrap())
        }).collect();
        for t in ts {
            t.join().unwrap();
        }
        assert_eq!(read_secrets(&p).unwrap().len(), 16);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn old_sync_state_without_holds_loads() {
        let db: SyncDb = serde_json::from_str(r#"{"pins":["a"],"items":{}}"#).unwrap();
        assert!(db.holds.is_empty());
    }

    #[test]
    fn under_matches_segments() {
        assert!(under("a/b", ""));
        assert!(under("a/b", "a"));
        assert!(under("a", "a"));
        assert!(!under("ab", "a"));
        assert!(!under("a", "a/b"));
    }
}
