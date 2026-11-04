//! SFTP client on russh + russh-sftp behind the same `Remote` interface as WebDAV.
//!
//! Security decisions: the host key is compared with the fingerprint the user confirmed before any
//! credentials are sent, and any other key is a hard failure. Names in listings are untrusted and
//! filtered with `names::safe_name`. Rel paths were checked by the caller and by `path_of`.
//!
//! The async client runs on one shared runtime; each method blocks its calling thread, which is
//! never the main one (`mod.rs` wraps this in `Guarded`). Everything has its own timeout too.
use super::dav::RemoteItem;
use super::names::{safe_name, safe_rel};
use russh::client;
use russh_sftp::{
    client::{error::Error as SftpError, rawsession::RawSftpSession, SftpSession},
    protocol::{FileAttributes, OpenFlags, Packet, StatusCode},
};
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    runtime::Runtime,
    time::timeout,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// One request, or one chunk of a transfer.
const OP_TIMEOUT: Duration = Duration::from_secs(60);
const CHUNK: usize = 256 * 1024;
const MAX_KEY_FILE: u64 = 1024 * 1024;
/// A connection idle this long is asked whether its SFTP channel still works before being reused.
const PROBE_AFTER: Duration = Duration::from_secs(5);
const POSIX_RENAME: &str = "posix-rename@openssh.com";
/// Prefix the frontend recognises: `host-key-changed: <server> <expected> <presented>`.
pub const HOST_KEY_CHANGED: &str = "host-key-changed";

static RT: LazyLock<Runtime> = LazyLock::new(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("sftp").enable_all().build().expect("sftp runtime"));

#[derive(Clone)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub login: String,
    /// Account root on the server; empty means the login folder.
    pub remote_path: String,
    /// Fingerprint the user confirmed.
    pub host_key: String,
}

/// No `Debug`: this holds secrets.
#[derive(Clone)]
pub enum Credential {
    Password(String),
    Key { path: PathBuf, passphrase: Option<String> },
}

/// "[::1]" → "::1". A host only: no scheme, path or `user@`.
pub fn check_host(host: &str) -> Result<String, String> {
    let h = host.trim();
    let h = h.strip_prefix('[').and_then(|x| x.strip_suffix(']')).unwrap_or(h);
    if h.is_empty() || h.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | '@')) {
        return Err("Enter the server's host name or IP address".into());
    }
    Ok(h.to_string())
}

