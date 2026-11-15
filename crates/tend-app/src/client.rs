//! An HTTP/1.1 client for the local dashboard that proves the server is
//! tendd before it sends the token.
//!
//! A loopback port authenticates nothing: while tendd is down, another
//! local user can bind it. So each TCP connection starts with
//! `GET /api/proof?nonce=N` (no credential); the server must answer
//! `HMAC-SHA256(token, label || N)`, which only a process that knows the
//! token can compute. Only then does a request with the token go out, and
//! only on that same connection, which no other process can take over.
//! A new connection (after the server closed one) is proved again.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Mutex;
use std::time::Duration;
use subtle::ConstantTimeEq;

use crate::settings::Dashboard;

/// Must match `tend::dashboard::auth::SERVER_PROOF_LABEL`.
pub const SERVER_PROOF_LABEL: &str = "tend-dashboard-proof/v1\n";

const HEAD_LIMIT: usize = 64 * 1024;
const BODY_LIMIT: usize = 16 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub enum ClientError {
    /// Nothing answered on the address.
    Unreachable(io::Error),
    /// Something answered but could not prove it knows the token: another
    /// process holds the port, or the token changed.
    NotTendd(String),
    /// The server's answer was not HTTP we understand.
    Protocol(String),
    Io(io::Error),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(error) => write!(f, "the dashboard does not answer ({error})"),
            Self::NotTendd(why) => {
                write!(f, "the server on the dashboard address is not tendd: {why}")
            }
            Self::Protocol(why) => write!(f, "unexpected answer from the dashboard: {why}"),
            Self::Io(error) => write!(f, "dashboard connection failed: {error}"),
        }
    }
}

