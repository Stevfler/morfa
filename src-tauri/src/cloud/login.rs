//! Nextcloud Login Flow v2: the user signs in in their browser, we poll for an app password.
//! https://docs.nextcloud.com/server/latest/developer_manual/client_apis/LoginFlow/
use super::dav::agent;
use serde::Deserialize;
use std::time::{Duration, Instant};

#[derive(Deserialize)]
struct Start {
    poll: Poll,
    login: String,
}
#[derive(Deserialize)]
struct Poll {
    token: String,
    endpoint: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    pub server: String,
    pub login_name: String,
    pub app_password: String,
}

/// "cloud.corp.ru/" → "https://cloud.corp.ru". Plain http is kept only when typed explicitly.
pub fn normalize_server(s: &str) -> Result<String, String> {
    let s = s.trim().trim_end_matches('/');
    let s = s.strip_suffix("/index.php").unwrap_or(s);
    if s.is_empty() || s.contains(char::is_whitespace) {
        return Err("Enter the server address, e.g. https://cloud.example.com".into());
    }
    Ok(if s.starts_with("https://") || s.starts_with("http://") { s.to_string() } else { format!("https://{s}") })
}

/// Waits for the browser sign-in; stops early when the argument returns true.
pub trait Poller: FnOnce(&dyn Fn() -> bool) -> Result<Credentials, String> {}
impl<F: FnOnce(&dyn Fn() -> bool) -> Result<Credentials, String>> Poller for F {}

/// Start the flow; returns the URL to open in the browser and a poller.
pub fn start(server: &str) -> Result<(String, impl Poller), String> {
    let a = agent();
    let mut resp = a.post(format!("{server}/index.php/login/v2")).send_empty().map_err(|e| format!("Can't reach the server: {e}"))?;
    if resp.status().as_u16() != 200 {
        return Err(format!("This doesn't look like a Nextcloud server (HTTP {})", resp.status().as_u16()));
    }
    let s: Start = resp.body_mut().read_to_string().ok().and_then(|b| serde_json::from_str(&b).ok()).ok_or_else(|| "This doesn't look like a Nextcloud server".to_string())?;
    let poll = move |cancelled: &dyn Fn() -> bool| {
        // The token lives 20 minutes; 404 means "not signed in yet".
        let deadline = Instant::now() + Duration::from_secs(20 * 60);
        while Instant::now() < deadline {
            for _ in 0..20 {
                if cancelled() {
                    return Err("Cancelled".to_string());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let Ok(mut r) = a.post(&s.poll.endpoint).send_form([("token", s.poll.token.as_str())]) else { continue };
            if r.status().as_u16() == 200 {
                let body = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
                return serde_json::from_str::<Credentials>(&body).map_err(|e| e.to_string());
            }
        }
        Err("Sign-in timed out".to_string())
    };
    Ok((s.login, poll))
}

/// Revoke the app password (best effort, when an account is removed).
pub fn revoke(server: &str, login: &str, password: &str) {
    use base64::Engine;
    let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{login}:{password}")));
    let _ = agent().delete(format!("{server}/ocs/v2.php/core/apppassword")).header("Authorization", &auth).header("OCS-APIREQUEST", "true").call();
}

#[cfg(test)]
mod tests {
    #[test]
    fn normalizes() {
        assert_eq!(super::normalize_server(" cloud.corp.ru/ ").unwrap(), "https://cloud.corp.ru");
        assert_eq!(super::normalize_server("http://10.0.0.5/index.php").unwrap(), "http://10.0.0.5");
        assert!(super::normalize_server("").is_err());
    }
}
