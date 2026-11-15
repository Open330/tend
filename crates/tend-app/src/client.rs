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
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;

use crate::settings::Dashboard;

/// Must match `tend::dashboard::auth::SERVER_PROOF_LABEL`.
pub const SERVER_PROOF_LABEL: &str = "tend-dashboard-proof/v1\n";

const HEAD_LIMIT: usize = 64 * 1024;
const BODY_LIMIT: usize = 16 * 1024 * 1024;
const REQUEST_BODY_LIMIT: usize = 64 * 1024;
/// The whole proof exchange, and any other whole exchange.
const PROOF_DEADLINE: Duration = Duration::from_secs(5);
const EXCHANGE_DEADLINE: Duration = Duration::from_secs(30);
/// Requests in flight at once; one slot is kept for decisions.
const MAX_ACTIVE: usize = 6;
const SLOT_WAIT: Duration = Duration::from_secs(30);

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

/// The proof tendd gives for `nonce` under `token`, bound to this host's
/// node id and the dashboard's address (lowercase hex), as
/// `tend::dashboard::auth::server_proof`. Binding to node and address is
/// what defeats a relay to another tend that shares the token.
#[must_use]
pub fn expected_proof(token: &str, node: &str, bind: &str, nonce: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("any key length");
    mac.update(SERVER_PROOF_LABEL.as_bytes());
    for part in [node, "\n", bind, "\n", nonce] {
        mac.update(part.as_bytes());
    }
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

/// A TCP stream whose reads and writes share one absolute deadline, so a
/// peer that trickles bytes cannot hold an exchange open past it.
struct Timed {
    stream: TcpStream,
    deadline: Instant,
}

impl Timed {
    fn left(&self) -> io::Result<Duration> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the dashboard took too long",
            ))
        } else {
            Ok(left)
        }
    }
}

impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.left()?;
        self.stream.set_read_timeout(Some(left))?;
        self.stream.read(buf)
    }
}

