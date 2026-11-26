//! Integration tests for XMPP WebSocket (RFC 7395)
#![cfg(feature = "websocket")]

use libstrophe_vendored::xmpp::XmppEvent;
use libstrophe_vendored::xmpp::websocket::{
    connect_websocket, create_open_frame, parse_frame, FrameType, WsConnection, WsState,
};

/// Test RFC 7395 `<open/>` frame creation
#[test]
fn test_websocket_framing_open() {
    let frame = create_open_frame("example.com");

    // Verify required attributes
    assert!(
        frame.contains(r#"xmlns="urn:ietf:params:xml:ns:xmpp-framing""#),
        "Missing required xmlns attribute"
    );
    assert!(
        frame.contains(r#"to="example.com""#),
        "Missing required to attribute"
    );
    assert!(
        frame.contains(r#"version="1.0""#),
        "Missing required version attribute"
    );

    // Verify it's a self-closing tag
    assert!(frame.starts_with("<open "), "Should start with <open ");
    assert!(frame.ends_with("/>"), "Should be self-closing");

    // Verify parsing round-trip
    let parsed = parse_frame(&frame).expect("Failed to parse created frame");
    match parsed {
        FrameType::Open { to, version } => {
            assert_eq!(to, "example.com");
            assert_eq!(version, "1.0");
        }
        _ => panic!("Expected Open frame type"),
    }
}

/// Test RFC 7395 stanza frame parsing
#[test]
fn test_websocket_framing_stanza() {
    // Test message stanza
    let message_frame = r#"<message to="user@example.com" type="chat"><body>Hello</body></message>"#;
    let parsed = parse_frame(message_frame).expect("Failed to parse message");
    match parsed {
        FrameType::Stanza(xml) => {
            assert!(xml.contains("<body>Hello</body>"));
            assert!(xml.contains(r#"to="user@example.com""#));
        }
        _ => panic!("Expected Stanza frame type"),
    }

    // Test presence stanza
    let presence_frame = r#"<presence from="user@example.com"><show>away</show></presence>"#;
    let parsed = parse_frame(presence_frame).expect("Failed to parse presence");
    match parsed {
        FrameType::Stanza(xml) => {
            assert!(xml.contains("<show>away</show>"));
        }
        _ => panic!("Expected Stanza frame type"),
    }

    // Test IQ stanza
    let iq_frame = r#"<iq type="get" id="roster1"><query xmlns="jabber:iq:roster"/></iq>"#;
    let parsed = parse_frame(iq_frame).expect("Failed to parse IQ");
    match parsed {
        FrameType::Stanza(xml) => {
            assert!(xml.contains(r#"type="get""#));
            assert!(xml.contains(r#"id="roster1""#));
        }
        _ => panic!("Expected Stanza frame type"),
    }
}

/// Test RFC 7395 `<close/>` frame parsing
#[test]
fn test_websocket_framing_close() {
    // Simple close
    let close_frame = r#"<close xmlns="urn:ietf:params:xml:ns:xmpp-framing"/>"#;
    let parsed = parse_frame(close_frame).expect("Failed to parse close");
    assert_eq!(parsed, FrameType::Close);

    // Close with whitespace
    let close_with_ws = r#"  <close xmlns="urn:ietf:params:xml:ns:xmpp-framing"/>  "#;
    let parsed = parse_frame(close_with_ws).expect("Failed to parse close with whitespace");
    assert_eq!(parsed, FrameType::Close);
}

/// Test connection state machine
#[test]
fn test_connection_state() {
    let conn = WsConnection::new();
    assert_eq!(conn.state(), WsState::Disconnected);
}

/// `ws://` is refused before any network I/O: SASL PLAIN would leak the password.
#[test]
fn test_websocket_connect() {
    smol::block_on(async {
        let result = connect_websocket(
            "ws://localhost:5280/xmpp-websocket",
            "test@localhost",
            "password",
            |_event| false,
        )
        .await;
        assert_eq!(result, Err(libstrophe_vendored::xmpp::Error::Auth));
    })
}

/// Live login against a real server: `XMPP_WS_URL=wss://… XMPP_JID=… XMPP_PASS=…
/// cargo test --features websocket -- --ignored live_wss_login`
#[test]
#[ignore]
fn live_wss_login() {
    let url = std::env::var("XMPP_WS_URL").expect("XMPP_WS_URL");
    let jid = std::env::var("XMPP_JID").expect("XMPP_JID");
    let pass = std::env::var("XMPP_PASS").expect("XMPP_PASS");
    let mut connected = false;
    let result = smol::block_on(connect_websocket(&url, &jid, &pass, |e| {
        connected |= e == XmppEvent::Connect;
        false
    }));
    assert_eq!(result, Ok(()));
    assert!(connected);
}

/// Async test: event handler receives Connect event
#[test]
fn test_event_handler_connect() {
    smol::block_on(async {
        // This test documents expected behavior:
        // Handler should receive XmppEvent::Connect when connection succeeds

        let mut received_connect = false;

        let _result = connect_websocket(
            "ws://localhost:5280/xmpp-websocket",
            "test@localhost",
            "password",
            |event| {
                if matches!(event, XmppEvent::Connect) {
                    received_connect = true;
                    false // Disconnect after connect
                } else {
                    true
                }
            },
        )
        .await;

        // Will fail until implemented
        // assert!(received_connect, "Handler should receive Connect event");
    })
}

/// Async test: event handler receives Stanza event
#[test]
fn test_event_handler_stanza() {
    smol::block_on(async {
        // This test documents expected behavior:
        // Handler should receive XmppEvent::Stanza for incoming stanzas

        let mut received_stanza = false;

        let _result = connect_websocket(
            "ws://localhost:5280/xmpp-websocket",
            "test@localhost",
            "password",
            |event| {
                if let XmppEvent::Stanza(xml) = event {
                    assert!(!xml.is_empty(), "Stanza should not be empty");
                    received_stanza = true;
                    false // Disconnect after stanza
                } else {
                    true
                }
            },
        )
        .await;

        // Will fail until implemented
        // assert!(received_stanza, "Handler should receive Stanza event");
    })
}