struct Checker {
    /// None: only record the key and refuse it (a probe).
    expected: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Checker {
    type Error = russh::Error;
    async fn check_server_key(&mut self, key: &russh::keys::PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let fp = key.public_key().fingerprint(russh::keys::HashAlg::Sha256).to_string();
        let ok = self.expected.as_deref() == Some(fp.as_str());
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(fp);
        Ok(ok)
    }
}

fn ssh_config() -> Arc<client::Config> {
    Arc::new(client::Config { keepalive_interval: Some(Duration::from_secs(20)), keepalive_max: 3, ..Default::default() })
}

/// Opens the TCP and SSH layers: past this point the host key has been accepted.
async fn handshake(host: &str, port: u16, expected: Option<String>) -> Result<client::Handle<Checker>, (russh::Error, Option<String>)> {
    let seen = Arc::new(Mutex::new(None));
    let h = Checker { expected, seen: seen.clone() };
    client::connect(ssh_config(), (host, port), h).await.map_err(|e| (e, seen.lock().unwrap_or_else(|p| p.into_inner()).clone()))
}

/// The server's key fingerprint, for the user to confirm. Nothing but the handshake happens.
pub fn probe(host: &str, port: u16) -> Result<String, String> {
    probe_within(host, port, CONNECT_TIMEOUT)
}

fn probe_within(host: &str, port: u16, limit: Duration) -> Result<String, String> {
    RT.block_on(async {
        match timeout(limit, handshake(host, port, None)).await {
            Err(_) => Err(format!("{host}:{port} didn't answer in time")),
            Ok(Err((_, Some(fp)))) => Ok(fp),
            Ok(Err((e, None))) => Err(format!("Can't reach {host}:{port}: {e}")),
            // Unreachable: with nothing expected the key is always refused.
            Ok(Ok(_)) => Err("The server's key couldn't be read".to_string()),
        }
    })
}

struct Conn {
    /// Dropping it ends the session.
    _ssh: client::Handle<Checker>,
    sftp: SftpSession,
    /// Absolute account root, no trailing slash unless it's "/".
    base: String,
    /// A second channel for `posix-rename`, which `SftpSession` can't send; None if not offered.
    raw: Option<RawSftpSession>,
    used: Mutex<std::time::Instant>,
}

impl Conn {
    fn closed(&self) -> bool {
        self._ssh.is_closed()
    }
    fn path_of(&self, rel: &str) -> Result<String, String> {
        if !safe_rel(rel) {
            return Err("Invalid path".into());
        }
        Ok(match (self.base.as_str(), rel) {
            (b, "") => b.to_string(),
            ("/", r) => format!("/{r}"),
            (b, r) => format!("{b}/{r}"),
        })
    }
}

fn describe(what: &str, e: &SftpError) -> String {
    match e {
        SftpError::Status(s) if s.status_code == StatusCode::PermissionDenied => format!("{what}: access denied"),
        SftpError::Status(s) if s.status_code == StatusCode::NoSuchFile => format!("{what}: not found"),
        e => format!("{what}: {e}"),
    }
}
fn missing(e: &SftpError) -> bool {
    matches!(e, SftpError::Status(s) if s.status_code == StatusCode::NoSuchFile)
}

fn etag(a: &FileAttributes) -> String {
    format!("m{}-{}", a.mtime.unwrap_or(0), a.size.unwrap_or(0))
}
fn item(path: String, a: &FileAttributes, symlink: bool) -> RemoteItem {
    let is_dir = a.is_dir();
    RemoteItem {
        path,
        is_dir,
        size: if is_dir { 0 } else { a.size.unwrap_or(0) },
        modified: u64::from(a.mtime.unwrap_or(0)) * 1000,
        // Folder dates say nothing about what's deeper; left empty so they never look "unchanged".
        etag: if is_dir { String::new() } else { etag(a) },
        symlink,
        ..Default::default()
    }
}

pub struct Sftp {
    target: Target,
    cred: Credential,
    conn: tokio::sync::Mutex<Option<Arc<Conn>>>,
    connect_timeout: Duration,
    op_timeout: Duration,
    probe_after: Duration,
}

impl Sftp {
    pub fn new(target: Target, cred: Credential) -> Sftp {
        Sftp { target, cred, conn: Default::default(), connect_timeout: CONNECT_TIMEOUT, op_timeout: OP_TIMEOUT, probe_after: PROBE_AFTER }
    }

    async fn open(&self) -> Result<Conn, String> {
        let t = &self.target;
        let mut ssh = handshake(&t.host, t.port, Some(t.host_key.clone())).await.map_err(|(e, seen)| match seen {
            Some(fp) if fp != t.host_key => format!("{HOST_KEY_CHANGED}: {}:{} {} {fp}", t.host, t.port, t.host_key),
            _ => format!("Can't reach {}:{}: {e}", t.host, t.port),
        })?;
        // The key matched; only now may credentials go out.
        let auth = match &self.cred {
            Credential::Password(p) => ssh.authenticate_password(&t.login, p).await.map_err(|e| format!("Sign-in failed: {e}"))?,
            Credential::Key { path, passphrase } => {
                let key = Arc::new(load_key(path.clone(), passphrase.clone()).await?);
                let hash = ssh.best_supported_rsa_hash().await.map_err(|e| format!("Sign-in failed: {e}"))?.flatten();
                let signed = russh::keys::PrivateKeyWithHashAlg::new(key, hash);
                ssh.authenticate_publickey(&t.login, signed).await.map_err(|e| format!("Sign-in failed: {e}"))?
            }
        };
        if !auth.success() {
            return Err("The server rejected the login, password or key".into());
        }
        let channel = ssh.channel_open_session().await.map_err(|e| format!("Can't open a session: {e}"))?;
        channel.request_subsystem(true, "sftp").await.map_err(|e| format!("Can't start SFTP: {e}"))?;
        let sftp = SftpSession::new(channel.into_stream()).await.map_err(|e| format!("This server doesn't offer SFTP: {e}"))?;
        sftp.set_timeout(self.op_timeout.as_secs().max(1));
        let start = if t.remote_path.is_empty() { "." } else { t.remote_path.as_str() };
        let base = sftp.canonicalize(start).await.map_err(|e| describe("Open the remote folder", &e))?;
        let base = if base.len() > 1 { base.trim_end_matches('/').to_string() } else { base };
        let secs = self.op_timeout.as_secs().max(1);
        let raw = async {
            let ch = ssh.channel_open_session().await.ok()?;
            ch.request_subsystem(true, "sftp").await.ok()?;
            let raw = RawSftpSession::new(ch.into_stream());
            raw.set_timeout(secs);
            let v = raw.init().await.ok()?;
            v.extensions.contains_key(POSIX_RENAME).then_some(raw)
        }
        .await;
        Ok(Conn { _ssh: ssh, sftp, base, raw, used: Mutex::new(std::time::Instant::now()) })
    }