impl Write for Timed {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let left = self.left()?;
        self.stream.set_write_timeout(Some(left))?;
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

struct Connection {
    reader: BufReader<Timed>,
}

impl Connection {
    /// Connect and prove the server before anything else is sent.
    fn open(dashboard: &Dashboard) -> Result<Self, ClientError> {
        let stream = TcpStream::connect_timeout(&dashboard.addr, Duration::from_secs(3))
            .map_err(ClientError::Unreachable)?;
        let _ = stream.set_nodelay(true);
        let mut connection = Self {
            reader: BufReader::new(Timed {
                stream,
                deadline: Instant::now(),
            }),
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
                PROOF_DEADLINE,
            )
            .map_err(|error| match error {
                ClientError::Io(error) => ClientError::NotTendd(format!("no proof ({error})")),
                ClientError::Protocol(why) => ClientError::NotTendd(format!("no proof ({why})")),
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
        let want = expected_proof(
            &dashboard.token,
            &dashboard.node,
            &dashboard.addr.to_string(),
            &nonce,
        );
        if got.len() == want.len() && bool::from(got.as_bytes().ct_eq(want.as_bytes())) {
            Ok(())
        } else {
            Err(ClientError::NotTendd(
                "its proof does not match the token (another process on the port, or a changed token)".into(),
            ))
        }
    }

    /// Write one request and read its response on this connection, all
    /// within `deadline`.
    #[allow(clippy::too_many_arguments)]
    fn exchange(
        &mut self,
        addr: SocketAddr,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
        token: Option<&str>,
        deadline: Duration,
    ) -> Result<Response, ClientError> {
        use std::fmt::Write as _;
        if !path.starts_with('/') || path.bytes().any(|b| b.is_ascii_control() || b == b' ') {
            return Err(ClientError::Protocol("bad request target".into()));
        }
        if body.len() > REQUEST_BODY_LIMIT {
            return Err(ClientError::Protocol("request body too large".into()));
        }
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
        self.reader.get_mut().deadline = Instant::now() + deadline;
        let stream = self.reader.get_mut();
        stream.write_all(head.as_bytes()).map_err(ClientError::Io)?;
        stream.write_all(body).map_err(ClientError::Io)?;
        stream.flush().map_err(ClientError::Io)?;
        read_response(&mut self.reader, method == "HEAD")
    }
}

/// Append one line (up to and including `\n`) to `out`, refusing to grow
/// it past `max` bytes: nothing an unproved peer sends is buffered beyond
/// the limit. Returns how many bytes were read; 0 at end of stream.
fn read_line_bounded(
    reader: &mut impl BufRead,
    out: &mut Vec<u8>,
    max: usize,
) -> Result<usize, ClientError> {
    let start = out.len();
    loop {
        let available = reader.fill_buf().map_err(ClientError::Io)?;
        if available.is_empty() {
            return Ok(out.len() - start);
        }
        let newline = available.iter().position(|b| *b == b'\n').map(|i| i + 1);
        let take = newline.unwrap_or(available.len());
        if out.len() - start + take > max {
            return Err(ClientError::Protocol("response line too long".into()));
        }
        out.extend_from_slice(&available[..take]);
        reader.consume(take);
        if newline.is_some() {
            return Ok(out.len() - start);
        }
    }
}

fn unexpected_eof(what: &str) -> ClientError {
    ClientError::Io(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        what.to_owned(),
    ))
}

fn read_response(reader: &mut impl BufRead, head_only: bool) -> Result<Response, ClientError> {
    let mut raw = Vec::new();
    loop {
        let left = HEAD_LIMIT.saturating_sub(raw.len());
        if left == 0 {
            return Err(ClientError::Protocol("response head too large".into()));
        }
        let line_start = raw.len();
        if read_line_bounded(reader, &mut raw, left)? == 0 {
            return Err(unexpected_eof("connection closed before a response"));
        }
        if &raw[line_start..] == b"\r\n" && line_start > 0 {
            break;
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
    let chunked = match response.header("transfer-encoding") {
        None => false,
        Some(coding) if coding.trim().eq_ignore_ascii_case("chunked") => true,
        Some(_) => return Err(ClientError::Protocol("unsupported transfer coding".into())),
    };
    let body = match (chunked, response.header("content-length")) {
        (true, Some(_)) => return Err(ClientError::Protocol("conflicting body framing".into())),
        (true, None) => read_chunked(reader)?,
        (false, Some(length)) => {
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
        }
        (false, None) => {
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
        }
    };
    Ok(Response { body, ..response })
}

const CHUNK_LINE_LIMIT: usize = 1024;
const TRAILER_LIMIT: usize = 8 * 1024;
const TRAILERS_MAX: usize = 32;

fn read_chunked(reader: &mut impl BufRead) -> Result<Vec<u8>, ClientError> {
    let mut body = Vec::new();
    loop {
        let mut line = Vec::new();
        if read_line_bounded(reader, &mut line, CHUNK_LINE_LIMIT)? == 0 {
            return Err(unexpected_eof("connection closed inside a chunked body"));
        }
        let line = std::str::from_utf8(&line)
            .map_err(|_| ClientError::Protocol("bad chunk size".into()))?;
        let size = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size, 16)
            .map_err(|_| ClientError::Protocol("bad chunk size".into()))?;
        if size == 0 {
            // Trailers, then the blank line.
            for _ in 0..TRAILERS_MAX {
                let mut trailer = Vec::new();
                if read_line_bounded(reader, &mut trailer, TRAILER_LIMIT)? == 0 {
                    return Err(unexpected_eof("connection closed inside the trailers"));
                }
                if trailer == b"\r\n" || trailer == b"\n" {
                    return Ok(body);
                }
            }
            return Err(ClientError::Protocol("too many trailers".into()));
        }
        if size > BODY_LIMIT - body.len() {
            return Err(ClientError::Protocol("response body too large".into()));
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader
            .read_exact(&mut body[start..])
            .map_err(ClientError::Io)?;
        let mut crlf = [0; 2];
        reader.read_exact(&mut crlf).map_err(ClientError::Io)?;
        if &crlf != b"\r\n" {
            return Err(ClientError::Protocol("chunk not followed by CRLF".into()));
        }
    }
}

/// Idle proved connections kept for reuse.
const POOL: usize = 4;

/// The dashboard client. Every connection it uses was proved when it was
/// opened; a few are kept alive. At most [`MAX_ACTIVE`] requests run at
/// once, and reads may take all but one slot, so a decision is never held
/// behind slow reads (a Fleet host that does not answer takes seconds).
pub struct Client {
    dashboard: Dashboard,
    idle: Mutex<Vec<Connection>>,
    active: Mutex<usize>,
    freed: Condvar,
}

struct Slot<'a>(&'a Client);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        let mut active = self
            .0
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *active -= 1;
        self.0.freed.notify_all();
    }
}

impl Client {
    #[must_use]
    pub fn new(dashboard: Dashboard) -> Self {
        Self {
            dashboard,
            idle: Mutex::new(Vec::new()),
            active: Mutex::new(0),
            freed: Condvar::new(),
        }
    }

