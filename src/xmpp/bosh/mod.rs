//! # XMPP over BOSH (XEP-0124 transport, XEP-0206 binding)
//!
//! Async client using the smol runtime and a hand-written HTTP/1.1 client.
//! `https://` runs a verified TLS handshake; SASL PLAIN is refused on plain
//! `http://`, so the password never leaves in cleartext.
//!
//! The client keeps one long-poll request open so the connection manager can
//! push stanzas, sends queued stanzas on a second request (`requests="2"`),
//! processes responses in `rid` order, and resends a request with the same
//! `rid` after a network error (XEP-0124 §14).
//!
//! ## Example
//!
//! ```rust,no_run
//! use libstrophe_vendored::xmpp::{bosh::connect_bosh, XmppEvent};
//!
//! let (tx, rx) = async_channel::unbounded::<String>();
//! smol::block_on(async {
//!     connect_bosh("https://example.com/http-bind", "user@example.com", "password", rx, |event| {
//!         if event == XmppEvent::Connect {
//!             tx.try_send("<presence/>".into()).ok();
//!         }
//!         !matches!(event, XmppEvent::Disconnect)
//!     })
//!     .await
//! });
//! ```

#![forbid(unsafe_code)]

mod body;
mod http;

use self::http::HttpConn;
use super::auth::{extract_domain_from_jid, Auth, Step};
use super::error::{Error, Result};
use super::event::XmppEvent;
use super::net::{self, Endpoint};
use async_channel::Receiver;
use futures_util::stream::{FuturesUnordered, StreamExt};
use std::collections::BTreeMap;
use std::future::Future;
use std::hash::{BuildHasher, Hasher};
use std::pin::Pin;
use std::time::Duration;

/// Requested long-poll duration (`wait`), in seconds.
const WAIT_SECS: u32 = 60;
/// Extra time beyond `wait` before a request counts as lost.
const WAIT_MARGIN: Duration = Duration::from_secs(10);
/// Resends of one `rid` after network errors before giving up.
const MAX_RETRIES: u32 = 3;
/// Upper bound on concurrent requests, whatever the server advertises.
const MAX_REQUESTS: usize = 2;

/// Connect to an XMPP server through a BOSH connection manager.
///
/// Performs session creation, SASL PLAIN, stream restart and resource binding,
/// then fires [`XmppEvent::Connect`]. Stanzas received on `outbound` are sent to
/// the server; a stanza without a namespace gets `xmlns='jabber:client'`.
/// The session ends with `type='terminate'` and unavailable presence when the
/// handler returns `false` or every `outbound` sender is dropped.
///
/// Returns [`Error::Auth`] for an `http://` URL, because PLAIN would expose the password.
pub async fn connect_bosh<F>(
    url: &str,
    jid: &str,
    password: &str,
    outbound: Receiver<String>,
    handler: F,
) -> Result<()>
where
    F: FnMut(XmppEvent) -> bool,
{
    connect_bosh_with(url, jid, password, outbound, handler, Options::default()).await
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Options {
    /// Lets crate tests run against a local `http://` mock server.
    pub allow_plaintext: bool,
    pub wait: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self { allow_plaintext: false, wait: WAIT_SECS }
    }
}

pub(crate) async fn connect_bosh_with<F>(
    url: &str,
    jid: &str,
    password: &str,
    outbound: Receiver<String>,
    mut handler: F,
    opts: Options,
) -> Result<()>
where
    F: FnMut(XmppEvent) -> bool,
{
    if !url.get(..4).is_some_and(|s| s.eq_ignore_ascii_case("http")) {
        return Err(Error::Protocol);
    }
    let ep = net::parse_url(url)?;
    if !ep.tls && !opts.allow_plaintext {
        return Err(Error::Auth);
    }
    let mut session = Session {
        ep,
        domain: extract_domain_from_jid(jid)?,
        auth: Auth::new(jid, password, "bosh-client")?,
        wait: opts.wait,
        sid: None,
        rid: initial_rid(),
        next_process: 0,
        requests: 1,
        idle: Vec::new(),
        queue: Vec::new(),
        ready: BTreeMap::new(),
        restart_pending: false,
        close_pending: false,
        terminate_rid: None,
        detail: None,
    };
    let result = session.run(&outbound, &mut handler).await;
    if let Err(e) = result {
        handler(XmppEvent::Error(session.detail.take().unwrap_or_else(|| e.to_string())));
    }
    handler(XmppEvent::Disconnect);
    result
}

