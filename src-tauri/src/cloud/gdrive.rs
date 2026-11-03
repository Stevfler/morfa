//! Google Drive (Drive API v3) behind the same path-based `Remote` interface as WebDAV.
//!
//! Drive addresses items by ID and allows several items with the same name in one folder,
//! so every listing fills a path → ID cache; duplicate names get a " (2)" suffix in paths.
//! Folder ETags stay empty: Drive doesn't change a folder when its content changes, so the
//! sync engine must always look inside. Google Docs/Sheets/… have no file content: they are
//! listed with a web link and never synced.
use super::dav::{agent, days_from_civil, RemoteItem};
use base64::Engine;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const API: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3";
const SCOPE: &str = "https://www.googleapis.com/auth/drive";
const FOLDER: &str = "application/vnd.google-apps.folder";
const FIELDS: &str = "id,name,mimeType,size,modifiedTime,md5Checksum,version,webViewLink";

/// OAuth client of this build: `MORFA_GDRIVE_CLIENT_ID` / `MORFA_GDRIVE_CLIENT_SECRET` at build
/// time, or the same variables at run time (handy for development). For a Desktop-type client
/// Google treats the secret as public, as every installed app ships it.
fn client() -> Result<(String, String), String> {
    let get = |name: &str, built: Option<&str>| std::env::var(name).ok().or(built.map(String::from)).filter(|v| !v.is_empty());
    match (get("MORFA_GDRIVE_CLIENT_ID", option_env!("MORFA_GDRIVE_CLIENT_ID")), get("MORFA_GDRIVE_CLIENT_SECRET", option_env!("MORFA_GDRIVE_CLIENT_SECRET"))) {
        (Some(id), Some(secret)) => Ok((id, secret)),
        _ => Err("Google Drive isn't set up in this build (no OAuth client ID)".into()),
    }
}

pub fn available() -> bool {
    client().is_ok()
}

fn enc(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

fn b64url(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

#[derive(Deserialize)]
struct Token {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

fn token_request(form: &[(&str, &str)]) -> Result<Token, String> {
    let mut r = agent().post(TOKEN_URL).send_form(form.iter().copied()).map_err(|e| e.to_string())?;
    let code = r.status().as_u16();
    let body = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    if code != 200 {
        if body.contains("invalid_grant") {
            return Err("Google sign-in expired or was revoked — reconnect the account".into());
        }
        return Err(format!("Google sign-in failed (HTTP {code})"));
    }
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

pub struct Login {
    pub refresh_token: String,
    pub email: String,
}

/// Browser sign-in with a loopback redirect and PKCE. `open` shows the consent page.
pub fn login(open: &dyn Fn(&str) -> Result<(), String>, cancelled: &dyn Fn() -> bool) -> Result<Login, String> {
    let (id, secret) = client()?;
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| e.to_string())?;
    let verifier = b64url(&raw);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    getrandom::fill(&mut raw).map_err(|e| e.to_string())?;
    let state = b64url(&raw[..16]);

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr().map_err(|e| e.to_string())?.port());
    let url = format!(
        "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={challenge}&code_challenge_method=S256&access_type=offline&prompt=consent&state={state}",
        enc(&id), enc(&redirect), enc(SCOPE)
    );
    open(&url)?;

    let deadline = Instant::now() + Duration::from_secs(10 * 60);
    let code = loop {
        if cancelled() {
            return Err("Cancelled".into());
        }
        if Instant::now() > deadline {
            return Err("Sign-in timed out".into());
        }
        let (mut stream, _) = match listener.accept() {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        let _ = BufReader::new(&stream).read_line(&mut line);
        // "GET /?state=…&code=…&scope=… HTTP/1.1"
        let query = line.split_whitespace().nth(1).and_then(|p| p.split_once('?')).map(|x| x.1).unwrap_or("");
        let params: HashMap<String, String> = query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.to_string(), percent_encoding::percent_decode_str(&v.replace('+', " ")).decode_utf8_lossy().into_owned()))
            .collect();
        if params.get("state") != Some(&state) {
            // Favicon requests and strays: answer and keep waiting.
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            continue;
        }
        let ok = params.contains_key("code");
        let page = if ok {
            "<h2>Morfa is connected to Google Drive.</h2><p>You can close this tab.</p>"
        } else {
            "<h2>Sign-in was cancelled.</h2><p>You can close this tab.</p>"
        };
        let html = format!("<!doctype html><meta charset=utf-8><title>Morfa</title><body style=\"font:16px system-ui;margin:3em\">{page}</body>");
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len());
        match params.get("code") {
            Some(c) => break c.clone(),
            None => return Err(params.get("error").map(|e| if e == "access_denied" { "Cancelled".to_string() } else { e.clone() }).unwrap_or_else(|| "Cancelled".into())),
        }
    };

    let t = token_request(&[
        ("client_id", &id),
        ("client_secret", &secret),
        ("code", &code),
        ("code_verifier", &verifier),
        ("redirect_uri", &redirect),
        ("grant_type", "authorization_code"),
    ])?;
    let refresh_token = t.refresh_token.ok_or("Google didn't return a refresh token")?;
    let drive = Drive::with_token("login", &refresh_token, Some((t.access_token, t.expires_in)));
    let about = drive.get_json(&format!("{API}/about?fields=user(emailAddress)"))?;
    let email = about["user"]["emailAddress"].as_str().unwrap_or("Google Drive").to_string();
    Ok(Login { refresh_token, email })
}