    fn idle(&self) -> std::sync::MutexGuard<'_, Vec<Connection>> {
        self.idle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn slot(&self, mutation: bool) -> Result<Slot<'_>, ClientError> {
        let limit = if mutation { MAX_ACTIVE } else { MAX_ACTIVE - 1 };
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let deadline = Instant::now() + SLOT_WAIT;
        while *active >= limit {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(ClientError::Protocol("too many requests in flight".into()));
            }
            active = self
                .freed
                .wait_timeout(active, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        *active += 1;
        Ok(Slot(self))
    }

    /// Send one request with the token, on a proved connection. A GET on a
    /// kept connection the server closed while it was idle is sent again
    /// once, on a newly proved one. A POST is never sent twice: if its
    /// answer is lost, the caller reads the state back instead.
    pub fn request(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<Response, ClientError> {
        let _slot = self.slot(crate::dispatch::is_write(method))?;
        let kept = self.idle().pop();
        let reused = kept.is_some();
        let mut connection = match kept {
            Some(connection) => connection,
            None => Connection::open(&self.dashboard)?,
        };
        let token = Some(self.dashboard.token.as_str());
        let addr = self.dashboard.addr;
        let result = match connection.exchange(
            addr,
            method,
            path,
            headers,
            body,
            token,
            EXCHANGE_DEADLINE,
        ) {
            Err(ClientError::Io(_)) if reused && method == "GET" => {
                connection = Connection::open(&self.dashboard)?;
                connection.exchange(addr, method, path, headers, body, token, EXCHANGE_DEADLINE)
            }
            other => other,
        };
        if let Ok(response) = &result {
            let closing = response
                .header("connection")
                .is_some_and(|v| v.eq_ignore_ascii_case("close"));
            let mut idle = self.idle();
            if !closing && idle.len() < POOL {
                idle.push(connection);
            }
        }
        result
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
    pub(crate) const NODE: &str = "node-1";

    pub(crate) struct FakeServer {
        pub addr: SocketAddr,
        pub seen: Arc<StdMutex<Vec<String>>>,
    }

    pub(crate) fn fake_server(
        proof_token: Option<&'static str>,
        close_after: Option<usize>,
    ) -> FakeServer {
        fake_server_for(proof_token, NODE, close_after)
    }

    /// A server whose proofs are for `node` (a relay to another tend).
    pub(crate) fn fake_server_for(
        proof_token: Option<&'static str>,
        node: &'static str,
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
                let proof = proof_token.map(|token| (token, node, addr.to_string()));
                thread::spawn(move || serve(stream, proof, close_after, &log));
            }
        });
        FakeServer { addr, seen }
    }

