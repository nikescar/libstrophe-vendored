//! # XMPP over WebSocket (RFC 7395)
//!
//! Async WebSocket connection handler using smol runtime.
//!
//! `wss://` runs a verified TLS handshake (rustls + Mozilla roots). SASL PLAIN
//! is refused on plain `ws://`, so the password never leaves in cleartext.
//!
//! ## Example
//!
//! ```rust,no_run
//! use libstrophe_vendored::xmpp::{websocket::connect_websocket, XmppEvent};
//!
//! smol::block_on(async {
//!     connect_websocket(
//!         "wss://example.com/xmpp-websocket",
//!         "user@example.com",
//!         "password",
//!         |event| !matches!(event, XmppEvent::Disconnect)
//!     ).await
//! });
//! ```

#![forbid(unsafe_code)]

use super::auth::{extract_domain_from_jid, Auth, Step};
use super::error::{Error, Result};
use super::event::XmppEvent;
use super::net::{self, Transport};
use async_tungstenite::{
    tungstenite::{client::IntoClientRequest, protocol::Message},
    WebSocketStream,
};
use futures_util::StreamExt;

const CLOSE_FRAME: &str = r#"<close xmlns="urn:ietf:params:xml:ns:xmpp-framing"/>"#;

/// WebSocket connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsState {
    /// Not connected
    Disconnected,
    /// WebSocket handshake in progress
    Connecting,
    /// Sent `<open/>`, waiting for response
    StreamOpening,
    /// Stream negotiated, ready to send/receive stanzas
    StreamReady,
    /// Sent `<close/>`, waiting for server close
    StreamClosing,
    /// Connection closed
    Closed,
}

/// Frame type parsed from WebSocket text frame
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameType {
    /// `<open/>` stream opening
    Open { to: String, version: String },
    /// `<close/>` stream closing
    Close,
    /// Regular XMPP stanza
    Stanza(String),
}

/// WebSocket connection handle
pub struct WsConnection {
    state: WsState,
}

impl WsConnection {
    /// Create a new WebSocket connection in disconnected state
    pub fn new() -> Self {
        Self {
            state: WsState::Disconnected,
        }
    }

    /// Get current connection state
    pub fn state(&self) -> WsState {
        self.state
    }
}

impl Default for WsConnection {
    fn default() -> Self {
        Self::new()
    }
}

/// Connect to XMPP server via WebSocket (RFC 7395)
///
/// Performs SASL PLAIN, stream restart and resource binding, then fires
/// [`XmppEvent::Connect`]. Returns [`Error::Auth`] for a `ws://` URL, because
/// PLAIN would expose the password.
///
/// # Arguments
///
/// * `url` - WebSocket URL (e.g., `wss://example.com/xmpp-websocket`)
/// * `jid` - XMPP JID (e.g., `user@example.com`)
/// * `password` - Authentication password
/// * `handler` - Event handler; returns `true` to continue, `false` to disconnect
///
/// # Example
///
/// ```rust,no_run
/// use libstrophe_vendored::xmpp::{websocket::connect_websocket, XmppEvent};
///
/// smol::block_on(async {
///     connect_websocket("wss://example.com/ws", "user@example.com", "pass", |event| {
///         match event {
///             XmppEvent::Connect => { println!("Connected!"); true }
///             XmppEvent::Stanza(xml) => { println!("Received: {}", xml); true }
///             XmppEvent::Disconnect => false,
///             _ => true
///         }
///     }).await
/// });
/// ```
pub async fn connect_websocket<F>(url: &str, jid: &str, password: &str, handler: F) -> Result<()>
where
    F: FnMut(XmppEvent) -> bool,
{
    connect_websocket_with(url, jid, password, false, handler).await
}

/// `allow_plaintext` lets crate tests run against a local `ws://` mock server.
pub(crate) async fn connect_websocket_with<F>(
    url: &str,
    jid: &str,
    password: &str,
    allow_plaintext: bool,
    mut handler: F,
) -> Result<()>
where
    F: FnMut(XmppEvent) -> bool,
{
    if !url.get(..2).is_some_and(|s| s.eq_ignore_ascii_case("ws")) {
        return Err(Error::Protocol);
    }
    let ep = net::parse_url(url)?;
    if !ep.tls && !allow_plaintext {
        return Err(Error::Auth);
    }
    let domain = extract_domain_from_jid(jid)?;
    let mut auth = Auth::new(jid, password, "websocket-client")?;

    let mut request = url.into_client_request().map_err(|_| Error::Protocol)?;
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", "xmpp".parse().unwrap());

    let transport = net::connect(&ep).await?;
    let (mut ws, _response) = async_tungstenite::client_async(request, transport)
        .await
        .map_err(|_| Error::Protocol)?;

    let result = run_websocket_loop(&mut ws, &domain, &mut auth, &mut handler).await;
    if let Err(e) = result {
        handler(XmppEvent::Error(e.to_string()));
    }
    handler(XmppEvent::Disconnect);
    result
}

