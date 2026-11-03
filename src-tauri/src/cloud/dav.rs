//! Minimal WebDAV client for Nextcloud (`/remote.php/dav/files/<login>`): list, stat,
//! download, upload, mkcol, delete. Paths are "rel" paths: decoded, `/`-separated,
//! no leading slash, `""` = the account root.
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
    /// Web link for items that exist only online (Google Docs/Sheets/…): shown, never synced.
    pub link: String,
}

pub struct Dav {
    agent: ureq::Agent,
    /// `https://host/remote.php/dav/files/<login>` without a trailing slash.
    base: String,
    /// Decoded URL path of `base`, used to turn hrefs back into rel paths.
    base_path: String,
    auth: String,
}

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_recv_body(None)
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
        Dav { agent: agent(), base, base_path, auth }
    }

    fn url(&self, rel: &str) -> String {
        let enc = encode_rel(rel);
        if enc.is_empty() { format!("{}/", self.base) } else { format!("{}/{}", self.base, enc) }
    }

    fn request(&self, method: &str, rel: &str) -> ureq::http::request::Builder {
        Request::builder()
            .method(Method::from_bytes(method.as_bytes()).expect("valid method"))
            .uri(self.url(rel))
            .header("Authorization", &self.auth)
    }

    fn propfind(&self, rel: &str, depth: &str) -> Result<Option<Vec<RemoteItem>>, String> {
        let req = self.request("PROPFIND", rel)
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY)
            .map_err(|e| e.to_string())?;
        let mut resp = self.agent.run(req).map_err(|e| e.to_string())?;
        match resp.status().as_u16() {
            207 => {}
            404 => return Ok(None),
            c => return Err(status_err("List folder", c)),
        }
        let body = resp.body_mut().with_config().limit(256 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())?;
        parse_multistatus(&body, &self.base_path).map(Some)
    }

    /// Children of the folder `rel` (not the folder itself).
    pub fn list(&self, rel: &str) -> Result<Vec<RemoteItem>, String> {
        let items = self.propfind(rel, "1")?.ok_or_else(|| status_err("List folder", 404))?;
        let me = rel.trim_matches('/');
        Ok(items.into_iter().filter(|i| i.path != me).collect())
    }

    /// The item at `rel`, or None if it does not exist.
    pub fn stat(&self, rel: &str) -> Result<Option<RemoteItem>, String> {
        Ok(self.propfind(rel, "0")?.and_then(|v| v.into_iter().next()))
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
    pub fn upload(&self, rel: &str, src: &Path, mtime_ms: u64) -> Result<Option<String>, String> {
        let file = File::open(src).map_err(|e| e.to_string())?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        let req = self.request("PUT", rel)
            .header("X-OC-Mtime", (mtime_ms / 1000).to_string())
            .header("Content-Length", len.to_string())
            .header("Content-Type", "application/octet-stream")
            .body(file)
            .map_err(|e| e.to_string())?;
        let resp = self.agent.run(req).map_err(|e| e.to_string())?;
        let code = resp.status().as_u16();
        if !(200..300).contains(&code) {
            return Err(status_err("Upload", code));
        }
        let h = resp.headers();
        Ok(h.get("oc-etag").or_else(|| h.get("etag")).and_then(|v| v.to_str().ok()).map(clean_etag))
    }

    /// Create a folder; an existing folder is fine.
    pub fn mkcol(&self, rel: &str) -> Result<(), String> {
        let req = self.request("MKCOL", rel).body(()).map_err(|e| e.to_string())?;
        let code = self.agent.run(req).map_err(|e| e.to_string())?.status().as_u16();
        match code {
            200..=299 | 405 => Ok(()),
            c => Err(status_err("Create folder", c)),
        }
    }

    /// Delete a file or folder (Nextcloud moves it to its trash bin). Missing is fine.
    pub fn delete(&self, rel: &str) -> Result<(), String> {
        let req = self.request("DELETE", rel).body(()).map_err(|e| e.to_string())?;
        let code = self.agent.run(req).map_err(|e| e.to_string())?.status().as_u16();
        match code {
            200..=299 | 404 => Ok(()),
            c => Err(status_err("Delete", c)),
        }
    }
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
                    "response" => {
                        if let Some(c) = cur.take().filter(|c| c.ok) {
                            if let Some(path) = href_to_rel(&c.href, &base) {
                                out.push(RemoteItem {
                                    path,
                                    is_dir: c.dir,
                                    size: if c.dir { c.oc_size.unwrap_or(0) } else { c.len.or(c.oc_size).unwrap_or(0) },
                                    modified: c.modified,
                                    etag: c.etag,
                                    link: String::new(),
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
    (rest.is_empty() || rest.starts_with('/')).then(|| rest.trim_matches('/').to_string())
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
        assert_eq!(v[0], RemoteItem { path: "Docs".into(), is_dir: true, size: 42, modified: 1_793_700_000_000, etag: "abc".into(), link: String::new() });
        assert_eq!(v[1].path, "Docs/a b & c.txt");
        assert_eq!((v[1].size, v[1].modified, v[1].is_dir), (12, 1000, false));
        assert_eq!(v[2].path, "Docs/Привет");
        assert!(v[2].is_dir);
    }

    #[test]
    fn encodes_paths() {
        let dav = Dav::new("https://h", "u", "p");
        assert_eq!(dav.url(""), "https://h/remote.php/dav/files/u/");
        assert_eq!(dav.url("a b/c#d?.txt"), "https://h/remote.php/dav/files/u/a%20b/c%23d%3F.txt");
    }
}
