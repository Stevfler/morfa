//! Against an in-process SFTP server on localhost with throwaway keys and passwords.
use super::*;
use crate::cloud::store::SyncDb;
use crate::cloud::sync;
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{self, Auth, ChannelOpenHandle, Msg, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{Attrs, Data, File, FileAttributes, FileMode, Handle, Name, Status, Version};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

const USER: &str = "tester";

#[derive(Clone)]
enum Node {
    Dir,
    File(Vec<u8>, u32),
    Link(String),
}

#[derive(Default)]
struct Fs {
    nodes: BTreeMap<String, Node>,
    /// Extra raw names a hostile server puts in a listing of "/home".
    hostile: Vec<String>,
    /// Offer posix-rename, as OpenSSH does.
    posix: bool,
    removes: usize,
    /// Plain rename answers this for everything.
    rename_status: Option<StatusCode>,
    /// `stat` on these fails with a permission error.
    stat_denied: Vec<String>,
}

fn attrs(n: &Node) -> FileAttributes {
    let mut a = FileAttributes::empty();
    match n {
        Node::Dir => a.permissions = Some(FileMode::DIR.bits() | 0o755),
        Node::File(d, m) => {
            a.permissions = Some(FileMode::REG.bits() | 0o644);
            a.size = Some(d.len() as u64);
            a.mtime = Some(*m);
        }
        Node::Link(_) => a.permissions = Some(FileMode::LNK.bits() | 0o777),
    }
    a
}

struct SftpSrv {
    fs: Arc<Mutex<Fs>>,
    served: HashSet<String>,
}
fn ok(id: u32) -> Status {
    Status { id, status_code: StatusCode::Ok, error_message: "Ok".into(), language_tag: "en".into() }
}
impl SftpSrv {
    fn fs(&self) -> std::sync::MutexGuard<'_, Fs> {
        self.fs.lock().unwrap()
    }
}
impl russh_sftp::server::Handler for SftpSrv {
    type Error = StatusCode;
    fn unimplemented(&self) -> StatusCode {
        StatusCode::OpUnsupported
    }
    async fn init(&mut self, _: u32, _: HashMap<String, String>) -> Result<Version, StatusCode> {
        let mut v = Version::new();
        if self.fs().posix {
            v.extensions.insert(POSIX_RENAME.into(), "1".into());
        }
        Ok(v)
    }
    async fn extended(&mut self, id: u32, request: String, data: Vec<u8>) -> Result<russh_sftp::protocol::Packet, StatusCode> {
        assert_eq!(request, POSIX_RENAME);
        let mut at = 0;
        let mut next = || {
            let n = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
            let s = String::from_utf8(data[at + 4..at + 4 + n].to_vec()).unwrap();
            at += 4 + n;
            s
        };
        let (from, to) = (next(), next());
        let mut fs = self.fs();
        let n = fs.nodes.remove(&from).ok_or(StatusCode::NoSuchFile)?;
        fs.nodes.insert(to, n);
        Ok(russh_sftp::protocol::Packet::Status(ok(id)))
    }
    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, StatusCode> {
        let p = if path == "." { "/home".to_string() } else { path };
        Ok(Name { id, files: vec![File::dummy(p)] })
    }
    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, StatusCode> {
        match self.fs().nodes.get(&path) {
            Some(Node::Dir) => {}
            _ => return Err(StatusCode::NoSuchFile),
        }
        self.served.remove(&path);
        Ok(Handle { id, handle: path })
    }
    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, StatusCode> {
        if !self.served.insert(handle.clone()) {
            return Err(StatusCode::Eof);
        }
        let fs = self.fs();
        let prefix = format!("{}/", handle.trim_end_matches('/'));
        let mut files = vec![File::new(".", attrs(&Node::Dir)), File::new("..", attrs(&Node::Dir))];
        for (k, n) in &fs.nodes {
            if let Some(name) = k.strip_prefix(&prefix) {
                if !name.is_empty() && !name.contains('/') {
                    files.push(File::new(name, attrs(n)));
                }
            }
        }
        if handle == "/home" {
            files.extend(fs.hostile.iter().map(|h| File::new(h.as_str(), attrs(&Node::File(b"evil".to_vec(), 1)))));
        }
        drop(fs);
        Ok(Name { id, files })
    }
    async fn close(&mut self, id: u32, _: String) -> Result<Status, StatusCode> {
        Ok(ok(id))
    }
    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        let n = self.fs().nodes.get(&path).cloned().ok_or(StatusCode::NoSuchFile)?;
        Ok(Attrs { id, attrs: attrs(&n) })
    }
    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        let fs = self.fs();
        if fs.stat_denied.contains(&path) {
            return Err(StatusCode::PermissionDenied);
        }
        let mut n = fs.nodes.get(&path).ok_or(StatusCode::NoSuchFile)?;
        if let Node::Link(t) = n {
            n = fs.nodes.get(t).ok_or(StatusCode::NoSuchFile)?;
        }
        Ok(Attrs { id, attrs: attrs(n) })
    }
    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, StatusCode> {
        self.stat(id, handle).await
    }
    async fn setstat(&mut self, id: u32, path: String, a: FileAttributes) -> Result<Status, StatusCode> {
        if let (Some(Node::File(_, m)), Some(t)) = (self.fs().nodes.get_mut(&path), a.mtime) {
            *m = t;
        }
        Ok(ok(id))
    }
    async fn open(&mut self, id: u32, path: String, flags: OpenFlags, _: FileAttributes) -> Result<Handle, StatusCode> {
        let mut fs = self.fs();
        if flags.contains(OpenFlags::CREATE) {
            fs.nodes.insert(path.clone(), Node::File(vec![], 1000));
        } else if !fs.nodes.contains_key(&path) {
            return Err(StatusCode::NoSuchFile);
        }
        drop(fs);
        Ok(Handle { id, handle: path })
    }
    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, StatusCode> {
        let fs = self.fs();
        let n = match fs.nodes.get(&handle) {
            Some(Node::Link(l)) => fs.nodes.get(l),
            n => n,
        };
        match n {
            Some(Node::File(d, _)) if (offset as usize) < d.len() => {
                let end = (offset as usize + len as usize).min(d.len());
                Ok(Data { id, data: d[offset as usize..end].to_vec() })
            }
            _ => Err(StatusCode::Eof),
        }
    }
    async fn write(&mut self, id: u32, handle: String, offset: u64, data: Vec<u8>) -> Result<Status, StatusCode> {
        if let Some(Node::File(d, _)) = self.fs().nodes.get_mut(&handle) {
            d.resize(offset as usize, 0);
            d.extend_from_slice(&data);
        }
        Ok(ok(id))
    }
    async fn remove(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        let mut fs = self.fs();
        fs.nodes.remove(&path).ok_or(StatusCode::NoSuchFile)?;
        fs.removes += 1;
        Ok(ok(id))
    }
    async fn mkdir(&mut self, id: u32, path: String, _: FileAttributes) -> Result<Status, StatusCode> {
        let mut fs = self.fs();
        if fs.nodes.contains_key(&path) {
            return Err(StatusCode::Failure);
        }
        fs.nodes.insert(path, Node::Dir);
        Ok(ok(id))
    }
    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        let mut fs = self.fs();
        let inside = format!("{path}/");
        if fs.nodes.keys().any(|k| k.starts_with(&inside)) {
            return Err(StatusCode::Failure);
        }
        fs.nodes.remove(&path);
        Ok(ok(id))
    }
    async fn rename(&mut self, id: u32, from: String, to: String) -> Result<Status, StatusCode> {
        let mut fs = self.fs();
        if let Some(s) = fs.rename_status {
            return Err(s);
        }
        // Like OpenSSH: plain SFTP rename doesn't overwrite.
        if fs.nodes.contains_key(&to) {
            return Err(StatusCode::Failure);
        }
        let n = fs.nodes.remove(&from).ok_or(StatusCode::NoSuchFile)?;
        fs.nodes.insert(to, n);
        Ok(ok(id))
    }
}

