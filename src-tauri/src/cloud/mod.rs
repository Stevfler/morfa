//! Cloud drives (Nextcloud or any WebDAV server, SFTP). The remote tree is browsed live under the
//! `morfa://cloud/<id>/…` pseudo-path; only what the user pins is mirrored into the
//! account's local folder and kept in two-way sync by a background worker.
mod dav;
mod login;
mod names;
mod sftp;
mod store;
mod sync;

use crate::CmdResult;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, MutexGuard,
    },
    time::Duration,
};
use store::{under, Account, SftpConfig, SyncDb};
use tauri::{AppHandle, Emitter, State};

/// How often pinned items are re-synced in the background.
const INTERVAL: Duration = Duration::from_secs(60);
/// No progress on a remote call for this long: it's abandoned. Longer than the clients' own
/// connect and response timeouts, so those report first when they can.
const REMOTE_STALL: Duration = Duration::from_secs(90);

/// Every remote call on its own bounded thread, per `guard`: a stuck server costs a few threads
/// and an error, never a hung sync worker. Progress is what the client reports between chunks.
struct Guarded {
    inner: Arc<dyn sync::Remote + Send + Sync>,
    key: String,
    label: String,
}
impl Guarded {
    fn call<T: Send + 'static>(&self, f: impl FnOnce(&(dyn sync::Remote + Send + Sync), &crate::guard::Beat) -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
        let inner = self.inner.clone();
        crate::guard::run_keyed(&self.key, &self.label, REMOTE_STALL, crate::guard::LIMITS, move |b| f(inner.as_ref(), b))
    }
}
impl sync::Remote for Guarded {
    fn list(&self, rel: &str) -> CmdResult<Vec<dav::RemoteItem>> {
        let rel = rel.to_string();
        self.call(move |r, _| r.list(&rel))
    }
    fn stat(&self, rel: &str) -> CmdResult<Option<dav::RemoteItem>> {
        let rel = rel.to_string();
        self.call(move |r, _| r.stat(&rel))
    }
    fn download(&self, rel: &str, dest: &Path, cancelled: &dyn Fn() -> bool) -> CmdResult<()> {
        let (rel, dest, inner) = (rel.to_string(), dest.to_path_buf(), self.inner.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let seen = stop.clone();
        let cancel = || {
            let c = cancelled();
            if c {
                stop.store(true, Ordering::Relaxed);
            }
            c
        };
        crate::guard::run_keyed_cancel(&self.key, &self.label, REMOTE_STALL, crate::guard::LIMITS, cancel, move |b| {
            inner.download(&rel, &dest, &|| {
                b.tick();
                seen.load(Ordering::Relaxed)
            })
        })
    }
    fn upload(&self, rel: &str, src: &Path, mtime_ms: u64, _tick: &dyn Fn()) -> CmdResult<Option<String>> {
        let (rel, src) = (rel.to_string(), src.to_path_buf());
        self.call(move |r, b| r.upload(&rel, &src, mtime_ms, &|| b.tick()))
    }
    fn mkcol(&self, rel: &str) -> CmdResult<()> {
        let rel = rel.to_string();
        self.call(move |r, _| r.mkcol(&rel))
    }
    fn delete(&self, rel: &str) -> CmdResult<()> {
        let rel = rel.to_string();
        self.call(move |r, _| r.delete(&rel))
    }
    fn recursive_etags(&self) -> bool {
        self.inner.recursive_etags()
    }
}

fn guarded(a: &Account, inner: Arc<dyn sync::Remote + Send + Sync>) -> Arc<dyn sync::Remote + Send + Sync> {
    Arc::new(Guarded { inner, key: format!("cloud:{}", a.id), label: a.name.clone() })
}

/// A rel path from the frontend, checked like one from a server.
fn clean_rel(rel: &str) -> CmdResult<String> {
    let rel = rel.trim_matches('/');
    if names::safe_rel(rel) { Ok(rel.to_string()) } else { Err("Invalid path".into()) }
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// "idle" | "syncing" | "error"
    state: String,
    error: Option<String>,
    /// Rel path being transferred.
    current: Option<String>,
    /// Unix ms of the last finished sync.
    last: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    #[serde(flatten)]
    account: Account,
    status: Status,
    pins: Vec<String>,
    holds: Vec<HoldInfo>,
}

/// Deletions waiting for the user's answer.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HoldInfo {
    pin: String,
    to_server: bool,
    count: usize,
    /// One whole side looked missing or empty.
    gone: bool,
    /// Already approved; the next run applies it.
    confirmed: bool,
    sample: Vec<String>,
    /// Identifies the exact items; sent back with the user's answer.
    digest: String,
}