/// Revoke the refresh token (best effort, when an account is removed).
pub fn revoke(refresh_token: &str) {
    let _ = agent().post("https://oauth2.googleapis.com/revoke").send_form([("token", refresh_token)]);
}

/// Access tokens per account, shared across `Drive` instances (they live an hour).
fn tokens() -> &'static Mutex<HashMap<String, (String, Instant)>> {
    static T: OnceLock<Mutex<HashMap<String, (String, Instant)>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveFile {
    id: String,
    name: String,
    mime_type: String,
    size: Option<String>,
    modified_time: Option<String>,
    md5_checksum: Option<String>,
    version: Option<String>,
    web_view_link: Option<String>,
}

pub struct Drive {
    account: String,
    refresh: String,
    agent: ureq::Agent,
    /// rel path → file ID ("" = "root").
    ids: Mutex<HashMap<String, String>>,
}

/// "2026-11-03T10:00:00.123Z" → Unix ms.
pub fn parse_rfc3339(s: &str) -> Option<u64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time = time.trim_end_matches('Z');
    let (hms, frac) = time.split_once('.').unwrap_or((time, "0"));
    let mut t = hms.split(':').map(|x| x.parse::<i64>().ok());
    let (h, mi, se) = (t.next()??, t.next()??, t.next()??);
    let ms: i64 = format!("{:0<3}", &frac[..frac.len().min(3)]).parse().ok()?;
    let secs = days_from_civil(y, m, day) * 86400 + h * 3600 + mi * 60 + se;
    u64::try_from(secs * 1000 + ms).ok()
}

fn rfc3339(ms: u64) -> String {
    let secs = ms / 1000;
    let stamp = super::stamp(secs); // "YYYY-MM-DD HHMMSS"
    let (d, t) = stamp.split_once(' ').unwrap_or((&stamp, "000000"));
    format!("{d}T{}:{}:{}.{:03}Z", &t[0..2], &t[2..4], &t[4..6], ms % 1000)
}

/// Unique display names inside one folder: the second "a.txt" becomes "a (2).txt".
fn dedupe(names: &[String]) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    names
        .iter()
        .map(|n| {
            let mut name = n.clone();
            let mut i = 2;
            while !seen.insert(name.to_lowercase()) {
                name = match n.rfind('.') {
                    Some(dot) if dot > 0 => format!("{} ({i}){}", &n[..dot], &n[dot..]),
                    _ => format!("{n} ({i})"),
                };
                i += 1;
            }
            name
        })
        .collect()
}

fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() { name.to_string() } else { format!("{parent}/{name}") }
}

impl Drive {
    pub fn new(account: &str, refresh: &str) -> Drive {
        Drive::with_token(account, refresh, None)
    }

