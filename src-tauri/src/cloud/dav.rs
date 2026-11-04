//! Minimal WebDAV client: Nextcloud (`/remote.php/dav/files/<login>`) or any server given by its
//! full folder URL. List, stat, download, upload, mkcol, delete. Paths are "rel" paths: decoded, `/`-separated,
//! no leading slash, `""` = the account root.
use super::names::safe_rel;
use base64::Engine;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use quick_xml::{events::Event, Reader};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    time::Duration,
};
use ureq::http::{Method, Request};

/// Everything except unreserved characters gets encoded inside a path segment.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ').add(b'"').add(b'#').add(b'%').add(b'<').add(b'>').add(b'?').add(b'`').add(b'{').add(b'}')
    .add(b'/').add(b'\\').add(b'[').add(b']').add(b'^').add(b'|').add(b'&').add(b'+').add(b';').add(b'=')
    .add(b'@').add(b':').add(b'$').add(b',').add(b'\'');

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RemoteItem {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Unix milliseconds.
    pub modified: u64,
    pub etag: String,
    /// Web link for items that exist only online (e.g. office documents that live only in the web app): shown, never synced.
    pub link: String,
    /// A symlink on the server. One to a folder is browsable but a sync walk doesn't descend it.
    pub symlink: bool,
}

pub struct Dav {
    agent: ureq::Agent,
    /// `https://host/remote.php/dav/files/<login>` without a trailing slash.
    base: String,
    /// Decoded URL path of `base`, used to turn hrefs back into rel paths.
    base_path: String,
    auth: String,
    /// Not Nextcloud: folder ETags and the PUT response's ETag can't be relied on, and a listing
    /// must contain the requested folder itself.
    generic: bool,
}

/// A body transfer stalling is caught by the caller's `guard` heartbeat; this cap only makes sure
/// an abandoned thread eventually returns.
const BODY_CAP: Duration = Duration::from_secs(6 * 3600);

pub fn agent() -> ureq::Agent {
    agent_with(10)
}