struct SshSrv {
    fs: Arc<Mutex<Fs>>,
    password: String,
    client_key: Option<russh::keys::PublicKey>,
    auths: Arc<AtomicUsize>,
    channels: HashMap<ChannelId, Channel<Msg>>,
}
impl server::Handler for SshSrv {
    type Error = russh::Error;
    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.auths.fetch_add(1, Ordering::SeqCst);
        Ok(if user == USER && password == self.password { Auth::Accept } else { Auth::reject() })
    }
    async fn auth_publickey(&mut self, user: &str, key: server::VerifiedPublicKey<'_>) -> Result<Auth, Self::Error> {
        self.auths.fetch_add(1, Ordering::SeqCst);
        Ok(if user == USER && self.client_key.as_ref() == Some(key.public_key) { Auth::Accept } else { Auth::reject() })
    }
    async fn channel_open_session(&mut self, channel: Channel<Msg>, reply: ChannelOpenHandle, _: &mut Session) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }
    async fn subsystem_request(&mut self, id: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        match (name, self.channels.remove(&id)) {
            ("sftp", Some(ch)) => {
                session.channel_success(id)?;
                let h = SftpSrv { fs: self.fs.clone(), served: HashSet::new() };
                tokio::spawn(russh_sftp::server::run(ch.into_stream(), h));
            }
            _ => session.channel_failure(id)?,
        }
        Ok(())
    }
}

