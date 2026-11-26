//! # XMPP over WebSocket (RFC 7395)
//!
//! Async WebSocket connection handler using smol runtime.
//!
//! ## Example
//!
//! ```rust,no_run
//! use libstrophe_vendored::xmpp::websocket::{connect_websocket, WsEvent};
//!
//! smol::block_on(async {
//!     connect_websocket(
//!         "wss://example.com/xmpp-websocket",
//!         "user@example.com",
//!         "password",
//!         |event| !matches!(event, WsEvent::Disconnect)
//!     ).await
//! });
//! ```

#![forbid(unsafe_code)]

use super::error::{Error, Result};
use async_tungstenite::{
    tungstenite::{client::IntoClientRequest, handshake::client::Request, protocol::Message},
    WebSocketStream,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures_util::StreamExt;
use smol::net::TcpStream;

/// WebSocket connection event
#[derive(Debug, Clone)]
pub enum WsEvent {
    /// Connected and stream opened
    Connect,
    /// Received a stanza
    Stanza(String),
    /// Disconnected
    Disconnect,
    /// Error occurred
    Error(String),
}

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

/// Authentication state during XMPP connection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthState {
    /// Initial state, waiting for stream features
    AwaitingFeatures,
    /// SASL authentication in progress
    SaslInProgress,
    /// SASL succeeded, need stream restart
    SaslSuccess,
    /// Stream restarted, awaiting post-auth features
    AwaitingBindFeatures,
    /// Resource binding in progress
    BindingResource,
    /// Fully authenticated and bound
    Bound,
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
/// use libstrophe_vendored::xmpp::websocket::{connect_websocket, WsEvent};
///
/// smol::block_on(async {
///     connect_websocket("wss://example.com/ws", "user@example.com", "pass", |event| {
///         match event {
///             WsEvent::Connect => { println!("Connected!"); true }
///             WsEvent::Stanza(xml) => { println!("Received: {}", xml); true }
///             WsEvent::Disconnect => false,
///             _ => true
///         }
///     }).await
/// });
/// ```
pub async fn connect_websocket<F>(
    url: &str,
    jid: &str,
    password: &str,
    mut handler: F,
) -> Result<()>
where
    F: FnMut(WsEvent) -> bool,
{
    // Parse URL and extract domain for <open/> frame
    let domain = extract_domain_from_jid(jid)?;

    // Create WebSocket request with XMPP sub-protocol
    let mut request: Request = url
        .into_client_request()
        .map_err(|_e| Error::InvalidOp)?; // ponytail: reuse existing error type

    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", "xmpp".parse().unwrap());

    // Connect TCP stream
    let tcp_stream = connect_tcp(url).await?;

    // Perform WebSocket handshake
    let (ws_stream, _response) =
        async_tungstenite::client_async(request, tcp_stream)
            .await
            .map_err(|_| Error::InvalidOp)?; // ponytail: map to existing error type

    // Run WebSocket event loop
    run_websocket_loop(ws_stream, domain, jid, password, handler).await
}

/// Extract domain from JID (user@domain/resource -> domain)
fn extract_domain_from_jid(jid: &str) -> Result<String> {
    jid.split('@')
        .nth(1)
        .and_then(|s| s.split('/').next())
        .map(|s| s.to_string())
        .ok_or(Error::InvalidOp)
}

/// Connect TCP stream from WebSocket URL
async fn connect_tcp(url: &str) -> Result<TcpStream> {
    // Parse URL to extract host and port
    // ponytail: simple parsing, good enough for ws:// and wss://
    let url = url.trim_start_matches("ws://").trim_start_matches("wss://");
    let (host, port) = if let Some(pos) = url.find('/') {
        let host_port = &url[..pos];
        parse_host_port(host_port)?
    } else {
        parse_host_port(url)?
    };

    let addr = format!("{}:{}", host, port);
    TcpStream::connect(addr)
        .await
        .map_err(|_| Error::InvalidOp)
}

/// Parse host:port from URL component
fn parse_host_port(s: &str) -> Result<(String, u16)> {
    if let Some(pos) = s.rfind(':') {
        let host = s[..pos].to_string();
        let port = s[pos + 1..]
            .parse::<u16>()
            .map_err(|_| Error::InvalidOp)?;
        Ok((host, port))
    } else {
        // Default port for ws:// is 80, wss:// is 443
        // ponytail: assume ws:// (80) for now
        Ok((s.to_string(), 80))
    }
}

/// Main WebSocket event loop with authentication
async fn run_websocket_loop<F>(
    mut ws_stream: WebSocketStream<TcpStream>,
    domain: String,
    jid: &str,
    password: &str,
    mut handler: F,
) -> Result<()>
where
    F: FnMut(WsEvent) -> bool,
{
    let mut auth_state = AuthState::AwaitingFeatures;
    let mut bound_jid: Option<String> = None;

    // Send initial <open/> frame
    let open_frame = create_open_frame(&domain);
    ws_stream
        .send(Message::Text(open_frame))
        .await
        .map_err(|_| Error::InvalidOp)?;

    // Message loop
    while let Some(msg) = ws_stream.next().await {
        let msg = msg.map_err(|_| Error::InvalidOp)?;

        match msg {
            Message::Text(text) => {
                // Parse frame
                match parse_frame(&text) {
                    Ok(FrameType::Open { .. }) => {
                        // Server acknowledged stream opening
                        // Wait for features
                    }
                    Ok(FrameType::Close) => {
                        // Server closing stream
                        handler(WsEvent::Disconnect);
                        break;
                    }
                    Ok(FrameType::Stanza(xml)) => {
                        // Handle authentication flow
                        if let Some(next_state) = handle_auth_stanza(
                            &mut ws_stream,
                            &xml,
                            auth_state,
                            jid,
                            password,
                            &domain,
                            &mut bound_jid,
                        )
                        .await?
                        {
                            auth_state = next_state;

                            // Fire Connect event when fully authenticated
                            if auth_state == AuthState::Bound {
                                if !handler(WsEvent::Connect) {
                                    // Handler requested disconnect
                                    let close_frame =
                                        r#"<close xmlns="urn:ietf:params:xml:ns:xmpp-framing"/>"#;
                                    ws_stream.send(Message::Text(close_frame.into())).await.ok();
                                    ws_stream.close(None).await.ok();
                                    break;
                                }
                            }
                        } else {
                            // Regular stanza (after authentication complete)
                            if auth_state == AuthState::Bound {
                                if !handler(WsEvent::Stanza(xml)) {
                                    // Handler requested disconnect
                                    let close_frame =
                                        r#"<close xmlns="urn:ietf:params:xml:ns:xmpp-framing"/>"#;
                                    ws_stream.send(Message::Text(close_frame.into())).await.ok();
                                    ws_stream.close(None).await.ok();
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        handler(WsEvent::Error(format!("Parse error: {:?}", e)));
                    }
                }
            }
            Message::Ping(payload) => {
                // Respond to ping
                ws_stream
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|_| Error::InvalidOp)?;
            }
            Message::Close(_) => {
                // WebSocket close frame
                handler(WsEvent::Disconnect);
                break;
            }
            _ => {
                // Ignore binary frames, pong, etc.
            }
        }
    }

    handler(WsEvent::Disconnect);
    Ok(())
}

/// Handle authentication-related stanzas, return new state if changed
async fn handle_auth_stanza(
    ws_stream: &mut WebSocketStream<TcpStream>,
    stanza: &str,
    current_state: AuthState,
    jid: &str,
    password: &str,
    domain: &str,
    bound_jid: &mut Option<String>,
) -> Result<Option<AuthState>> {
    match current_state {
        AuthState::AwaitingFeatures => {
            // Check for SASL mechanisms in features
            if stanza.contains("<mechanisms") && supports_sasl_plain(stanza) {
                // Send SASL PLAIN auth
                let username = extract_username(jid).ok_or(Error::InvalidOp)?;
                let auth_stanza = create_sasl_plain_auth(&username, password);
                ws_stream
                    .send(Message::Text(auth_stanza))
                    .await
                    .map_err(|_| Error::InvalidOp)?;
                return Ok(Some(AuthState::SaslInProgress));
            }
            Ok(None)
        }

        AuthState::SaslInProgress => {
            // Check for SASL success
            if stanza.contains("<success") {
                // Send stream restart (new <open/>)
                let open_frame = create_open_frame(domain);
                ws_stream
                    .send(Message::Text(open_frame))
                    .await
                    .map_err(|_| Error::InvalidOp)?;
                return Ok(Some(AuthState::SaslSuccess));
            }

            // Check for SASL failure
            if stanza.contains("<failure") {
                return Err(Error::InvalidOp); // Auth failed
            }

            Ok(None)
        }

        AuthState::SaslSuccess => {
            // After stream restart, state changes to AwaitingBindFeatures
            // when we receive the new <open/> from server
            if stanza.contains("<open ") {
                return Ok(Some(AuthState::AwaitingBindFeatures));
            }
            Ok(None)
        }

        AuthState::AwaitingBindFeatures => {
            // Check for bind feature
            if stanza.contains("<features") && supports_bind(stanza) {
                // Send bind request
                let bind_request = create_bind_request(Some("websocket-client"));
                ws_stream
                    .send(Message::Text(bind_request))
                    .await
                    .map_err(|_| Error::InvalidOp)?;
                return Ok(Some(AuthState::BindingResource));
            }
            Ok(None)
        }

        AuthState::BindingResource => {
            // Check for bind response
            if stanza.contains(r#"id='bind-1'"#) || stanza.contains(r#"id="bind-1""#) {
                if let Some(jid_str) = parse_bound_jid(stanza) {
                    *bound_jid = Some(jid_str);
                    return Ok(Some(AuthState::Bound));
                }
            }
            Ok(None)
        }

        AuthState::Bound => {
            // Fully authenticated, let regular stanza handler process
            Ok(None)
        }
    }
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
        to
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
        // Simple parsing: extract 'to' and 'version' attributes
        // ponytail: simple string matching, upgrade to xml parser if needed
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

// ponytail: minimal attribute extractor, good enough for <open/> parsing
fn extract_attribute(xml: &str, attr: &str) -> Option<String> {
    let pattern = format!(r#"{}=""#, attr);
    let start = xml.find(&pattern)? + pattern.len();
    let end = xml[start..].find('"')? + start;
    Some(xml[start..end].to_string())
}

/// Create SASL PLAIN authentication stanza
///
/// Format: base64(\0username\0password)
/// WARNING: Only use over TLS (wss://), not plain ws://
fn create_sasl_plain_auth(username: &str, password: &str) -> String {
    // SASL PLAIN format: \0username\0password
    let auth_string = format!("\0{}\0{}", username, password);
    let encoded = BASE64.encode(auth_string.as_bytes());

    format!(
        r#"<auth xmlns='urn:ietf:params:xml:ns:xmpp-sasl' mechanism='PLAIN'>{}</auth>"#,
        encoded
    )
}

/// Extract username from JID (user@domain/resource -> user)
fn extract_username(jid: &str) -> Option<String> {
    jid.split('@').next().map(|s| s.to_string())
}

/// Check if stanza contains SASL PLAIN mechanism
fn supports_sasl_plain(features_stanza: &str) -> bool {
    // ponytail: simple string check, good enough for SASL mechanism detection
    features_stanza.contains("<mechanism>PLAIN</mechanism>")
        || features_stanza.contains("<mechanism>PLAIN</mechanism")
}

/// Check if features contain resource binding
fn supports_bind(features_stanza: &str) -> bool {
    features_stanza.contains("xmlns='urn:ietf:params:xml:ns:xmpp-bind'")
        || features_stanza.contains(r#"xmlns="urn:ietf:params:xml:ns:xmpp-bind""#)
}

/// Create resource binding IQ stanza
fn create_bind_request(resource: Option<&str>) -> String {
    let resource_xml = resource
        .map(|r| format!("<resource>{}</resource>", r))
        .unwrap_or_default();

    format!(
        r#"<iq type='set' id='bind-1'><bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'>{}</bind></iq>"#,
        resource_xml
    )
}

/// Parse bound JID from bind response
fn parse_bound_jid(iq_response: &str) -> Option<String> {
    // ponytail: simple extraction, works for standard bind response
    let start = iq_response.find("<jid>")?;
    let end = iq_response.find("</jid>")?;
    Some(iq_response[start + 5..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // Authentication tests

    #[test]
    fn test_sasl_plain_encoding() {
        let auth = create_sasl_plain_auth("juliet", "r0m30myr0m30");

        // Should contain base64-encoded \0juliet\0r0m30myr0m30
        assert!(auth.contains("<auth"));
        assert!(auth.contains("xmlns='urn:ietf:params:xml:ns:xmpp-sasl'"));
        assert!(auth.contains("mechanism='PLAIN'"));

        // Decode and verify
        let start = auth.find(">").unwrap() + 1;
        let end = auth.find("</auth>").unwrap();
        let encoded = &auth[start..end];
        let decoded = BASE64.decode(encoded.as_bytes()).unwrap();
        let expected = b"\0juliet\0r0m30myr0m30";
        assert_eq!(decoded, expected);
    }

    #[test]
    fn test_extract_username() {
        assert_eq!(extract_username("user@example.com"), Some("user".to_string()));
        assert_eq!(extract_username("user@example.com/resource"), Some("user".to_string()));
        assert_eq!(extract_username("user"), Some("user".to_string()));
    }

    #[test]
    fn test_extract_domain_from_jid() {
        assert_eq!(
            extract_domain_from_jid("user@example.com").unwrap(),
            "example.com"
        );
        assert_eq!(
            extract_domain_from_jid("user@example.com/resource").unwrap(),
            "example.com"
        );
        assert!(extract_domain_from_jid("invalid").is_err());
    }

    #[test]
    fn test_supports_sasl_plain() {
        let features = r#"
            <stream:features>
                <mechanisms xmlns='urn:ietf:params:xml:ns:xmpp-sasl'>
                    <mechanism>PLAIN</mechanism>
                    <mechanism>SCRAM-SHA-1</mechanism>
                </mechanisms>
            </stream:features>
        "#;
        assert!(supports_sasl_plain(features));

        let no_plain = r#"
            <stream:features>
                <mechanisms xmlns='urn:ietf:params:xml:ns:xmpp-sasl'>
                    <mechanism>SCRAM-SHA-1</mechanism>
                </mechanisms>
            </stream:features>
        "#;
        assert!(!supports_sasl_plain(no_plain));
    }

    #[test]
    fn test_supports_bind() {
        let features = r#"
            <stream:features>
                <bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'/>
                <session xmlns='urn:ietf:params:xml:ns:xmpp-session'/>
            </stream:features>
        "#;
        assert!(supports_bind(features));

        let no_bind = r#"
            <stream:features>
                <session xmlns='urn:ietf:params:xml:ns:xmpp-session'/>
            </stream:features>
        "#;
        assert!(!supports_bind(no_bind));
    }

    #[test]
    fn test_create_bind_request() {
        let req_with_resource = create_bind_request(Some("mobile"));
        assert!(req_with_resource.contains(r#"type='set'"#));
        assert!(req_with_resource.contains(r#"id='bind-1'"#));
        assert!(req_with_resource.contains("<resource>mobile</resource>"));
        assert!(req_with_resource.contains("xmlns='urn:ietf:params:xml:ns:xmpp-bind'"));

        let req_no_resource = create_bind_request(None);
        assert!(!req_no_resource.contains("<resource>"));
    }

    #[test]
    fn test_parse_bound_jid() {
        let iq = r#"
            <iq type='result' id='bind-1'>
                <bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'>
                    <jid>user@example.com/websocket-client</jid>
                </bind>
            </iq>
        "#;
        assert_eq!(
            parse_bound_jid(iq),
            Some("user@example.com/websocket-client".to_string())
        );

        let invalid = "<iq type='result' id='bind-1'><bind/></iq>";
        assert_eq!(parse_bound_jid(invalid), None);
    }
}