/// Main WebSocket event loop with authentication
async fn run_websocket_loop<F>(
    ws: &mut WebSocketStream<Transport>,
    domain: &str,
    auth: &mut Auth,
    handler: &mut F,
) -> Result<()>
where
    F: FnMut(XmppEvent) -> bool,
{
    send_text(ws, create_open_frame(domain)).await?;

    while let Some(msg) = ws.next().await {
        match msg.map_err(|_| Error::Io)? {
            Message::Text(text) => match parse_frame(&text)? {
                FrameType::Open { .. } => {}
                FrameType::Close => return Ok(()),
                FrameType::Stanza(xml) if !auth.is_bound() => match auth.on_stanza(&xml)? {
                    Step::Wait => {}
                    Step::Send(s) => send_text(ws, s).await?,
                    Step::Restart => send_text(ws, create_open_frame(domain)).await?,
                    Step::Bound(_) => {
                        if !handler(XmppEvent::Connect) {
                            return close(ws).await;
                        }
                    }
                },
                FrameType::Stanza(xml) => {
                    if !handler(XmppEvent::Stanza(xml)) {
                        return close(ws).await;
                    }
                }
            },
            Message::Ping(payload) => ws.send(Message::Pong(payload)).await.map_err(|_| Error::Io)?,
            Message::Close(_) => return Ok(()),
            _ => {} // binary, pong
        }
    }
    Ok(())
}

async fn send_text(ws: &mut WebSocketStream<Transport>, text: String) -> Result<()> {
    ws.send(Message::Text(text)).await.map_err(|_| Error::Io)
}

async fn close(ws: &mut WebSocketStream<Transport>) -> Result<()> {
    send_text(ws, CLOSE_FRAME.into()).await.ok();
    ws.close(None).await.ok();
    Ok(())
}

/// Create RFC 7395 `<open/>` frame
///
/// # Arguments
///
/// * `to` - Target domain (e.g., `example.com`)
///
/// # Returns
///
/// XML string: `<open xmlns="urn:ietf:params:xml:ns:xmpp-framing" to="domain" version="1.0"/>`
pub fn create_open_frame(to: &str) -> String {
    format!(
        r#"<open xmlns="urn:ietf:params:xml:ns:xmpp-framing" to="{}" version="1.0"/>"#,
        super::auth::escape(to)
    )
}

/// Parse incoming WebSocket text frame
///
/// # Arguments
///
/// * `data` - UTF-8 text frame content
///
/// # Returns
///
/// Parsed frame type: `Open`, `Close`, or `Stanza`
pub fn parse_frame(data: &str) -> Result<FrameType> {
    let trimmed = data.trim();

    // Check for <open/>
    if trimmed.starts_with("<open ") && trimmed.ends_with("/>") {
        // shortcut: string matching, upgrade to an XML parser if servers send odd framing
        let to = extract_attribute(trimmed, "to").unwrap_or_default();
        let version = extract_attribute(trimmed, "version").unwrap_or_default();
        return Ok(FrameType::Open { to, version });
    }

    // Check for <close/>
    if trimmed.starts_with("<close") && trimmed.ends_with("/>") {
        return Ok(FrameType::Close);
    }

    // Everything else is a stanza
    Ok(FrameType::Stanza(data.to_string()))
}