fn agent_with(max_redirects: u32) -> ureq::Agent {
    ureq::Agent::config_builder()
        .max_redirects(max_redirects)
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_recv_body(Some(BODY_CAP))
        .timeout_send_body(Some(BODY_CAP))
        .http_status_as_error(false)
        .allow_non_standard_methods(true)
        .user_agent(concat!("Morfa/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

pub fn encode_rel(rel: &str) -> String {
    rel.split('/').filter(|s| !s.is_empty()).map(|s| utf8_percent_encode(s, SEGMENT).to_string()).collect::<Vec<_>>().join("/")
}

fn status_err(what: &str, code: u16) -> String {
    match code {
        301 | 302 | 303 | 307 | 308 => format!("{what}: the server redirected the request. Enter the final address"),
        401 => "The server rejected the credentials — reconnect the account".into(),
        403 => format!("{what}: access denied"),
        404 => format!("{what}: not found"),
        507 => format!("{what}: no space left on the server"),
        c => format!("{what}: HTTP {c}"),
    }
}

impl Dav {
    pub fn new(server: &str, login: &str, password: &str) -> Dav {
        let base = format!("{}/remote.php/dav/files/{}", server.trim_end_matches('/'), utf8_percent_encode(login, SEGMENT));
        let path_start = base.find("://").map(|i| i + 3).and_then(|i| base[i..].find('/').map(|j| i + j)).unwrap_or(base.len());
        let base_path = percent_decode_str(&base[path_start..]).decode_utf8_lossy().into_owned();
        let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{login}:{password}")));
        Dav { agent: agent(), base, base_path, auth, generic: false }
    }

    /// Any WebDAV server; `url` comes from `normalize_url`. Redirects aren't followed, so they
    /// can't take the request to plain http or another host.
    pub fn generic(url: &str, login: &str, password: &str) -> Dav {
        let mut d = Dav::new(url, login, password);
        d.base = url.trim_end_matches('/').to_string();
        let path_start = d.base.find("://").map(|i| i + 3).and_then(|i| d.base[i..].find('/').map(|j| i + j)).unwrap_or(d.base.len());
        d.base_path = percent_decode_str(&d.base[path_start..]).decode_utf8_lossy().into_owned();
        d.agent = agent_with(0);
        d.generic = true;
        d
    }

    pub fn recursive_etags(&self) -> bool {
        !self.generic
    }

    fn url(&self, rel: &str) -> String {
        let enc = encode_rel(rel);
        if enc.is_empty() { format!("{}/", self.base) } else { format!("{}/{}", self.base, enc) }
    }

    fn request(&self, method: &str, rel: &str) -> ureq::http::request::Builder {
        self.request_at(method, &self.url(rel))
    }

    fn request_at(&self, method: &str, url: &str) -> ureq::http::request::Builder {
        Request::builder()
            .method(Method::from_bytes(method.as_bytes()).expect("valid method"))
            .uri(url)
            .header("Authorization", &self.auth)
    }

    /// Sends a request about something that may be a folder. Servers like Apache redirect a folder
    /// without its trailing slash; redirects aren't followed, so it's retried here with the slash,
    /// on our own URL, wherever the redirect pointed.
    fn run_on_item(&self, rel: &str, send: impl Fn(&str) -> Result<ureq::http::Response<ureq::Body>, String>) -> Result<ureq::http::Response<ureq::Body>, String> {
        let url = self.url(rel);
        let resp = send(&url)?;
        if self.generic && matches!(resp.status().as_u16(), 301 | 302 | 307 | 308) && !url.ends_with('/') {
            return send(&format!("{url}/"));
        }
        Ok(resp)
    }

    fn propfind(&self, rel: &str, depth: &str) -> Result<Option<Vec<RemoteItem>>, String> {
        let mut resp = self.run_on_item(rel, |url| {
            let req = self.request_at("PROPFIND", url)
                .header("Depth", depth)
                .header("Content-Type", "application/xml; charset=utf-8")
                .body(PROPFIND_BODY)
                .map_err(|e| e.to_string())?;
            self.agent.run(req).map_err(|e| e.to_string())
        })?;
        match resp.status().as_u16() {
            207 => {}
            404 => return Ok(None),
            c => return Err(status_err("List folder", c)),
        }
        let body = resp.body_mut().with_config().limit(256 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())?;
        let mut items = parse_multistatus(&body, &self.base_path)?;
        if self.generic {
            // Servers without ETags: size and date stand in, so changes are still seen.
            for i in items.iter_mut().filter(|i| !i.is_dir && i.etag.is_empty()) {
                i.etag = format!("m{}-{}", i.modified, i.size);
            }
            // A listing that doesn't contain the folder itself can't be trusted: its hrefs may not
            // map onto this URL, and an empty result must not read as "everything was deleted".
            let me = rel.trim_matches('/');
            if !items.iter().any(|i| i.path == me) {
                return Err("The server's answer doesn't match this address. Check the folder URL".into());
            }
        }
        Ok(Some(items))
    }

    /// Children of the folder `rel` (not the folder itself).
    pub fn list(&self, rel: &str) -> Result<Vec<RemoteItem>, String> {
        let items = self.propfind(rel, "1")?.ok_or_else(|| status_err("List folder", 404))?;
        let me = rel.trim_matches('/');
        Ok(items.into_iter().filter(|i| i.path != me).collect())
    }

    /// The item at `rel`, or None if it does not exist.
    pub fn stat(&self, rel: &str) -> Result<Option<RemoteItem>, String> {
        let me = rel.trim_matches('/');
        Ok(self.propfind(rel, "0")?.and_then(|v| {
            if self.generic { v.into_iter().find(|i| i.path == me) } else { v.into_iter().next() }
        }))
    }

    /// Stream `rel` into `dest` (overwritten). Checks `cancelled` between chunks.
    pub fn download(&self, rel: &str, dest: &Path, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
        let req = self.request("GET", rel).header("Accept-Encoding", "identity").body(()).map_err(|e| e.to_string())?;
        let mut resp = self.agent.run(req).map_err(|e| e.to_string())?;
        let code = resp.status().as_u16();
        if code != 200 {
            return Err(status_err("Download", code));
        }
        let mut file = File::create(dest).map_err(|e| e.to_string())?;
        let mut body = resp.body_mut().as_reader();
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

    /// Upload `src` to `rel`, keeping its mtime. Returns the new ETag when the server sends one.
    pub fn upload(&self, rel: &str, src: &Path, mtime_ms: u64, tick: &dyn Fn()) -> Result<Option<String>, String> {
        let mut file = File::open(src).map_err(|e| e.to_string())?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        let req = self.request("PUT", rel)
            .header("X-OC-Mtime", (mtime_ms / 1000).to_string())
            .header("Content-Length", len.to_string())
            .header("Content-Type", "application/octet-stream")
            .body(())
            .map_err(|e| e.to_string())?;
        let (parts, ()) = req.into_parts();
        let mut reader = Ticking { inner: &mut file, tick };
        let req = Request::from_parts(parts, ureq::SendBody::from_reader(&mut reader));
        let resp = self.agent.run(req).map_err(|e| e.to_string())?;
        let code = resp.status().as_u16();
        if !(200..300).contains(&code) {
            return Err(status_err("Upload", code));
        }
        if self.generic {
            // The PUT header may differ from what PROPFIND reports; the caller stats instead.
            return Ok(None);
        }
        let h = resp.headers();
        Ok(h.get("oc-etag").or_else(|| h.get("etag")).and_then(|v| v.to_str().ok()).map(clean_etag))
    }

    /// Create a folder; an existing folder is fine.
    pub fn mkcol(&self, rel: &str) -> Result<(), String> {
        let code = self.run_on_item(rel, |url| self.agent.run(self.request_at("MKCOL", url).body(()).map_err(|e| e.to_string())?).map_err(|e| e.to_string()))?.status().as_u16();
        match code {
            200..=299 | 405 => Ok(()),
            c => Err(status_err("Create folder", c)),
        }
    }

    /// Delete a file or folder (Nextcloud moves it to its trash bin). Missing is fine.
    pub fn delete(&self, rel: &str) -> Result<(), String> {
        let code = self.run_on_item(rel, |url| self.agent.run(self.request_at("DELETE", url).body(()).map_err(|e| e.to_string())?).map_err(|e| e.to_string()))?.status().as_u16();
        match code {
            200..=299 | 404 => Ok(()),
            c => Err(status_err("Delete", c)),
        }
    }
}

/// Reports each chunk read while a body is being sent.
struct Ticking<'a> {
    inner: &'a mut File,
    tick: &'a dyn Fn(),
}
impl Read for Ticking<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        (self.tick)();
        self.inner.read(buf)
    }
}

/// "dav.example.com/files/" → "https://dav.example.com/files". Credentials, queries and fragments
/// don't belong in the address; plain http is kept only when typed explicitly.
pub fn normalize_url(s: &str) -> Result<String, String> {
    let s = s.trim();
    let s = if s.ends_with("://") { s } else { s.trim_end_matches('/') };
    if s.is_empty() || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("Enter the WebDAV address, e.g. https://dav.example.com/files".into());
    }
    let url = if s.starts_with("https://") || s.starts_with("http://") { s.to_string() } else if s.contains("://") { return Err("The address must start with https://".into()) } else { format!("https://{s}") };
    let rest = url.split_once("://").map_or("", |x| x.1);
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() {
        return Err("Enter the WebDAV address, e.g. https://dav.example.com/files".into());
    }
    if host.contains('@') {
        return Err("Don't put the login or password in the address. Use the fields below".into());
    }
    if url.contains(['?', '#']) {
        return Err("The address can't contain “?” or “#”".into());
    }
    Ok(url)
}

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">
  <d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/><d:getetag/><oc:size/></d:prop>