/// One HTTP response, body read in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// The expected proof for `nonce` under `token`, lowercase hex.
#[must_use]
pub fn expected_proof(token: &str, nonce: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("any key length");
    mac.update(SERVER_PROOF_LABEL.as_bytes());
    mac.update(nonce.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

fn fresh_nonce() -> String {
    // Two v4 UUIDs: 244 random bits from the OS generator, as 64 hex digits.
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

struct Connection {
    reader: BufReader<TcpStream>,
}

impl Connection {
    /// Connect and prove the server before anything else is sent.
    fn open(dashboard: &Dashboard) -> Result<Self, ClientError> {
        let stream = TcpStream::connect_timeout(&dashboard.addr, Duration::from_secs(3))
            .map_err(ClientError::Unreachable)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(ClientError::Io)?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(ClientError::Io)?;
        let _ = stream.set_nodelay(true);
        let mut connection = Self {
            reader: BufReader::new(stream),
        };
        connection.prove(dashboard)?;
        Ok(connection)
    }

    fn prove(&mut self, dashboard: &Dashboard) -> Result<(), ClientError> {
        let nonce = fresh_nonce();
        let response = self
            .exchange(
                dashboard.addr,
                "GET",
                &format!("/api/proof?nonce={nonce}"),
                &[],
                &[],
                None,
            )
            .map_err(|error| match error {
                ClientError::Io(error) => ClientError::NotTendd(format!("no proof ({error})")),
                other => other,
            })?;
        if response.status != 200 {
            return Err(ClientError::NotTendd(format!(
                "the proof route answered {} (is tendd older than this app?)",
                response.status
            )));
        }
        let body: serde_json::Value = serde_json::from_slice(&response.body)
            .map_err(|_| ClientError::NotTendd("the proof is not JSON".into()))?;
        let got = body
            .get("proof")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let want = expected_proof(&dashboard.token, &nonce);
        if got.len() == want.len() && bool::from(got.as_bytes().ct_eq(want.as_bytes())) {
            Ok(())
        } else {
            Err(ClientError::NotTendd(
                "its proof does not match the token (another process on the port, or a changed token)".into(),
            ))
        }
    }

    /// Write one request and read its response on this connection.
    fn exchange(
        &mut self,
        addr: SocketAddr,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
        token: Option<&str>,
    ) -> Result<Response, ClientError> {
        use std::fmt::Write as _;
        let mut head =
            format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: keep-alive\r\n");
        if let Some(token) = token {
            let _ = write!(head, "Authorization: Bearer {token}\r\n");
        }
        for (name, value) in headers {
            if name.contains(['\r', '\n', ':']) || value.contains(['\r', '\n']) {
                return Err(ClientError::Protocol("header with a line break".into()));
            }
            let _ = write!(head, "{name}: {value}\r\n");
        }
        if !body.is_empty() || method != "GET" {
            let _ = write!(head, "Content-Length: {}\r\n", body.len());
        }
        head.push_str("\r\n");
        let stream = self.reader.get_mut();
        stream.write_all(head.as_bytes()).map_err(ClientError::Io)?;
        stream.write_all(body).map_err(ClientError::Io)?;
        stream.flush().map_err(ClientError::Io)?;
        read_response(&mut self.reader, method == "HEAD")
    }
}

fn read_response(reader: &mut impl BufRead, head_only: bool) -> Result<Response, ClientError> {
    let mut raw = Vec::new();
    loop {
        let read = reader
            .read_until(b'\n', &mut raw)
            .map_err(ClientError::Io)?;
        if read == 0 {
            return Err(ClientError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before a response",
            )));
        }
        if raw.ends_with(b"\r\n\r\n") {
            break;
        }
        if raw.len() > HEAD_LIMIT {
            return Err(ClientError::Protocol("response head too large".into()));
        }
    }
    let mut parsed_headers = [httparse::EMPTY_HEADER; 64];
    let mut parsed = httparse::Response::new(&mut parsed_headers);
    match parsed.parse(&raw) {
        Ok(httparse::Status::Complete(_)) => {}
        _ => return Err(ClientError::Protocol("malformed response head".into())),
    }
    let status = parsed.code.unwrap_or(0);
    let headers: Vec<(String, String)> = parsed
        .headers
        .iter()
        .map(|h| {
            (
                h.name.to_owned(),
                String::from_utf8_lossy(h.value).into_owned(),
            )
        })
        .collect();
    let response = Response {
        status,
        headers,
        body: Vec::new(),
    };
    let no_body = head_only || status == 204 || status == 304 || (100..200).contains(&status);
    if no_body {
        return Ok(response);
    }
    let body = if response
        .header("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
    {
        read_chunked(reader)?
    } else if let Some(length) = response.header("content-length") {
        let length: usize = length
            .trim()
            .parse()
            .map_err(|_| ClientError::Protocol("bad content-length".into()))?;
        if length > BODY_LIMIT {
            return Err(ClientError::Protocol("response body too large".into()));
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).map_err(ClientError::Io)?;
        body
    } else {
        // Neither: the body runs to the end of the connection.
        let mut body = Vec::new();
        reader
            .take(BODY_LIMIT as u64 + 1)
            .read_to_end(&mut body)
            .map_err(ClientError::Io)?;
        if body.len() > BODY_LIMIT {
            return Err(ClientError::Protocol("response body too large".into()));
        }
        body
    };
    Ok(Response { body, ..response })
}

fn read_chunked(reader: &mut impl BufRead) -> Result<Vec<u8>, ClientError> {
    let mut body = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).map_err(ClientError::Io)?;
        let size = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size, 16)
            .map_err(|_| ClientError::Protocol("bad chunk size".into()))?;
        if size == 0 {
            // Trailers, then the blank line.
            loop {
                let mut trailer = String::new();
                let read = reader.read_line(&mut trailer).map_err(ClientError::Io)?;
                if read == 0 || trailer == "\r\n" || trailer == "\n" {
                    return Ok(body);
                }
            }
        }
        if body.len() + size > BODY_LIMIT {
            return Err(ClientError::Protocol("response body too large".into()));
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader
            .read_exact(&mut body[start..])
            .map_err(ClientError::Io)?;
        let mut crlf = [0; 2];
        reader.read_exact(&mut crlf).map_err(ClientError::Io)?;
    }
}

/// The dashboard client: one proved, kept-alive connection at a time.
pub struct Client {
    dashboard: Dashboard,
    connection: Mutex<Option<Connection>>,
}

