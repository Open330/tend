//! What the Home window may ask the dashboard for.
//!
//! The `WebView` loads Home from the app's own origin and every request it
//! makes comes here; the app forwards it over a proved connection and adds
//! the token, so the page never holds it. Only what Home uses is forwarded:
//! its pages and assets, the health, board, Fleet and decision reads, and
//! the audited decision writes (answer or decide an item, mark a finished
//! unit looked at). Pane prompts, Fleet commands, terminal input, work
//! control and operator administration are refused even though the token
//! would allow them. `/api/access` is refused too: without it the page
//! hides Explore, Settings, pane links and Start Work, which need those.

use crate::client::{Client, ClientError, Response};

/// The scheme wry serves on Windows as `http://tend.localhost/`.
pub const SCHEME: &str = "tend";
/// The origin the page sees.
pub const ORIGIN: &str = "http://tend.localhost";

/// Whether the Home window may send this request.
#[must_use]
pub fn allowed(method: &str, path: &str) -> bool {
    let path = path.split(['?', '#']).next().unwrap_or("");
    if path.contains("..") || path.contains('\\') || path.contains("//") {
        return false;
    }
    match method {
        "GET" | "HEAD" => {
            matches!(
                path,
                "/home"
                    | "/needs"
                    | "/api/health"
                    | "/api/home"
                    | "/api/fleet"
                    | "/api/decisions"
                    | "/api/decisions/history"
            ) || path.strip_prefix("/static/").is_some_and(asset_path)
        }
        "POST" => {
            path == "/api/home/reviewed"
                || ["/api/decisions/requests/", "/api/decisions/reviews/"]
                    .iter()
                    .any(|prefix| path.strip_prefix(prefix).is_some_and(single_segment))
        }
        _ => false,
    }
}

fn single_segment(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// An asset name: letters, digits and `_ - . /`, no `%` (so nothing is
/// decoded into a separator or a dot segment later).
fn asset_path(rest: &str) -> bool {
    !rest.is_empty()
        && rest.len() <= 200
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/'))
}

/// The largest request body the window may send (a decision is a few
/// hundred bytes; an answer is capped at 16 KiB by the page).
pub const BODY_LIMIT: usize = 64 * 1024;

/// Whether a navigation inside the window stays in the window: only the
/// app origin's Home pages. Parsed, so `http://tend.localhost.evil/` and
/// `http://user@tend.localhost/` do not pass.
#[must_use]
pub fn stays_in_window(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "http"
        && parsed.host_str() == Some("tend.localhost")
        && parsed.port().is_none()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && matches!(parsed.path(), "/home" | "/needs")
}

/// Where a link that leaves the window goes instead, if anywhere: a web
/// link opens in the default browser; a link to another dashboard page
/// (the pane drawer) opens that page on the real dashboard there, where
/// the browser's own sign-in applies. Anything else (`file:`, custom
/// schemes) goes nowhere.
#[must_use]
pub fn outside_target(url: &str, dashboard: std::net::SocketAddr) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    match parsed.scheme() {
        "http" if parsed.host_str() == Some("tend.localhost") => {
            let mut real = url::Url::parse(&format!("http://{dashboard}/")).ok()?;
            real.set_path(parsed.path());
            real.set_query(parsed.query());
            real.set_fragment(parsed.fragment());
            Some(real.to_string())
        }
        "http" | "https" if parsed.username().is_empty() && parsed.password().is_none() => {
            Some(parsed.to_string())
        }
        _ => None,
    }
}

/// The answer the window gets for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub content_type: Option<String>,
    pub cache_control: Option<String>,
    pub body: Vec<u8>,
}

impl Reply {
    fn text(status: u16, text: &str) -> Self {
        Self {
            status,
            content_type: Some("text/plain; charset=utf-8".into()),
            cache_control: Some("no-store".into()),
            body: text.as_bytes().to_vec(),
        }
    }
}