</d:propfind>"#;

fn clean_etag(s: &str) -> String {
    s.trim().trim_start_matches("W/").trim_matches('"').to_string()
}

/// Parse a 207 Multi-Status body; hrefs are turned into rel paths under `base_path`.
pub fn parse_multistatus(xml: &str, base_path: &str) -> Result<Vec<RemoteItem>, String> {
    #[derive(Default)]
    struct Cur {
        href: String,
        dir: bool,
        len: Option<u64>,
        oc_size: Option<u64>,
        modified: u64,
        etag: String,
        ok: bool,
    }
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut out = vec![];
    let mut closed = false;
    let mut cur: Option<Cur> = None;
    let mut text = String::new();
    // Props from a non-200 propstat (e.g. 404 for oc:size on files) are ignored.
    let mut in_propstat = false;
    let mut stash: Vec<(String, String)> = vec![];
    let mut stash_dir = false;
    let base = base_path.trim_end_matches('/').to_string();
    loop {
        match r.read_event().map_err(|e| format!("Bad server response: {e}"))? {
            Event::Start(e) => {
                match e.local_name().as_ref() {
                    "response" => cur = Some(Cur::default()),
                    "propstat" => {
                        in_propstat = true;
                        stash.clear();
                        stash_dir = false;
                    }
                    "collection" => stash_dir = true,
                    _ => {}
                }
                text.clear();
            }
            Event::Empty(e) => {
                if e.local_name().as_ref() == "collection" {
                    stash_dir = true;
                }
            }
            Event::Text(t) => text.push_str(&t.into_inner()),
            Event::GeneralRef(e) => {
                match e.as_ref() {
                    "amp" => text.push('&'),
                    "lt" => text.push('<'),
                    "gt" => text.push('>'),
                    "quot" => text.push('"'),
                    "apos" => text.push('\''),
                    n => {
                        let code = n.strip_prefix("#x").map(|h| u32::from_str_radix(h, 16)).or_else(|| n.strip_prefix('#').map(|d| d.parse()));
                        if let Some(c) = code.and_then(|c| c.ok()).and_then(char::from_u32) {
                            text.push(c);
                        }
                    }
                }
            }
            Event::End(e) => {
                let name = e.local_name().as_ref().to_string();
                match name.as_str() {
                    "href" => {
                        if let Some(c) = cur.as_mut() {
                            c.href = std::mem::take(&mut text);
                        }
                    }
                    "propstat" => {
                        in_propstat = false;
                        let ok = stash.iter().any(|(k, v)| k == "status" && v.contains(" 200"));
                        if let (Some(c), true) = (cur.as_mut(), ok) {
                            c.ok = true;
                            c.dir |= stash_dir;
                            for (k, v) in &stash {
                                match k.as_str() {
                                    "getcontentlength" => c.len = v.trim().parse().ok(),
                                    "size" => c.oc_size = v.trim().parse().ok(),
                                    "getlastmodified" => c.modified = parse_http_date(v).unwrap_or(0),
                                    "getetag" => c.etag = clean_etag(v),
                                    _ => {}
                                }
                            }
                        }
                    }
                    "multistatus" => closed = true,
                    "response" => {
                        if let Some(c) = cur.take() {
                            // An item we can't read must not look like one that isn't there.
                            if !c.ok {
                                return Err("Bad server response: an item came without usable properties".into());
                            }
                            if let Some(path) = href_to_rel(&c.href, &base) {
                                out.push(RemoteItem {
                                    path,
                                    is_dir: c.dir,
                                    size: if c.dir { c.oc_size.unwrap_or(0) } else { c.len.or(c.oc_size).unwrap_or(0) },
                                    modified: c.modified,
                                    etag: c.etag,
                                    link: String::new(),
                                    symlink: false,
                                });
                            }
                        }
                    }
                    _ if in_propstat && !text.is_empty() => stash.push((name, std::mem::take(&mut text))),
                    _ => {}
                }
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    // Cut off mid-way: what arrived isn't the whole folder, and the rest isn't deleted.
    if !closed {
        return Err("Bad server response: the listing was cut off".into());
    }
    Ok(out)
}

fn href_to_rel(href: &str, base: &str) -> Option<String> {
    let href = href.trim();
    // Absolute URL → path part.
    let href = match href.find("://") {
        Some(i) => href[i + 3..].find('/').map(|j| &href[i + 3 + j..]).unwrap_or("/"),
        None => href,
    };
    let decoded = percent_decode_str(href).decode_utf8_lossy().into_owned();
    let rest = decoded.strip_prefix(base)?;
    // The server names things here: `..` and the like are dropped, never mapped to a local path.
    (rest.is_empty() || rest.starts_with('/')).then(|| rest.trim_matches('/').to_string()).filter(|r| safe_rel(r))
}

/// RFC 1123 date ("Tue, 03 Nov 2026 10:00:00 GMT") → Unix ms.
pub fn parse_http_date(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let [_, d, mon, y, time, ..] = parts.as_slice() else { return None };
    let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].iter().position(|m| m == mon)? as i64 + 1;
    let (d, y): (i64, i64) = (d.parse().ok()?, y.parse().ok()?);
    let t: Vec<i64> = time.split(':').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let [h, mi, se] = t.as_slice() else { return None };
    let secs = days_from_civil(y, mon, d) * 86400 + h * 3600 + mi * 60 + se;
    u64::try_from(secs).ok().map(|s| s * 1000)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    const XML: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">
 <d:response><d:href>/remote.php/dav/files/ivan%40corp/Docs/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype><d:getlastmodified>Tue, 03 Nov 2026 10:00:00 GMT</d:getlastmodified><d:getetag>&quot;abc&quot;</d:getetag><oc:size>42</oc:size></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  <d:propstat><d:prop><d:getcontentlength/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>
 </d:response>
 <d:response><d:href>/remote.php/dav/files/ivan%40corp/Docs/a%20b%20%26%20c.txt</d:href>
  <d:propstat><d:prop><d:resourcetype/><d:getcontentlength>12</d:getcontentlength><d:getlastmodified>Thu, 01 Jan 1970 00:00:01 GMT</d:getlastmodified><d:getetag>"e1"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
 <d:response><d:href>https://cloud.example.com/remote.php/dav/files/ivan%40corp/Docs/%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype><d:getetag>"e2"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
</d:multistatus>"#;

    #[test]
    fn parses_multistatus() {
        let dav = Dav::new("https://cloud.example.com/", "ivan@corp", "x");
        assert_eq!(dav.base_path, "/remote.php/dav/files/ivan@corp");
        let v = parse_multistatus(XML, &dav.base_path).unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], RemoteItem { path: "Docs".into(), is_dir: true, size: 42, modified: 1_793_700_000_000, etag: "abc".into(), ..Default::default() });
        assert_eq!(v[1].path, "Docs/a b & c.txt");
        assert_eq!((v[1].size, v[1].modified, v[1].is_dir), (12, 1000, false));
        assert_eq!(v[2].path, "Docs/Привет");
        assert!(v[2].is_dir);
    }

    #[test]
    fn truncated_or_unusable_listings_are_errors() {
        assert!(parse_multistatus(XML, "/remote.php/dav/files/ivan@corp").is_ok());
        let cut = XML.find("</d:response>").unwrap() + "</d:response>".len();
        assert!(parse_multistatus(&XML[..cut], "/remote.php/dav/files/ivan@corp").unwrap_err().contains("cut off"));
        assert!(parse_multistatus("", "/x").is_err());
        let bad = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/x/a</d:href><d:propstat><d:prop><d:getetag>"e"</d:getetag></d:prop><d:status>HTTP/1.1 500 Oops</d:status></d:propstat></d:response></d:multistatus>"#;
        assert!(parse_multistatus(bad, "/x").unwrap_err().contains("usable"));
    }

    #[test]
    fn drops_hostile_hrefs() {
        let row = |href: &str| format!(r#"<d:response><d:href>{href}</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>1</d:getcontentlength></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"#);
        let hostile = ["/dav/../../etc/passwd", "/dav/a/%2e%2e/%2e%2e/%2e%2e/x", "/dav/a%2F..%2F..%2Fx", "/dav/..%5C..%5Cx", "/dav/a%00b", "/other/x", "https://evil.example/dav/%2e%2e/x"];
        let body: String = hostile.iter().map(|h| row(h)).collect();
        let xml = format!(r#"<d:multistatus xmlns:d="DAV:">{body}{}</d:multistatus>"#, row("/dav/ok.txt"));
        let v = parse_multistatus(&xml, "/dav").unwrap();
        assert_eq!(v.iter().map(|i| i.path.as_str()).collect::<Vec<_>>(), ["ok.txt"]);
    }

    #[test]
    fn normalizes_urls() {
        assert_eq!(normalize_url(" dav.example.com/files/ ").unwrap(), "https://dav.example.com/files");
        assert_eq!(normalize_url("http://10.0.0.5:8080/dav").unwrap(), "http://10.0.0.5:8080/dav");
        for bad in ["", "https://u:p@host/dav", "https://u@host", "https://h/d?x=1", "https://h/d#f", "ftp://h", "https://", "a b"] {
            assert!(normalize_url(bad).is_err(), "{bad}");
        }
        let e = normalize_url("https://user:hunter2@host/dav").unwrap_err();
        assert!(!e.contains("hunter2"));
    }

    /// A scripted HTTP server on localhost; records every request.
    struct Http {
        url: String,
        seen: Arc<Mutex<Vec<Seen>>>,
    }
    #[derive(Clone, Debug)]
    struct Seen {
        line: String,
        auth: String,
        body: Vec<u8>,
    }
    type Reply = (u16, Vec<(String, String)>, Vec<u8>);
    fn http(handler: impl Fn(&str, &str) -> Reply + Send + Sync + 'static) -> Http {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let seen: Arc<Mutex<Vec<Seen>>> = Default::default();
        let (rec, handler) = (seen.clone(), Arc::new(handler));
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let (rec, handler) = (rec.clone(), handler.clone());
                std::thread::spawn(move || {
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    let mut s = s;
                    loop {
                        let mut line = String::new();
                        if r.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let (mut auth, mut len) = (String::new(), 0usize);
                        loop {
                            let mut h = String::new();
                            r.read_line(&mut h).unwrap();
                            let h = h.trim_end();
                            if h.is_empty() {
                                break;
                            }
                            let (k, v) = h.split_once(": ").unwrap_or((h, ""));
                            match k.to_ascii_lowercase().as_str() {
                                "authorization" => auth = v.to_string(),
                                "content-length" => len = v.parse().unwrap(),
                                _ => {}
                            }
                        }
                        let mut body = vec![0; len];
                        r.read_exact(&mut body).unwrap();
                        let mut parts = line.split_whitespace();
                        let (m, p) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                        rec.lock().unwrap().push(Seen { line: format!("{m} {p}"), auth, body });
                        let (code, headers, out) = handler(m, p);
                        let mut head = format!("HTTP/1.1 {code} X\r\nContent-Length: {}\r\n", out.len());
                        for (k, v) in headers {
                            head.push_str(&format!("{k}: {v}\r\n"));
                        }
                        head.push_str("\r\n");
                        if s.write_all(head.as_bytes()).and_then(|_| s.write_all(&out)).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        Http { url, seen }
    }

    fn file_row(href: &str, size: u64) -> String {
        format!(r#"<D:response><D:href>{href}</D:href><D:propstat><D:prop><D:resourcetype/><D:getcontentlength>{size}</D:getcontentlength><D:getlastmodified>Thu, 01 Jan 1970 00:00:02 GMT</D:getlastmodified></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"#)
    }
    fn dir_row(href: &str) -> String {
        format!(r#"<D:response><D:href>{href}</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype><D:getetag>"same-forever"</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"#)
    }
    fn multi(rows: &[String]) -> Reply {
        (207, vec![], format!(r#"<D:multistatus xmlns:D="DAV:">{}</D:multistatus>"#, rows.concat()).into_bytes())
    }

    #[test]
    fn generic_server_lists_with_basic_auth_and_no_credentials_in_urls() {
        let srv = http(|_, _| multi(&[dir_row("/dav/"), dir_row("/dav/Sub/"), file_row("/dav/a%20b.txt", 3)]));
        let d = Dav::generic(&format!("{}/dav/", srv.url), "u", "pw-throwaway");
        let items = d.list("").unwrap();
        assert_eq!(items.iter().map(|i| i.path.as_str()).collect::<Vec<_>>(), ["Sub", "a b.txt"]);
        // No getetag from this server: size and date stand in, so changes stay visible.
        assert_eq!(items[1].etag, "m2000-3");
        let seen = srv.seen.lock().unwrap();
        assert_eq!(seen[0].line, "PROPFIND /dav/");
        assert_eq!(seen[0].auth, format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("u:pw-throwaway")));
        assert!(!seen[0].line.contains("pw-throwaway"));
        assert!(!d.recursive_etags());
    }

    /// An answer that doesn't map onto the address must be an error: an empty result would read as
    /// "everything was deleted on the server" and send local copies to the Trash.
    #[test]
    fn generic_answer_for_another_path_is_an_error_not_an_empty_folder() {
        let srv = http(|_, _| multi(&[dir_row("/elsewhere/"), file_row("/elsewhere/a.txt", 1)]));
        let d = Dav::generic(&format!("{}/dav", srv.url), "u", "p");
        assert!(d.list("").is_err());
        assert!(d.stat("x").is_err());
    }

    #[test]
    fn generic_does_not_follow_redirects() {
        let elsewhere = http(|_, _| multi(&[]));
        let to = format!("{}/stolen", elsewhere.url);
        let srv = http(move |_, _| (301, vec![("Location".into(), to.clone())], vec![]));
        let e = Dav::generic(&format!("{}/dav", srv.url), "u", "p").list("").unwrap_err();
        assert!(e.contains("redirected"), "{e}");
        assert!(elsewhere.seen.lock().unwrap().is_empty());
    }

    /// Apache: a folder without its slash gets a redirect, which is retried on our own URL.
    #[test]
    fn generic_retries_folders_with_a_slash() {
        let srv = http(|_, p| match p {
            "/dav/Sub" => (301, vec![("Location".into(), "http://evil.example/x".into())], vec![]),
            _ => multi(&[dir_row("/dav/Sub/"), file_row("/dav/Sub/a.txt", 1)]),
        });
        let d = Dav::generic(&format!("{}/dav", srv.url), "u", "p");
        assert_eq!(d.list("Sub").unwrap()[0].path, "Sub/a.txt");
        assert_eq!(srv.seen.lock().unwrap().iter().map(|s| s.line.as_str()).collect::<Vec<_>>(), ["PROPFIND /dav/Sub", "PROPFIND /dav/Sub/"]);
    }

    #[test]
    fn generic_reports_rejected_credentials() {
        let srv = http(|_, _| (401, vec![], vec![]));
        let e = Dav::generic(&format!("{}/dav", srv.url), "u", "secret-pw").stat("").unwrap_err();
        assert!(e.contains("rejected") && !e.contains("secret-pw"), "{e}");
    }

    #[test]
    fn generic_upload_sends_the_file_and_asks_the_caller_to_stat() {
        let srv = http(|_, _| (201, vec![("ETag".into(), "\"from-put\"".into())], vec![]));
        let dir = std::env::temp_dir().join(format!("morfa-dav-up-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.bin");
        std::fs::write(&f, b"payload").unwrap();
        let ticks = std::cell::Cell::new(0);
        let d = Dav::generic(&format!("{}/dav", srv.url), "u", "p");
        assert_eq!(d.upload("a b/x.bin", &f, 1000, &|| ticks.set(ticks.get() + 1)).unwrap(), None);
        assert!(ticks.get() > 0);
        let seen = srv.seen.lock().unwrap();
        assert_eq!(seen[0].line, "PUT /dav/a%20b/x.bin");
        assert_eq!(seen[0].body, b"payload");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Selective sync over real HTTP: only the pinned folder comes down, deep changes are seen even
    /// though the folder's ETag never changes, and hostile hrefs land nowhere.
    #[test]
    fn selective_sync_over_generic_webdav() {
        use crate::cloud::store::SyncDb;
        use crate::cloud::sync;
        let size = Arc::new(Mutex::new(1u64));
        let s2 = size.clone();
        let srv = http(move |m, p| {
            let n = *s2.lock().unwrap();
            match (m, p) {
                ("PROPFIND", "/dav/Docs") | ("PROPFIND", "/dav/Docs/") => multi(&[dir_row("/dav/Docs/"), dir_row("/dav/Docs/sub/"), file_row("/dav/Docs/%2e%2e/%2e%2e/%2e%2e/escape.txt", 1)]),
                ("PROPFIND", "/dav/Docs/sub") | ("PROPFIND", "/dav/Docs/sub/") => multi(&[dir_row("/dav/Docs/sub/"), file_row("/dav/Docs/sub/a.txt", n)]),
                ("GET", "/dav/Docs/sub/a.txt") => (200, vec![], vec![b'x'; n as usize]),
                _ => (404, vec![], vec![]),
            }
        });
        let root = std::env::temp_dir().join(format!("morfa-dav-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mirror = root.join("a/b/mirror");
        std::fs::create_dir_all(&mirror).unwrap();
        let d = Dav::generic(&format!("{}/dav", srv.url), "u", "p");
        let db = std::sync::Mutex::new(SyncDb::default());
        db.lock().unwrap().pins.insert("Docs".into());
        let trash = |_: &Path| -> Result<(), String> { panic!("nothing to trash") };
        let ctx = sync::Ctx { remote: &d, root: mirror.clone(), db: &db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
        let rep = sync::sync_pin(&ctx, "Docs").unwrap();
        assert!(rep.errors.is_empty(), "{:?}", rep.errors);
        assert_eq!(std::fs::read(mirror.join("Docs/sub/a.txt")).unwrap(), b"x");
        assert!(!root.join("escape.txt").exists() && !root.join("a/escape.txt").exists() && !root.join("a/b/escape.txt").exists());
        *size.lock().unwrap() = 2;
        let rep = sync::sync_pin(&ctx, "Docs").unwrap();
        assert!(rep.errors.is_empty(), "{:?}", rep.errors);
        assert_eq!(std::fs::read(mirror.join("Docs/sub/a.txt")).unwrap(), b"xx");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Interop with a real server started by hand with throwaway credentials: MORFA_DAV_URL,
    /// MORFA_DAV_USER, MORFA_DAV_PASS, MORFA_DAV_DIR (the folder that URL serves).
    #[test]
    #[ignore = "needs a local WebDAV server"]
    fn interop_with_local_server() {
        use crate::cloud::store::SyncDb;
        use crate::cloud::sync;
        let env = |k: &str| std::env::var(k).unwrap();
        let dir = env("MORFA_DAV_DIR");
        std::fs::create_dir_all(format!("{dir}/Docs/sub")).unwrap();
        std::fs::write(format!("{dir}/Docs/sub/a b.txt"), b"hello").unwrap();
        let d = Dav::generic(&env("MORFA_DAV_URL"), &env("MORFA_DAV_USER"), &env("MORFA_DAV_PASS"));
        assert!(Dav::generic(&env("MORFA_DAV_URL"), &env("MORFA_DAV_USER"), "wrong").stat("").unwrap_err().contains("rejected"));
        assert!(d.stat("").unwrap().unwrap().is_dir);
        let t = std::env::temp_dir().join(format!("morfa-dav-interop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        let db = std::sync::Mutex::new(SyncDb::default());
        db.lock().unwrap().pins.insert("Docs".into());
        let trash = |p: &Path| std::fs::remove_file(p).map_err(|e| e.to_string());
        let ctx = sync::Ctx { remote: &d, root: t.clone(), db: &db, cancelled: &|| false, trash: &trash, progress: &|_| {} };
        let go = || {
            let r = sync::sync_pin(&ctx, "Docs").unwrap();
            assert!(r.errors.is_empty(), "{:?}", r.errors);
            r
        };
        go();
        assert_eq!(std::fs::read(t.join("Docs/sub/a b.txt")).unwrap(), b"hello");
        assert!(!go().changed);
        std::fs::create_dir_all(t.join("Docs/new")).unwrap();
        std::fs::write(t.join("Docs/new/b.txt"), vec![7u8; 3_000_000]).unwrap();
        go();
        assert_eq!(std::fs::metadata(format!("{dir}/Docs/new/b.txt")).unwrap().len(), 3_000_000);
        assert!(!go().changed);
        // Deep server-side change; Apache's folder ETags don't reflect it.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(format!("{dir}/Docs/sub/a b.txt"), b"changed").unwrap();
        go();
        assert_eq!(std::fs::read(t.join("Docs/sub/a b.txt")).unwrap(), b"changed");
        std::fs::remove_file(t.join("Docs/new/b.txt")).unwrap();
        go();
        assert!(!Path::new(&format!("{dir}/Docs/new/b.txt")).exists());
        std::fs::remove_dir_all(&t).unwrap();
    }

    #[test]
    fn encodes_paths() {
        let dav = Dav::new("https://h", "u", "p");
        assert_eq!(dav.url(""), "https://h/remote.php/dav/files/u/");
        assert_eq!(dav.url("a b/c#d?.txt"), "https://h/remote.php/dav/files/u/a%20b/c%23d%3F.txt");
    }
}