/// Random start below 2^48, leaving room for increments within 2^53 (XEP-0124 §14).
fn initial_rid() -> u64 {
    let r = std::collections::hash_map::RandomState::new().build_hasher().finish();
    (r & ((1 << 48) - 1)) + 1
}

/// A request whose response is pending; owns its connection while in flight.
struct Flight {
    conn: HttpConn,
    rid: u64,
    body: String,
    attempt: u32,
    result: Result<(u16, String)>,
}

type FlightFut = Pin<Box<dyn Future<Output = Flight>>>;

fn fly(mut conn: HttpConn, rid: u64, body: String, attempt: u32, limit: Duration) -> FlightFut {
    Box::pin(async move {
        let result = conn.post(&body, limit).await;
        Flight { conn, rid, body, attempt, result }
    })
}

enum Wake {
    Flight(Option<Flight>),
    Outbound(std::result::Result<String, async_channel::RecvError>),
}

struct Session {
    ep: Endpoint,
    domain: String,
    auth: Auth,
    /// Server's `wait` once known.
    wait: u32,
    sid: Option<String>,
    /// Next `rid` to assign.
    rid: u64,
    /// Next `rid` whose response is handed to `handle`.
    next_process: u64,
    /// Concurrent requests allowed (server `requests`, capped).
    requests: usize,
    idle: Vec<HttpConn>,
    /// Stanzas waiting for a free request slot.
    queue: Vec<String>,
    /// Responses that arrived ahead of a lower `rid`.
    ready: BTreeMap<u64, String>,
    restart_pending: bool,
    close_pending: bool,
    terminate_rid: Option<u64>,
    /// Human-readable cause for the final `XmppEvent::Error`.
    detail: Option<String>,
}

impl Session {
    fn limit(&self) -> Duration {
        Duration::from_secs(self.wait.into()) + WAIT_MARGIN
    }

    fn send(&mut self, inflight: &mut FuturesUnordered<FlightFut>, body: impl FnOnce(u64, &str) -> String) {
        let rid = self.rid;
        self.rid += 1;
        let body = body(rid, self.sid.as_deref().unwrap_or_default());
        let conn = self.idle.pop().unwrap_or_else(|| HttpConn::new(self.ep.clone()));
        inflight.push(fly(conn, rid, body, 0, self.limit()));
    }