impl Client {
    #[must_use]
    pub fn new(dashboard: Dashboard) -> Self {
        Self {
            dashboard,
            connection: Mutex::new(None),
        }
    }

    /// Send one request with the token, on a proved connection. A kept
    /// connection the server has since closed is replaced (and proved)
    /// once; a fresh connection that fails is an error.
    pub fn request(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<Response, ClientError> {
        let mut slot = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let reused = slot.is_some();
        for attempt in 0..2 {
            if slot.is_none() {
                *slot = Some(Connection::open(&self.dashboard)?);
            }
            let connection = slot.as_mut().expect("opened above");
            match connection.exchange(
                self.dashboard.addr,
                method,
                path,
                headers,
                body,
                Some(&self.dashboard.token),
            ) {
                Ok(response) => {
                    if response
                        .header("connection")
                        .is_some_and(|v| v.eq_ignore_ascii_case("close"))
                    {
                        *slot = None;
                    }
                    return Ok(response);
                }
                Err(ClientError::Io(error)) if reused && attempt == 0 => {
                    // The kept connection went stale (the server closed it
                    // while idle). Prove a new one and send again; the
                    // decision routes are idempotent for the same body.
                    let _ = error;
                    *slot = None;
                }
                Err(error) => {
                    *slot = None;
                    return Err(error);
                }
            }
        }
        unreachable!("the loop returns on its second attempt")
    }

    /// A GET whose 2xx body is JSON.
    pub fn get_json(&self, path: &str) -> Result<serde_json::Value, ClientError> {
        let response = self.request("GET", path, &[], &[])?;
        if !(200..300).contains(&response.status) {
            return Err(ClientError::Protocol(format!(
                "{path} answered {}",
                response.status
            )));
        }
        serde_json::from_slice(&response.body)
            .map_err(|error| ClientError::Protocol(format!("{path}: {error}")))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex as StdMutex};
    use std::thread;

    /// A tiny HTTP server: `/api/proof` answers with `proof_token`'s HMAC
    /// (or garbage when `None`), everything else echoes what it got. Keeps
    /// each connection alive and records every request head.
    pub(crate) struct FakeServer {
        pub addr: SocketAddr,
        pub seen: Arc<StdMutex<Vec<String>>>,
    }

    pub(crate) fn fake_server(
        proof_token: Option<&'static str>,
        close_after: Option<usize>,
    ) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let log = seen.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let log = log.clone();
                thread::spawn(move || serve(stream, proof_token, close_after, &log));
            }
        });
        FakeServer { addr, seen }
    }

    fn serve(
        stream: TcpStream,
        proof_token: Option<&str>,
        close_after: Option<usize>,
        log: &StdMutex<Vec<String>>,
    ) {
        let mut reader = BufReader::new(stream);
        let mut served = 0;
        loop {
            let mut head = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let length = head
                .lines()
                .find_map(|l| l.strip_prefix("Content-Length: "))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            log.lock().unwrap().push(head.clone());
            let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
            let (status, payload, chunked) =
                if let Some(nonce) = path.strip_prefix("/api/proof?nonce=") {
                    let proof =
                        proof_token.map_or_else(|| "0".repeat(64), |t| expected_proof(t, nonce));
                    (
                        200,
                        format!("{{\"version\":1,\"proof\":\"{proof}\"}}"),
                        false,
                    )
                } else if path == "/chunked" {
                    (200, "hello chunked".into(), true)
                } else {
                    (
                        200,
                        format!("{{\"path\":\"{path}\",\"body\":{}}}", body.len()),
                        false,
                    )
                };
            served += 1;
            let close = close_after.is_some_and(|n| served >= n);
            let stream = reader.get_mut();
            let connection = if close { "close" } else { "keep-alive" };
            if chunked {
                let (a, b) = payload.split_at(5);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} OK\r\nTransfer-Encoding: chunked\r\nConnection: {connection}\r\n\r\n{:x}\r\n{a}\r\n{:x};ext=1\r\n{b}\r\n0\r\n\r\n",
                    a.len(),
                    b.len()
                );
            } else {
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: {connection}\r\n\r\n{payload}",
                    payload.len()
                );
            }
            if close {
                return;
            }
        }
    }

    pub(crate) fn dashboard(addr: SocketAddr, token: &str) -> Dashboard {
        Dashboard {
            addr,
            token: token.into(),
        }
    }

    #[test]
    fn the_token_goes_out_only_after_the_server_proves_it_knows_it() {
        let server = fake_server(Some("right"), None);
        let client = Client::new(dashboard(server.addr, "right"));
        let response = client.request("GET", "/api/decisions", &[], &[]).unwrap();
        assert_eq!(response.status, 200);
        let seen = server.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        assert!(seen[0].starts_with("GET /api/proof?nonce="));
        assert!(
            !seen[0].contains("Authorization"),
            "the proof request carries no token"
        );
        assert!(seen[1].contains("Authorization: Bearer right"));
    }

    #[test]
    fn an_impostor_never_sees_the_token() {
        for server in [
            fake_server(None, None),
            fake_server(Some("other-token"), None),
        ] {
            let client = Client::new(dashboard(server.addr, "secret-token"));
            let error = client
                .request("GET", "/api/decisions", &[], &[])
                .unwrap_err();
            assert!(matches!(error, ClientError::NotTendd(_)), "{error}");
            let seen = server.seen.lock().unwrap().join("");
            assert!(!seen.contains("secret-token"), "{seen}");
            assert!(!seen.contains("Authorization"), "{seen}");
        }
    }

    #[test]
    fn a_kept_connection_is_reused_and_a_new_one_is_proved_again() {
        // The server closes after two requests (the proof and one call).
        let server = fake_server(Some("k"), Some(2));
        let client = Client::new(dashboard(server.addr, "k"));
        client.request("GET", "/a", &[], &[]).unwrap();
        client.request("GET", "/b", &[], &[]).unwrap();
        let seen = server.seen.lock().unwrap().clone();
        let paths: Vec<&str> = seen
            .iter()
            .map(|h| h.split_whitespace().nth(1).unwrap())
            .collect();
        assert_eq!(paths.len(), 4, "{paths:?}");
        assert!(paths[0].starts_with("/api/proof"));
        assert_eq!(paths[1], "/a");
        assert!(
            paths[2].starts_with("/api/proof"),
            "a new connection is proved: {paths:?}"
        );
        assert_eq!(paths[3], "/b");

        let kept = fake_server(Some("k"), None);
        let client = Client::new(dashboard(kept.addr, "k"));
        for path in ["/1", "/2", "/3"] {
            client.request("GET", path, &[], &[]).unwrap();
        }
        let proofs = kept
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|h| h.contains("/api/proof"))
            .count();
        assert_eq!(proofs, 1, "one proof for a kept connection");
    }

    #[test]
    fn bodies_by_length_and_chunked_and_posts() {
        let server = fake_server(Some("k"), None);
        let client = Client::new(dashboard(server.addr, "k"));
        let chunked = client.request("GET", "/chunked", &[], &[]).unwrap();
        assert_eq!(chunked.body, b"hello chunked");
        let posted = client
            .request(
                "POST",
                "/api/decisions/requests/r1",
                &[("Content-Type", "application/json")],
                b"{\"answer\":\"yes\"}",
            )
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&posted.body).unwrap();
        assert_eq!(json["body"], 16);
        assert!(client
            .request("GET", "/x", &[("Bad\r\nHeader", "v")], &[])
            .is_err());
    }

    #[test]
    fn nothing_listening_is_unreachable() {
        let addr = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let client = Client::new(dashboard(addr, "k"));
        assert!(matches!(
            client.request("GET", "/", &[], &[]),
            Err(ClientError::Unreachable(_))
        ));
    }

    #[test]
    fn the_proof_label_matches_the_daemon() {
        // tend::dashboard::auth::SERVER_PROOF_LABEL; this crate does not
        // depend on tend, so the literal is pinned here.
        assert_eq!(SERVER_PROOF_LABEL, "tend-dashboard-proof/v1\n");
        assert_eq!(fresh_nonce().len(), 64);
    }
}