    /// The live connection, reconnecting when it died.
    async fn conn(&self) -> Result<Arc<Conn>, String> {
        let mut slot = self.conn.lock().await;
        if let Some(c) = slot.as_ref().filter(|c| !c.closed()) {
            // The SSH session can outlive the SFTP channel inside it.
            let idle = c.used.lock().unwrap_or_else(|e| e.into_inner()).elapsed() >= self.probe_after;
            let alive = !idle || matches!(timeout(self.op_timeout, c.sftp.canonicalize(".")).await, Ok(Ok(_)));
            if alive {
                *c.used.lock().unwrap_or_else(|e| e.into_inner()) = std::time::Instant::now();
                return Ok(c.clone());
            }
        }
        *slot = None;
        let c = match timeout(self.connect_timeout, self.open()).await {
            Err(_) => return Err(format!("{}:{} didn't answer in time", self.target.host, self.target.port)),
            Ok(r) => Arc::new(r?),
        };
        *slot = Some(c.clone());
        Ok(c)
    }

    fn run<T>(&self, fut: impl Future<Output = Result<T, String>>) -> Result<T, String> {
        RT.block_on(fut)
    }

    /// Bounds one operation that isn't a whole transfer.
    async fn timed<T>(&self, what: &str, fut: impl Future<Output = Result<T, String>>) -> Result<T, String> {
        timeout(self.op_timeout, fut).await.unwrap_or_else(|_| Err(format!("{what}: the server didn't answer in time")))
    }

    pub fn list(&self, rel: &str) -> Result<Vec<RemoteItem>, String> {
        self.run(async {
            let c = self.conn().await?;
            self.timed("List folder", async {
                let dir = c.path_of(rel)?;
                let rd = c.sftp.read_dir(dir).await.map_err(|e| describe("List folder", &e))?;
                let mut out = vec![];
                for e in rd {
                    let name = e.file_name();
                    // Left by a crash or lost connection; a running upload has a fresh date.
                    if is_stale_part(&name, e.metadata().mtime) {
                        let _ = c.sftp.remove_file(format!("{}/{name}", c.path_of(rel)?.trim_end_matches('/'))).await;
                        continue;
                    }
                    // `.`, `..`, and anything a hostile server made up.
                    if !safe_name(&name) {
                        continue;
                    }
                    let child = if rel.is_empty() { name } else { format!("{rel}/{name}") };
                    if let Some(i) = resolve(&c, child, e.metadata()).await? {
                        out.push(i);
                    }
                }
                Ok(out)
            })
            .await
        })
    }

    pub fn stat(&self, rel: &str) -> Result<Option<RemoteItem>, String> {
        self.run(async {
            let c = self.conn().await?;
            self.timed("Read item", async {
                match c.sftp.symlink_metadata(c.path_of(rel)?).await {
                    Ok(a) => resolve(&c, rel.to_string(), a).await,
                    Err(e) if missing(&e) => Ok(None),
                    Err(e) => Err(describe("Read item", &e)),
                }
            })
            .await
        })
    }

    /// Stream `rel` into `dest`. Checks `cancelled` between chunks.
    pub fn download(&self, rel: &str, dest: &Path, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
        self.run(async {
            let c = self.conn().await?;
            let path = c.path_of(rel)?;
            let mut src = self.timed("Download", async { c.sftp.open(path).await.map_err(|e| describe("Download", &e)) }).await?;
            let expected = self.timed("Download", async { src.metadata().await.map_err(|e| describe("Download", &e)) }).await?.size;
            let mut out = tokio::fs::File::create(dest).await.map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; CHUNK];
            let mut got = 0u64;
            loop {
                if cancelled() {
                    return Err("Cancelled".to_string());
                }
                let n = match timeout(self.op_timeout, src.read(&mut buf)).await {
                    Err(_) => return Err("Download: the server stopped sending".to_string()),
                    Ok(r) => r.map_err(|e| format!("Download: {e}"))?,
                };
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
                got += n as u64;
            }
            out.sync_all().await.map_err(|e| e.to_string())?;
            let _ = timeout(self.op_timeout, src.shutdown()).await;
            // A short read must not be taken for the whole file.
            if expected.is_some_and(|e| got < e) {
                return Err("Download: the connection ended before the whole file arrived".to_string());
            }
            Ok(())
        })
    }