struct Srv {
    port: u16,
    fingerprint: String,
    fs: Arc<Mutex<Fs>>,
    auths: Arc<AtomicUsize>,
    password: String,
}

fn throwaway(tag: &str) -> String {
    use sha2::Digest;
    let seed = format!("{tag}-{}-{:?}", std::process::id(), std::time::SystemTime::now());
    sha2::Sha256::digest(seed).iter().map(|b| format!("{b:02x}")).collect()
}

fn fresh_key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn serve(port: u16, key: PrivateKey, client_key: Option<russh::keys::PublicKey>) -> Srv {
    let fp = key.public_key().fingerprint(russh::keys::HashAlg::Sha256).to_string();
    let fs = Arc::new(Mutex::new(Fs::default()));
    {
        let mut f = fs.lock().unwrap();
        f.nodes.insert("/home".into(), Node::Dir);
        f.nodes.insert("/home/Docs".into(), Node::Dir);
        f.nodes.insert("/home/Docs/a.txt".into(), Node::File(b"hello".to_vec(), 2000));
        f.nodes.insert("/secret-outside".into(), Node::File(b"no".to_vec(), 2000));
    }
    let password = throwaway("pw");
    let auths = Arc::new(AtomicUsize::new(0));
    let cfg = Arc::new(server::Config { keys: vec![key], auth_rejection_time: Duration::from_millis(10), auth_rejection_time_initial: Some(Duration::ZERO), ..Default::default() });
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (f2, pw, a2) = (fs.clone(), password.clone(), auths.clone());
    RT.spawn(async move {
        let l = tokio::net::TcpListener::from_std(listener).unwrap();
        while let Ok((s, _)) = l.accept().await {
            let h = SshSrv { fs: f2.clone(), password: pw.clone(), client_key: client_key.clone(), auths: a2.clone(), channels: HashMap::new() };
            let cfg = cfg.clone();
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(cfg, s, h).await {
                    let _ = session.await;
                }
            });
        }
    });
    Srv { port, fingerprint: fp, fs, auths, password }
}

impl Srv {
    fn start() -> Srv {
        serve(0, fresh_key(), None)
    }
    fn target(&self, fingerprint: &str) -> Target {
        Target { host: "127.0.0.1".into(), port: self.port, login: USER.into(), remote_path: String::new(), host_key: fingerprint.into() }
    }
    fn client(&self) -> Sftp {
        Sftp::new(self.target(&self.fingerprint), Credential::Password(self.password.clone()))
    }
    fn file(&self, path: &str) -> Option<Vec<u8>> {
        match self.fs.lock().unwrap().nodes.get(path) {
            Some(Node::File(d, _)) => Some(d.clone()),
            _ => None,
        }
    }
}

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("morfa-sftp-{tag}-{}", throwaway("d")));
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn names(v: &[RemoteItem]) -> Vec<&str> {
    let mut n: Vec<&str> = v.iter().map(|i| i.path.as_str()).collect();
    n.sort();
    n
}

#[test]
fn probe_returns_the_fingerprint_and_sends_no_credentials() {
    let s = Srv::start();
    assert_eq!(probe("127.0.0.1", s.port).unwrap(), s.fingerprint);
    assert_eq!(s.auths.load(Ordering::SeqCst), 0);
}

#[test]
fn lists_and_stats() {
    let s = Srv::start();
    let c = s.client();
    let root = c.list("").unwrap();
    assert_eq!(names(&root), ["Docs"]);
    assert!(root[0].is_dir);
    let f = c.list("Docs").unwrap();
    assert_eq!(names(&f), ["Docs/a.txt"]);
    assert_eq!((f[0].size, f[0].modified), (5, 2_000_000));
    assert!(!f[0].etag.is_empty());
    assert_eq!(c.stat("Docs/a.txt").unwrap().unwrap().size, 5);
    assert_eq!(c.stat("nope").unwrap(), None);
    assert!(c.list("nope").is_err());
}

