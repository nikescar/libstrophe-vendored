//! WebSocket-to-libstrophe bridge: integrate async WebSocket with sync libstrophe event loop.
//!
//! This example demonstrates how to run WebSocket connection in a separate thread
//! and communicate with libstrophe's synchronous event loop via channels.
//!
//! Architecture:
//! - WebSocket thread: runs `smol` async runtime, handles WebSocket I/O
//! - Main thread: runs libstrophe event loop for other connections or processing
//! - Communication: `async-channel` for passing events between threads
#![forbid(unsafe_code)]
#![cfg(feature = "websocket")]

mod common;
use async_channel::{bounded, Receiver, Sender};
use common::XmppConfig;
use libstrophe_vendored::xmpp::{websocket::connect_websocket, XmppEvent};
use std::thread;
use std::time::Duration;

/// Event sent from WebSocket thread to main thread
#[derive(Debug, Clone)]
enum BridgeEvent {
    WsConnected,
    WsStanza(String),
    WsDisconnected,
    WsError(String),
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    // Parse CLI args
    let (cli_url, cli_jid, cli_pass) = match args.len() {
        4 => (Some(args[1].as_str()), Some(args[2].as_str()), Some(args[3].as_str())),
        1 => (None, None, None),
        _ => {
            eprintln!(
                "Usage: websocket_bridge [<url> <jid> <password>]\n\n\
                 Environment variables (priority over CLI):\n  \
                 XMPP_WS_URL   WebSocket URL\n  \
                 XMPP_JID      Jabber ID\n  \
                 XMPP_PASS     Password\n\n\
                 Example:\n  \
                 websocket_bridge wss://xmpp.example.com/websocket user@example.com secret\n"
            );
            std::process::exit(1);
        }
    };

    // Get configuration (ENV > CLI)
    let config = XmppConfig::from_env_and_args(cli_jid, cli_pass, None, None, cli_url);
    if !config.is_valid() {
        eprintln!("Error: JID and password required");
        std::process::exit(1);
    }
    if config.ws_url.is_none() {
        eprintln!("Error: WebSocket URL required");
        std::process::exit(1);
    }

    let url = config.ws_url.unwrap();
    let jid = config.jid;
    let password = config.password;

    // Create channel for WebSocket -> Main communication
    let (ws_tx, ws_rx) = bounded::<BridgeEvent>(100);

    // Spawn WebSocket thread
    let ws_handle = thread::spawn(move || {
        eprintln!("[WS Thread] Starting WebSocket connection");

        smol::block_on(async {
            let tx = ws_tx.clone();

            let result = connect_websocket(&url, &jid, &password, move |event| {
                let bridge_event = match event {
                    XmppEvent::Connect => BridgeEvent::WsConnected,
                    XmppEvent::Stanza(xml) => BridgeEvent::WsStanza(xml),
                    XmppEvent::Disconnect => BridgeEvent::WsDisconnected,
                    XmppEvent::Error(msg) => BridgeEvent::WsError(msg),
                };

                // Send to main thread
                if tx.send_blocking(bridge_event).is_err() {
                    eprintln!("[WS Thread] Main thread closed channel, disconnecting");
                    return false; // Stop WebSocket
                }

                // Continue receiving (main thread controls lifecycle)
                true
            })
            .await;

            match result {
                Ok(_) => eprintln!("[WS Thread] WebSocket connection closed normally"),
                Err(e) => eprintln!("[WS Thread] WebSocket error: {:?}", e),
            }
        });

        eprintln!("[WS Thread] Exiting");
    });

    // Main thread event loop
    eprintln!("[Main Thread] Starting event loop");

    let mut running = true;
    let mut stanza_count = 0;

    while running {
        // Check for WebSocket events (non-blocking)
        match ws_rx.try_recv() {
            Ok(BridgeEvent::WsConnected) => {
                eprintln!("[Main Thread] WebSocket connected!");
            }
            Ok(BridgeEvent::WsStanza(xml)) => {
                stanza_count += 1;
                eprintln!("[Main Thread] Received stanza #{}: {}", stanza_count, xml);

                // Example: Stop after receiving 5 stanzas
                if stanza_count >= 5 {
                    eprintln!("[Main Thread] Received 5 stanzas, stopping...");
                    running = false;
                }
            }
            Ok(BridgeEvent::WsDisconnected) => {
                eprintln!("[Main Thread] WebSocket disconnected");
                running = false;
            }
            Ok(BridgeEvent::WsError(msg)) => {
                eprintln!("[Main Thread] WebSocket error: {}", msg);
                running = false;
            }
            Err(async_channel::TryRecvError::Empty) => {
                // No events, continue main loop work
            }
            Err(async_channel::TryRecvError::Closed) => {
                eprintln!("[Main Thread] WebSocket thread closed channel");
                running = false;
            }
        }

        // Simulate other work in main thread
        // In real app, this would be:
        // - ctx.run_once(100) for libstrophe event loop
        // - Other application logic
        // - UI updates, etc.
        thread::sleep(Duration::from_millis(100));
    }

    eprintln!("[Main Thread] Waiting for WebSocket thread to finish...");
    ws_handle.join().unwrap();

    eprintln!("[Main Thread] Done");
    Ok(())
}