    /// Upload to a temporary name and rename over the target, keeping the target's permissions.
    /// Returns None: the caller stats the result for its ETag.
    pub fn upload(&self, rel: &str, src: &Path, mtime_ms: u64, tick: &dyn Fn()) -> Result<Option<String>, String> {
        self.run(async {
            let c = self.conn().await?;
            let dest = c.path_of(rel)?;
            let old = self.timed("Upload", async {
                match c.sftp.symlink_metadata(dest.clone()).await {
                    Ok(a) => Ok(Some(a)),
                    Err(e) if missing(&e) => Ok(None),
                    Err(e) => Err(describe("Upload", &e)),
                }
            })
            .await?;
            let is_link = old.as_ref().is_some_and(|a| a.is_symlink());
            // A symlink is written through, so it stays a link.
            let tmp = if is_link {
                dest.clone()
            } else {
                let dir = dest.rsplit_once('/').map_or("", |x| x.0);
                format!("{dir}/{}", super::sync::part_name())
            };
            // Set before the original is removed: from then on the temp file is the only copy.
            let only_copy = std::cell::Cell::new(false);
            let attrs = FileAttributes { permissions: old.as_ref().filter(|_| !is_link).and_then(|a| a.permissions).map(|p| p & 0o7777), ..FileAttributes::empty() };
            let res = async {
                let mut input = tokio::fs::File::open(src).await.map_err(|e| e.to_string())?;
                let flags = OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE;
                let mut f = self.timed("Upload", async { c.sftp.open_with_flags_and_attributes(tmp.clone(), flags, attrs).await.map_err(|e| describe("Upload", &e)) }).await?;
                let mut buf = vec![0u8; CHUNK];
                loop {
                    tick();
                    let n = input.read(&mut buf).await.map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    match timeout(self.op_timeout, f.write_all(&buf[..n])).await {
                        Err(_) => return Err("Upload: the server stopped accepting data".to_string()),
                        Ok(r) => r.map_err(|e| format!("Upload: {e}"))?,
                    }
                }
                match timeout(self.op_timeout, f.shutdown()).await {
                    Err(_) => return Err("Upload: the server stopped accepting data".to_string()),
                    Ok(r) => r.map_err(|e| format!("Upload: {e}"))?,
                }
                if is_link {
                    return Ok(());
                }
                self.timed("Upload", async {
                    let secs = u32::try_from(mtime_ms / 1000).unwrap_or(0);
                    replace(&c, &tmp, &dest, old.is_some(), &only_copy).await?;
                    // After the rename: a temp file with an old date would look abandoned. Best
                    // effort, the ETag is read back from the server either way.
                    let _ = c.sftp.set_metadata(dest.clone(), FileAttributes { mtime: Some(secs), atime: Some(secs), ..FileAttributes::empty() }).await;
                    Ok(())
                })
                .await
            }
            .await;
            if res.is_err() && !is_link && !only_copy.get() {
                let _ = timeout(self.op_timeout, c.sftp.remove_file(tmp.clone())).await;
            }
            res.map(|_| None)
        })
    }

    /// Create a folder; an existing folder is fine.
    pub fn mkcol(&self, rel: &str) -> Result<(), String> {
        self.run(async {
            let c = self.conn().await?;
            self.timed("Create folder", async {
                let p = c.path_of(rel)?;
                match c.sftp.create_dir(p.clone()).await {
                    Ok(()) => Ok(()),
                    Err(e) => match c.sftp.metadata(p).await {
                        Ok(a) if a.is_dir() => Ok(()),
                        _ => Err(describe("Create folder", &e)),
                    },
                }
            })
            .await
        })
    }

    /// Delete a file, a link, or an empty folder (never recursively: SFTP has no trash). Missing is fine.
    pub fn delete(&self, rel: &str) -> Result<(), String> {
        self.run(async {
            let c = self.conn().await?;
            self.timed("Delete", async {
                let p = c.path_of(rel)?;
                if rel.is_empty() {
                    return Err("Delete: refusing to remove the account root".to_string());
                }
                let a = match c.sftp.symlink_metadata(p.clone()).await {
                    Ok(a) => a,
                    Err(e) if missing(&e) => return Ok(()),
                    Err(e) => return Err(describe("Delete", &e)),
                };
                let r = if a.is_dir() { c.sftp.remove_dir(p).await } else { c.sftp.remove_file(p).await };
                r.map_err(|e| describe("Delete", &e))
            })
            .await
        })
    }
}

