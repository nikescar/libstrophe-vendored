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
//! **ONLY use with TLS** (`wss://`), never plain `ws://`.
#![forbid(unsafe_code)]
#![cfg(feature = "websocket")]

use libstrophe_vendored::xmpp::websocket::{connect_websocket, WsEvent};
use std::process::exit;

fn usage(exit_code: i32) -> ! {
    eprint!(
        "Usage: websocket_basic [options] <url>\n\n\
         Options:\n  \
         --jid <jid>              The JID to use to authenticate.\n  \
         --pass <pass>            The password of the JID.\n\n\
         Arguments:\n  \
         <url>                    WebSocket URL (e.g., ws://localhost:5280/xmpp-websocket)\n\n\
         Examples:\n  \
         websocket_basic --jid user@example.com --pass secret ws://example.com/ws\n  \
         websocket_basic --jid user@example.com --pass secret wss://example.com:5281/xmpp-websocket\n"
    );
    exit(exit_code)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (mut jid, mut password, mut url) = (None, None, None);

    // Parse command-line arguments
    let mut i = 1;
    while i < args.len() {
        let has_value = i + 1 < args.len();
        match args[i].as_str() {
            "--help" => usage(0),
            "--jid" if has_value => {
                i += 1;
                jid = Some(args[i].clone());
            }
            "--pass" if has_value => {
                i += 1;
                password = Some(args[i].clone());
            }
            arg if !arg.starts_with("--") => {
                url = Some(arg.to_string());
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

    // Validate required arguments
    if jid.is_none() || password.is_none() || url.is_none() {
        eprintln!("Error: Missing required arguments\n");
        usage(1);
    }

    let jid = jid.unwrap();
    let password = password.unwrap();
    let url = url.unwrap();

    eprintln!("DEBUG: Connecting to {} as {}", url, jid);

    // Run async WebSocket connection with SASL authentication
    // Connect event fires AFTER full authentication (SASL + bind)
    smol::block_on(async {
        connect_websocket(&url, &jid, &password, |event| {
            match event {
                WsEvent::Connect => {
                    eprintln!("DEBUG: connected and authenticated");
                    eprintln!("DEBUG: (SASL PLAIN auth + resource binding complete)");
                    false // Disconnect immediately (like basic.rs)
                }
                WsEvent::Stanza(xml) => {
                    eprintln!("DEBUG: received stanza: {}", xml);
                    true // Continue receiving
                }
                WsEvent::Disconnect => {
                    eprintln!("DEBUG: disconnected");
                    false // Stop
                }
                WsEvent::Error(msg) => {
                    eprintln!("ERROR: {}", msg);
                    false // Disconnect on error
                }
            }
        })
        .await
    })?;

    Ok(())
}