#[test]
fn downloads_and_cancels() {
    let s = Srv::start();
    let c = s.client();
    let t = Tmp::new("dl");
    let dest = t.0.join("a.txt");
    c.download("Docs/a.txt", &dest, &|| false).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
    assert_eq!(c.download("Docs/a.txt", &dest, &|| true).unwrap_err(), "Cancelled");
}

#[test]
fn changed_host_key_is_a_hard_failure_before_credentials() {
    let s = Srv::start();
    let pinned = fresh_key().public_key().fingerprint(russh::keys::HashAlg::Sha256).to_string();
    let c = Sftp::new(s.target(&pinned), Credential::Password(s.password.clone()));
    let e = c.list("").unwrap_err();
    assert!(e.starts_with(HOST_KEY_CHANGED), "{e}");
    assert!(e.contains(&pinned) && e.contains(&s.fingerprint), "{e}");
    assert!(!e.contains(&s.password));
    assert_eq!(s.auths.load(Ordering::SeqCst), 0, "no credentials may reach that server");
    // Still refused on retry: nothing was remembered.
    assert!(c.stat("Docs").unwrap_err().starts_with(HOST_KEY_CHANGED));
}

/// The same address comes back with another key while a client holds a working pin.
#[test]
fn key_change_on_the_same_address_stops_a_working_account() {
    let first = Srv::start();
    let c = first.client();
    c.list("").unwrap();
    // A second server plays the replacement; the account's pin is still the first key.
    let other = Srv::start();
    let moved = Sftp::new(other.target(&first.fingerprint), Credential::Password(other.password.clone()));
    assert!(moved.list("").unwrap_err().starts_with(HOST_KEY_CHANGED));
    assert_eq!(other.auths.load(Ordering::SeqCst), 0);
}

#[test]
fn wrong_password_is_refused_without_echoing_it() {
    let s = Srv::start();
    let wrong = throwaway("wrong");
    let c = Sftp::new(s.target(&s.fingerprint), Credential::Password(wrong.clone()));
    let e = c.list("").unwrap_err();
    assert!(e.contains("rejected") && !e.contains(&wrong), "{e}");
}

#[test]
fn signs_in_with_a_passphrase_protected_key_file() {
    let client = fresh_key();
    let s = serve(0, fresh_key(), Some(client.public_key().clone()));
    let t = Tmp::new("key");
    let path = t.0.join("id_test");
    let pass = throwaway("pass");
    let enc = client.encrypt(&mut rand::rng(), pass.as_bytes()).unwrap();
    std::fs::write(&path, enc.to_openssh(russh::keys::ssh_key::LineEnding::LF).unwrap().as_bytes()).unwrap();
    let with = |p: Option<String>| Sftp::new(s.target(&s.fingerprint), Credential::Key { path: path.clone(), passphrase: p });
    assert_eq!(names(&with(Some(pass.clone())).list("").unwrap()), ["Docs"]);
    let bad = throwaway("bad");
    let e = with(Some(bad.clone())).list("").unwrap_err();
    assert!(e.contains("private key") && !e.contains(&bad), "{e}");
    assert!(with(None).list("").unwrap_err().contains("passphrase"));
}

#[test]
fn hostile_names_are_filtered_from_listings() {
    let s = Srv::start();
    s.fs.lock().unwrap().hostile = ["../../escape.txt", "..", "/etc/evil", "a/../../b", "..\\..\\win", "ctl\nname", "fine.txt"].map(String::from).to_vec();
    let c = s.client();
    assert_eq!(names(&c.list("").unwrap()), ["Docs", "fine.txt"]);
}