/// Forward one request from the window, or refuse it. `content_type` is the
/// only header passed on; cookies, `Origin` and any `Authorization` the page
/// tried to send are dropped (the client adds the token itself).
pub fn handle(
    client: &Client,
    method: &str,
    path_and_query: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> Reply {
    if !allowed(method, path_and_query) {
        return Reply::text(403, "not available in the Tend app");
    }
    if body.len() > BODY_LIMIT {
        return Reply::text(413, "request too large");
    }
    let headers: Vec<(&str, &str)> = content_type
        .filter(|_| method == "POST")
        .map(|value| vec![("Content-Type", value)])
        .unwrap_or_default();
    match client.request(method, path_and_query, &headers, body) {
        Ok(Response {
            status,
            headers,
            body,
        }) => {
            let find = |name: &str| {
                headers
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.clone())
            };
            Reply {
                status,
                content_type: find("content-type"),
                cache_control: find("cache-control"),
                body,
            }
        }
        Err(error @ ClientError::NotTendd(_)) => Reply::text(502, &error.to_string()),
        Err(error) => Reply::text(503, &error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{dashboard, fake_server};

    #[test]
    fn only_homes_reads_and_the_decision_writes_pass() {
        for (method, path) in [
            ("GET", "/home"),
            ("GET", "/needs"),
            ("GET", "/static/needs.mjs"),
            ("GET", "/static/v/0.10.18/needs.css"),
            ("GET", "/api/fleet"),
            ("GET", "/api/decisions?limit=50"),
            ("GET", "/api/decisions/history"),
            ("GET", "/api/health"),
            ("GET", "/api/home"),
            ("GET", "/api/home?limit=200"),
            ("POST", "/api/home/reviewed"),
            ("POST", "/api/decisions/requests/req_18e8_1"),
            ("POST", "/api/decisions/reviews/rv_b884376fd0844f2e"),
        ] {
            assert!(allowed(method, path), "{method} {path}");
        }
        for (method, path) in [
            ("GET", "/"),
            // The retired workbench's address serves the full shell; the
            // window is only Home.
            ("GET", "/workbench"),
            ("GET", "/api/agents"),
            ("GET", "/api/panes/%251/output"),
            ("GET", "/api/operators"),
            ("GET", "/api/access"),
            ("GET", "/api/home/"),
            ("GET", "/api/home/reviewed"),
            ("GET", "/api/device/home"),
            ("POST", "/api/home"),
            ("POST", "/api/home/reviewed/x"),
            ("PUT", "/api/home/reviewed"),
            ("GET", "/static/"),
            ("GET", "/static/../api/agents"),
            ("POST", "/api/panes/%251/prompt"),
            ("POST", "/api/fleet/mini/command"),
            ("POST", "/api/work-control/up"),
            ("POST", "/api/terminal-sessions/x/input"),
            ("POST", "/api/decisions/requests/"),
            ("POST", "/api/decisions/requests/a/b"),
            ("PUT", "/api/work-metadata"),
            ("DELETE", "/api/decisions/requests/x"),
            ("GET", "/home//x"),
            ("POST", "/api/decisions/requests/a%2F..%2F..%2Fpanes"),
            ("POST", "/api/decisions/requests/%2e%2e"),
            ("GET", "/static/%2e%2e/api/agents"),
            ("GET", "/static/a%5cb"),
        ] {
            assert!(!allowed(method, path), "{method} {path}");
        }
    }

    #[test]
    fn navigation_stays_on_the_app_origins_home_pages() {
        assert!(stays_in_window("http://tend.localhost/home"));
        assert!(stays_in_window("http://tend.localhost/needs#item=rq_1"));
        for url in [
            "http://tend.localhost/",
            "http://tend.localhost/workbench#pane=%251",
            "http://tend.localhost.evil/home",
            "http://user@tend.localhost/home",
            "http://tend.localhost:8080/home",
            "https://tend.localhost/home",
            "file:///C:/Windows/win.ini",
            "javascript:alert(1)",
        ] {
            assert!(!stays_in_window(url), "{url}");
        }
    }

    #[test]
    fn links_out_go_to_the_browser_or_nowhere() {
        let addr = "127.0.0.1:7878".parse().unwrap();
        assert_eq!(
            outside_target("https://github.com/Open330/tend/pull/1", addr).as_deref(),
            Some("https://github.com/Open330/tend/pull/1")
        );
        assert_eq!(
            outside_target("http://tend.localhost/?x=1#pane=%251", addr).as_deref(),
            Some("http://127.0.0.1:7878/?x=1#pane=%251")
        );
        for url in [
            "file:///C:/x",
            "ms-settings:",
            "https://u:p@example.com/",
            "not a url",
        ] {
            assert_eq!(outside_target(url, addr), None, "{url}");
        }
    }

    #[test]
    fn a_refused_request_never_reaches_the_dashboard() {
        let server = fake_server(Some("k"), None);
        let client = Client::new(dashboard(server.addr, "k"));
        let reply = handle(
            &client,
            "POST",
            "/api/panes/%251/prompt",
            Some("application/json"),
            b"{}",
        );
        assert_eq!(reply.status, 403);
        assert!(server.seen.lock().unwrap().is_empty());
        let reply = handle(
            &client,
            "POST",
            "/api/decisions/requests/r1",
            Some("application/json"),
            b"{}",
        );
        assert_eq!(reply.status, 200);
        let seen = server.seen.lock().unwrap().join("");
        assert!(seen.contains("Content-Type: application/json"));
    }

    #[test]
    fn an_impostor_gets_a_clear_refusal() {
        let server = fake_server(None, None);
        let client = Client::new(dashboard(server.addr, "k"));
        let reply = handle(&client, "GET", "/home", None, &[]);
        assert_eq!(reply.status, 502);
        assert!(String::from_utf8_lossy(&reply.body).contains("not tendd"));
    }
}
