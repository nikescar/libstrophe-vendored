//! WebSocket port of `basic.rs`: connect via WebSocket with SASL authentication, then disconnect.
//!
//! This example demonstrates:
//! - RFC 7395 XMPP-over-WebSocket connection
//! - SASL PLAIN authentication (requires TLS/wss://)
//! - Stream restart after authentication
//! - Resource binding
//!
//! ## Authentication Flow
//!
//! 1. Send `<open/>` frame
//! 2. Receive stream features with SASL mechanisms
//! 3. Send SASL PLAIN auth (base64-encoded credentials)
//! 4. Receive `<success/>` response
//! 5. Stream restart: send new `<open/>`
//! 6. Receive post-auth features with `<bind/>`
//! 7. Send resource binding IQ
//! 8. Receive bound JID
//! 9. Fire `Connect` event (now fully authenticated)
//!
//! ## Security Note
//!
//! SASL PLAIN sends password as base64 (encoding, not encryption).
//! `wss://` runs a verified TLS handshake (rustls, Mozilla roots); `connect_websocket`
//! refuses plain `ws://` URLs with `Error::Auth`.
#![forbid(unsafe_code)]
#![cfg(feature = "websocket")]

mod common;
use common::XmppConfig;
use libstrophe_vendored::xmpp::{websocket::connect_websocket, XmppEvent};
use std::process::exit;

fn usage(exit_code: i32) -> ! {
    eprint!(
        "Usage: websocket_basic [options] [<url>]\n\n\
         Options:\n  \
         --jid <jid>              The JID to use to authenticate.\n  \
         --pass <pass>            The password of the JID.\n\n\
         Arguments:\n  \
         <url>                    WebSocket URL (e.g., wss://xmpp.example.com/websocket)\n\n\
         Environment variables (priority over CLI):\n  \
         XMPP_JID                 Jabber ID\n  \
         XMPP_PASS                Password\n  \
         XMPP_WS_URL              WebSocket URL\n\n\
         Examples:\n  \
         websocket_basic --jid user@example.com --pass secret wss://example.com/ws\n  \
         XMPP_JID=user@example.com XMPP_PASS=secret XMPP_WS_URL=wss://example.com/ws websocket_basic\n"
    );
    exit(exit_code)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (mut cli_jid, mut cli_pass, mut cli_url) = (None, None, None);

    // Parse command-line arguments
    let mut i = 1;
    while i < args.len() {
        let has_value = i + 1 < args.len();
        match args[i].as_str() {
            "--help" => usage(0),
            "--jid" if has_value => {
                i += 1;
                cli_jid = Some(args[i].as_str());
            }
            "--pass" if has_value => {
                i += 1;
                cli_pass = Some(args[i].as_str());
            }
            arg if !arg.starts_with("--") => {
                cli_url = Some(arg);
                i += 1;
                break; // URL is last argument
            }
            _ => {
                eprintln!("Unknown option: {}", args[i]);
                usage(1);
            }
        }
        i += 1;
    }

    // Get configuration (ENV > CLI)
    let config = XmppConfig::from_env_and_args(cli_jid, cli_pass, None, None, cli_url);
    if !config.is_valid() {
        eprintln!("Error: JID and password required\n");
        usage(1);
    }
    if config.ws_url.is_none() {
        eprintln!("Error: WebSocket URL required\n");
        usage(1);
    }

    let jid = config.jid;
    let password = config.password;
    let url = config.ws_url.unwrap();

    eprintln!("DEBUG: Connecting to {} as {}", url, jid);

    // Run async WebSocket connection with SASL authentication
    // Connect event fires AFTER full authentication (SASL + bind)
    smol::block_on(async {
        connect_websocket(&url, &jid, &password, |event| {
            match event {
                XmppEvent::Connect => {
                    eprintln!("DEBUG: connected and authenticated");
                    eprintln!("DEBUG: (SASL PLAIN auth + resource binding complete)");
                    false // Disconnect immediately (like basic.rs)
                }
                XmppEvent::Stanza(xml) => {
                    eprintln!("DEBUG: received stanza: {}", xml);
                    true // Continue receiving
                }
                XmppEvent::Disconnect => {
                    eprintln!("DEBUG: disconnected");
                    false // Stop
                }
                XmppEvent::Error(msg) => {
                    eprintln!("ERROR: {}", msg);
                    false // Disconnect on error
                }
            }
        })
        .await
    })?;

    Ok(())
}