fn is_stale_part(name: &str, mtime: Option<u32>) -> bool {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    name.starts_with(".morfa-") && name.ends_with(super::sync::PART) && mtime.is_some_and(|m| now.saturating_sub(u64::from(m)) > super::sync::STALE_PART.as_secs())
}

/// Puts the finished temp file in place. Atomic with posix-rename where offered. Plain SFTP rename
/// won't overwrite, so remove-then-rename follows, but only after the server clearly answered
/// "failure": never after a timeout or lost connection, when the rename may yet have happened.
async fn replace(c: &Conn, tmp: &str, dest: &str, exists: bool, only_copy: &std::cell::Cell<bool>) -> Result<(), String> {
    if let Some(raw) = &c.raw {
        let mut data = vec![];
        for s in [tmp, dest] {
            data.extend_from_slice(&(s.len() as u32).to_be_bytes());
            data.extend_from_slice(s.as_bytes());
        }
        match raw.extended(POSIX_RENAME, data).await {
            Ok(Packet::Status(s)) if s.status_code == StatusCode::Ok => return Ok(()),
            Ok(Packet::Status(s)) => return Err(format!("Upload: {}", s.error_message)),
            Ok(_) => return Err("Upload: unexpected answer from the server".to_string()),
            Err(SftpError::Status(s)) if s.status_code == StatusCode::OpUnsupported => {}
            Err(e) => return Err(describe("Upload", &e)),
        }
    }
    match c.sftp.rename(tmp.to_string(), dest.to_string()).await {
        Ok(()) => return Ok(()),
        Err(SftpError::Status(s)) if s.status_code == StatusCode::Failure && exists => {}
        Err(e) => return Err(describe("Upload", &e)),
    }
    only_copy.set(true);
    if let Err(e) = c.sftp.remove_file(dest.to_string()).await {
        // Whether it was removed is unknown after a lost answer; a clear refusal means it wasn't.
        if matches!(e, SftpError::Status(_)) {
            only_copy.set(false);
        }
        return Err(describe("Upload", &e));
    }
    c.sftp.rename(tmp.to_string(), dest.to_string()).await.map_err(|e| describe(&format!("Upload: the new version is kept on the server as {tmp}"), &e))
}

/// What a listed entry stands for: a link takes its target's type. A dangling link or a device,
/// socket or FIFO isn't offered (reading one can hang). A target that can't be checked right now
/// is an error, so the item doesn't read as deleted.
async fn resolve(c: &Conn, rel: String, a: FileAttributes) -> Result<Option<RemoteItem>, String> {
    if a.is_symlink() {
        let target = match c.sftp.metadata(c.path_of(&rel)?).await {
            Ok(t) => t,
            Err(e) if missing(&e) => return Ok(None),
            Err(e) => return Err(describe(&rel, &e)),
        };
        return Ok((target.is_dir() || target.is_regular()).then(|| item(rel, &target, true)));
    }
    Ok((a.is_dir() || a.is_regular()).then(|| item(rel, &a, false)))
}

async fn load_key(path: PathBuf, passphrase: Option<String>) -> Result<russh::keys::PrivateKey, String> {
    let shown = path.display().to_string();
    let read = tokio::task::spawn_blocking(move || {
        let m = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if !m.is_file() || m.len() > MAX_KEY_FILE {
            return Err("not a key file".to_string());
        }
        russh::keys::load_secret_key(&path, passphrase.as_deref()).map_err(|e| match e {
            russh::keys::Error::KeyIsEncrypted => "it needs a passphrase, or the passphrase is wrong".to_string(),
            e => e.to_string(),
        })
    });
    read.await.map_err(|e| e.to_string())?.map_err(|e| format!("Can't use the private key {shown}: {e}"))
}

impl super::sync::Remote for Sftp {
    fn list(&self, rel: &str) -> Result<Vec<RemoteItem>, String> { self.list(rel) }
    fn stat(&self, rel: &str) -> Result<Option<RemoteItem>, String> { self.stat(rel) }
    fn download(&self, rel: &str, dest: &Path, c: &dyn Fn() -> bool) -> Result<(), String> { self.download(rel, dest, c) }
    fn upload(&self, rel: &str, src: &Path, m: u64, t: &dyn Fn()) -> Result<Option<String>, String> { self.upload(rel, src, m, t) }
    fn mkcol(&self, rel: &str) -> Result<(), String> { self.mkcol(rel) }
    fn delete(&self, rel: &str) -> Result<(), String> { self.delete(rel) }
}

#[cfg(test)]
mod tests;