/// A row of the remote listing; same shape as a local `Entry` plus sync info.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudEntry {
    name: String,
    path: String,
    is_dir: bool,
    is_symlink: bool,
    size: u64,
    modified: u64,
    ext: String,
    hidden: bool,
    /// "synced" | "pending" | "partial" | "" (only on the server)
    cloud: String,
    /// Where the item lives (or will live) on this device.
    local: String,
    /// Online-only document: opened in the browser, never synced.
    link: String,
}

#[derive(Default)]
struct Inner {
    accounts: Mutex<Vec<Account>>,
    dbs: Mutex<HashMap<String, Arc<Mutex<SyncDb>>>>,
    runs: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    remotes: Mutex<HashMap<String, Arc<dyn sync::Remote + Send + Sync>>>,
    status: Mutex<HashMap<String, Status>>,
    queue: Mutex<HashSet<String>>,
    wake: Condvar,
    login_cancel: AtomicBool,
}

#[derive(Default, Clone)]
pub struct CloudState(Arc<Inner>);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// "2026-11-03 153012" (UTC) for conflict copy names.
pub(crate) fn stamp(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Inner {
    fn account(&self, id: &str) -> CmdResult<Account> {
        lock(&self.accounts).iter().find(|a| a.id == id).cloned().ok_or_else(|| "Unknown cloud account".into())
    }
    fn db(&self, id: &str) -> Arc<Mutex<SyncDb>> {
        lock(&self.dbs).entry(id.into()).or_insert_with(|| Arc::new(Mutex::new(store::load_db(id)))).clone()
    }
    fn run_lock(&self, id: &str) -> Arc<Mutex<()>> {
        lock(&self.runs).entry(id.into()).or_default().clone()
    }
    fn save(&self, id: &str) {
        let db = self.db(id);
        let snapshot = lock(&db).clone();
        if let Err(e) = store::save_db(id, &snapshot) {
            eprintln!("cloud: can't save sync state for {id}: {e}");
        }
    }
    /// One client per account, kept between calls.
    fn remote(&self, a: &Account) -> CmdResult<Arc<dyn sync::Remote + Send + Sync>> {
        if let Some(r) = lock(&self.remotes).get(&a.id) {
            return Ok(r.clone());
        }
        let secret = store::secret(&a.id)?;
        let need = || secret.clone().ok_or_else(|| "No saved credentials — reconnect the account".to_string());
        let client: Arc<dyn sync::Remote + Send + Sync> = match (a.kind.as_str(), &a.sftp) {
            ("nextcloud", _) => Arc::new(dav::Dav::new(&a.server, &a.login, &need()?)),
            ("webdav", _) => Arc::new(dav::Dav::generic(&a.server, &a.login, &need()?)),
            ("sftp", Some(c)) => {
                let cred = if c.key_path.is_empty() { sftp::Credential::Password(need()?) } else { sftp::Credential::Key { path: c.key_path.clone().into(), passphrase: secret.clone() } };
                Arc::new(sftp::Sftp::new(sftp::Target { host: c.host.clone(), port: c.port, login: a.login.clone(), remote_path: c.remote_path.clone(), host_key: c.host_key.clone() }, cred))
            }
            _ => return Err("Unsupported account type".into()),
        };
        let r = guarded(a, client);
        lock(&self.remotes).insert(a.id.clone(), r.clone());
        Ok(r)
    }
    fn set_status(&self, app: &AppHandle, id: &str, f: impl FnOnce(&mut Status)) {
        let s = {
            let mut m = lock(&self.status);
            let s = m.entry(id.into()).or_insert_with(|| Status { state: "idle".into(), ..Default::default() });
            f(s);
            s.clone()
        };
        #[derive(Serialize, Clone)]
        struct Ev {
            id: String,
            #[serde(flatten)]
            status: Status,
        }
        let _ = app.emit("cloud-status", Ev { id: id.into(), status: s });
    }
    fn enqueue(&self, id: &str) {
        lock(&self.queue).insert(id.into());
        self.wake.notify_all();
    }

    /// Sync every pin of the account (or just `only`), under the account's run lock.
    fn sync_account(&self, app: &AppHandle, id: &str, only: Option<&str>) -> CmdResult<sync::Report> {
        let run = self.run_lock(id);
        let _guard = lock(&run);
        let a = self.account(id)?;
        let remote = self.remote(&a)?;
        let db = self.db(id);
        let pins: Vec<String> = match only {
            Some(p) => vec![p.to_string()],
            None => lock(&db).pins.iter().cloned().collect(),
        };
        if pins.is_empty() {
            return Ok(sync::Report::default());
        }
        self.set_status(app, id, |s| {
            s.state = "syncing".into();
            s.current = None;
        });
        let gone = || lock(&self.accounts).iter().all(|x| x.id != id);
        let trash = |p: &Path| crate::fsops::trashcan::trash_blocking(&[p.to_string_lossy().into_owned()]);
        let progress = |rel: &str| self.set_status(app, id, |s| s.current = Some(rel.into()));
        let ctx = sync::Ctx { remote: remote.as_ref(), root: PathBuf::from(&a.local_root), db: &db, cancelled: &gone, trash: &trash, progress: &progress };
        let mut total = sync::Report::default();
        for pin in pins {
            if gone() {
                break;
            }
            match sync::sync_pin(&ctx, &pin) {
                Ok(r) => {
                    total.changed |= r.changed;
                    total.errors.extend(r.errors);
                }
                Err(e) => total.errors.push(e),
            }
        }
        if gone() {
            return Ok(total);
        }
        self.save(id);
        let err = total.errors.first().map(|e| if total.errors.len() > 1 { format!("{e} (+{} more)", total.errors.len() - 1) } else { e.clone() });
        self.set_status(app, id, |s| {
            s.state = if err.is_some() { "error".into() } else { "idle".into() };
            s.error = err;
            s.current = None;
            s.last = now_ms();
        });
        if total.changed {
            let _ = app.emit("cloud-changed", id);
        }
        Ok(total)
    }
}

/// Load accounts and start the background sync worker.
pub fn init(app: AppHandle, state: &CloudState) {
    *lock(&state.0.accounts) = store::load_accounts();
    let inner = state.0.clone();
    std::thread::Builder::new()
        .name("cloud-sync".into())
        .spawn(move || loop {
            let ids: Vec<String> = {
                let q = lock(&inner.queue);
                let (mut q, timeout) = inner.wake.wait_timeout_while(q, INTERVAL, |q| q.is_empty()).unwrap_or_else(|e| e.into_inner());
                if timeout.timed_out() {
                    lock(&inner.accounts).iter().map(|a| a.id.clone()).collect()
                } else {
                    q.drain().collect()
                }
            };
            for id in ids {
                let _ = inner.sync_account(&app, &id, None);
            }
        })
        .expect("spawn cloud-sync thread");
    state.0.enqueue_all();
}

impl Inner {
    fn enqueue_all(&self) {
        let ids: Vec<String> = lock(&self.accounts).iter().map(|a| a.id.clone()).collect();
        for id in ids {
            self.enqueue(&id);
        }
    }
}

fn info(inner: &Inner, a: &Account) -> AccountInfo {
    let status = lock(&inner.status).get(&a.id).cloned().unwrap_or(Status { state: "idle".into(), ..Default::default() });
    let db = inner.db(&a.id);
    let db = lock(&db);
    let pins = db.pins.iter().cloned().collect();
    let holds = db.holds.iter().map(|h| HoldInfo { pin: h.pin.clone(), to_server: h.to_server, count: h.items.len(), gone: h.gone, confirmed: h.confirmed, sample: h.items.iter().take(5).cloned().collect(), digest: sync::hold_digest(&h.items) }).collect();
    AccountInfo { account: a.clone(), status, pins, holds }
}

#[tauri::command]
pub fn cloud_accounts(state: State<'_, CloudState>) -> Vec<AccountInfo> {
    let accounts = lock(&state.0.accounts).clone();
    accounts.iter().map(|a| info(&state.0, a)).collect()
}

fn check_root(inner: &Inner, root: &str) -> CmdResult<PathBuf> {
    let p = PathBuf::from(root.trim());
    if !p.is_absolute() {
        return Err("Choose a folder for the synced files".into());
    }
    std::fs::create_dir_all(&p).map_err(|e| format!("Can't create {}: {e}", p.display()))?;
    let canon = std::fs::canonicalize(&p).map_err(|e| e.to_string())?;
    // On the real path, so a symlink to the home folder doesn't pass.
    let home = dirs::home_dir().map(|h| std::fs::canonicalize(&h).unwrap_or(h));
    if !dedicated(&canon, home.as_deref()) {
        return Err("Pick a dedicated folder, not the whole disk or home folder".into());
    }
    // Cloned: the sync commands need this lock, and canonicalize can block on a dead mount.
    let accounts = lock(&inner.accounts).clone();
    for a in &accounts {
        let other = std::fs::canonicalize(&a.local_root).unwrap_or_else(|_| PathBuf::from(&a.local_root));
        if canon.starts_with(&other) || other.starts_with(&canon) {
            return Err(format!("This folder overlaps the folder of “{}”", a.name));
        }
    }
    // The real folder: a symlink here must not make the mirror look like something else later.
    Ok(canon)
}

/// Not the disk root, the home folder, or anything containing it.
fn dedicated(canon: &Path, home: Option<&Path>) -> bool {
    canon.parent().is_some() && !home.is_some_and(|h| h.starts_with(canon))
}

/// Sign in through the browser (Login Flow v2) and add the account.
#[tauri::command]
pub async fn cloud_connect(app: AppHandle, state: State<'_, CloudState>, server: String, local_root: String) -> CmdResult<AccountInfo> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let root = check_root(&inner, &local_root)?;
        inner.login_cancel.store(false, Ordering::SeqCst);
        let open = |url: &str| {
            use tauri_plugin_opener::OpenerExt;
            app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
        };
        let cancelled = || inner.login_cancel.load(Ordering::SeqCst);
        let local_root = root.to_string_lossy().into_owned();
        let (a, secret) = {
            let server = login::normalize_server(&server)?;
            let (url, poll) = login::start(&server)?;
            open(&url)?;
            let creds = poll(&cancelled)?;
            let server = login::normalize_server(&creds.server).unwrap_or(server);
            let host = server.split("://").nth(1).unwrap_or(&server).to_string();
            let a = Account { id: format!("nc{:x}", now_ms()), kind: "nextcloud".into(), name: host, server, login: creds.login_name, local_root, sftp: None };
            // Check the credentials actually work before saving anything.
            dav::Dav::new(&a.server, &a.login, &creds.app_password).stat("")?;
            (a, creds.app_password)
        };
        store::set_secret(&a.id, Some(&secret))?;
        let mut list = lock(&inner.accounts);
        list.push(a.clone());
        store::save_accounts(&list)?;
        drop(list);
        Ok(info(&inner, &a))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Checks the connection works, then saves the account and its secret.
fn add_account(inner: &Inner, a: Account, client: Arc<dyn sync::Remote + Send + Sync>, secret: Option<&str>) -> CmdResult<AccountInfo> {
    let client = guarded(&a, client);
    match client.stat("")? {
        Some(i) if i.is_dir => {}
        _ => return Err("This address isn't a folder on the server".into()),
    }
    if inner.login_cancel.load(Ordering::SeqCst) {
        return Err("Cancelled".into());
    }
    store::set_secret(&a.id, secret)?;
    let mut list = lock(&inner.accounts);
    list.push(a.clone());
    store::save_accounts(&list)?;
    drop(list);
    lock(&inner.remotes).insert(a.id.clone(), client);
    Ok(info(inner, &a))
}

/// Any WebDAV server: its folder URL, a login and an app password. Plain http needs `allow_insecure`,
/// which the dialog sets only after warning that the password would travel unencrypted.
#[tauri::command]
pub async fn cloud_connect_webdav(state: State<'_, CloudState>, url: String, login: String, password: String, local_root: String, allow_insecure: bool) -> CmdResult<AccountInfo> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let server = dav::normalize_url(&url)?;
        if server.starts_with("http://") && !allow_insecure {
            return Err("This address isn't encrypted. Confirm sending the password over plain http".into());
        }
        let login = login.trim().to_string();
        if login.is_empty() || login.contains(':') {
            return Err("Enter a login without “:”".into());
        }
        if password.is_empty() {
            return Err("Enter the password".into());
        }
        let root = check_root(&inner, &local_root)?;
        inner.login_cancel.store(false, Ordering::SeqCst);
        let name = server.split("://").nth(1).unwrap_or(&server).to_string();
        let a = Account { id: format!("dav{:x}", now_ms()), kind: "webdav".into(), name, server, login, local_root: root.to_string_lossy().into_owned(), sftp: None };
        let client = Arc::new(dav::Dav::generic(&a.server, &a.login, &password));
        add_account(&inner, a, client, Some(&password))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// First contact with an SFTP server: its host key fingerprint, for the user to confirm. No
/// credentials are involved.
#[tauri::command]
pub async fn cloud_sftp_probe(host: String, port: u16) -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(move || sftp::probe(&sftp::check_host(&host)?, port)).await.map_err(|e| e.to_string())?
}

/// Add an SFTP account. `fingerprint` is the key the user confirmed; a server presenting any other
/// is refused before credentials are sent. Without `key_path` the sign-in uses the password.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn cloud_connect_sftp(state: State<'_, CloudState>, host: String, port: u16, login: String, password: Option<String>, key_path: Option<String>, passphrase: Option<String>, remote_path: Option<String>, fingerprint: String, local_root: String) -> CmdResult<AccountInfo> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let host = sftp::check_host(&host)?;
        let login = login.trim().to_string();
        if login.is_empty() || port == 0 || !fingerprint.starts_with("SHA256:") {
            return Err("Fill in the login and confirm the server's fingerprint".into());
        }
        let key_path = key_path.map(|k| k.trim().to_string()).unwrap_or_default();
        let (cred, secret) = if key_path.is_empty() {
            let p = password.filter(|p| !p.is_empty()).ok_or("Enter the password")?;
            (sftp::Credential::Password(p.clone()), Some(p))
        } else {
            if !Path::new(&key_path).is_absolute() {
                return Err("Choose the private key file".into());
            }
            let p = passphrase.filter(|p| !p.is_empty());
            (sftp::Credential::Key { path: key_path.clone().into(), passphrase: p.clone() }, p)
        };
        let root = check_root(&inner, &local_root)?;
        inner.login_cancel.store(false, Ordering::SeqCst);
        let remote_path = remote_path.map(|r| r.trim().to_string()).unwrap_or_default();
        let target = sftp::Target { host: host.clone(), port, login: login.clone(), remote_path: remote_path.clone(), host_key: fingerprint.clone() };
        let client = Arc::new(sftp::Sftp::new(target, cred));
        let shown = if host.contains(':') { format!("[{host}]") } else { host.clone() };
        let name = if port == 22 { shown.clone() } else { format!("{shown}:{port}") };
        let a = Account {
            id: format!("sftp{:x}", now_ms()),
            kind: "sftp".into(),
            server: format!("sftp://{shown}:{port}"),
            name,
            login,
            local_root: root.to_string_lossy().into_owned(),
            sftp: Some(SftpConfig { host, port, key_path, remote_path, host_key: fingerprint }),
        };
        add_account(&inner, a, client, secret.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cloud_connect_cancel(state: State<'_, CloudState>) {
    state.0.login_cancel.store(true, Ordering::SeqCst);
}

/// Forget the account (local files stay) and revoke its app password.
#[tauri::command]
pub async fn cloud_remove(state: State<'_, CloudState>, id: String) -> CmdResult<()> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let a = inner.account(&id)?;
        {
            let mut list = lock(&inner.accounts);
            list.retain(|x| x.id != id);
            store::save_accounts(&list)?;
        }
        // Wait for a running sync to notice and stop.
        let run = inner.run_lock(&id);
        drop(lock(&run));
        // Only Nextcloud has an app password to revoke; nothing is sent to other servers.
        if a.kind == "nextcloud" {
            if let Ok(Some(secret)) = store::secret(&id) {
                login::revoke(&a.server, &a.login, &secret);
            }
        }
        store::set_secret(&id, None)?;
        store::remove_db(&id);
        lock(&inner.dbs).remove(&id);
        lock(&inner.remotes).remove(&id);
        lock(&inner.status).remove(&id);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn mark(db: &SyncDb, rel: &str, is_dir: bool, etag: Option<&str>) -> String {
    if db.pin_of(rel).is_some() {
        let synced = db.items.get(rel).is_some_and(|r| is_dir || etag.is_none_or(|e| e == r.etag));
        return if synced { "synced" } else { "pending" }.into();
    }
    if is_dir && db.has_pins_inside(rel) { "partial".into() } else { String::new() }
}

/// List a remote folder with sync marks.
#[tauri::command]
pub async fn cloud_list(state: State<'_, CloudState>, id: String, rel: String) -> CmdResult<Vec<CloudEntry>> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let a = inner.account(&id)?;
        let rel = clean_rel(&rel)?;
        let items = inner.remote(&a)?.list(&rel)?;
        let db = inner.db(&id);
        let db = lock(&db);
        Ok(items
            .into_iter()
            .filter(|i| !i.path.is_empty() && names::safe_rel(&i.path) && !i.path.ends_with(sync::PART))
            .map(|i| {
                let name = i.path.rsplit('/').next().unwrap_or(&i.path).to_string();
                let ext = if i.is_dir { String::new() } else { Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default() };
                CloudEntry {
                    cloud: if i.link.is_empty() { mark(&db, &i.path, i.is_dir, Some(&i.etag)) } else { String::new() },
                    link: i.link,
                    local: sync::local_path(Path::new(&a.local_root), &i.path).to_string_lossy().into_owned(),
                    path: format!("morfa://cloud/{id}/{}", i.path),
                    hidden: name.starts_with('.'),
                    is_dir: i.is_dir,
                    is_symlink: i.symlink,
                    size: i.size,
                    modified: i.modified,
                    ext,
                    name,
                }
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Account id and rel path for a local path inside some account's folder.
fn resolve_local(inner: &Inner, path: &str) -> Option<(String, String)> {
    let p = Path::new(path);
    lock(&inner.accounts).iter().find_map(|a| {
        let rest = p.strip_prefix(&a.local_root).ok()?;
        let rel = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
        Some((a.id.clone(), rel))
    })
}

/// Sync marks for the entries of a local folder (empty if it isn't inside a cloud folder).
#[tauri::command]
pub async fn cloud_marks(state: State<'_, CloudState>, dir: String, names: Vec<String>) -> CmdResult<HashMap<String, String>> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
    let Some((id, rel)) = resolve_local(&inner, &dir) else { return HashMap::new() };
    let db = inner.db(&id);
    let db = lock(&db);
    names
        .into_iter()
        .filter(|n| !sync::ignored(n))
        .filter_map(|n| {
            let child = if rel.is_empty() { n.clone() } else { format!("{rel}/{n}") };
            let is_dir = db.items.get(&child).map(|r| r.dir).unwrap_or_else(|| db.has_pins_inside(&child));
            let m = mark(&db, &child, is_dir, None);
            (!m.is_empty()).then_some((n, m))
        })
        .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

/// Where a local path sits in the cloud: `[id, rel]`.
#[tauri::command]
pub fn cloud_resolve(state: State<'_, CloudState>, path: String) -> Option<(String, String)> {
    resolve_local(&state.0, &path)
}

/// Keep `rel` on this device and sync it in the background.
#[tauri::command]
pub async fn cloud_pin(state: State<'_, CloudState>, id: String, rels: Vec<String>) -> CmdResult<()> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        inner.account(&id)?;
        let db = inner.db(&id);
        {
            let mut db = lock(&db);
            for rel in rels {
                let rel = clean_rel(&rel)?;
                if db.pin_of(&rel).is_some() {
                    continue;
                }
                // A wider pin replaces the narrower ones inside it.
                db.pins.retain(|p| !under(p, &rel));
                db.pins.insert(rel);
            }
        }
        inner.save(&id);
        inner.enqueue(&id);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Stop syncing `rel`. With `delete_local`, local copies that match the server go to the trash.
#[tauri::command]
pub async fn cloud_unpin(state: State<'_, CloudState>, id: String, rels: Vec<String>, delete_local: bool) -> CmdResult<()> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let a = inner.account(&id)?;
        let rels = rels.iter().map(|r| clean_rel(r)).collect::<CmdResult<Vec<_>>>()?;
        let run = inner.run_lock(&id);
        let _guard = lock(&run);
        let db = inner.db(&id);
        let mut trash = vec![];
        {
            let mut db = lock(&db);
            for rel in &rels {
                let rel = rel.as_str();
                if let Some(wider) = db.pin_of(rel).filter(|p| p.as_str() != rel) {
                    return Err(format!("“{}” is synced as a whole — stop syncing that folder instead", if wider.is_empty() { &a.name } else { wider }));
                }
                if !db.pins.contains(rel) && !db.has_pins_inside(rel) {
                    continue;
                }
                db.pins.retain(|p| !under(p, rel));
                if delete_local {
                    // Only files still identical to what was synced; edited ones are kept.
                    for (k, r) in db.items.range(rel.to_string()..) {
                        if !under(k, rel) {
                            break;
                        }
                        let p = sync::local_path(Path::new(&a.local_root), k);
                        let same = !r.dir && std::fs::metadata(&p).is_ok_and(|m| {
                            let mt = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64);
                            m.len() == r.size && mt == Some(r.mtime)
                        });
                        if same {
                            trash.push(p.to_string_lossy().into_owned());
                        }
                    }
                }
                db.drop_under(rel);
            }
        }
        inner.save(&id);
        if !trash.is_empty() {
            crate::fsops::trashcan::trash_blocking(&trash)?;
            // Remove folders left empty by the cleanup, deepest first.
            for rel in &rels {
                remove_empty_dirs(&sync::local_path(Path::new(&a.local_root), rel));
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn remove_empty_dirs(p: &Path) {
    let Ok(rd) = std::fs::read_dir(p) else { return };
    for e in rd.flatten() {
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            remove_empty_dirs(&e.path());
        }
    }
    let _ = std::fs::remove_dir(p);
}

/// Pin `rel` and sync it right away; returns the local path (to open a file from the cloud view).
#[tauri::command]
pub async fn cloud_fetch(app: AppHandle, state: State<'_, CloudState>, id: String, rel: String) -> CmdResult<String> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let a = inner.account(&id)?;
        let rel = clean_rel(&rel)?;
        let pin = {
            let db = inner.db(&id);
            let mut db = lock(&db);
            match db.pin_of(&rel) {
                Some(p) => p.clone(),
                None => {
                    db.pins.insert(rel.clone());
                    rel.clone()
                }
            }
        };
        inner.save(&id);
        let report = inner.sync_account(&app, &id, Some(&pin))?;
        let local = sync::local_path(Path::new(&a.local_root), &rel);
        if local.exists() {
            Ok(local.to_string_lossy().into_owned())
        } else {
            Err(report.errors.into_iter().next().unwrap_or_else(|| "Download failed".into()))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Answer a held deletion: `confirm` applies exactly what was held, otherwise it's restored from the
/// side that still has it. Either way the work happens in the next sync, queued here.
#[tauri::command]
pub async fn cloud_hold_resolve(state: State<'_, CloudState>, id: String, pin: String, to_server: bool, confirm: bool, digest: String) -> CmdResult<()> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        inner.account(&id)?;
        let run = inner.run_lock(&id);
        let _guard = lock(&run);
        let db = inner.db(&id);
        sync::resolve_hold(&mut lock(&db), &pin, to_server, confirm, &digest)?;
        inner.save(&id);
        inner.enqueue(&id);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Sync now (one account, or all).
#[tauri::command]
pub fn cloud_sync(state: State<'_, CloudState>, id: Option<String>) {
    match id {
        Some(id) => state.0.enqueue(&id),
        None => state.0.enqueue_all(),
    }
}

#[cfg(test)]
mod tests {
    use super::{dav::RemoteItem, sync::Remote, Guarded};
    use std::{path::Path, sync::Arc};

    struct Ok1;
    impl Remote for Ok1 {
        fn list(&self, _: &str) -> Result<Vec<RemoteItem>, String> { Ok(vec![RemoteItem { path: "a".into(), ..Default::default() }]) }
        fn stat(&self, _: &str) -> Result<Option<RemoteItem>, String> { Err("offline".into()) }
        fn download(&self, _: &str, _: &Path, c: &dyn Fn() -> bool) -> Result<(), String> {
            while !c() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err("Cancelled".into())
        }
        fn upload(&self, _: &str, _: &Path, _: u64, t: &dyn Fn()) -> Result<Option<String>, String> {
            t();
            Ok(Some("e".into()))
        }
        fn mkcol(&self, _: &str) -> Result<(), String> { Ok(()) }
        fn delete(&self, _: &str) -> Result<(), String> { Ok(()) }
    }

    #[test]
    fn guarded_passes_calls_through_and_can_be_cancelled() {
        let g = Guarded { inner: Arc::new(Ok1), key: "cloud:test".into(), label: "test".into() };
        assert_eq!(g.list("").unwrap().len(), 1);
        assert_eq!(g.stat("").unwrap_err(), "offline");
        assert_eq!(g.upload("a", Path::new("/x"), 0, &|| {}).unwrap(), Some("e".to_string()));
        let flag = std::sync::atomic::AtomicUsize::new(0);
        let e = g.download("a", Path::new("/x"), &|| flag.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 3).unwrap_err();
        assert!(e.to_lowercase().contains("cancelled"), "{e}");
    }

    #[test]
    fn dedicated_folder_check() {
        let home = Some(Path::new("/Users/me"));
        assert!(super::dedicated(Path::new("/Users/me/Cloud"), home));
        for bad in ["/", "/Users", "/Users/me"] {
            assert!(!super::dedicated(Path::new(bad), home), "{bad}");
        }
    }

    #[test]
    fn rejects_hostile_rels_from_the_frontend() {
        assert_eq!(super::clean_rel("/a/b/").unwrap(), "a/b");
        assert!(super::clean_rel("../x").is_err());
        assert!(super::clean_rel("a/../../x").is_err());
    }

    #[test]
    fn stamps() {
        assert_eq!(super::stamp(1_793_700_000), "2026-11-03 100000");
    }
}