    async fn run<F>(&mut self, outbound: &Receiver<String>, handler: &mut F) -> Result<()>
    where
        F: FnMut(XmppEvent) -> bool,
    {
        let mut inflight = FuturesUnordered::new();
        self.next_process = self.rid;
        let (domain, wait) = (self.domain.clone(), self.wait);
        self.send(&mut inflight, |rid, _| body::session_create(rid, &domain, wait, 1));
        let mut outbound_open = true;

        loop {
            if self.sid.is_some() && self.terminate_rid.is_none() {
                self.fill(&mut inflight, outbound);
            }
            let listen = outbound_open && self.auth.is_bound() && !self.close_pending && self.terminate_rid.is_none();
            let wake = if listen {
                smol::future::or(
                    async { Wake::Flight(inflight.next().await) },
                    async { Wake::Outbound(outbound.recv().await) },
                )
                .await
            } else {
                Wake::Flight(inflight.next().await)
            };
            let flight = match wake {
                Wake::Outbound(Ok(stanza)) => {
                    self.queue.push(ensure_client_ns(stanza));
                    continue;
                }
                Wake::Outbound(Err(_)) => {
                    outbound_open = false;
                    self.close_pending = true;
                    continue;
                }
                Wake::Flight(None) => return Ok(()), // nothing in flight: nothing left to do
                Wake::Flight(Some(f)) => f,
            };

            let closing = self.terminate_rid.is_some();
            match flight.result {
                _ if closing && Some(flight.rid) == self.terminate_rid => return Ok(()),
                _ if closing => {} // drop leftover polls while terminating
                Err(Error::Io) if flight.attempt < MAX_RETRIES => {
                    let limit = self.limit();
                    inflight.push(fly(flight.conn, flight.rid, flight.body, flight.attempt + 1, limit));
                }
                Err(e) => {
                    self.detail = Some(format!("request rid={} failed: {e}", flight.rid));
                    return Err(e);
                }
                Ok((status, text)) => {
                    self.idle.push(flight.conn);
                    if status != 200 {
                        self.detail = Some(format!("HTTP status {status}"));
                        return Err(Error::Protocol);
                    }
                    self.ready.insert(flight.rid, text);
                    while let Some(text) = self.ready.remove(&self.next_process) {
                        self.next_process += 1;
                        if !self.handle(&text, handler)? {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    /// Start requests for pending work, keeping one long-poll open.
    fn fill(&mut self, inflight: &mut FuturesUnordered<FlightFut>, outbound: &Receiver<String>) {
        if self.close_pending {
            if inflight.len() < self.requests {
                while let Ok(s) = outbound.try_recv() {
                    self.queue.push(ensure_client_ns(s));
                }
                let stanzas = std::mem::take(&mut self.queue);
                self.terminate_rid = Some(self.rid);
                self.send(inflight, |rid, sid| body::terminate(rid, sid, &stanzas));
            }
            return;
        }
        if self.restart_pending && inflight.len() < self.requests {
            self.restart_pending = false;
            let domain = self.domain.clone();
            self.send(inflight, |rid, sid| body::restart(rid, sid, &domain));
        }
        if !self.queue.is_empty() && inflight.len() < self.requests {
            let stanzas = std::mem::take(&mut self.queue);
            self.send(inflight, |rid, sid| body::payload(rid, sid, &stanzas));
        }
        if inflight.is_empty() {
            self.send(inflight, |rid, sid| body::payload(rid, sid, &[]));
        }
    }

    /// Process one response in `rid` order; `Ok(false)` ends the session cleanly.
    fn handle<F>(&mut self, text: &str, handler: &mut F) -> Result<bool>
    where
        F: FnMut(XmppEvent) -> bool,
    {
        let b = body::parse_body(text)?;
        if b.is_terminate() {
            return match b.attr("condition") {
                Some(c) => {
                    self.detail = Some(format!("session terminated by server: {c}"));
                    Err(Error::Protocol)
                }
                None => Ok(false),
            };
        }
        if self.sid.is_none() {
            self.sid = Some(b.attr("sid").ok_or(Error::Protocol)?.to_owned());
            self.requests = b
                .attr("requests")
                .and_then(|r| r.parse().ok())
                .unwrap_or(MAX_REQUESTS)
                .clamp(1, MAX_REQUESTS);
            if let Some(w) = b.attr("wait").and_then(|w| w.parse().ok()) {
                self.wait = w;
            }
        }
        for child in b.children {
            if child.starts_with("<stream:error") {
                self.detail = Some(format!("stream error: {child}"));
                return Err(Error::Protocol);
            }
            if !self.auth.is_bound() {
                match self.auth.on_stanza(&child) {
                    Err(e) => {
                        self.detail = Some(format!("{e}: {child}"));
                        return Err(e);
                    }
                    Ok(Step::Wait) => {}
                    Ok(Step::Send(s)) => self.queue.push(s),
                    Ok(Step::Restart) => self.restart_pending = true,
                    Ok(Step::Bound(_)) => self.close_pending |= !handler(XmppEvent::Connect),
                }
            } else if !self.close_pending {
                self.close_pending = !handler(XmppEvent::Stanza(child));
            }
        }
        Ok(true)
    }
}

/// Add `xmlns='jabber:client'` to a top-level stanza that lacks a namespace:
/// inside `<body xmlns='…httpbind'>` it would otherwise be in the wrong namespace.
fn ensure_client_ns(stanza: String) -> String {
    let t = stanza.trim_start();
    let Some(n) = t
        .strip_prefix('<')
        .and_then(|rest| rest.find(|c: char| c.is_whitespace() || c == '/' || c == '>'))
        .map(|i| i + 1)
    else {
        return stanza;
    };
    let first_tag = &t[..t.find('>').unwrap_or(t.len())];
    if !matches!(&t[1..n], "message" | "presence" | "iq") || first_tag.contains("xmlns=") {
        return stanza;
    }
    format!("{} xmlns='jabber:client'{}", &t[..n], &t[n..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    const FEATURES_PLAIN: &str = "<stream:features><mechanisms xmlns='urn:ietf:params:xml:ns:xmpp-sasl'><mechanism>PLAIN</mechanism></mechanisms></stream:features>";
    const FEATURES_BIND: &str = "<stream:features><bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'/></stream:features>";
    const BIND_RESULT: &str = "<iq type='result' id='bind-1' xmlns='jabber:client'><bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'><jid>u@example.com/bosh-client</jid></bind></iq>";
    const HELLO: &str = "<message from='a@example.com' xmlns='jabber:client'><body>hi</body></message>";

    /// Reply decided per request body; `None` drops the connection without answering.
    type Script = dyn Fn(&str) -> Option<String> + Send + Sync;

    fn wrap(children: &str) -> String {
        format!("<body xmlns='http://jabber.org/protocol/httpbind' xmlns:stream='http://etherx.jabber.org/streams'>{children}</body>")
    }

    /// Happy-path connection manager. Idle polls are answered empty after 50 ms.
    fn happy(body: &str) -> Option<String> {
        Some(if !body.contains("sid=") {
            "<body xmlns='http://jabber.org/protocol/httpbind' sid='s1' wait='5' requests='2' hold='1' xmlns:stream='http://etherx.jabber.org/streams'>".to_owned()
                + FEATURES_PLAIN
                + "</body>"
        } else if body.contains("<auth") {
            wrap("<success xmlns='urn:ietf:params:xml:ns:xmpp-sasl'/>")
        } else if body.contains("xmpp:restart='true'") {
            wrap(FEATURES_BIND)
        } else if body.contains("id='bind-1'") {
            wrap(&format!("{BIND_RESULT}{HELLO}"))
        } else if body.contains("type='terminate'") {
            "<body type='terminate' xmlns='http://jabber.org/protocol/httpbind'/>".into()
        } else {
            std::thread::sleep(Duration::from_millis(50));
            wrap("")
        })
    }

    /// Mock BOSH connection manager on 127.0.0.1; returns its port and the log of request bodies.
    fn mock_server(script: Arc<Script>) -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let (script, log) = (script.clone(), log2.clone());
                std::thread::spawn(move || serve(stream.unwrap(), &*script, &log));
            }
        });
        (port, log)
    }

    fn serve(stream: TcpStream, script: &Script, log: &Mutex<Vec<String>>) {
        let mut r = BufReader::new(stream.try_clone().unwrap());
        let mut w = stream;
        loop {
            let mut len = 0;
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap_or(0) == 0 {
                    return; // client closed
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            let body = String::from_utf8(body).unwrap();
            log.lock().unwrap().push(body.clone());
            let Some(reply) = script(&body) else { return };
            // Chunked for bind replies, to exercise both framings end to end.
            let resp = if body.contains("id='bind-1'") {
                format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{reply}\r\n0\r\n\r\n", reply.len())
            } else {
                format!("HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\n\r\n{reply}", reply.len())
            };
            if w.write_all(resp.as_bytes()).is_err() {
                return;
            }
        }
    }

    fn rid_of(body: &str) -> u64 {
        let s = body.find("rid='").unwrap() + 5;
        body[s..s + body[s..].find('\'').unwrap()].parse().unwrap()
    }

    fn run(port: u16, rx: Receiver<String>, mut f: impl FnMut(&XmppEvent) -> bool) -> (Result<()>, Vec<XmppEvent>) {
        let mut events = Vec::new();
        let url = format!("http://127.0.0.1:{port}/http-bind");
        let opts = Options { allow_plaintext: true, wait: 5 };
        let r = smol::block_on(connect_bosh_with(&url, "u@example.com", "pw", rx, |e| {
            let more = f(&e);
            events.push(e);
            more
        }, opts));
        (r, events)
    }

    #[test]
    fn full_session_with_outbound_and_terminate() {
        let (port, log) = mock_server(Arc::new(happy));
        let (tx, rx) = async_channel::unbounded();
        let (r, events) = run(port, rx, |e| {
            if *e == XmppEvent::Connect {
                tx.try_send("<message to='b@example.com'><body>out</body></message>".into()).unwrap();
            }
            !matches!(e, XmppEvent::Stanza(_))
        });
        assert_eq!(r, Ok(()));
        assert_eq!(events, vec![XmppEvent::Connect, XmppEvent::Stanza(HELLO.into()), XmppEvent::Disconnect]);

        let log = log.lock().unwrap().clone();
        let mut rids: Vec<u64> = log.iter().map(|b| rid_of(b)).collect();
        rids.sort();
        assert!(rids.windows(2).all(|w| w[1] == w[0] + 1), "rids must be consecutive: {rids:?}");
        assert!(log[0].contains("xmpp:version='1.0'") && log[0].contains("to='example.com'"));
        assert!(log.iter().any(|b| b.contains("xmpp:restart='true'") && b.contains("sid='s1'")));
        assert!(log.iter().any(|b| b.contains("<iq type='set' id='bind-1' xmlns='jabber:client'>")));
        let last = log.iter().find(|b| b.contains("type='terminate'")).expect("terminate sent");
        assert!(last.contains("<presence type='unavailable' xmlns='jabber:client'/>"));
        assert!(
            log.iter().any(|b| b.contains("<message xmlns='jabber:client' to='b@example.com'><body>out</body></message>")),
            "outbound stanza sent with jabber:client namespace: {log:#?}"
        );
    }

    #[test]
    fn closing_outbound_channel_terminates() {
        let (port, log) = mock_server(Arc::new(happy));
        let (tx, rx) = async_channel::unbounded::<String>();
        let mut tx = Some(tx);
        let (r, events) = run(port, rx, |e| {
            if *e == XmppEvent::Connect {
                tx.take(); // drop the only sender
            }
            true
        });
        assert_eq!(r, Ok(()));
        assert_eq!(events.first(), Some(&XmppEvent::Connect));
        assert_eq!(events.last(), Some(&XmppEvent::Disconnect));
        assert!(log.lock().unwrap().iter().any(|b| b.contains("type='terminate'")));
    }

    #[test]
    fn sasl_failure_is_auth_error() {
        let (port, _) = mock_server(Arc::new(|b: &str| {
            if b.contains("<auth") {
                Some(wrap("<failure xmlns='urn:ietf:params:xml:ns:xmpp-sasl'><not-authorized/></failure>"))
            } else {
                happy(b)
            }
        }));
        let (_tx, rx) = async_channel::unbounded();
        let (r, events) = run(port, rx, |_| true);
        assert_eq!(r, Err(Error::Auth));
        assert!(matches!(&events[0], XmppEvent::Error(m) if m.contains("not-authorized")));
        assert_eq!(events[1], XmppEvent::Disconnect);
    }

    #[test]
    fn server_terminate_with_condition_is_reported() {
        let (port, _) = mock_server(Arc::new(|b: &str| {
            if b.contains("<auth") {
                Some("<body type='terminate' condition='item-not-found' xmlns='http://jabber.org/protocol/httpbind'/>".into())
            } else {
                happy(b)
            }
        }));
        let (_tx, rx) = async_channel::unbounded();
        let (r, events) = run(port, rx, |_| true);
        assert_eq!(r, Err(Error::Protocol));
        assert!(matches!(&events[0], XmppEvent::Error(m) if m.contains("item-not-found")));
    }

    #[test]
    fn dropped_request_is_resent_with_same_rid() {
        let dropped = Arc::new(Mutex::new(false));
        let d = dropped.clone();
        let (port, log) = mock_server(Arc::new(move |b: &str| {
            let mut d = d.lock().unwrap();
            if b.contains("<auth") && !*d {
                *d = true;
                return None; // lose the first auth request
            }
            happy(b)
        }));
        let (_tx, rx) = async_channel::unbounded();
        let (r, events) = run(port, rx, |e| !matches!(e, XmppEvent::Stanza(_)));
        assert_eq!(r, Ok(()));
        assert_eq!(events[0], XmppEvent::Connect);
        let log = log.lock().unwrap();
        let auths: Vec<&String> = log.iter().filter(|b| b.contains("<auth")).collect();
        assert_eq!(auths.len(), 2);
        assert_eq!(auths[0], auths[1], "retry must resend the identical body and rid");
    }

    /// First request runs on a fresh connection, so only the BOSH-level retry can recover it.
    #[test]
    fn dropped_session_create_is_resent_by_bosh_retry() {
        let drops = Arc::new(Mutex::new(0));
        let d = drops.clone();
        let (port, log) = mock_server(Arc::new(move |b: &str| {
            let mut d = d.lock().unwrap();
            if !b.contains("sid=") && *d < 2 {
                *d += 1;
                return None;
            }
            happy(b)
        }));
        let (_tx, rx) = async_channel::unbounded();
        let (r, events) = run(port, rx, |e| !matches!(e, XmppEvent::Stanza(_)));
        assert_eq!(r, Ok(()));
        assert_eq!(events[0], XmppEvent::Connect);
        let log = log.lock().unwrap();
        let creates: Vec<u64> = log.iter().filter(|b| !b.contains("sid=")).map(|b| rid_of(b)).collect();
        assert_eq!(creates.len(), 3);
        assert!(creates.iter().all(|r| *r == creates[0]));
    }

    #[test]
    fn request_lost_beyond_retry_budget_is_io_error() {
        let (port, _) = mock_server(Arc::new(|b: &str| if b.contains("sid=") { happy(b) } else { None }));
        let (_tx, rx) = async_channel::unbounded();
        let (r, events) = run(port, rx, |_| true);
        assert_eq!(r, Err(Error::Io));
        assert!(matches!(&events[0], XmppEvent::Error(m) if m.contains("rid=")));
    }

    #[test]
    fn http_error_status_is_protocol_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            while r.read_line(&mut line).unwrap() > 2 {
                line.clear();
            }
            (&s).write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").unwrap();
            let _ = r.read(&mut [0u8; 1]); // keep the socket open until the client is done
        });
        let (_tx, rx) = async_channel::unbounded();
        let (r, events) = run(port, rx, |_| true);
        assert_eq!(r, Err(Error::Protocol));
        assert!(matches!(&events[0], XmppEvent::Error(m) if m.contains("404")));
    }

    #[test]
    fn plain_http_is_refused_before_connecting() {
        let (_tx, rx) = async_channel::unbounded();
        let r = smol::block_on(connect_bosh("http://127.0.0.1:1/http-bind", "u@example.com", "pw", rx.clone(), |_| true));
        assert_eq!(r, Err(Error::Auth));
        let r = smol::block_on(connect_bosh("wss://example.com/ws", "u@example.com", "pw", rx, |_| true));
        assert_eq!(r, Err(Error::Protocol));
    }

    #[test]
    fn initial_rid_leaves_headroom() {
        for _ in 0..100 {
            let r = initial_rid();
            assert!(r > 0 && r < 1 << 48);
        }
    }

    #[test]
    fn client_namespace_is_added_only_when_missing() {
        assert_eq!(ensure_client_ns("<presence/>".into()), "<presence xmlns='jabber:client'/>");
        assert_eq!(ensure_client_ns("<iq type='get'>x</iq>".into()), "<iq xmlns='jabber:client' type='get'>x</iq>");
        let keep = "<message xmlns='jabber:client'/>";
        assert_eq!(ensure_client_ns(keep.into()), keep);
        let other = "<r xmlns='urn:xmpp:sm:3'/>";
        assert_eq!(ensure_client_ns(other.into()), other);
        assert_eq!(ensure_client_ns(String::new()), "");
        assert_eq!(ensure_client_ns("<".into()), "<");
    }
}