    fn with_token(account: &str, refresh: &str, token: Option<(String, u64)>) -> Drive {
        if let Some((t, exp)) = token {
            lock(tokens()).insert(account.into(), (t, Instant::now() + Duration::from_secs(exp.saturating_sub(60))));
        }
        Drive { account: account.into(), refresh: refresh.into(), agent: agent(), ids: Mutex::new(HashMap::from([(String::new(), "root".into())])) }
    }

    fn token(&self) -> Result<String, String> {
        if let Some((t, exp)) = lock(tokens()).get(&self.account) {
            if Instant::now() < *exp {
                return Ok(t.clone());
            }
        }
        let (id, secret) = client()?;
        let t = token_request(&[("client_id", &id), ("client_secret", &secret), ("refresh_token", &self.refresh), ("grant_type", "refresh_token")])?;
        lock(tokens()).insert(self.account.clone(), (t.access_token.clone(), Instant::now() + Duration::from_secs(t.expires_in.saturating_sub(60))));
        Ok(t.access_token)
    }

    /// Send with auth; retries rate limits and server errors with backoff.
    fn send(&self, build: &dyn Fn(&str) -> Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<ureq::http::Response<ureq::Body>, String> {
        let mut wait = Duration::from_millis(500);
        for attempt in 0..5 {
            let auth = format!("Bearer {}", self.token()?);
            match build(&auth) {
                Ok(r) => {
                    let code = r.status().as_u16();
                    // 403 is how Drive reports rate limits too; a real "forbidden" just costs a few retries.
                    let retry = code == 429 || code >= 500 || code == 403;
                    if code == 401 {
                        lock(tokens()).remove(&self.account);
                        if attempt == 0 {
                            continue;
                        }
                    }
                    if !retry || attempt == 4 {
                        return Ok(r);
                    }
                }
                Err(e) if attempt == 4 => return Err(e.to_string()),
                Err(_) => {}
            }
            std::thread::sleep(wait);
            wait *= 2;
        }
        Err("Google Drive is not responding".into())
    }

    fn get_json(&self, url: &str) -> Result<serde_json::Value, String> {
        let mut r = self.send(&|auth| self.agent.get(url).header("Authorization", auth).call())?;
        json(&mut r, "Google Drive")
    }

    fn file_id(&self, rel: &str) -> Result<Option<String>, String> {
        if let Some(id) = lock(&self.ids).get(rel) {
            return Ok(Some(id.clone()));
        }
        // Not seen yet: list the parent (recursively resolving it), which fills the cache.
        let parent = rel.rsplit_once('/').map(|x| x.0).unwrap_or("");
        if self.file_id(parent)?.is_none() {
            return Ok(None);
        }
        self.list_children(parent)?;
        Ok(lock(&self.ids).get(rel).cloned())
    }

    fn list_children(&self, rel: &str) -> Result<Vec<RemoteItem>, String> {
        let Some(id) = (if rel.is_empty() { Some("root".to_string()) } else { lock(&self.ids).get(rel).cloned() }) else {
            return Err(format!("{rel}: not found"));
        };
        let q = enc(&format!("'{id}' in parents and trashed = false"));
        let mut files: Vec<DriveFile> = vec![];
        let mut page = String::new();
        loop {
            let url = format!("{API}/files?q={q}&fields=nextPageToken,files({FIELDS})&pageSize=1000&orderBy=name,createdTime&spaces=drive&supportsAllDrives=true{page}");
            let v = self.get_json(&url)?;
            files.extend(serde_json::from_value::<Vec<DriveFile>>(v["files"].clone()).map_err(|e| e.to_string())?);
            match v["nextPageToken"].as_str() {
                Some(t) => page = format!("&pageToken={}", enc(t)),
                None => break,
            }
        }
        let names = dedupe(&files.iter().map(|f| f.name.replace('/', "∕")).collect::<Vec<_>>());
        let mut ids = lock(&self.ids);
        // Forget stale children of this folder before re-adding the current ones.
        let prefix = if rel.is_empty() { String::new() } else { format!("{rel}/") };
        ids.retain(|k, _| !(k.starts_with(&prefix) && !k.is_empty() && !k[prefix.len()..].contains('/')));
        Ok(files
            .into_iter()
            .zip(names)
            .map(|(f, name)| {
                let path = join(rel, &name);
                ids.insert(path.clone(), f.id.clone());
                item(path, f)
            })
            .collect())
    }

    fn create(&self, rel: &str, folder: bool) -> Result<serde_json::Value, String> {
        let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        let pid = self.file_id(parent)?.ok_or_else(|| format!("{parent}: not found"))?;
        let mut meta = serde_json::json!({ "name": name, "parents": [pid] });
        if folder {
            meta["mimeType"] = FOLDER.into();
        }
        let body = meta.to_string();
        let url = format!("{API}/files?fields={FIELDS}&supportsAllDrives=true");
        let mut r = self.send(&|auth| self.agent.post(&url).header("Authorization", auth).header("Content-Type", "application/json").send(&body))?;
        let v = json(&mut r, "Create")?;
        if let Some(id) = v["id"].as_str() {
            lock(&self.ids).insert(rel.into(), id.into());
        }
        Ok(v)
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn json(r: &mut ureq::http::Response<ureq::Body>, what: &str) -> Result<serde_json::Value, String> {
    let code = r.status().as_u16();
    let body = r.body_mut().with_config().limit(64 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())?;
    if !(200..300).contains(&code) {
        let msg = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v["error"]["message"].as_str().map(String::from));
        return Err(match code {
            401 => "Google sign-in expired — reconnect the account".into(),
            404 => format!("{what}: not found"),
            _ => format!("{what}: {}", msg.unwrap_or_else(|| format!("HTTP {code}"))),
        });
    }
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

fn item(path: String, f: DriveFile) -> RemoteItem {
    let is_dir = f.mime_type == FOLDER;
    let online = !is_dir && f.mime_type.starts_with("application/vnd.google-apps.");
    RemoteItem {
        path,
        is_dir,
        size: f.size.and_then(|s| s.parse().ok()).unwrap_or(0),
        modified: f.modified_time.as_deref().and_then(parse_rfc3339).unwrap_or(0),
        etag: if is_dir { String::new() } else { f.md5_checksum.or(f.version).unwrap_or_default() },
        link: if online { f.web_view_link.unwrap_or_else(|| format!("https://drive.google.com/open?id={}", f.id)) } else { String::new() },
    }
}

impl super::sync::Remote for Drive {
    fn list(&self, rel: &str) -> Result<Vec<RemoteItem>, String> {
        if self.file_id(rel)?.is_none() {
            return Err(format!("{rel}: not found"));
        }
        self.list_children(rel)
    }

    fn stat(&self, rel: &str) -> Result<Option<RemoteItem>, String> {
        if rel.is_empty() {
            return Ok(Some(RemoteItem { is_dir: true, ..Default::default() }));
        }
        let parent = rel.rsplit_once('/').map(|x| x.0).unwrap_or("");
        if self.file_id(parent)?.is_none() {
            return Ok(None);
        }
        Ok(self.list_children(parent)?.into_iter().find(|i| i.path == rel))
    }

    fn download(&self, rel: &str, dest: &Path, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
        let id = self.file_id(rel)?.ok_or_else(|| format!("{rel}: not found"))?;
        let url = format!("{API}/files/{id}?alt=media&supportsAllDrives=true");
        let mut r = self.send(&|auth| self.agent.get(&url).header("Authorization", auth).call())?;
        if r.status().as_u16() != 200 {
            return json(&mut r, "Download").map(|_| ());
        }
        let mut file = File::create(dest).map_err(|e| e.to_string())?;
        let mut body = r.body_mut().as_reader();
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            if cancelled() {
                return Err("Cancelled".into());
            }
            let n = body.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        }
        file.sync_all().map_err(|e| e.to_string())
    }

    /// Resumable upload (new file or new content for an existing one), keeping the mtime.
    fn upload(&self, rel: &str, src: &Path, mtime_ms: u64) -> Result<Option<String>, String> {
        let existing = self.file_id(rel)?;
        let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        let len = std::fs::metadata(src).map_err(|e| e.to_string())?.len();
        let mut meta = serde_json::json!({ "modifiedTime": rfc3339(mtime_ms) });
        let (method, url) = match &existing {
            Some(id) => ("PATCH", format!("{UPLOAD}/files/{id}?uploadType=resumable&supportsAllDrives=true&fields={FIELDS}")),
            None => {
                let pid = self.file_id(parent)?.ok_or_else(|| format!("{parent}: not found"))?;
                meta["name"] = name.into();
                meta["parents"] = serde_json::json!([pid]);
                ("POST", format!("{UPLOAD}/files?uploadType=resumable&supportsAllDrives=true&fields={FIELDS}"))
            }
        };
        let body = meta.to_string();
        let r = self.send(&|auth| {
            let req = ureq::http::Request::builder()
                .method(method)
                .uri(&url)
                .header("Authorization", auth)
                .header("Content-Type", "application/json; charset=UTF-8")
                .header("X-Upload-Content-Length", len.to_string())
                .body(body.clone())
                .expect("valid request");
            self.agent.run(req)
        })?;
        let mut r = r;
        if r.status().as_u16() != 200 {
            return json(&mut r, "Upload").map(|_| None);
        }
        let session = r.headers().get("location").and_then(|v| v.to_str().ok()).ok_or("Upload: no session URL")?.to_string();
        let file = File::open(src).map_err(|e| e.to_string())?;
        let req = ureq::http::Request::builder()
            .method("PUT")
            .uri(&session)
            .header("Content-Length", len.to_string())
            .body(file)
            .map_err(|e| e.to_string())?;
        let mut r = self.agent.run(req).map_err(|e| e.to_string())?;
        let v = json(&mut r, "Upload")?;
        let f: DriveFile = serde_json::from_value(v).map_err(|e| e.to_string())?;
        lock(&self.ids).insert(rel.into(), f.id.clone());
        Ok(Some(f.md5_checksum.or(f.version).unwrap_or_default()))
    }

    fn mkcol(&self, rel: &str) -> Result<(), String> {
        if self.file_id(rel)?.is_some() {
            return Ok(());
        }
        self.create(rel, true).map(|_| ())
    }

    /// Move to the Drive trash.
    fn delete(&self, rel: &str) -> Result<(), String> {
        let Some(id) = self.file_id(rel)? else { return Ok(()) };
        let url = format!("{API}/files/{id}?supportsAllDrives=true");
        let body = r#"{"trashed":true}"#;
        let mut r = self.send(&|auth| {
            let req = ureq::http::Request::builder()
                .method("PATCH")
                .uri(&url)
                .header("Authorization", auth)
                .header("Content-Type", "application/json")
                .body(body)
                .expect("valid request");
            self.agent.run(req)
        })?;
        match r.status().as_u16() {
            404 => {}
            _ => {
                json(&mut r, "Delete")?;
            }
        }
        lock(&self.ids).retain(|k, _| !super::store::under(k, rel));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_rfc3339("2026-11-03T10:00:00.123Z"), Some(1_793_700_000_123));
        assert_eq!(parse_rfc3339("1970-01-01T00:00:01Z"), Some(1000));
        assert_eq!(rfc3339(1_793_700_000_123), "2026-11-03T10:00:00.123Z");
    }

    #[test]
    fn dedupes_names() {
        let v = dedupe(&["a.txt".into(), "A.txt".into(), "a.txt".into(), "b".into(), "b".into()]);
        assert_eq!(v, ["a.txt", "A (2).txt", "a (3).txt", "b", "b (2)"]);
    }

    #[test]
    fn google_docs_are_online_only() {
        let f = |mime: &str| DriveFile { id: "x".into(), name: "n".into(), mime_type: mime.into(), size: None, modified_time: None, md5_checksum: None, version: Some("7".into()), web_view_link: None };
        let doc = item("Report".into(), f("application/vnd.google-apps.document"));
        assert!(!doc.link.is_empty() && !doc.is_dir);
        let dir = item("D".into(), f(FOLDER));
        assert!(dir.is_dir && dir.link.is_empty() && dir.etag.is_empty());
        assert_eq!(item("a.bin".into(), f("application/octet-stream")).etag, "7");
    }
}