#[test]
fn hostile_names_never_land_outside_the_mirror() {
    let s = Srv::start();
    s.fs.lock().unwrap().hostile = ["../escape.txt", "../../escape.txt", "/tmp/morfa-sftp-escape.txt"].map(String::from).to_vec();
    let t = Tmp::new("evil");
    let root = t.0.join("mirror");
    std::fs::create_dir_all(&root).unwrap();
    let c = s.client();
    let db = Mutex::new(SyncDb::default());
    db.lock().unwrap().pins.insert(String::new());
    let trash = |_: &Path| -> Result<(), String> { panic!("nothing to trash") };
    let ctx = sync::Ctx { remote: &c, root: root.clone(), db: &db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
    let rep = sync::sync_pin(&ctx, "").unwrap();
    assert!(rep.errors.is_empty(), "{:?}", rep.errors);
    assert_eq!(std::fs::read(root.join("Docs/a.txt")).unwrap(), b"hello");
    assert!(!t.0.join("escape.txt").exists());
    assert!(!Path::new("/tmp/morfa-sftp-escape.txt").exists());
}

#[test]
fn rel_paths_cannot_climb_above_the_account_root() {
    let s = Srv::start();
    let c = s.client();
    for bad in ["../secret-outside", "Docs/../../secret-outside", "/secret-outside"] {
        assert_eq!(c.stat(bad).unwrap_err(), "Invalid path", "{bad}");
    }
    assert!(c.delete("").is_err());
    assert!(s.file("/secret-outside").is_some());
}

#[test]
fn links_are_followed_for_type_and_dead_ones_hidden() {
    let s = Srv::start();
    {
        let mut f = s.fs.lock().unwrap();
        f.nodes.insert("/home/up".into(), Node::Link("/home".into()));
        f.nodes.insert("/home/file-link".into(), Node::Link("/home/Docs/a.txt".into()));
        f.nodes.insert("/home/dead".into(), Node::Link("/gone".into()));
    }
    let c = s.client();
    let v = c.list("").unwrap();
    assert_eq!(names(&v), ["Docs", "file-link", "up"]);
    let up = v.iter().find(|i| i.path == "up").unwrap();
    assert!(up.symlink && up.is_dir);
    let fl = v.iter().find(|i| i.path == "file-link").unwrap();
    assert!(fl.symlink && !fl.is_dir && fl.size == 5);
    // The walk stops at the link, so this loop doesn't recurse forever.
    let t = Tmp::new("loop");
    let db = Mutex::new(SyncDb::default());
    db.lock().unwrap().pins.insert(String::new());
    let trash = |_: &Path| -> Result<(), String> { Ok(()) };
    let ctx = sync::Ctx { remote: &c, root: t.0.clone(), db: &db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
    assert!(sync::sync_pin(&ctx, "").unwrap().errors.is_empty());
    assert!(!t.0.join("up").exists());
}

/// The whole engine over SFTP: pin, download, edit both ways, delete both ways.
#[test]
fn pinned_folder_syncs_both_ways() {
    let s = Srv::start();
    let c = s.client();
    let t = Tmp::new("sync");
    let db = Mutex::new(SyncDb::default());
    db.lock().unwrap().pins.insert("Docs".into());
    let trash = |p: &Path| std::fs::remove_file(p).map_err(|e| e.to_string());
    let ctx = sync::Ctx { remote: &c, root: t.0.clone(), db: &db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
    let sync_ok = || {
        let r = sync::sync_pin(&ctx, "Docs").unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        r
    };
    sync_ok();
    let local = t.0.join("Docs/a.txt");
    assert_eq!(std::fs::read(&local).unwrap(), b"hello");
    assert!(!sync_ok().changed, "second pass has nothing to do");

    // Local edit and new local folder go up, over an existing file (rename can't overwrite).
    std::fs::write(&local, b"edited").unwrap();
    let f = std::fs::File::options().write(true).open(&local).unwrap();
    f.set_modified(std::time::SystemTime::now() + Duration::from_secs(5)).unwrap();
    drop(f);
    std::fs::create_dir_all(t.0.join("Docs/new")).unwrap();
    std::fs::write(t.0.join("Docs/new/b.txt"), b"B").unwrap();
    sync_ok();
    assert_eq!(s.file("/home/Docs/a.txt").unwrap(), b"edited");
    assert_eq!(s.file("/home/Docs/new/b.txt").unwrap(), b"B");
    assert!(!s.fs.lock().unwrap().nodes.keys().any(|k| k.ends_with(sync::PART)), "no temp files left");
    assert!(!sync_ok().changed, "uploads are recognised on the next pass");

    // A change deep inside on the server is noticed although SFTP has no folder ETags.
    s.fs.lock().unwrap().nodes.insert("/home/Docs/new/b.txt".into(), Node::File(b"B2!".to_vec(), 9000));
    sync_ok();
    assert_eq!(std::fs::read(t.0.join("Docs/new/b.txt")).unwrap(), b"B2!");

    // Deleted locally → deleted on the server; deleted on the server → local copy goes to trash.
    std::fs::remove_file(t.0.join("Docs/new/b.txt")).unwrap();
    s.fs.lock().unwrap().nodes.remove("/home/Docs/a.txt");
    sync_ok();
    assert!(s.file("/home/Docs/new/b.txt").is_none());
    assert!(!local.exists());
}

#[test]
fn delete_never_removes_a_nonempty_folder() {
    let s = Srv::start();
    let c = s.client();
    assert!(c.delete("Docs").is_err());
    assert!(s.file("/home/Docs/a.txt").is_some());
    c.delete("Docs/a.txt").unwrap();
    c.delete("Docs/a.txt").unwrap();
}

/// Accepts the connection and never says a word.
#[test]
fn silent_server_times_out() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let started = std::time::Instant::now();
    assert!(probe_within("127.0.0.1", port, Duration::from_millis(300)).unwrap_err().contains("in time"));
    let mut c = Sftp::new(Target { host: "127.0.0.1".into(), port, login: USER.into(), remote_path: String::new(), host_key: "SHA256:x".into() }, Credential::Password("unused".into()));
    c.connect_timeout = Duration::from_millis(300);
    assert!(c.list("").unwrap_err().contains("in time"));
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(l);
}

fn local_file(t: &Tmp, data: &[u8]) -> PathBuf {
    let f = t.0.join("up.bin");
    std::fs::write(&f, data).unwrap();
    f
}
fn parts(s: &Srv) -> usize {
    s.fs.lock().unwrap().nodes.keys().filter(|k| k.ends_with(sync::PART)).count()
}

#[test]
fn posix_rename_overwrites_without_removing() {
    let s = Srv::start();
    s.fs.lock().unwrap().posix = true;
    let t = Tmp::new("posix");
    s.client().upload("Docs/a.txt", &local_file(&t, b"new"), 5000, &|| {}).unwrap();
    assert_eq!(s.file("/home/Docs/a.txt").unwrap(), b"new");
    assert_eq!(s.fs.lock().unwrap().removes, 0);
    assert_eq!(parts(&s), 0);
}

/// Only a plain "failure" may lead to removing the original.
#[test]
fn denied_rename_keeps_the_original_and_cleans_up() {
    let s = Srv::start();
    s.fs.lock().unwrap().rename_status = Some(StatusCode::PermissionDenied);
    let t = Tmp::new("denied");
    assert!(s.client().upload("Docs/a.txt", &local_file(&t, b"new"), 5000, &|| {}).is_err());
    assert_eq!(s.file("/home/Docs/a.txt").unwrap(), b"hello");
    assert_eq!(parts(&s), 0);
}

/// Original removed, then the rename fails too: the temp file is the only copy, so it stays.
#[test]
fn temp_is_kept_when_the_original_is_already_gone() {
    let s = Srv::start();
    s.fs.lock().unwrap().rename_status = Some(StatusCode::Failure);
    let t = Tmp::new("kept");
    let e = s.client().upload("Docs/a.txt", &local_file(&t, b"new"), 5000, &|| {}).unwrap_err();
    assert!(e.contains("kept"), "{e}");
    assert!(s.file("/home/Docs/a.txt").is_none());
    assert_eq!(parts(&s), 1);
}

/// A link whose target can't be checked right now fails the listing rather than vanishing from it.
#[test]
fn transient_stat_failure_does_not_hide_an_item() {
    let s = Srv::start();
    {
        let mut f = s.fs.lock().unwrap();
        f.nodes.insert("/home/link".into(), Node::Link("/home/Docs/a.txt".into()));
        f.stat_denied.push("/home/link".into());
    }
    assert!(s.client().list("").is_err());
}

#[test]
fn reconnects_when_the_sftp_channel_dies() {
    let s = Srv::start();
    let mut c = s.client();
    c.probe_after = Duration::ZERO;
    c.list("").unwrap();
    let conn = RT.block_on(c.conn()).unwrap();
    RT.block_on(conn.sftp.close()).unwrap();
    assert!(!conn.closed(), "the SSH session itself is fine");
    drop(conn);
    assert_eq!(names(&c.list("").unwrap()), ["Docs"]);
}

#[test]
fn stale_server_parts_are_reclaimed_during_listing() {
    let s = Srv::start();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as u32;
    {
        let mut f = s.fs.lock().unwrap();
        f.nodes.insert("/home/Docs/.morfa-old.morfa-part".into(), Node::File(b"t".to_vec(), 1000));
        f.nodes.insert("/home/Docs/.morfa-new.morfa-part".into(), Node::File(b"t".to_vec(), now));
        f.nodes.insert("/home/Docs/.other.morfa-part".into(), Node::File(b"t".to_vec(), 1000));
    }
    let v = s.client().list("Docs").unwrap();
    assert!(!names(&v).contains(&"Docs/.morfa-old.morfa-part"));
    assert!(s.file("/home/Docs/.morfa-old.morfa-part").is_none());
    assert!(s.file("/home/Docs/.morfa-new.morfa-part").is_some());
    assert!(s.file("/home/Docs/.other.morfa-part").is_some(), "not ours by name");
}

#[test]
fn checks_hosts() {
    assert_eq!(check_host(" example.com ").unwrap(), "example.com");
    assert_eq!(check_host("[::1]").unwrap(), "::1");
    for bad in ["", "a b", "sftp://x", "me@x", "x/y"] {
        assert!(check_host(bad).is_err(), "{bad}");
    }
}

/// Interop with a real OpenSSH on localhost, started by hand with throwaway keys:
/// MORFA_SFTP_PORT, MORFA_SFTP_KEY (private key file), MORFA_SFTP_DIR (scratch folder on the "server").
#[cfg(unix)]
#[test]
#[ignore = "needs a local sshd"]
fn interop_with_local_openssh() {
    let env = |k: &str| std::env::var(k).unwrap();
    let port: u16 = env("MORFA_SFTP_PORT").parse().unwrap();
    let dir = env("MORFA_SFTP_DIR");
    let fp = probe("127.0.0.1", port).unwrap();
    let c = Sftp::new(
        Target { host: "127.0.0.1".into(), port, login: env("USER"), remote_path: dir.clone(), host_key: fp },
        Credential::Key { path: env("MORFA_SFTP_KEY").into(), passphrase: None },
    );
    std::fs::create_dir_all(format!("{dir}/Docs/sub")).unwrap();
    std::fs::write(format!("{dir}/Docs/sub/a.txt"), b"hello").unwrap();
    std::fs::write(format!("{dir}/Docs/private.txt"), b"p").unwrap();
    std::fs::set_permissions(format!("{dir}/Docs/private.txt"), std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink("..", format!("{dir}/Docs/up")).unwrap();
    let t = Tmp::new("interop");
    let db = Mutex::new(SyncDb::default());
    db.lock().unwrap().pins.insert("Docs".into());
    let trash = |p: &Path| std::fs::remove_file(p).map_err(|e| e.to_string());
    let ctx = sync::Ctx { remote: &c, root: t.0.clone(), db: &db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
    let go = || {
        let r = sync::sync_pin(&ctx, "Docs").unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        r
    };
    go();
    assert_eq!(std::fs::read(t.0.join("Docs/sub/a.txt")).unwrap(), b"hello");
    assert!(!t.0.join("Docs/up").exists());
    assert!(!go().changed);
    // Overwrite keeps the mode; new file and folder go up.
    let local = t.0.join("Docs/private.txt");
    std::fs::write(&local, b"edited").unwrap();
    std::fs::File::options().write(true).open(&local).unwrap().set_modified(std::time::SystemTime::now() + Duration::from_secs(5)).unwrap();
    std::fs::create_dir_all(t.0.join("Docs/new")).unwrap();
    std::fs::write(t.0.join("Docs/new/b.txt"), vec![7u8; 3_000_000]).unwrap();
    go();
    assert_eq!(std::fs::read(format!("{dir}/Docs/private.txt")).unwrap(), b"edited");
    assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(format!("{dir}/Docs/private.txt")).unwrap().permissions()) & 0o777, 0o600);
    assert_eq!(std::fs::metadata(format!("{dir}/Docs/new/b.txt")).unwrap().len(), 3_000_000);
    assert!(!go().changed);
    std::fs::remove_file(t.0.join("Docs/new/b.txt")).unwrap();
    go();
    assert!(!Path::new(&format!("{dir}/Docs/new/b.txt")).exists());
}