/// Attribute value in either quote style.
fn extract_attribute(xml: &str, attr: &str) -> Option<String> {
    for q in ['"', '\''] {
        let pattern = format!("{attr}={q}");
        if let Some(start) = xml.find(&pattern).map(|i| i + pattern.len()) {
            let end = xml[start..].find(q)? + start;
            return Some(xml[start..end].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    #[test]
    fn test_create_open_frame() {
        let frame = create_open_frame("example.com");
        assert!(frame.contains(r#"to="example.com""#));
        assert!(frame.contains(r#"version="1.0""#));
        assert!(frame.contains(r#"xmlns="urn:ietf:params:xml:ns:xmpp-framing""#));
    }

    #[test]
    fn test_parse_open_frame() {
        let frame = r#"<open xmlns="urn:ietf:params:xml:ns:xmpp-framing" to="example.com" version="1.0"/>"#;
        let parsed = parse_frame(frame).unwrap();
        match parsed {
            FrameType::Open { to, version } => {
                assert_eq!(to, "example.com");
                assert_eq!(version, "1.0");
            }
            _ => panic!("Expected Open frame"),
        }
        let single = "<open xmlns='urn:ietf:params:xml:ns:xmpp-framing' from='example.com' version='1.0'/>";
        assert!(matches!(parse_frame(single).unwrap(), FrameType::Open { version, .. } if version == "1.0"));
    }

    #[test]
    fn test_parse_close_frame() {
        let frame = r#"<close xmlns="urn:ietf:params:xml:ns:xmpp-framing"/>"#;
        let parsed = parse_frame(frame).unwrap();
        assert_eq!(parsed, FrameType::Close);
    }

    #[test]
    fn test_parse_stanza_frame() {
        let frame = r#"<message to="user@example.com" type="chat"><body>Hello</body></message>"#;
        let parsed = parse_frame(frame).unwrap();
        match parsed {
            FrameType::Stanza(xml) => assert!(xml.contains("<body>Hello</body>")),
            _ => panic!("Expected Stanza frame"),
        }
    }

    #[test]
    fn test_ws_connection_initial_state() {
        let conn = WsConnection::new();
        assert_eq!(conn.state(), WsState::Disconnected);
    }

    #[test]
    fn plain_ws_is_refused_before_connecting() {
        let r = smol::block_on(connect_websocket("ws://127.0.0.1:1/ws", "u@example.com", "pw", |_| true));
        assert_eq!(r, Err(Error::Auth));
        let r = smol::block_on(connect_websocket("https://example.com/ws", "u@example.com", "pw", |_| true));
        assert_eq!(r, Err(Error::Protocol));
    }

    // --- minimal RFC 6455 server for the full login flow (unmasked server frames) ---

    fn read_frame(s: &mut impl Read) -> (u8, String) {
        let mut h = [0u8; 2];
        s.read_exact(&mut h).unwrap();
        let mut len = (h[1] & 0x7f) as u64;
        if len == 126 {
            let mut b = [0u8; 2];
            s.read_exact(&mut b).unwrap();
            len = u16::from_be_bytes(b) as u64;
        } else if len == 127 {
            let mut b = [0u8; 8];
            s.read_exact(&mut b).unwrap();
            len = u64::from_be_bytes(b);
        }
        let mut mask = [0u8; 4];
        s.read_exact(&mut mask).unwrap(); // client frames are always masked
        let mut data = vec![0u8; len as usize];
        s.read_exact(&mut data).unwrap();
        for (i, b) in data.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
        (h[0] & 0x0f, String::from_utf8(data).unwrap())
    }

    fn write_text(s: &mut impl Write, text: &str) {
        let mut f = vec![0x81u8];
        if text.len() < 126 {
            f.push(text.len() as u8);
        } else {
            f.push(126);
            f.extend_from_slice(&(text.len() as u16).to_be_bytes());
        }
        f.extend_from_slice(text.as_bytes());
        s.write_all(&f).unwrap();
    }

    /// Full login over a scripted `ws://` server, incl. the stream restart that
    /// used to hang: server answers `<open/>` frames, prefixed `<stream:features>`.
    #[test]
    fn login_flow_against_mock_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut w = s;
            let mut key = String::new();
            loop {
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                if let Some(v) = line.strip_prefix("Sec-WebSocket-Key: ") {
                    key = v.trim().to_owned();
                }
                if line == "\r\n" {
                    break;
                }
            }
            let accept = async_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
            write!(w, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: xmpp\r\n\r\n").unwrap();

            let mut got = Vec::new();
            let open = "<open xmlns='urn:ietf:params:xml:ns:xmpp-framing' from='example.com' version='1.0'/>";
            got.push(read_frame(&mut r).1); // <open/>
            write_text(&mut w, open);
            write_text(&mut w, "<stream:features xmlns:stream='http://etherx.jabber.org/streams'><mechanisms xmlns='urn:ietf:params:xml:ns:xmpp-sasl'><mechanism>PLAIN</mechanism></mechanisms></stream:features>");
            got.push(read_frame(&mut r).1); // <auth/>
            write_text(&mut w, "<success xmlns='urn:ietf:params:xml:ns:xmpp-sasl'/>");
            got.push(read_frame(&mut r).1); // restart <open/>
            write_text(&mut w, open);
            write_text(&mut w, "<stream:features xmlns:stream='http://etherx.jabber.org/streams'><bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'/></stream:features>");
            got.push(read_frame(&mut r).1); // bind iq
            write_text(&mut w, "<iq type='result' id='bind-1'><bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'><jid>u@example.com/websocket-client</jid></bind></iq>");
            write_text(&mut w, "<message from='a@example.com'><body>hi</body></message>");
            got.push(read_frame(&mut r).1); // <close/>
            got
        });

        let mut events = Vec::new();
        let url = format!("ws://127.0.0.1:{port}/ws");
        let r = smol::block_on(connect_websocket_with(&url, "u@example.com", "pw", true, |e| {
            let more = !matches!(e, XmppEvent::Stanza(_));
            events.push(e);
            more
        }));
        assert_eq!(r, Ok(()));
        assert_eq!(
            events,
            vec![
                XmppEvent::Connect,
                XmppEvent::Stanza("<message from='a@example.com'><body>hi</body></message>".into()),
                XmppEvent::Disconnect
            ]
        );
        let got = server.join().unwrap();
        assert!(got[0].starts_with("<open ") && got[0].contains(r#"to="example.com""#));
        assert!(got[1].contains("mechanism='PLAIN'"));
        assert!(got[2].starts_with("<open "));
        assert!(got[3].contains("id='bind-1'"));
        assert_eq!(got[4], CLOSE_FRAME);
    }
}