    fn serve(
        stream: TcpStream,
        proof: Option<(&str, &str, String)>,
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
                    let proof = proof.as_ref().map_or_else(
                        || "0".repeat(64),
                        |(token, node, bind)| expected_proof(token, node, bind, nonce),
                    );
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
            node: NODE.into(),
        }
    }

    #[test]
    fn a_proof_relayed_from_another_tend_with_the_same_token_is_refused() {
        let relay = fake_server_for(Some("shared-token"), "another-node", None);
        let client = Client::new(dashboard(relay.addr, "shared-token"));
        let error = client
            .request("GET", "/api/decisions", &[], &[])
            .unwrap_err();
        assert!(matches!(error, ClientError::NotTendd(_)), "{error}");
        assert!(!relay.seen.lock().unwrap().join("").contains("shared-token"));
    }

    /// A server that answers the proof and one request per connection,
    /// says nothing about closing, and closes: the client's kept connection
    /// is dead by the time it is reused.
    fn silently_closing_server(token: &'static str) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let log = seen.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let log = log.clone();
                let bind = addr.to_string();
                thread::spawn(move || {
                    let mut reader = BufReader::new(stream);
                    for _ in 0..2 {
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
                        log.lock().unwrap().push(head.clone());
                        let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                        let payload = match path.strip_prefix("/api/proof?nonce=") {
                            Some(nonce) => format!(
                                "{{\"proof\":\"{}\"}}",
                                expected_proof(token, NODE, &bind, nonce)
                            ),
                            None => "{}".to_owned(),
                        };
                        let _ = write!(
                            reader.get_mut(),
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{payload}",
                            payload.len()
                        );
                    }
                });
            }
        });
        FakeServer { addr, seen }
    }

    #[test]
    fn a_stale_get_is_retried_on_a_new_proved_connection() {
        let server = silently_closing_server("k");
        let client = Client::new(dashboard(server.addr, "k"));
        client.request("GET", "/a", &[], &[]).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        client.request("GET", "/b", &[], &[]).unwrap();
        let seen = server.seen.lock().unwrap().clone();
        let paths: Vec<&str> = seen
            .iter()
            .map(|h| h.split_whitespace().nth(1).unwrap())
            .collect();
        assert_eq!(paths.len(), 4, "{paths:?}");
        assert!(paths[2].starts_with("/api/proof"), "{paths:?}");
        assert_eq!(paths[3], "/b");
    }

    #[test]
    fn a_post_is_never_sent_twice() {
        let server = silently_closing_server("k");
        let client = Client::new(dashboard(server.addr, "k"));
        client.request("GET", "/a", &[], &[]).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let result = client.request("POST", "/api/decisions/requests/r1", &[], b"{}");
        assert!(
            result.is_err(),
            "the answer was lost: the caller reads back"
        );
        let posts = server
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|h| h.starts_with("POST"))
            .count();
        assert!(posts <= 1, "sent at most once");
        // The dead connection was not kept: the next request proves a new one.
        client.request("GET", "/c", &[], &[]).unwrap();
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

    fn parse(raw: &[u8]) -> Result<Response, ClientError> {
        read_response(&mut BufReader::new(raw), false)
    }

    #[test]
    fn an_unproved_peer_cannot_grow_buffers_past_their_limits() {
        // A head with no newline at all.
        let endless = vec![b'a'; HEAD_LIMIT * 4];
        assert!(matches!(parse(&endless), Err(ClientError::Protocol(_))));
        // A chunk-size line without end.
        let mut chunk = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        chunk.extend(vec![b'1'; CHUNK_LINE_LIMIT * 4]);
        assert!(matches!(parse(&chunk), Err(ClientError::Protocol(_))));
    }

    #[test]
    fn a_chunk_size_that_would_overflow_is_an_error_not_a_panic() {
        let raw =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\nffffffffffffffff\r\n";
        assert!(matches!(parse(raw), Err(ClientError::Protocol(_))));
    }

    #[test]
    fn malformed_framing_is_refused() {
        for raw in [
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 3\r\n\r\n"[..],
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nxYZ0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\nTrailer: x\r\n",
        ] {
            assert!(parse(raw).is_err(), "{}", String::from_utf8_lossy(raw));
        }
        let ok = parse(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n0\r\nX: y\r\n\r\n",
        )
        .unwrap();
        assert_eq!(ok.body, b"hi");
    }

    #[test]
    fn a_trickling_peer_is_cut_off_at_the_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 1024];
            let _ = stream.read(&mut buf);
            for _ in 0..100 {
                if stream.write_all(b"H").is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(200));
            }
        });
        let started = Instant::now();
        let error = Client::new(dashboard(addr, "k"))
            .request("GET", "/x", &[], &[])
            .unwrap_err();
        assert!(matches!(error, ClientError::NotTendd(_)), "{error}");
        assert!(
            started.elapsed() < PROOF_DEADLINE + Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }
}
