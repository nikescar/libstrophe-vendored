# libstrophe-vendored Examples

## FFI Connection Example

Demonstrates connecting to an XMPP server using raw FFI with rustls TLS backend.

### Usage

```bash
# Using default test server (jabber.org)
cargo run --example ffi_connection --features tls-rustls

# With custom credentials
XMPP_JID="user@example.com" \
XMPP_PASS="password" \
cargo run --example ffi_connection --features tls-rustls

# With custom server
XMPP_JID="user@example.com" \
XMPP_PASS="password" \
XMPP_HOST="xmpp.example.com" \
cargo run --example ffi_connection --features tls-rustls
```

### Environment Variables

- `XMPP_JID` - XMPP Jabber ID (default: test@jabber.org)
- `XMPP_PASS` - Password (default: password)
- `XMPP_HOST` - Optional custom server hostname

### What It Demonstrates

1. **Library initialization**: `xmpp_initialize()`
2. **Context creation**: `xmpp_ctx_new()`
3. **Connection setup**: `xmpp_conn_new()`, JID/password
4. **TLS configuration**: Mandatory TLS flag
5. **Connection**: `xmpp_connect_client()` with callback
6. **Event loop**: `xmpp_run_once()` polling
7. **TLS certificate info**: XMPP addresses from SANs
8. **Cleanup**: Proper resource release order

### Expected Output

```
libstrophe-vendored basic connection example
============================================

Connecting as: test@jabber.org

Initiating connection...
Running event loop (Ctrl+C to stop)...

Tick 0s
Connected!
Tick 5s
Tick 10s
...
```

### Notes

- Requires a valid XMPP account to actually connect
- Example runs for 30 seconds then exits
- TLS is mandatory (uses rustls + ring backend)
- Certificate validation uses webpki-roots (Mozilla CA bundle)

## WebSocket Examples (RFC 7395)

### websocket_basic

Demonstrates async WebSocket connection to XMPP server using `smol` runtime.

#### Usage

```bash
# Connect to WebSocket XMPP server
cargo run --features websocket --example websocket_basic -- \
  --jid user@example.com \
  --pass password \
  ws://example.com:5280/xmpp-websocket

# With TLS (wss://)
cargo run --features websocket --example websocket_basic -- \
  --jid user@example.com \
  --pass secret \
  wss://example.com:5281/xmpp-websocket
```

#### What It Demonstrates

1. **RFC 7395 compliance** - `<open/>` / `<close/>` framing
2. **Sub-protocol negotiation** - `Sec-WebSocket-Protocol: xmpp`
3. **Async WebSocket** - Using `smol::block_on` and `async-tungstenite`
4. **Event handling** - Connect, Stanza, Disconnect, Error events
5. **TLS support** - `wss://` URLs with `futures-rustls`

#### Expected Output

```
DEBUG: Connecting to ws://localhost:5280/xmpp-websocket as user@localhost
DEBUG: connected
DEBUG: disconnected
```

### websocket_bridge

Demonstrates integrating async WebSocket with libstrophe's synchronous event loop using threads and channels.

#### Usage

```bash
cargo run --features websocket --example websocket_bridge -- \
  ws://localhost:5280/xmpp-websocket \
  user@localhost \
  password
```

#### What It Demonstrates

1. **Thread architecture** - WebSocket in async thread, main thread for other work
2. **Channel communication** - `async-channel` for event passing
3. **Event loop integration** - How to combine WebSocket with `ctx.run_once()`
4. **Lifecycle management** - Coordinating shutdown between threads

#### Expected Output

```
[Main Thread] Starting event loop
[WS Thread] Starting WebSocket connection
[Main Thread] WebSocket connected!
[Main Thread] Received stanza #1: <message>...</message>
[Main Thread] Received stanza #2: <presence>...</presence>
...
[Main Thread] Received 5 stanzas, stopping...
[WS Thread] Main thread closed channel, disconnecting
[WS Thread] Exiting
[Main Thread] Done
```

#### Architecture

```
┌─────────────────────────────┐
│  WebSocket Thread (async)   │
│                             │
│  smol::block_on(async {     │
│    connect_websocket()      │
│  })                         │
└──────────┬──────────────────┘
           │ async-channel
           │ (BridgeEvent)
           ▼
┌─────────────────────────────┐
│   Main Thread (sync)        │
│                             │
│   loop {                    │
│     rx.try_recv()           │
│     ctx.run_once(100)       │
│   }                         │
└─────────────────────────────┘
```

## Testing Without a Server

See `tests/integration.rs` for API tests that don't require a live server.

```bash
cargo test --test integration
```

For WebSocket tests:

```bash
cargo test --features websocket --test websocket
```
