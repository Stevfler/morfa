//! App-side tag store: a JSON map path → tags in the user data dir.
use crate::CmdResult;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

pub type Db = HashMap<String, Vec<String>>;

// Serialises read-modify-write cycles from concurrent commands.
static LOCK: Mutex<()> = Mutex::new(());

fn file() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("orbit").join("tags.json"))
}

pub fn load() -> Db {
    let _g = LOCK.lock();
    read()
}

/// Load, change and save under one lock so concurrent edits can't drop each other.
pub fn update(f: impl FnOnce(&mut Db) -> bool) -> CmdResult<()> {
    let _g = LOCK.lock();
    let mut db = read();
    if f(&mut db) {
        write(&db)?;
    }
    Ok(())
}

fn read() -> Db {
    file().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write(db: &Db) -> CmdResult<()> {
    let f = file().ok_or("no data directory")?;
    if let Some(dir) = f.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = f.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(db).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &f).map_err(|e| e.to_string())
}
